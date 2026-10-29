//! The monitor input's explicit ownership-release contract.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};

/// An input whose native ownership can be released and awaited.
///
/// Successful close means all underlying readers have stopped and the resource
/// is released. Implementations must retain close progress across cancellation,
/// and preserve a failed close result on subsequent calls. Drop is best effort.
pub trait MonitorRead: AsyncRead + Unpin + Send {
    /// Stop reading and poll until ownership is released, preserving failures.
    fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>>;
}

impl<R: MonitorRead + ?Sized> MonitorRead for Box<R> {
    fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        (**self).poll_close(cx)
    }
}

/// Explicit adapter for inputs whose resource is released synchronously by Drop.
///
/// Use this for in-memory streams. Do not wrap readers with background I/O or
/// deferred native cleanup: they need their own [`MonitorRead`] implementation.
#[derive(Debug)]
pub struct DropOnClose<R>(Option<R>);

impl<R> DropOnClose<R> {
    /// Declare that dropping `reader` synchronously releases its ownership.
    pub fn new(reader: R) -> Self {
        Self(Some(reader))
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for DropOnClose<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut self.0 {
            Some(reader) => Pin::new(reader).poll_read(cx, buf),
            None => Poll::Ready(Ok(())),
        }
    }
}

impl<R: AsyncRead + Unpin + Send> MonitorRead for DropOnClose<R> {
    fn poll_close(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        drop(self.0.take());
        Poll::Ready(Ok(()))
    }
}

/// A repeatable error retains the original I/O source and kind.
#[derive(Clone, Debug)]
pub(crate) struct Failure(std::sync::Arc<io::Error>);
impl Failure {
    pub(crate) fn new(error: io::Error) -> Self {
        Self(std::sync::Arc::new(error))
    }
    pub(crate) fn error(&self) -> io::Error {
        io::Error::new(self.0.kind(), self.clone())
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

pub(crate) fn combine(first: io::Result<()>, close: io::Result<()>) -> io::Result<()> {
    match (first, close) {
        (Ok(()), result) | (result, Ok(())) => result,
        (Err(first), Err(close)) => {
            Err(io::Error::new(first.kind(), CloseFailures { first, close }))
        }
    }
}

#[derive(Debug)]
struct CloseFailures {
    first: io::Error,
    close: io::Error,
}
impl std::fmt::Display for CloseFailures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}; closing input also failed: {}",
            self.first, self.close
        )
    }
}
impl std::error::Error for CloseFailures {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.first)
    }
}

pub(crate) fn context(error: io::Error, context: String) -> io::Error {
    io::Error::new(error.kind(), ContextError { context, error })
}
#[derive(Debug)]
struct ContextError {
    context: String,
    error: io::Error,
}
impl std::fmt::Display for ContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.context, self.error)
    }
}
impl std::error::Error for ContextError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}
