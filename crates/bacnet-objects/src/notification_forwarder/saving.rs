//! When a forwarder saves its lists (Clauses 12.51.8 and 12.51.9, #1270).
//!
//! A write to Recipient_List or Subscribed_Recipients saves both lists before
//! the forwarder serves the new one, and a write that cannot be saved is
//! refused with DEVICE / OPERATIONAL_PROBLEM, leaving the old list. The bundled
//! server stages such a write ([`DurableWrites`](crate::durable::DurableWrites)):
//! the save runs on the forwarder's writer thread while the database guard is
//! dropped, and the write then takes the saved lists without saving again. A
//! WritePropertyMultiple that writes both lists, or one more than once,
//! stages one save of what its last write leaves (#1423). A write that was
//! not staged queues its save and waits for it where it is.
//!
//! A staged write its request never makes, because the request failed first
//! or is gone, is dropped. Its save may already have put lists in storage
//! that the forwarder never served, so the forwarder queues a save of the
//! lists it does serve at once; a restart then serves those. That holds at
//! the end too (#1363): the server's `stop()` drops a staged write still
//! held and waits for that save, and a forwarder dropped with one still
//! held saves the lists it serves before its writer stops. The drop has no
//! chance to read the lists afresh, so it saves them as they stood at the
//! forwarder's last storage call, at most one operation-task tick old: with
//! a bound clock, an entry that lapsed since that tick can come back with a
//! minute left. `stop()` reads them as they stand.
//!
//! Between writes Subscribed_Recipients still changes: entries lapse, and each
//! entry's served minutes fall by one a minute. The operation task calls the
//! object every second, and each call compares the lists the forwarder serves
//! with the copy storage last confirmed. A lapse queues a save at once;
//! falling minutes queue one at most once a minute. These saves coalesce on
//! the writer, so a burst costs one save of the latest lists. A saved entry
//! therefore never carries more than about a minute beyond what it had left,
//! so a device that restarts often still sees each entry run out.
//!
//! A failed save is logged and counted in [`ForwarderSaveCounters`]. The
//! operation task retries its saves a minute later, and storage keeps the last
//! lists that were saved.

use crate::prelude::*;
use core::sync::atomic::Ordering;

use crate::sync::{AtomicU64, Mutex};
use alloc::sync::Arc;
use core::time::Duration;
#[cfg(all(test, feature = "std"))]
use std::time::Instant;

use bacnet_types::constructed::BACnetDestination;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::persistence::{ForwarderSnapshot, NotificationForwarderPersistence};
use crate::durable::staged::{StagedSaves, Step};
use crate::durable::{SaveWait, SaveWriter, StageStep};
use crate::subscribed_recipients::SubscribedRecipients;

/// The least time between two saves the operation task makes for falling
/// minutes or to retry a failed save.
const RESAVE_INTERVAL: Duration = Duration::from_secs(60);

/// Lifetime totals of one forwarder's saves, shared with the object: take it
/// with [`save_counters`](super::NotificationForwarderObject::save_counters)
/// before the object joins a database, and read it at any time after. Each
/// total saturates at `u64::MAX`.
#[derive(Clone, Debug, Default)]
pub struct ForwarderSaveCounters {
    failed_saves: Arc<AtomicU64>,
}

impl ForwarderSaveCounters {
    /// Saves the persistence refused: those for a write, which the write then
    /// fails with, and those the operation task makes, which it retries.
    pub fn failed_saves(&self) -> u64 {
        self.failed_saves.load(Ordering::Relaxed)
    }

    fn record_failure(&self) {
        let _ = self
            .failed_saves
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_add(1))
            });
    }
}

/// The list a write leaves, in the form the forwarder holds it.
pub(super) enum NextList {
    RecipientList(Vec<BACnetDestination>),
    SubscribedRecipients(SubscribedRecipients),
}

/// Put the list `next` leaves into `snapshot`.
pub(super) fn apply(snapshot: &mut ForwarderSnapshot, next: &NextList) {
    match next {
        NextList::RecipientList(list) => snapshot.recipient_list = Some(list.clone()),
        NextList::SubscribedRecipients(store) => {
            snapshot.subscribed_recipients = store.subscriptions();
        }
    }
}

/// A forwarder's storage, its writer and staged write, and what storage
/// last confirmed.
pub(super) struct Storage {
    saves: StagedSaves<ForwarderSnapshot, NextList>,
    /// The lists storage last confirmed, updated by the writer's thread.
    confirmed: Arc<Mutex<ForwarderSnapshot>>,
    /// When the operation task last queued a save, on the store's clock.
    last_attempt: Option<Duration>,
    /// A lapse wants the next operation task call to save without waiting
    /// for the minute.
    resave_now: bool,
}

impl Storage {
    /// Storage for `oid` that holds `confirmed`, as loaded when the
    /// forwarder was built.
    pub(super) fn new(
        oid: ObjectIdentifier,
        persistence: Arc<dyn NotificationForwarderPersistence>,
        confirmed: ForwarderSnapshot,
        counters: ForwarderSaveCounters,
    ) -> Self {
        let confirmed = Arc::new(Mutex::new(confirmed));
        let saved = Arc::clone(&confirmed);
        let writer = SaveWriter::new(
            format!("bacnet-nf-{}-save", oid.instance_number()),
            move |snapshot: &ForwarderSnapshot| persistence.save(oid, snapshot),
            move |snapshot, result| match result {
                Ok(()) => {
                    *saved.lock() = snapshot.clone();
                }
                Err(error) => {
                    counters.record_failure();
                    log_warn!(
                        forwarder = %oid,
                        %error,
                        "Failed to save Notification Forwarder lists"
                    );
                }
            },
        );
        Self {
            saves: StagedSaves::new(writer),
            confirmed,
            last_attempt: None,
            resave_now: false,
        }
    }

    /// The wait for a staged write that still holds the forwarder (see
    /// [`StagedSaves::busy`]).
    pub(super) fn busy(&mut self) -> Option<SaveWait> {
        self.saves.busy()
    }

    /// [`busy`](Self::busy), judged at `now`.
    #[cfg(all(test, feature = "std"))]
    pub(super) fn busy_at(&mut self, now: Instant) -> Option<SaveWait> {
        self.saves.busy_at(now)
    }

    /// Stage one save of the lists `steps` leave, made from write count
    /// `base` on; `served` holds the lists the forwarder serves meanwhile
    /// (see [`StagedSaves::stage`]).
    pub(super) fn stage(
        &mut self,
        steps: Vec<Step<NextList>>,
        base: u64,
        served: ForwarderSnapshot,
    ) -> StageStep {
        self.saves.stage(steps, base, served, apply)
    }

    /// Take the next staged list for a write (see [`StagedSaves::claim`]).
    /// Both lists are BACnetLISTs, written whole.
    pub(super) fn claim(
        &mut self,
        property: PropertyIdentifier,
        value: &PropertyValue,
        base: u64,
    ) -> Option<Result<NextList, Error>> {
        self.saves.claim(property, None, value, base)
    }

    /// Whether a list write is staged (see [`StagedSaves::is_staged`]).
    pub(super) fn is_staged(&self) -> bool {
        self.saves.is_staged()
    }

    /// Drop the staged write, for a write that supersedes it (see
    /// [`StagedSaves::drop_staged`]).
    pub(super) fn drop_staged(&mut self) {
        self.saves.drop_staged();
    }

    /// Save `snapshot` and wait for the outcome, for a write nobody staged.
    pub(super) fn save_now(&mut self, snapshot: ForwarderSnapshot) -> Result<(), Error> {
        self.saves.save_now(snapshot)
    }

    /// The request that staged a write and was given `wait` is done. A
    /// staged write another request made is left alone.
    pub(super) fn release(&mut self, wait: &SaveWait) {
        self.saves.release(wait);
    }

    /// After a storage call, with `served` giving the lists the forwarder
    /// serves (see [`StagedSaves::correct`]).
    pub(super) fn correct(&mut self, served: impl FnOnce() -> ForwarderSnapshot) {
        self.saves.correct(served);
    }

    /// No request is left to take or release a staged write (see
    /// [`StagedSaves::drop_forgotten`]).
    pub(super) fn drop_forgotten(
        &mut self,
        served: impl FnOnce() -> ForwarderSnapshot,
    ) -> SaveWait {
        self.saves.drop_forgotten(served)
    }

    /// Bring storage up to date from the operation task: at once when an
    /// entry lapsed, and otherwise at most once every [`RESAVE_INTERVAL`]
    /// while the served lists differ from the confirmed copy. Nothing is
    /// queued while a staged write holds the forwarder, since its lists would
    /// land after this save.
    pub(super) fn keep_current(
        &mut self,
        now: Duration,
        lapsed: bool,
        current: impl FnOnce() -> ForwarderSnapshot,
    ) {
        if lapsed {
            self.resave_now = true;
        }
        // A staged write whose request is gone lets go here, on the store's
        // clock or the wall clock, whichever runs out first.
        self.saves.expire(now);
        if self.busy().is_some() {
            return;
        }
        // A staged write dropped just now: save the served lists below.
        self.resave_now |= self.saves.take_correction();
        let current = current();
        if current == *self.confirmed.lock() {
            self.resave_now = false;
            return;
        }
        let due = self.resave_now
            || self
                .last_attempt
                .is_none_or(|at| now.saturating_sub(at) >= RESAVE_INTERVAL);
        if !due {
            return;
        }
        self.last_attempt = Some(now);
        self.resave_now = false;
        self.saves.submit_coalescing(current);
    }

    /// The store's clock changed: times kept on the old one no longer apply,
    /// and a staged list holds deadlines on the old clock.
    pub(super) fn clock_changed(&mut self) {
        self.last_attempt = None;
        self.saves.drop_staged();
    }

    /// Block until every queued save has run.
    pub(super) fn wait_idle(&self) {
        self.saves.wait_idle();
    }
}
