//! A throttle for a warning that could otherwise go out once per datagram
//! (#1624, #1643). The first occurrence warns; later ones log at debug until
//! [`WARNING_INTERVAL`] has passed, and the next warning reports how many
//! there were since the last one. A flood then shows in the log without
//! flooding it.

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
}
