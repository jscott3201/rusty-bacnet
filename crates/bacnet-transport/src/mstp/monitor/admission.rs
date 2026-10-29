//! Admission failures retain whether the acquired input was released.
use bacnet_types::error::Error;
use std::io;

/// A monitor open/start failure and its native ownership-release acknowledgment.
///
/// A failed attempt permits replacement only when [`release_confirmed`](Self::release_confirmed)
/// is true. Otherwise ownership is unproven; retain the error and refuse reuse.
#[derive(Debug)]
pub struct MonitorAdmissionError {
    error: Error,
    release_confirmed: bool,
}
impl MonitorAdmissionError {
    /// Report a failure that occurred before any input owner was acquired.
    pub fn without_input(error: Error) -> Self {
        Self {
            error,
            release_confirmed: true,
        }
    }
    /// Whether no input was acquired, or its explicit cleanup succeeded.
    #[must_use]
    pub fn release_confirmed(&self) -> bool {
        self.release_confirmed
    }
    /// The original admission error, including any cleanup failure.
    #[must_use]
    pub fn error(&self) -> &Error {
        &self.error
    }
    #[cfg(feature = "serial")]
    pub(crate) fn unopened(error: io::Error) -> Self {
        Self::without_input(Error::Transport(error))
    }
    pub(crate) fn after_close(error: Error, closed: io::Result<()>) -> Self {
        match closed {
            Ok(()) => Self::without_input(error),
            Err(close) => Self {
                error: Error::Transport(
                    super::combine(
                        Err(match error {
                            Error::Transport(error) => error,
                            error => io::Error::other(error),
                        }),
                        Err(super::context(
                            close,
                            "closing rejected input failed".into(),
                        )),
                    )
                    .unwrap_err(),
                ),
                release_confirmed: false,
            },
        }
    }
    #[cfg(feature = "serial")]
    pub(crate) fn context(mut self, label: String) -> Self {
        self.error = match self.error {
            Error::Transport(error) => Error::Transport(super::context(error, label)),
            error => Error::Transport(super::context(io::Error::other(error), label)),
        };
        self
    }
}
impl std::fmt::Display for MonitorAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}
impl std::error::Error for MonitorAdmissionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}
