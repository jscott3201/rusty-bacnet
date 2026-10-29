//! A throttle for a warning that could otherwise go out once per datagram
//! (#1624, #1643). The first occurrence warns; later ones log at debug until
//! [`WARNING_INTERVAL`] has passed, and the next warning reports how many
//! there were since the last one. A flood then shows in the log without
//! flooding it.
//!
//! [`warn_throttled!`](crate::warn_throttled) logs one event through a
//! throttle, for a warning a peer can set off once per frame or connection.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

/// Least time between two warnings from one [`WarnThrottle`].
pub const WARNING_INTERVAL: Duration = Duration::from_secs(10);

/// When one warning last went out, and the occurrences it hasn't reported.
#[derive(Debug, Default)]
pub struct WarnThrottle {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    last: Option<Instant>,
    unreported: u64,
}

impl WarnThrottle {
    /// Note one occurrence, and say whether to warn about it now: `Some`
    /// with the occurrences since the last warning, this one included, at
    /// most once per [`WARNING_INTERVAL`]; `None` when the caller logs it at
    /// debug instead. The clock is Tokio's, so a test can pause it.
    pub fn note(&self) -> Option<u64> {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.unreported = state.unreported.saturating_add(1);
        let quiet = state
            .last
            .is_some_and(|last| now.duration_since(last) < WARNING_INTERVAL);
        if quiet {
            return None;
        }
        state.last = Some(now);
        Some(std::mem::take(&mut state.unreported))
    }
}

#[doc(hidden)]
pub use tracing as __tracing;

/// Log a warning through a [`WarnThrottle`]: at warn level, with an
/// `occurrences` field counting the occurrences since the last warning (this
/// one included), at most once per [`WARNING_INTERVAL`], and at debug level
/// otherwise. The first argument is the throttle, or a reference to it; the
/// rest are fields and a message, written as `tracing::warn!` takes them.
/// `target:` and `parent:` aren't accepted: the event's target is the
/// caller's module. Evaluates to what
/// [`WarnThrottle::note`] returned, so a test can see whether it warned.
///
/// ```
/// use bacnet_transport::warn_throttle::WarnThrottle;
///
/// let throttle = WarnThrottle::default();
/// let error = "truncated";
/// assert_eq!(
///     bacnet_transport::warn_throttled!(throttle, %error, "Failed to decode a frame"),
///     Some(1)
/// );
/// assert_eq!(
///     bacnet_transport::warn_throttled!(throttle, %error, "Failed to decode a frame"),
///     None,
///     "logged at debug"
/// );
/// ```
#[macro_export]
macro_rules! warn_throttled {
    ($throttle:expr_2021, $($event:tt)+) => {{
        let occurrences = $crate::warn_throttle::WarnThrottle::note(&$throttle);
        match occurrences {
            Some(occurrences) => {
                $crate::warn_throttle::__tracing::warn!(occurrences, $($event)+)
            }
            None => $crate::warn_throttle::__tracing::debug!($($event)+),
        }
        occurrences
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn warns_at_most_once_per_interval_with_the_count_since_the_last() {
        let throttle = WarnThrottle::default();
        assert_eq!(throttle.note(), Some(1));
        assert_eq!(throttle.note(), None);
        assert_eq!(throttle.note(), None);
        tokio::time::advance(WARNING_INTERVAL - Duration::from_millis(1)).await;
        assert_eq!(throttle.note(), None, "still inside the interval");
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(throttle.note(), Some(4), "three held back, and this one");
        assert_eq!(throttle.note(), None);
        tokio::time::advance(WARNING_INTERVAL * 3).await;
        assert_eq!(
            throttle.note(),
            Some(2),
            "the one held back is still reported"
        );
        assert_eq!(throttle.note(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn the_macro_warns_through_the_throttle_it_is_given() {
        let throttle = WarnThrottle::default();
        let by_ref = &throttle;
        let peer = "192.0.2.1";
        assert_eq!(
            crate::warn_throttled!(throttle, %peer, "bad frame"),
            Some(1)
        );
        assert_eq!(
            crate::warn_throttled!(by_ref, "bad frame from {peer}"),
            None
        );
        tokio::time::advance(WARNING_INTERVAL).await;
        assert_eq!(crate::warn_throttled!(by_ref, "bad frame"), Some(2));
    }
}
