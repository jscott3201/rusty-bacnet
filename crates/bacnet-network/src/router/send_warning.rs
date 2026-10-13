//! The warning a router port's sender gives when a send fails (#1672).
//!
//! A port can refuse sends in bursts, as a B/IPv6 port does while a VMAC it
//! queues for doesn't resolve, so a warning per failed frame would flood the
//! log. Each port warns at most once per
//! [`WARNING_INTERVAL`](bacnet_transport::warn_throttle::WARNING_INTERVAL),
//! with the failures since the last warning, and logs the rest at debug.

use bacnet_transport::warn_throttle::WarnThrottle;
use bacnet_types::error::Error;
use tracing::{debug, warn};

/// One port's throttles, one for unicasts and one for broadcasts.
#[derive(Debug, Default)]
pub(super) struct SendWarnings {
    unicast: WarnThrottle,
    broadcast: WarnThrottle,
}

impl SendWarnings {
    /// Log a failed unicast. Returns the failures reported, when it warns.
    pub(super) fn unicast_failed(&self, error: &Error) -> Option<u64> {
        failed(&self.unicast, "unicast", error)
    }

    /// Log a failed broadcast. Returns the failures reported, when it warns.
    pub(super) fn broadcast_failed(&self, error: &Error) -> Option<u64> {
        failed(&self.broadcast, "broadcast", error)
    }
}

fn failed(throttle: &WarnThrottle, send: &'static str, error: &Error) -> Option<u64> {
    let failed = throttle.note();
    match failed {
        Some(failed) => warn!(
            %error,
            send,
            failed,
            "Router send failed (failures since the last warning)"
        ),
        None => debug!(%error, send, "Router send failed"),
    }
    failed
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bacnet_transport::warn_throttle::WARNING_INTERVAL;

    use super::*;

    /// A burst of failures warns once, then again an interval later with
    /// the count since; unicasts and broadcasts are throttled apart.
    #[tokio::test(start_paused = true)]
    async fn a_burst_of_failed_sends_warns_once_per_interval() {
        let warnings = SendWarnings::default();
        let error = Error::Encoding("refused".into());
        assert_eq!(warnings.unicast_failed(&error), Some(1));
        for _ in 0..5 {
            assert_eq!(warnings.unicast_failed(&error), None);
        }
        assert_eq!(warnings.broadcast_failed(&error), Some(1));
        tokio::time::advance(WARNING_INTERVAL - Duration::from_millis(1)).await;
        assert_eq!(warnings.unicast_failed(&error), None);
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(warnings.unicast_failed(&error), Some(7));
    }
}
