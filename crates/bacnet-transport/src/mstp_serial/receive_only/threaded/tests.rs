use super::*;
use crate::mstp::monitor::{MonitorConfig, MstpMonitor};
use std::collections::VecDeque;
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;

enum Outcome {
    Data(Vec<u8>),
    Timeout,
    Eof,
    Error,
    Panic,
}
struct Fixture {
    outcomes: std::sync::mpsc::Receiver<Outcome>,
    reads: mpsc::UnboundedSender<usize>,
    count: usize,
    closed: Arc<AtomicBool>,
    closing: Option<oneshot::Sender<()>>,
    release_close: Option<std::sync::mpsc::Receiver<()>>,
    fail_close: bool,
    finished: Option<oneshot::Sender<()>>,
}
impl Read for Fixture {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.count += 1;
        let _ = self.reads.send(self.count);
        match self.outcomes.recv().expect("scripted read outcome") {
            Outcome::Data(data) => {
                bytes[..data.len()].copy_from_slice(&data);
                Ok(data.len())
            }
            Outcome::Timeout => Err(io::Error::new(io::ErrorKind::TimedOut, "scripted timeout")),
            Outcome::Eof => Ok(0),
            Outcome::Error => Err(io::Error::other("scripted read failure")),
            Outcome::Panic => panic!("scripted read panic"),
        }
    }
}
impl Input for Fixture {
    fn close(&mut self) -> io::Result<()> {
        if let Some(closing) = self.closing.take() {
            let _ = closing.send(());
        }
        if let Some(release) = self.release_close.take() {
            release.recv().unwrap();
        }
        self.closed.store(true, Ordering::Release);
        let _ = self.finished.take().unwrap().send(());
        if self.fail_close {
            Err(io::Error::other("scripted close failure"))
        } else {
            Ok(())
        }
    }
}
struct Control {
    outcomes: std::sync::mpsc::Sender<Outcome>,
    reads: mpsc::UnboundedReceiver<usize>,
    closed: Arc<AtomicBool>,
    closing: oneshot::Receiver<()>,
    release_close: std::sync::mpsc::Sender<()>,
    finished: oneshot::Receiver<()>,
}
fn fixture(fail_close: bool) -> (Fixture, Control) {
    let (outcomes, input) = std::sync::mpsc::channel();
    let (reads, read_events) = mpsc::unbounded_channel();
    let (closing, close_event) = oneshot::channel();
    let (release_close, close_gate) = std::sync::mpsc::channel();
    let closed = Arc::new(AtomicBool::new(false));
    let (finished, done) = oneshot::channel();
    (
        Fixture {
            outcomes: input,
            reads,
            count: 0,
            closed: Arc::clone(&closed),
            closing: Some(closing),
            release_close: Some(close_gate),
            fail_close,
            finished: Some(finished),
        },
        Control {
            outcomes,
            reads: read_events,
            closed,
            closing: close_event,
            release_close,
            finished: done,
        },
    )
}
async fn poll_pending_close(reader: &mut Reader) {
    std::future::poll_fn(|cx| {
        assert!(
            reader.poll_close(cx).is_pending(),
            "held owner cannot report closed"
        );
        Poll::Ready(())
    })
    .await;
}
async fn close(reader: &mut Reader) -> io::Result<()> {
    std::future::poll_fn(|cx| reader.poll_close(cx)).await
}

#[tokio::test]
async fn cancelled_close_retains_read_and_join_barriers_and_stops_before_next_read() {
    let (input, mut control) = fixture(false);
    let mut reader = Reader::start(input).unwrap();
    assert_eq!(control.reads.recv().await, Some(1));
    poll_pending_close(&mut reader).await; // This close future ends while read is held.
    assert!(!control.closed.load(Ordering::Acquire));
    control.outcomes.send(Outcome::Data(vec![1, 2])).unwrap();
    control.closing.await.unwrap();
    poll_pending_close(&mut reader).await; // Thread is now held inside explicit close.
    assert!(!control.closed.load(Ordering::Acquire));
    control.release_close.send(()).unwrap();
    close(&mut reader).await.unwrap();
    assert!(control.closed.load(Ordering::Acquire));
    assert_eq!(
        control.reads.recv().await,
        None,
        "no second native read after stop"
    );
    close(&mut reader).await.unwrap();
}
#[tokio::test]
async fn closing_releases_a_full_channel_without_reading_or_dropping_buffer_ownership() {
    let (input, mut control) = fixture(false);
    let mut reader = Reader::start(input).unwrap();
    for count in 1..=SLOTS + 1 {
        assert_eq!(control.reads.recv().await, Some(count));
        if count == SLOTS + 1 {
            assert_eq!(reader.receiver.len(), SLOTS);
        }
        control
            .outcomes
            .send(Outcome::Data(vec![count as u8]))
            .unwrap();
    }
    // The worker cannot begin another read until the fifth send is accepted.
    poll_pending_close(&mut reader).await;
    control.closing.await.unwrap();
    control.release_close.send(()).unwrap();
    close(&mut reader).await.unwrap();
    assert_eq!(control.reads.recv().await, None);
}
#[tokio::test]
async fn partial_chunks_and_zero_length_reads_preserve_every_byte() {
    let (input, mut control) = fixture(false);
    let mut reader = Reader::start(input).unwrap();
    assert_eq!(control.reads.recv().await, Some(1));
    control
        .outcomes
        .send(Outcome::Data(vec![1, 2, 3, 4, 5]))
        .unwrap();
    let mut first = [0; 2];
    reader.read_exact(&mut first).await.unwrap();
    reader.read_exact(&mut []).await.unwrap();
    let mut suffix = [0; 3];
    reader.read_exact(&mut suffix).await.unwrap();
    assert_eq!(first, [1, 2]);
    assert_eq!(suffix, [3, 4, 5]);
    assert_eq!(control.reads.recv().await, Some(2));
    poll_pending_close(&mut reader).await;
    control.outcomes.send(Outcome::Timeout).unwrap();
    control.closing.await.unwrap();
    control.release_close.send(()).unwrap();
    close(&mut reader).await.unwrap();
    assert_eq!(control.reads.recv().await, None);
}
#[tokio::test]
async fn timeout_is_a_stop_check_not_eof_and_preexisting_stop_does_not_read() {
    let (input, mut control) = fixture(false);
    let mut reader = Reader::start(input).unwrap();
    assert_eq!(control.reads.recv().await, Some(1));
    control.outcomes.send(Outcome::Timeout).unwrap();
    assert_eq!(control.reads.recv().await, Some(2));
    poll_pending_close(&mut reader).await;
    control.outcomes.send(Outcome::Timeout).unwrap();
    control.closing.await.unwrap();
    control.release_close.send(()).unwrap();
    close(&mut reader).await.unwrap();
    assert_eq!(control.reads.recv().await, None);

    let (input, mut control) = fixture(false);
    let mut reader = Reader::start_with_stop(input, Arc::new(AtomicBool::new(true))).unwrap();
    control.closing.await.unwrap();
    control.release_close.send(()).unwrap();
    close(&mut reader).await.unwrap();
    assert_eq!(control.reads.recv().await, None);
}
#[tokio::test]
async fn eof_error_panic_and_close_error_are_retained_after_join() {
    for (outcome, expected, fail_close) in [
        (Outcome::Eof, None, false),
        (Outcome::Error, Some("scripted read failure"), false),
        (Outcome::Panic, Some("scripted read panic"), false),
        (Outcome::Eof, Some("scripted close failure"), true),
        (
            Outcome::Error,
            Some("scripted read failure; closing input also failed: scripted close failure"),
            true,
        ),
    ] {
        let (input, mut control) = fixture(fail_close);
        let mut reader = Reader::start(input).unwrap();
        assert_eq!(control.reads.recv().await, Some(1));
        control.outcomes.send(outcome).unwrap();
        control.closing.await.unwrap();
        poll_pending_close(&mut reader).await;
        control.release_close.send(()).unwrap();
        let result = close(&mut reader).await;
        if let Some(expected) = expected {
            assert!(result.unwrap_err().to_string().contains(expected));
            assert!(
                close(&mut reader)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains(expected)
            );
        } else {
            result.unwrap();
        }
        assert!(control.closed.load(Ordering::Acquire));
    }
}
#[tokio::test]
async fn cancelled_monitor_stop_retains_ownership_until_explicit_close_and_join() {
    let (input, mut control) = fixture(false);
    let reader = Reader::start(input).unwrap();
    let (requested, stopped_reading) = oneshot::channel();
    let reader = CloseProbe {
        reader,
        requested: Some(requested),
    };
    let mut monitor = MstpMonitor::start(reader, MonitorConfig::new(38_400))
        .await
        .unwrap();
    assert_eq!(control.reads.recv().await, Some(1));
    {
        let mut stopping = Box::pin(monitor.stop());
        std::future::poll_fn(|cx| {
            assert!(stopping.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        tokio::select! {
            result = &mut stopping => panic!("stop completed with held read: {result:?}"),
            result = stopped_reading => result.unwrap(),
        }
        control.outcomes.send(Outcome::Timeout).unwrap();
        tokio::select! {
            result = &mut stopping => panic!("stop completed before native close: {result:?}"),
            result = &mut control.closing => result.unwrap(),
        }
    }
    assert!(!monitor.is_finished());
    assert!(!control.closed.load(Ordering::Acquire));
    control.release_close.send(()).unwrap();
    monitor.stop().await.unwrap();
    assert!(control.closed.load(Ordering::Acquire));
    assert!(monitor.is_finished());
    monitor.stop().await.unwrap();
    assert_eq!(control.reads.recv().await, None);
}
#[tokio::test]
async fn rejected_startup_awaits_close_and_preserves_cleanup_failure() {
    let (input, mut control) = fixture(true);
    let reader = Reader::start(input).unwrap();
    assert_eq!(control.reads.recv().await, Some(1));
    let mut config = MonitorConfig::new(38_400);
    config.record_capacity = 0;
    let mut starting = Box::pin(MstpMonitor::start(reader, config));
    std::future::poll_fn(|cx| {
        assert!(starting.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    control.outcomes.send(Outcome::Timeout).unwrap();
    control.closing.await.unwrap();
    assert!(!control.closed.load(Ordering::Acquire));
    control.release_close.send(()).unwrap();
    let error = starting.await.unwrap_err();
    assert!(!error.release_confirmed());
    let error = error.to_string();
    assert!(error.contains("closing rejected input failed"));
    assert!(error.contains("scripted close failure"));
    assert!(control.closed.load(Ordering::Acquire));
}

/// A finite script used to prove EOF/error cleanup through the monitor task.
struct Finite {
    steps: VecDeque<io::Result<Vec<u8>>>,
    closed: Arc<AtomicBool>,
    finished: Option<oneshot::Sender<()>>,
}
impl Read for Finite {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let bytes = self.steps.pop_front().unwrap_or(Ok(Vec::new()))?;
        buf[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }
}
impl Input for Finite {
    fn close(&mut self) -> io::Result<()> {
        self.closed.store(true, Ordering::Release);
        let _ = self.finished.take().unwrap().send(());
        Ok(())
    }
}
#[tokio::test]
async fn monitor_eof_and_error_close_before_completion_and_repeat_failures() {
    for fail in [false, true] {
        let closed = Arc::new(AtomicBool::new(false));
        let (finished, done) = oneshot::channel();
        let input = Finite {
            steps: VecDeque::from([if fail {
                Err(io::Error::other("terminal failure"))
            } else {
                Ok(Vec::new())
            }]),
            closed: Arc::clone(&closed),
            finished: Some(finished),
        };
        let reader = Reader::start(input).unwrap();
        let mut monitor = MstpMonitor::start(reader, MonitorConfig::new(38_400))
            .await
            .unwrap();
        // Read owner completion is observed before asking the monitor to stop.
        done.await.unwrap();
        for _ in 0..2 {
            let result = monitor.stop().await;
            if fail {
                assert!(result.unwrap_err().to_string().contains("terminal failure"));
            } else {
                result.unwrap();
            }
        }
        assert!(closed.load(Ordering::Acquire));
    }
}

struct CloseProbe {
    reader: Reader,
    requested: Option<oneshot::Sender<()>>,
}
impl AsyncRead for CloseProbe {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}
impl MonitorRead for CloseProbe {
    fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let result = self.reader.poll_close(cx);
        if let Some(requested) = self.requested.take() {
            let _ = requested.send(());
        }
        result
    }
}

#[tokio::test]
async fn cancelled_rejected_startup_keeps_stop_requested_until_owner_exits() {
    let (input, mut control) = fixture(false);
    let reader = Reader::start(input).unwrap();
    assert_eq!(control.reads.recv().await, Some(1));
    let mut config = MonitorConfig::new(38_400);
    config.record_capacity = 0;
    let mut starting = Box::pin(MstpMonitor::start(reader, config));
    std::future::poll_fn(|cx| {
        assert!(starting.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    control.outcomes.send(Outcome::Timeout).unwrap();
    control.closing.await.unwrap();
    drop(starting);
    assert!(!control.closed.load(Ordering::Acquire));
    control.release_close.send(()).unwrap();
    control.finished.await.unwrap();
    assert!(control.closed.load(Ordering::Acquire));
    assert_eq!(control.reads.recv().await, None);
}

struct ExitHeld {
    dropping: Option<oneshot::Sender<()>>,
    exit: std::sync::mpsc::Receiver<()>,
}
impl Read for ExitHeld {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Ok(0)
    }
}
impl Input for ExitHeld {
    fn close(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Drop for ExitHeld {
    fn drop(&mut self) {
        let _ = self.dropping.take().unwrap().send(());
        self.exit.recv().unwrap();
    }
}
#[tokio::test]
async fn close_completion_alone_does_not_replace_actual_thread_join() {
    let (dropping, entered_drop) = oneshot::channel();
    let (exit, exit_gate) = std::sync::mpsc::channel();
    let mut reader = Reader::start(ExitHeld {
        dropping: Some(dropping),
        exit: exit_gate,
    })
    .unwrap();
    entered_drop.await.unwrap(); // Explicit close returned, but thread is still alive.
    poll_pending_close(&mut reader).await;
    exit.send(()).unwrap();
    close(&mut reader).await.unwrap();
}

#[tokio::test]
async fn dropping_unpolled_startup_does_not_leave_an_unrequested_owner() {
    let (input, mut control) = fixture(false);
    let reader = Reader::start(input).unwrap();
    assert_eq!(control.reads.recv().await, Some(1));
    let startup = MstpMonitor::start(reader, MonitorConfig::new(38_400));
    drop(startup);
    control.outcomes.send(Outcome::Timeout).unwrap();
    control.closing.await.unwrap();
    assert!(!control.closed.load(Ordering::Acquire));
    control.release_close.send(()).unwrap();
    control.finished.await.unwrap();
    assert!(control.closed.load(Ordering::Acquire));
    assert_eq!(control.reads.recv().await, None);
}
