//! The Ethernet transport's packet socket, registered with Tokio's reactor.
//!
//! Tokio's `AsyncFd::register` is unsafe (RUSTSEC-2026-0386): the reactor
//! keeps the descriptor number, so the descriptor must stay open, on the
//! same file description, while it is registered. [`AsyncSocket`] upholds
//! that by holding a clone of the shared socket and handing out only shared
//! access to it.

use socket2::Socket;
use std::io;
use std::sync::Arc;
use tokio::io::Interest;
use tokio::io::unix::{AsyncFd, AsyncFdReadyGuard};

/// A shared packet socket registered for read readiness.
#[derive(Debug)]
pub(crate) struct AsyncSocket(AsyncFd<Arc<Socket>>);

impl AsyncSocket {
    /// Register `socket` for read readiness. The socket's other clones may
    /// keep sending on it.
    ///
    /// # Errors
    /// Returns the reactor's registration failure.
    pub(crate) fn register(socket: Arc<Socket>) -> io::Result<Self> {
        // SAFETY: the `AsyncFd` owns a clone of the `Arc`, so the `Socket`
        // can't be dropped, and its descriptor closed, before the `AsyncFd`
        // deregisters it: `AsyncFd`'s drop deregisters first and only then
        // drops its clone. `Socket` closes its descriptor only in its own
        // drop, and nothing in the crate dups over or closes it by number.
        // `Socket::as_raw_fd` returns the same descriptor on every call, and
        // this type never hands out the `AsyncFd` mutably, so the `Arc` it
        // holds can't be swapped for another socket.
        unsafe { AsyncFd::register_with_interest(socket, Interest::READABLE) }
            .map(Self)
            .map_err(|error| error.into_parts().1)
    }

    /// Wait until the socket may be readable. Read through the guard's
    /// `try_io`, which sees the socket only by shared reference.
    ///
    /// # Errors
    /// Returns the reactor's readiness failure.
    pub(crate) async fn readable(&self) -> io::Result<AsyncFdReadyGuard<'_, Arc<Socket>>> {
        self.0.readable().await
    }
}
