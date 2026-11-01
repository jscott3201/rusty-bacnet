//! A request whose handler panics (a bug, perhaps one an agent's request
//! reached) ends its session (#2203). The SDK serves each request on a
//! task of its own and drops that task's panic, so without this the
//! client would wait for an answer that never comes while the session
//! carried on with whatever state the panic left. Instead the handler's
//! panic is caught, the client gets an internal error, and the session
//! ends with the panic as its reason, which the TUI's listener audits.
//!
//! Catching the panic doesn't stop the process's panic hook from seeing
//! it first: the TUI tells a session's panic from its own by the thread
//! it runs on (`attach::SESSION_THREADS`).

use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::OnceLock;

use futures_util::FutureExt as _;
use rmcp::ErrorData;
use tokio_util::sync::CancellationToken;

/// The first panic of a session's requests, and a token cancelled by it.
#[derive(Debug, Default)]
pub(crate) struct Panics {
    why: OnceLock<String>,
    seen: CancellationToken,
}

impl Panics {
    /// Resolves once a request has panicked.
    pub(crate) async fn seen(&self) {
        self.seen.cancelled().await;
    }

    /// What the first panic said, once one has happened.
    pub(crate) fn why(&self) -> Option<&str> {
        self.why.get().map(String::as_str)
    }

    fn record(&self, why: String) {
        let _ = self.why.set(why);
        self.seen.cancel();
    }
}

/// Run a request's handler, catching a panic: recorded in `panics`, which
/// ends the session, and answered with an internal error.
pub(crate) async fn contained<R>(
    panics: &Panics,
    handler: impl Future<Output = Result<R, ErrorData>>,
) -> Result<R, ErrorData> {
    match AssertUnwindSafe(handler).catch_unwind().await {
        Ok(result) => result,
        Err(payload) => {
            panics.caught(payload.as_ref());
            Err(ErrorData::internal_error(
                "the server failed serving this request, and this session ends",
                None,
            ))
        }
    }
}

/// [`contained`] for a synchronous handler: `None` when it panicked.
pub(crate) fn contained_sync<R>(panics: &Panics, handler: impl FnOnce() -> R) -> Option<R> {
    std::panic::catch_unwind(AssertUnwindSafe(handler))
        .map_err(|payload| panics.caught(payload.as_ref()))
        .ok()
}

impl Panics {
    fn caught(&self, payload: &(dyn Any + Send)) {
        let why = message(payload);
        tracing::error!("an MCP request panicked ({why}); its session ends");
        self.record(why);
    }
}

/// The most of a panic's message kept: it can echo an agent's input, and
/// it goes to the audit log.
const MESSAGE_CAP: usize = 256;

/// A panic payload's message, when it has one, cut to [`MESSAGE_CAP`].
fn message(payload: &(dyn Any + Send)) -> String {
    let text = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no message");
    capped(text, MESSAGE_CAP)
}

/// `text` cut to at most `cap` bytes on a character boundary, marked with
/// an ellipsis when cut.
pub(crate) fn capped(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_string();
    }
    let mut end = cap;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A long message is cut on a character boundary; a short one is
    /// kept whole.
    #[test]
    fn a_panic_message_is_capped() {
        assert_eq!(capped("short", 8), "short");
        assert_eq!(capped("ééé", 3), "é…", "never inside a character");
        let long = "x".repeat(1000);
        let kept = message(&long);
        assert_eq!(kept.len(), MESSAGE_CAP + '…'.len_utf8());
    }
}
