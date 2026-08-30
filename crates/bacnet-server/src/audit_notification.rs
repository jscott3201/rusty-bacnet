//! Receiver policy and bounded duplicate detection for ConfirmedAuditNotification.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bacnet_encoding::npdu::NpduAddress;
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use bytes::Bytes;

/// Local maximum accepted ConfirmedAuditNotification service payload.
pub const MAX_AUDIT_NOTIFICATION_BYTES: usize = 64 * 1024;
/// Local maximum number of notifications in one accepted request.
pub const MAX_AUDIT_NOTIFICATIONS: usize = 256;
const DUPLICATE_WINDOW: Duration = Duration::from_secs(60);
const MAX_DUPLICATE_ENTRIES: usize = 256;

/// Transport provenance and decoded content supplied to the Audit authorizer.
///
/// The source/target identities inside `request` are peer-reported audit data,
/// not authenticated transport provenance. Policy should use `source_network`
/// for a usable routed origin and `source_mac` for the immediate data-link peer.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditNotificationAuthorizationContext {
    /// Immediate data-link peer (normally a router for routed traffic).
    pub source_mac: MacAddr,
    /// Originating NPDU source when one was present.
    pub source_network: Option<NpduAddress>,
    /// Outer Confirmed-Request invoke identifier.
    pub invoke_id: u8,
    /// Explicitly configured local Audit Log sink.
    pub audit_log_sink: ObjectIdentifier,
    /// Decoded peer-reported notification list, preserved verbatim.
    pub request: AuditNotificationRequest,
}

/// Fast, nonblocking authorization callback for ConfirmedAuditNotification.
///
/// Absence, `false`, or a panic denies the request before storage mutation.
pub type AuditNotificationAuthorizer =
    Arc<dyn Fn(&AuditNotificationAuthorizationContext) -> bool + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq)]
enum CanonicalPeer {
    Direct(MacAddr),
    Routed(NpduAddress),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryState {
    Pending,
    Completed,
}

struct Entry {
    id: u64,
    peer: CanonicalPeer,
    invoke_id: u8,
    request: Bytes,
    expires_at: Option<Instant>,
    state: EntryState,
}

#[derive(Default)]
struct TrackerState {
    next_id: u64,
    entries: VecDeque<Entry>,
}

/// Bounded process-local exact-request duplicate tracker.
///
/// This tracker retains request bytes for collision-free equality. It never
/// caches or replays responses; detected pending and completed duplicates are
/// silently discarded. Expiry or restart permits normal service again.
#[derive(Default)]
pub(crate) struct AuditNotificationTracker {
    state: Mutex<TrackerState>,
}

pub(crate) enum DuplicateAdmission {
    Duplicate,
    New(PendingAuditNotification),
}

pub(crate) struct PendingAuditNotification {
    tracker: Arc<AuditNotificationTracker>,
    id: Option<u64>,
    completed: bool,
}

impl AuditNotificationTracker {
    pub(crate) fn begin(
        self: &Arc<Self>,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        invoke_id: u8,
        request: Bytes,
    ) -> DuplicateAdmission {
        self.begin_at(
            source_mac,
            source_network,
            invoke_id,
            request,
            Instant::now(),
        )
    }

    fn begin_at(
        self: &Arc<Self>,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        invoke_id: u8,
        request: Bytes,
        now: Instant,
    ) -> DuplicateAdmission {
        let peer = canonical_peer(source_mac, source_network);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.entries.retain(|entry| match entry.state {
            EntryState::Pending => true,
            EntryState::Completed => entry.expires_at.is_some_and(|expires_at| expires_at > now),
        });
        if state.entries.iter().any(|entry| {
            entry.peer == peer && entry.invoke_id == invoke_id && entry.request == request
        }) {
            return DuplicateAdmission::Duplicate;
        }

        if state.entries.len() >= MAX_DUPLICATE_ENTRIES {
            if let Some(index) = state
                .entries
                .iter()
                .position(|entry| entry.state == EntryState::Completed)
            {
                state.entries.remove(index);
            } else {
                // If every bounded slot is pending, this request cannot be
                // distinguished safely. Clause 5 permits servicing normally.
                return DuplicateAdmission::New(PendingAuditNotification {
                    tracker: Arc::clone(self),
                    id: None,
                    completed: false,
                });
            }
        }

        let id = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        state.entries.push_back(Entry {
            id,
            peer,
            invoke_id,
            request,
            expires_at: None,
            state: EntryState::Pending,
        });
        DuplicateAdmission::New(PendingAuditNotification {
            tracker: Arc::clone(self),
            id: Some(id),
            completed: false,
        })
    }
}

impl PendingAuditNotification {
    /// Mark the request completed, regardless of service success or failure.
    /// A byte-exact retransmission is then still a detected duplicate.
    pub(crate) fn complete(self) {
        self.complete_at(Instant::now());
    }

    fn complete_at(mut self, now: Instant) {
        if let Some(id) = self.id {
            let mut state = self
                .tracker
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(entry) = state.entries.iter_mut().find(|entry| entry.id == id) {
                entry.state = EntryState::Completed;
                entry.expires_at = Some(now + DUPLICATE_WINDOW);
            }
        }
        self.completed = true;
    }
}

impl Drop for PendingAuditNotification {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        if let Some(id) = self.id {
            let mut state = self
                .tracker
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.entries.retain(|entry| entry.id != id);
        }
    }
}

fn canonical_peer(source_mac: &[u8], source_network: Option<&NpduAddress>) -> CanonicalPeer {
    source_network
        .filter(|source| (1..=0xfffe).contains(&source.network) && !source.mac_address.is_empty())
        .cloned()
        .map(CanonicalPeer::Routed)
        .unwrap_or_else(|| CanonicalPeer::Direct(MacAddr::from_slice(source_mac)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routed(network: u16, mac: &[u8]) -> NpduAddress {
        NpduAddress {
            network,
            mac_address: MacAddr::from_slice(mac),
        }
    }

    #[test]
    fn exact_pending_completed_and_invoke_reuse_are_distinguished() {
        let tracker = Arc::new(AuditNotificationTracker::default());
        let now = Instant::now();
        let pending = match tracker.begin_at(b"a", None, 1, Bytes::from_static(b"one"), now) {
            DuplicateAdmission::New(pending) => pending,
            DuplicateAdmission::Duplicate => panic!("first request was duplicate"),
        };
        assert!(matches!(
            tracker.begin_at(b"a", None, 1, Bytes::from_static(b"one"), now),
            DuplicateAdmission::Duplicate
        ));
        pending.complete();
        assert!(matches!(
            tracker.begin_at(b"a", None, 1, Bytes::from_static(b"one"), now),
            DuplicateAdmission::Duplicate
        ));
        assert!(matches!(
            tracker.begin_at(b"a", None, 1, Bytes::from_static(b"two"), now),
            DuplicateAdmission::New(_)
        ));
    }

    #[test]
    fn expiry_capacity_and_canonical_routed_peer_are_bounded() {
        let tracker = Arc::new(AuditNotificationTracker::default());
        let now = Instant::now();
        let first = match tracker.begin_at(
            b"router-a",
            Some(&routed(5, b"origin")),
            1,
            Bytes::from_static(b"x"),
            now,
        ) {
            DuplicateAdmission::New(pending) => pending,
            DuplicateAdmission::Duplicate => unreachable!(),
        };
        first.complete_at(now);
        assert!(matches!(
            tracker.begin_at(
                b"router-b",
                Some(&routed(5, b"origin")),
                1,
                Bytes::from_static(b"x"),
                now
            ),
            DuplicateAdmission::Duplicate
        ));
        assert!(matches!(
            tracker.begin_at(
                b"router-b",
                Some(&routed(6, b"origin")),
                1,
                Bytes::from_static(b"x"),
                now
            ),
            DuplicateAdmission::New(_)
        ));
        assert!(matches!(
            tracker.begin_at(
                b"router-a",
                Some(&routed(5, b"origin")),
                1,
                Bytes::from_static(b"x"),
                now + DUPLICATE_WINDOW
            ),
            DuplicateAdmission::New(_)
        ));

        let tracker = Arc::new(AuditNotificationTracker::default());
        for invoke in 0..MAX_DUPLICATE_ENTRIES {
            let pending = match tracker.begin_at(
                b"a",
                None,
                invoke as u8,
                Bytes::from(vec![invoke as u8, (invoke >> 8) as u8]),
                now,
            ) {
                DuplicateAdmission::New(pending) => pending,
                DuplicateAdmission::Duplicate => unreachable!(),
            };
            pending.complete_at(now);
        }
        let newest = match tracker.begin_at(b"a", None, 1, Bytes::from_static(b"new"), now) {
            DuplicateAdmission::New(pending) => pending,
            DuplicateAdmission::Duplicate => unreachable!(),
        };
        newest.complete_at(now);
        assert_eq!(
            tracker.state.lock().unwrap().entries.len(),
            MAX_DUPLICATE_ENTRIES
        );
    }

    #[test]
    fn pending_never_expires_and_completed_window_starts_at_completion() {
        let tracker = Arc::new(AuditNotificationTracker::default());
        let started_at = Instant::now();
        let pending =
            match tracker.begin_at(b"peer", None, 1, Bytes::from_static(b"request"), started_at) {
                DuplicateAdmission::New(pending) => pending,
                DuplicateAdmission::Duplicate => unreachable!(),
            };

        let completed_at = started_at + DUPLICATE_WINDOW + Duration::from_secs(30);
        assert!(matches!(
            tracker.begin_at(
                b"peer",
                None,
                1,
                Bytes::from_static(b"request"),
                completed_at
            ),
            DuplicateAdmission::Duplicate
        ));

        pending.complete_at(completed_at);
        assert!(matches!(
            tracker.begin_at(
                b"peer",
                None,
                1,
                Bytes::from_static(b"request"),
                completed_at + DUPLICATE_WINDOW - Duration::from_millis(1)
            ),
            DuplicateAdmission::Duplicate
        ));
        assert!(matches!(
            tracker.begin_at(
                b"peer",
                None,
                1,
                Bytes::from_static(b"request"),
                completed_at + DUPLICATE_WINDOW
            ),
            DuplicateAdmission::New(_)
        ));
    }
}
