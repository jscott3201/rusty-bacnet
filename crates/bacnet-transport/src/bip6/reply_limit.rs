//! A bounded quota for the replies the B/IPv6 receive loop sends on its own.
//!
//! Some received messages oblige an answer: the NAKs a node that isn't a BBMD
//! sends for foreign-device registration, FDT entry deletion and broadcast
//! distribution (U.4.4, U.4.5.3, U.4.5.4), and the Address-Resolution-ACK
//! for a Forwarded-Address-Resolution that names this node (U.5). Each
//! answer goes to an address the request chose, and a request is a datagram
//! anyone can forge, so a flood of them would turn this node into a
//! reflector. This fixed-window quota caps those answers per destination
//! address and in total, the pattern the B/IPv4 transport uses for its BBMD
//! management requests (`bip::rate_limit`). It is its own type rather than
//! that one, which is keyed by IPv4 address and private to the B/IPv4
//! transport. The receive loop keeps one for the NAKs and another for the
//! Forwarded-Address-Resolution answers, so a NAK flood can't spend the
//! budget those answers need. In normal mode a sender on the link can still
//! forge group Forwarded-Address-Resolutions that name this node and a
//! different asker each time, and so spend the answers' per-window total
//! for that window. That is an accepted trade-off: the cost is a missed
//! answer the asker retries, against an unbounded reflector without it.
//!
//! The answers to Address-Resolution and Virtual-Address-Resolution stay
//! outside both: each is one datagram back to its sender, and a spent quota
//! would silence them. Another node's VMAC collision probe is an
//! Address-Resolution for its own VMAC (see the transport's startup), and a
//! Virtual-Address-Resolution is how a peer learns this node's VMAC before
//! it sends a unicast (U.5).
//!
//! Over-quota replies are dropped silently: none of these answers is
//! retried by the transport, and the requester's own timeout covers the loss.

use std::collections::HashMap;
use std::net::Ipv6Addr;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

/// Length of one accounting window.
pub(super) const REPLY_WINDOW: Duration = Duration::from_secs(1);
/// Replies admitted per destination IPv6 address per window.
pub(super) const REPLIES_PER_DESTINATION: u32 = 16;
/// Replies admitted in total per window. It also bounds the destinations
/// tracked: a destination is only added by a reply that is admitted.
pub(super) const REPLIES_PER_WINDOW: u32 = 256;

/// The reply quota, shared by every reply that counts against it.
#[derive(Debug, Default)]
pub(super) struct ReplyLimiter {
    window: Mutex<Window>,
}

#[derive(Debug, Default)]
struct Window {
    start: Option<Instant>,
    admitted: u32,
    per_destination: HashMap<Ipv6Addr, u32>,
}

impl ReplyLimiter {
    /// Admit (`true`) or refuse (`false`) one reply to `destination` at
    /// `now`. Only an admitted reply is counted.
    pub(super) fn admit(&self, destination: Ipv6Addr, now: Instant) -> bool {
        self.window
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .admit(destination, now)
    }

    /// [`Self::admit`] at the current Tokio time, which a test can pause.
    pub(super) fn admit_now(&self, destination: Ipv6Addr) -> bool {
        self.admit(destination, Instant::now())
    }
}

impl Window {
    fn admit(&mut self, destination: Ipv6Addr, now: Instant) -> bool {
        let expired = self
            .start
            .is_none_or(|start| now.saturating_duration_since(start) >= REPLY_WINDOW);
        if expired {
            self.start = Some(now);
            self.admitted = 0;
            self.per_destination.clear();
        }
        if self.admitted >= REPLIES_PER_WINDOW {
            return false;
        }
        let sent = self.per_destination.entry(destination).or_insert(0);
        if *sent >= REPLIES_PER_DESTINATION {
            return false;
        }
        *sent += 1;
        self.admitted += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(index: u16) -> Ipv6Addr {
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 1, index)
    }

    #[test]
    fn one_destination_gets_its_share_per_window() {
        let limiter = ReplyLimiter::default();
        let start = Instant::now();
        for _ in 0..REPLIES_PER_DESTINATION {
            assert!(limiter.admit(host(1), start));
        }
        assert!(!limiter.admit(host(1), start));
        assert!(
            !limiter.admit(host(1), start + REPLY_WINDOW - Duration::from_millis(1)),
            "still the same window"
        );
        assert!(limiter.admit(host(2), start), "another destination");
        assert!(
            limiter.admit(host(1), start + REPLY_WINDOW),
            "a new window starts afresh"
        );
    }

    #[test]
    fn every_destination_shares_one_total() {
        let limiter = ReplyLimiter::default();
        let start = Instant::now();
        let destinations = u16::try_from(REPLIES_PER_WINDOW / REPLIES_PER_DESTINATION).unwrap();
        for index in 0..destinations {
            for _ in 0..REPLIES_PER_DESTINATION {
                assert!(limiter.admit(host(index), start));
            }
        }
        assert!(!limiter.admit(host(destinations), start), "total spent");
        let window = limiter.window.lock().unwrap();
        assert_eq!(window.admitted, REPLIES_PER_WINDOW);
        assert_eq!(window.per_destination.len(), usize::from(destinations));
    }

    #[test]
    fn the_total_bounds_the_destinations_tracked() {
        let limiter = ReplyLimiter::default();
        let start = Instant::now();
        let mut admitted = 0;
        for index in 0..=u16::MAX {
            admitted += u32::from(limiter.admit(host(index), start));
        }
        assert_eq!(admitted, REPLIES_PER_WINDOW);
        let window = limiter.window.lock().unwrap();
        assert_eq!(window.per_destination.len(), REPLIES_PER_WINDOW as usize);
    }
}
