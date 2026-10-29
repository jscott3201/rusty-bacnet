//! The synchronous Windows input owner, also exercised by portable fixtures.
use crate::mstp::monitor::{Failure, MonitorAdmissionError, MonitorRead, combine};
use bacnet_types::error::Error;
use std::io::{self, Read};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};
use tokio::sync::mpsc;

const SLOTS: usize = 4;
const CHUNK: usize = 512;

pub(crate) trait Input: Read + Send + 'static {
    fn close(&mut self) -> io::Result<()>;
}

#[derive(Debug)]
pub(crate) struct Reader {
    stop: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Vec<u8>>,
    pending: Vec<u8>,
    offset: usize,
    thread: Option<std::thread::JoinHandle<io::Result<()>>>,
    joining: Option<tokio::task::JoinHandle<io::Result<()>>>,
    closed: Option<Result<(), Failure>>,
}

impl Reader {
    pub(crate) fn start<I: Input>(input: I) -> Result<Self, MonitorAdmissionError> {
        Self::start_with_stop(input, Arc::new(AtomicBool::new(false)))
    }
    fn start_with_stop<I: Input>(
        mut input: I,
        stop: Arc<AtomicBool>,
    ) -> Result<Self, MonitorAdmissionError> {
        let (sender, receiver) = mpsc::channel(SLOTS);
        let worker_stop = Arc::clone(&stop);
        // Retain input until the OS thread exists, so spawn failure can close it.
        let (admit, admission) = std::sync::mpsc::sync_channel::<I>(1);
        let thread = match std::thread::Builder::new()
            .name("mstp-receive".into())
            .spawn(move || {
                let mut input = admission
                    .recv()
                    .map_err(|_| io::Error::other("input admission cancelled"))?;
                let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    receive(&mut input, &worker_stop, &sender)
                }))
                .unwrap_or_else(|panic| Err(io::Error::other(panic_message(panic))));
                // Explicit close precedes thread completion; joining is the barrier.
                combine(read, input.close())
            }) {
            Ok(thread) => thread,
            Err(error) => {
                return Err(MonitorAdmissionError::after_close(
                    Error::Transport(error),
                    input.close(),
                ));
            }
        };
        if let Err(mut error) = admit.send(input) {
            let close = error.0.close();
            let joined = join(thread);
            return Err(MonitorAdmissionError::after_close(
                Error::Transport(io::Error::other("input admission failed")),
                combine(close, joined),
            ));
        }
        Ok(Self {
            stop,
            receiver,
            pending: Vec::new(),
            offset: 0,
            thread: Some(thread),
            joining: None,
            closed: None,
        })
    }
    fn request_stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Wakes a blocking_send even when every bounded slot is occupied.
        self.receiver.close();
    }
}

fn receive(
    input: &mut impl Input,
    stop: &AtomicBool,
    sender: &mpsc::Sender<Vec<u8>>,
) -> io::Result<()> {
    let mut bytes = [0; CHUNK];
    while !stop.load(Ordering::Acquire) {
        let read = input.read(&mut bytes);
        // Preserve a completed read error even when stop arrived concurrently.
        let count = match read {
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        if stop.load(Ordering::Acquire) || count == 0 {
            return Ok(());
        }
        if sender.blocking_send(bytes[..count].to_vec()).is_err() {
            return Ok(());
        }
    }
    Ok(())
}
fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    let message = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic");
    format!("serial input thread panicked: {message}")
}
fn join(thread: std::thread::JoinHandle<io::Result<()>>) -> io::Result<()> {
    thread
        .join()
        .unwrap_or_else(|panic| Err(io::Error::other(panic_message(panic))))
}
impl AsyncRead for Reader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if self.stop.load(Ordering::Acquire) {
            return self.poll_close(cx);
        }
        if self.offset == self.pending.len() {
            match std::task::ready!(self.receiver.poll_recv(cx)) {
                Some(chunk) => {
                    self.pending = chunk;
                    self.offset = 0;
                }
                None => return self.poll_close(cx),
            }
        }
        let count = buf.remaining().min(self.pending.len() - self.offset);
        buf.put_slice(&self.pending[self.offset..self.offset + count]);
        self.offset += count;
        Poll::Ready(Ok(()))
    }
}
impl MonitorRead for Reader {
    fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.request_stop();
        if self.closed.is_none() {
            if let Some(thread) = self.thread.take() {
                self.joining = Some(tokio::task::spawn_blocking(move || join(thread)));
            }
            let result = std::task::ready!(
                Pin::new(self.joining.as_mut().expect("retained thread join")).poll(cx)
            )
            .unwrap_or_else(|error| {
                Err(io::Error::other(format!(
                    "serial join task failed: {error}"
                )))
            });
            self.joining = None;
            self.closed = Some(result.map_err(Failure::new));
            self.pending.clear();
        }
        Poll::Ready(
            self.closed
                .as_ref()
                .expect("close result")
                .as_ref()
                .copied()
                .map_err(Failure::error),
        )
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.request_stop();
    }
}

#[cfg(test)]
mod tests;
