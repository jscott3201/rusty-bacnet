//! Received event notifications into the Event Logs that collect them
//! (#1346, Clause 12.27; the AE-EL-E-B collection of Annex K.2.22).
//!
//! An Event Log opts in with
//! [`EventLogObject::set_log_received_notifications`](bacnet_objects::event_log::EventLogObject::set_log_received_notifications).
//! Both receive paths then hand each Confirmed and UnconfirmedEventNotification
//! that decodes in full here, unicast or broadcast, after any forwarding
//! locks are gone and before the forwarders see it. A confirmed one is logged
//! once, at its first receipt.
//!
//! Every source gets [`RECEIVED_EVENT_LOG_RATE`] records in each one-second
//! window, shared by all the opted-in logs: a notification within the
//! allowance goes into each of them, and one past it into none, counted in
//! [`EventNotificationCounters::received_not_logged`](super::EventNotificationCounters::received_not_logged).
//! [`RECEIVED_EVENT_LOG_SOURCES`] sources hold an allowance of their own at a
//! time. While the table is full, a new source takes the place of one that
//! has sent nothing for a whole window since its last notification, whose
//! window has therefore ended; when none has been that quiet, the new source
//! shares one more allowance with every other source left out. Each of those
//! 33 allowances opens a window only after its last one ended, so however
//! many sources send, the logs take at most (32 + 1) x 5 = 165 received
//! records in a window, and at most twice that in any one second straddling
//! two windows, whatever a log's Buffer_Size. Normal traffic, a few
//! notifications a second from any one device, is logged in full.
//!
//! Nothing is spent on a notification no log would take: when no Event Log
//! has opted in, which is found before the notification is decoded, or when
//! it is one the logs keep out (see
//! [`ObjectDatabase::log_received_event_notification`]).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use bacnet_encoding::constructed::decode_event_notification_tolerant;
use bacnet_endpoint_core::coordinator::CanonicalPeer;
use bacnet_objects::database::ObjectDatabase;
use tokio::sync::RwLock;
use tracing::debug;

use super::event_suppression::{EventSuppression, EventSuppressions};
use super::runtime_clock;

/// The records one source may add to the Event Logs that collect received
/// notifications in each one-second window.
pub const RECEIVED_EVENT_LOG_RATE: u32 = 5;

/// The sources that hold an allowance of their own at a time; any other
/// source shares one more.
pub const RECEIVED_EVENT_LOG_SOURCES: usize = 32;

const WINDOW: Duration = Duration::from_secs(1);

/// One allowance: the records taken in the window that opened at `opened`,
/// and when its source last sent a notification, taken or not.
#[derive(Debug, Default, Clone, Copy)]
struct Allowance {
    opened: Option<Instant>,
    taken: u32,
    last: Option<Instant>,
}

impl Allowance {
    /// Whether the window has run out, or never opened.
    fn lapsed(&self, now: Instant) -> bool {
        self.opened
            .is_none_or(|opened| now.saturating_duration_since(opened) >= WINDOW)
    }

    /// Whether the source has sent nothing for a whole window. Its window
    /// opened at or before its last notification, so it has ended too.
    fn idle(&self, now: Instant) -> bool {
        self.last
            .is_none_or(|last| now.saturating_duration_since(last) >= WINDOW)
    }

    /// Take one record, opening a new window first if the last has run out.
    fn take(&mut self, now: Instant) -> bool {
        if self.lapsed(now) {
            self.opened = Some(now);
            self.taken = 0;
        }
        self.last = Some(now);
        let within = self.taken < RECEIVED_EVENT_LOG_RATE;
        if within {
            self.taken += 1;
        }
        within
    }
}

/// The allowances behind the bound: one per tracked source, never more than
/// [`RECEIVED_EVENT_LOG_SOURCES`], and the one the others share.
#[derive(Debug, Default)]
struct Allowances {
    sources: Vec<(CanonicalPeer, Allowance)>,
    shared: Allowance,
}

impl Allowances {
    fn take(&mut self, source: &CanonicalPeer, now: Instant) -> bool {
        let at = if let Some(at) = self.sources.iter().position(|(peer, _)| peer == source) {
            at
        } else if self.sources.len() < RECEIVED_EVENT_LOG_SOURCES {
            self.sources.push((source.clone(), Allowance::default()));
            self.sources.len() - 1
        } else if let Some(at) = self.sources.iter().position(|(_, a)| a.idle(now)) {
            // An idle source's window has ended, so handing its place over
            // adds nothing to the bound, and a source still sending keeps
            // its place even once its window has run out.
            self.sources[at] = (source.clone(), Allowance::default());
            at
        } else {
            return self.shared.take(now);
        };
        self.sources[at].1.take(now)
    }
}

/// The per-source allowances both receive paths share. See the module docs.
#[derive(Debug, Default)]
pub(super) struct ReceivedEventLog(Mutex<Allowances>);

impl ReceivedEventLog {
    /// Spend one of `source`'s records at `now`, or say there are none left.
    ///
    /// A source is keyed by the network address it sent from, as the stack
    /// reads every request's sender ([`CanonicalPeer::from_source`], #1465),
    /// not by the Initiating Device Identifier in the notification. That
    /// identifier is whatever the sender writes, so a node keyed by it could
    /// claim a new one for each notification and take a fresh allowance
    /// every time, or claim a quiet device's and spend that device's. A
    /// forged network source at least needs another link address or another
    /// routed source, and the table's size caps what any number of those get.
    fn admit(&self, source: &CanonicalPeer, now: Instant) -> bool {
        // Never held across an await; a poisoned lock only means a panic
        // elsewhere, and the allowances stay usable.
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take(source, now)
    }
}

/// Log a received event notification, `service_request` as it came from
/// `source`, into each Event Log that collects them, within `source`'s
/// allowance. One past it is counted in `suppressions` instead.
///
/// The database is locked twice, and nothing else is held with it: read to
/// ask whether any log collects received notifications and would take this
/// one, then written to log it. The allowance's own lock is taken between the
/// two, so the server's lock order (ObjectDatabase before COVTable) is
/// untouched. With no log collecting, the notification isn't even decoded.
/// The opt-in lives on each Event Log, which the application can change
/// through the database at any time, so there is no cheaper flag to read.
pub(super) async fn log_received_event_notification(
    db: &RwLock<ObjectDatabase>,
    allowances: &ReceivedEventLog,
    suppressions: &EventSuppressions,
    source: CanonicalPeer,
    service_request: &[u8],
) {
    let notification = {
        let db = db.read().await;
        if !db.collects_received_event_notifications() {
            return;
        }
        let Ok(notification) = decode_event_notification_tolerant(service_request) else {
            debug!("Received event notification doesn't decode in full; not logged");
            return;
        };
        if !db.takes_received_event_notification(&notification) {
            return;
        }
        notification
    };
    if !allowances.admit(&source, runtime_clock::now()) {
        suppressions.record(EventSuppression::ReceivedNotLogged);
        debug!(
            ?source,
            "Received event notification over its source's allowance; not logged"
        );
        return;
    }
    db.write()
        .await
        .log_received_event_notification(&notification);
}

#[cfg(test)]
#[path = "received_event_log_tests.rs"]
mod tests;
