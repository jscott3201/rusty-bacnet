//! Native read-only serial ownership for passive monitoring.
use super::SerialConfig;
#[cfg(unix)]
use crate::mstp::monitor::Failure;
use crate::mstp::monitor::{MonitorAdmissionError, MonitorRead};
#[cfg(unix)]
use bacnet_types::error::Error;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};

#[cfg(any(windows, test))]
mod threaded;
#[cfg(unix)]
use crate::sys::serial_receive_unix::{AsyncInput, Input};

/// A serial input opened with native read-only access and exclusive ownership.
///
/// This type has no write interface, configures no kernel RS-485 mode and
/// requests no GPIO. Opening/configuring a driver can still change modem lines;
/// native read-only access does not guarantee electrical silence. An explicit
/// RTS level is applied after open, so it cannot prevent an open-time transition.
///
/// Await [`close`](Self::close), or the monitor's stop, before reopening the
/// adapter. Drop requests shutdown but is not a completion barrier on Windows.
#[derive(Debug)]
pub struct ReceiveOnlyPort {
    #[cfg(unix)]
    input: Option<AsyncInput>,
    #[cfg(unix)]
    closed: Option<Result<(), Failure>>,
    #[cfg(windows)]
    input: threaded::Reader,
}
impl ReceiveOnlyPort {
    /// Open exclusive raw binary 8N1 input at the requested baud, purging input.
    ///
    /// `request_to_send` optionally sets RTS after configuration. No rate is
    /// silently substituted. macOS custom rates use IOSSIOSPEED, whose actual
    /// physical rate cannot be read back through termios.
    ///
    /// # Errors
    /// Returns native open, exclusivity, configuration or purge failures.
    /// Check [`MonitorAdmissionError::release_confirmed`] before reopening.
    pub fn open(
        config: &SerialConfig,
        request_to_send: Option<bool>,
    ) -> Result<Self, MonitorAdmissionError> {
        Self::open_native(config, request_to_send).map_err(|error| {
            error.context(format!(
                "receive-only serial {} at {} baud",
                config.port_name, config.baud_rate
            ))
        })
    }

    #[cfg(unix)]
    fn open_native(
        config: &SerialConfig,
        rts: Option<bool>,
    ) -> Result<Self, MonitorAdmissionError> {
        let input = Input::open(&config.port_name, config.baud_rate, rts)?;
        let input = match AsyncInput::register(input) {
            Ok(input) => input,
            Err((mut input, error)) => {
                return Err(MonitorAdmissionError::after_close(
                    Error::Transport(error),
                    input.close(),
                ));
            }
        };
        Ok(Self {
            input: Some(input),
            closed: None,
        })
    }
    #[cfg(windows)]
    fn open_native(
        config: &SerialConfig,
        rts: Option<bool>,
    ) -> Result<Self, MonitorAdmissionError> {
        let input = crate::sys::serial_receive_windows::Input::open(
            &config.port_name,
            config.baud_rate,
            rts,
        )?;
        Ok(Self {
            input: threaded::Reader::start(input)?,
        })
    }
    /// Stop reading and await native release, retaining progress if cancelled.
    ///
    /// # Errors
    /// Reports read, close and reader-thread failures. A repeated call retains
    /// a previous failure. A driver that never completes a read can delay close.
    pub async fn close(&mut self) -> io::Result<()> {
        std::future::poll_fn(|cx| self.poll_close(cx)).await
    }
}
impl AsyncRead for ReceiveOnlyPort {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        #[cfg(windows)]
        {
            Pin::new(&mut self.input).poll_read(cx, buf)
        }
        #[cfg(unix)]
        {
            if buf.remaining() == 0 {
                return Poll::Ready(Ok(()));
            }
            let Some(input) = self.input.as_mut() else {
                return Poll::Ready(Ok(()));
            };
            input.poll_read(cx, buf)
        }
    }
}
impl MonitorRead for ReceiveOnlyPort {
    fn poll_close(&mut self, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        #[cfg(windows)]
        {
            self.input.poll_close(_cx)
        }
        #[cfg(unix)]
        {
            if self.closed.is_none() {
                let result = self.input.take().map_or(Ok(()), AsyncInput::close);
                self.closed = Some(result.map_err(Failure::new));
            }
            Poll::Ready(
                self.closed
                    .as_ref()
                    .expect("closed result")
                    .as_ref()
                    .copied()
                    .map_err(Failure::error),
            )
        }
    }
}

#[cfg(windows)]
impl threaded::Input for crate::sys::serial_receive_windows::Input {
    fn close(&mut self) -> io::Result<()> {
        self.close()
    }
}
#[cfg(all(test, unix))]
mod unix_tests;
