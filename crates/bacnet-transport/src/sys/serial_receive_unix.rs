//! Owned read-only serial descriptors. No transmit operation is exposed.
use crate::mstp::monitor::MonitorAdmissionError;
use bacnet_types::error::Error;

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, IntoRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::task::{Context, Poll};
use tokio::io::unix::AsyncFd;
use tokio::io::{Interest, ReadBuf};

#[derive(Debug)]
pub(crate) struct Input {
    file: Option<File>,
    exclusive: bool,
}

impl Input {
    pub(crate) fn open(
        path: &str,
        baud: u32,
        rts: Option<bool>,
    ) -> Result<Self, MonitorAdmissionError> {
        if baud == 0 {
            return Err(MonitorAdmissionError::unopened(io::Error::new(
                io::ErrorKind::InvalidInput,
                "zero baud rate",
            )));
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_CLOEXEC)
            .open(path)
            .map_err(|error| {
                MonitorAdmissionError::unopened(phase(error, "open read-only descriptor"))
            })?;
        let mut input = Self {
            file: Some(file),
            exclusive: false,
        };
        let configured = input.configure(baud, rts);
        if let Err(error) = configured {
            return Err(MonitorAdmissionError::after_close(
                Error::Transport(error),
                input.close(),
            ));
        }
        Ok(input)
    }

    fn configure(&mut self, baud: u32, rts: Option<bool>) -> io::Result<()> {
        let fd = self.as_raw_fd();
        // SAFETY: fd is live; flock takes no pointers.
        check(unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) })
            .map_err(|error| phase(error, "exclusive flock"))?;
        // SAFETY: TIOCEXCL takes no third argument and fd is live.
        check(unsafe { libc::ioctl(fd, libc::TIOCEXCL as _) })
            .map_err(|error| phase(error, "exclusive terminal ownership"))?;
        self.exclusive = true;
        super::serial_receive_baud::configure(fd, baud)
            .map_err(|error| phase(error, "configure baud and raw input"))?;
        if let Some(level) = rts {
            let bits = libc::TIOCM_RTS;
            let command = if level {
                libc::TIOCMBIS
            } else {
                libc::TIOCMBIC
            };
            // SAFETY: command reads one initialized integer; fd remains owned.
            check(unsafe { libc::ioctl(fd, command as _, &bits) })
                .map_err(|error| phase(error, "set RTS"))?;
        }
        // SAFETY: fd is owned; TCIFLUSH discards input only.
        check(unsafe { libc::tcflush(fd, libc::TCIFLUSH) })
            .map_err(|error| phase(error, "purge input"))
    }

    pub(crate) fn close(&mut self) -> io::Result<()> {
        let Some(file) = self.file.take() else {
            return Ok(());
        };
        let fd = file.into_raw_fd();
        let unlock = if self.exclusive {
            // SAFETY: fd was just transferred from File and is still live.
            check(unsafe { libc::ioctl(fd, libc::TIOCNXCL as _) })
                .map_err(|error| phase(error, "release terminal exclusivity"))
        } else {
            Ok(())
        };
        self.exclusive = false;
        // SAFETY: sole ownership was transferred above; close exactly once,
        // including on EINTR, to avoid closing a reused descriptor.
        let closed =
            check(unsafe { libc::close(fd) }).map_err(|error| phase(error, "close descriptor"));
        crate::mstp::monitor::combine(unlock, closed)
    }
}
impl AsRawFd for Input {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_ref().expect("live serial input").as_raw_fd()
    }
}
impl Input {
    /// Read into `bytes` through the open file.
    fn read(&self, bytes: &mut [u8]) -> io::Result<usize> {
        let mut file = self
            .file
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "closed serial input"))?;
        file.read(bytes).map_err(|error| phase(error, "read input"))
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// An [`Input`] registered with Tokio's reactor for read readiness.
///
/// Tokio's `AsyncFd::register` is unsafe (RUSTSEC-2026-0386): the reactor
/// keeps the descriptor number, so the descriptor must stay open, on the
/// same file description, while it is registered. This type owns the
/// `AsyncFd` and never hands out the `Input` mutably, so nothing can close it
/// or swap it for another while it is registered; [`close`](Self::close)
/// deregisters before it closes.
#[derive(Debug)]
pub(crate) struct AsyncInput(AsyncFd<Input>);

impl AsyncInput {
    /// Register `input` for read readiness. On failure the input comes back,
    /// still open, with the reactor's error.
    pub(crate) fn register(input: Input) -> Result<Self, (Input, io::Error)> {
        // SAFETY: `input` is open (only `close` empties it), and its file
        // closes only in `close` or its drop, both of which need it by
        // `&mut` or by value. The `AsyncFd` owns it and this type hands out
        // only `&Input`, so neither runs while it is registered: `close`
        // below takes it back with `into_inner`, which deregisters first, and
        // `AsyncFd`'s drop deregisters before it drops the input. While open,
        // `as_raw_fd` returns the same descriptor on every call.
        unsafe { AsyncFd::register_with_interest(input, Interest::READABLE) }
            .map(Self)
            .map_err(|error| error.into_parts())
    }

    /// The registered input, for tests that inspect its descriptor.
    #[cfg(test)]
    pub(crate) fn get_ref(&self) -> &Input {
        self.0.get_ref()
    }

    /// Read what is queued into `buf`, or register `cx` for readiness.
    pub(crate) fn poll_read(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            let mut ready = std::task::ready!(self.0.poll_read_ready(cx))?;
            match ready.try_io(|input| input.get_ref().read(buf.initialize_unfilled())) {
                Ok(Ok(count)) => {
                    buf.advance(count);
                    return Poll::Ready(Ok(()));
                }
                Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => continue,
                Ok(Err(error)) => return Poll::Ready(Err(error)),
                Err(_) => continue,
            }
        }
    }

    /// Deregister, then release exclusivity and close the descriptor.
    pub(crate) fn close(self) -> io::Result<()> {
        self.0.into_inner().close()
    }
}
pub(super) fn check(result: libc::c_int) -> io::Result<()> {
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn phase(error: io::Error, phase: &str) -> io::Error {
    crate::mstp::monitor::context(error, phase.into())
}
