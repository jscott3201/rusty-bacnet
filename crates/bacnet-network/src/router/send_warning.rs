//! The warning a router port's sender gives when a send fails (#1672).
//!
//! A port can refuse sends in bursts, as a B/IPv6 port does while a VMAC it
//! queues for doesn't resolve, so a warning per failed frame would flood the
//! log. Each port warns at most once per
//! [`WARNING_INTERVAL`](bacnet_transport::warn_throttle::WARNING_INTERVAL),
//! with the failures since the last warning, and logs the rest at debug.
//! Each names the port by its network number, since a transport's error
//! needn't name its link (a send too long for it doesn't, #1833).

use bacnet_transport::warn_throttle::WarnThrottle;
use bacnet_types::error::Error;
use tracing::{debug, warn};

/// One port's throttles, one for unicasts and one for broadcasts, and the
/// port's network number, which each warning names.
#[derive(Debug)]
pub(super) struct SendWarnings {
    network: u16,
    unicast: WarnThrottle,
    broadcast: WarnThrottle,
}

impl SendWarnings {
    /// The warnings of the port on `network`.
    pub(super) fn new(network: u16) -> Self {
        Self {
            network,
            unicast: WarnThrottle::default(),
            broadcast: WarnThrottle::default(),
        }
    }

    /// Log a failed unicast. Returns the failures reported, when it warns.
    pub(super) fn unicast_failed(&self, error: &Error) -> Option<u64> {
        self.failed(&self.unicast, "unicast", error)
    }

    /// Log a failed broadcast. Returns the failures reported, when it warns.
    pub(super) fn broadcast_failed(&self, error: &Error) -> Option<u64> {
        self.failed(&self.broadcast, "broadcast", error)
    }

    fn failed(&self, throttle: &WarnThrottle, send: &'static str, error: &Error) -> Option<u64> {
        let network = self.network;
        let failed = throttle.note();
        match failed {
            Some(failed) => warn!(
                %error,
                network,
                send,
                failed,
                "Router send failed (failures since the last warning)"
            ),
            None => debug!(%error, network, send, "Router send failed"),
        }
        failed
    }
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
        let warnings = SendWarnings::new(1);
        let error = Error::Transport(std::io::Error::other("refused"));
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
