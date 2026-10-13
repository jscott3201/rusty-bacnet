//! Waits for SC tests on tokio's paused clock (#1547, #1549).
//!
//! A paused clock moves only when every task is idle, and then straight to
//! the next timer, so a wait that needs a transport timer to fire is written
//! against tokio's clock (a `timeout`, or a poll that sleeps) and steps in
//! virtual time. A wait for progress that needs no timer spins on
//! [`until`] instead: yielding never idles the runtime, so a time bound
//! could never end such a spin there.

use std::future::Future;

/// Scheduler rounds [`until`] gives the transport. Yielding never moves a
/// paused clock, so a time bound can't end a spin there; this counts rounds
/// instead, which a stalled runner can't use up either.
pub(super) const ROUNDS: usize = 10_000;

/// Yield until `done` resolves true, failing after [`ROUNDS`] rounds with
/// `what` in the message. On a paused clock only timer-free progress can
/// satisfy it.
pub(super) async fn until<F: Future<Output = bool>>(what: &str, mut done: impl FnMut() -> F) {
    for _ in 0..ROUNDS {
        if done().await {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("{what}: not reached in {ROUNDS} scheduler rounds");
}
