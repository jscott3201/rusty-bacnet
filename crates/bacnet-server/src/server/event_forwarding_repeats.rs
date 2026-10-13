//! A short-lived record of the ConfirmedEventNotifications this server has
//! offered to its Notification Forwarders, so that a retransmission is
//! acknowledged again without being forwarded again.
//!
//! A sender that misses the acknowledgment sends the same request again with
//! the same invoke ID once its APDU timeout runs out. The confirmed-request
//! tracker only discards a duplicate while the first request is still being
//! answered; once the acknowledgment has gone, the copy is a new transaction.
//! Each received notification that decodes is recorded here, keyed by the
//! canonical source address and the invoke ID, and matched only by a request
//! with the same service-request octets (compared by a 64-bit digest), so a
//! sender that reuses the invoke ID for another notification is forwarded as
//! usual.
//!
//! The record keeps at most [`MAX_RECORDED`] notifications for
//! [`RECORD_WINDOW`] each, dropping the oldest first when full. A
//! retransmission that comes later than that, or whose entry was pushed out
//! by newer ones, is forwarded again, as it would be without the record. It
//! is in memory only: a restart clears it.

use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hasher};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::request_peer::CanonicalRequester;

/// How long a received confirmed notification is remembered. It covers a
/// sender's retries at the usual APDU timeouts (a few seconds each, three
/// retries) with room to spare.
pub(super) const RECORD_WINDOW: Duration = Duration::from_secs(60);

/// The most received confirmed notifications remembered at once.
pub(super) const MAX_RECORDED: usize = 256;

struct Entry {
    requester: CanonicalRequester,
    invoke_id: u8,
    digest: u64,
    at: Instant,
}

/// The recently received ConfirmedEventNotifications. See the module docs.
#[derive(Default)]
pub(super) struct ConfirmedEventRepeats {
    entries: Mutex<VecDeque<Entry>>,
}

impl ConfirmedEventRepeats {
    /// Record a received ConfirmedEventNotification, returning `false` when
    /// the same request from the same source was recorded within
    /// [`RECORD_WINDOW`]. `now` reads the clock, which the request path
    /// passes as [`runtime_clock::now`](crate::runtime_clock::now) (#1556);
    /// it is called with the record locked, so entries go in in time order.
    /// A repeat does not renew the entry, so a sender that keeps repeating
    /// is forwarded again once the window ends.
    pub(super) fn first_receipt(
        &self,
        requester: CanonicalRequester,
        invoke_id: u8,
        service_request: &[u8],
        now: impl FnOnce() -> Instant,
    ) -> bool {
        let mut hasher = DefaultHasher::new();
        hasher.write(service_request);
        let digest = hasher.finish();
        // Never held across an await; a poisoned lock only means a panic
        // elsewhere, and the record stays usable.
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = now();
        let live = |entry: &Entry| now.saturating_duration_since(entry.at) < RECORD_WINDOW;
        while entries.front().is_some_and(|entry| !live(entry)) {
            entries.pop_front();
        }
        // The age is checked again here, so an entry out of time order can
        // never match once its window has ended.
        if entries.iter().any(|entry| {
            entry.invoke_id == invoke_id
                && entry.digest == digest
                && entry.requester == requester
                && live(entry)
        }) {
            return false;
        }
        if entries.len() == MAX_RECORDED {
            entries.pop_front();
        }
        entries.push_back(Entry {
            requester,
            invoke_id,
            digest,
            at: now,
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::MacAddr;

    fn source(last: u8) -> CanonicalRequester {
        CanonicalRequester::Direct(MacAddr::from_slice(&[10, 0, 0, last, 0xBA, 0xC0]))
    }

    #[test]
    fn a_repeat_is_matched_by_source_invoke_id_and_octets() {
        let repeats = ConfirmedEventRepeats::default();
        let now = Instant::now();
        assert!(repeats.first_receipt(source(1), 7, b"alarm", || now));
        assert!(!repeats.first_receipt(source(1), 7, b"alarm", || now));
        assert!(repeats.first_receipt(source(2), 7, b"alarm", || now));
        assert!(repeats.first_receipt(source(1), 8, b"alarm", || now));
        assert!(repeats.first_receipt(source(1), 7, b"other", || now));
    }

    #[test]
    fn an_entry_lasts_for_the_window_from_its_first_receipt() {
        let repeats = ConfirmedEventRepeats::default();
        let start = Instant::now();
        assert!(repeats.first_receipt(source(1), 7, b"alarm", || start));
        let just_inside = start + RECORD_WINDOW - Duration::from_millis(1);
        assert!(!repeats.first_receipt(source(1), 7, b"alarm", || just_inside));
        // The repeat did not renew the entry.
        assert!(repeats.first_receipt(source(1), 7, b"alarm", || start + RECORD_WINDOW));
    }

    #[test]
    fn an_expired_entry_out_of_time_order_does_not_match() {
        let repeats = ConfirmedEventRepeats::default();
        let start = Instant::now();
        // A newer entry sits in front of an older one, as a clock read out of
        // order would leave them; pruning from the front stops at the newer.
        assert!(repeats.first_receipt(source(2), 7, b"newer", || start + RECORD_WINDOW));
        assert!(repeats.first_receipt(source(1), 7, b"alarm", || start));
        let later = start + RECORD_WINDOW + Duration::from_secs(1);
        assert!(repeats.first_receipt(source(1), 7, b"alarm", || later));
    }

    #[test]
    fn a_full_record_drops_its_oldest_entry() {
        let repeats = ConfirmedEventRepeats::default();
        let now = Instant::now();
        for n in 0..MAX_RECORDED {
            let request = n.to_be_bytes();
            assert!(repeats.first_receipt(source(1), 7, &request, || now));
        }
        assert!(repeats.first_receipt(source(1), 7, b"one more", || now));
        assert_eq!(repeats.entries.lock().unwrap().len(), MAX_RECORDED);
        // The first entry made room; the second is still remembered.
        assert!(!repeats.first_receipt(source(1), 7, &1usize.to_be_bytes(), || now));
        assert!(repeats.first_receipt(source(1), 7, &0usize.to_be_bytes(), || now));
    }
}
