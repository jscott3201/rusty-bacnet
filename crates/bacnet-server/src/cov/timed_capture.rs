//! Capture of timestamped COV-multiple changes at the mutation boundary.
//!
//! Producers capture while still holding the database write guard that
//! committed the change, so each captured value is paired with the Device
//! clock frame of the commit rather than the later time of notification
//! preparation. Producers collect the affected references under a short COV
//! table read and read objects after releasing it; the admission capture runs
//! inside the subscribe handler, which already holds the table. The timed
//! store is always locked last (database, then table, then timed store).
//!
//! Ordinary objects capture by each reference's reporting criterion. Life
//! Safety objects report exactly the properties a mutation changed, so they
//! capture through [`CovSubscriptionTable::timed_capture_exact`] with the same
//! selection as their exact fanout. WritePropertyMultiple captures every
//! successful attempt as it commits, through [`TimedWriteCapture`].
use bacnet_objects::clock::ClockFrame;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::multiple_reads::MultipleReads;
use super::timed::{TimedChange, TimedStore};
use super::{
    CovNotificationKind, CovSubscriptionKey, CovSubscriptionSnapshot, CovSubscriptionTable,
};
use crate::handlers::{WriteCommitObserver, WriteTarget};
use crate::life_safety_cov::{LifeSafetyCovChange, LifeSafetyCovSnapshots, is_life_safety_object};

/// Live timestamped references whose changes a producer must capture.
pub(crate) struct TimedCapture {
    store: TimedStore,
    refs: Vec<CovSubscriptionSnapshot>,
    force: bool,
    /// Device clock sample the caller already validated, instead of a read.
    frame: Option<ClockFrame>,
}

/// Whether a Life Safety exact change selects `sub`, as the exact fanout
/// does: its property changed, or Status_Flags did.
fn exact_selects(sub: &CovSubscriptionSnapshot, change: &LifeSafetyCovChange) -> bool {
    sub.monitored_object_identifier == change.object_identifier
        && (change
            .changed_properties
            .contains(&PropertyIdentifier::STATUS_FLAGS)
            || sub
                .monitored_property
                .is_some_and(|property| change.changed_properties.contains(&property)))
}

impl CovSubscriptionTable {
    fn timed_refs(&self, select: impl Fn(&CovSubscriptionSnapshot) -> bool) -> TimedCapture {
        TimedCapture {
            store: self.timed.clone(),
            refs: self
                .subs
                .values()
                .filter(|sub| {
                    sub.timestamped
                        && sub.notification_kind == CovNotificationKind::Multiple
                        && select(sub)
                        && self.is_current(sub)
                })
                .cloned()
                .collect(),
            force: false,
            frame: None,
        }
    }

    /// References of `oid` an ordinary committing mutation must capture. A
    /// Life Safety object captures its exact changes instead.
    pub(crate) fn timed_capture(&self, oid: ObjectIdentifier) -> TimedCapture {
        let ordinary = !is_life_safety_object(oid);
        self.timed_refs(|sub| ordinary && sub.monitored_object_identifier == oid)
    }

    /// References the exact Life Safety fanout of `changes` reports: Life
    /// Safety mutations, and LifeSafetyOperation changes on any object.
    pub(crate) fn timed_capture_exact(&self, changes: &[LifeSafetyCovChange]) -> TimedCapture {
        self.timed_refs(|sub| changes.iter().any(|change| exact_selects(sub, change)))
    }

    /// Every live timestamped reference, for a producer that commits several
    /// objects one after another under one guard.
    pub(crate) fn timed_capture_all(&self) -> TimedCapture {
        self.timed_refs(|_| true)
    }

    /// Initial reports of newly (re)subscribed timestamped references. No
    /// change has been observed yet, so the local convention stamps the
    /// current value with the Device time of admission: `frame`, the sample
    /// the admission check validated.
    pub(crate) fn initial_timed_capture(
        &self,
        snapshots: &[CovSubscriptionSnapshot],
        frame: ClockFrame,
    ) -> TimedCapture {
        TimedCapture {
            store: self.timed.clone(),
            refs: snapshots
                .iter()
                .filter(|sub| sub.timestamped && self.is_current(sub))
                .cloned()
                .collect(),
            force: true,
            frame: Some(frame),
        }
    }
}

impl TimedCapture {
    /// The kept references only: this runs per write attempt under the
    /// database guard, so nothing else is cloned.
    fn select(&self, keep: impl Fn(&CovSubscriptionSnapshot) -> bool) -> Self {
        Self {
            store: self.store.clone(),
            refs: self.refs.iter().filter(|sub| keep(sub)).cloned().collect(),
            force: self.force,
            frame: self.frame,
        }
    }

    /// The ordinary references of one written object.
    fn object(&self, oid: ObjectIdentifier) -> Self {
        let ordinary = !is_life_safety_object(oid);
        self.select(|sub| ordinary && sub.monitored_object_identifier == oid)
    }

    /// The references one Life Safety exact change selects.
    fn exact(&self, change: &LifeSafetyCovChange) -> Self {
        self.select(|sub| exact_selects(sub, change))
    }

    fn watches(&self, oid: ObjectIdentifier) -> bool {
        self.refs
            .iter()
            .any(|sub| sub.monitored_object_identifier == oid)
    }

    /// Queue each qualifying change under the committing mutation's database
    /// guard, and return the references that queued one. A missing or invalid
    /// Device clock captures nothing; the notification builder then applies
    /// its clockless policy.
    pub(crate) fn run(self, db: &ObjectDatabase) -> Vec<CovSubscriptionKey> {
        if self.refs.is_empty() {
            return Vec::new();
        }
        let Some(frame) = self.frame.or_else(|| {
            db.clock_frame()
                .filter(|frame| frame.is_valid_actual_datetime())
        }) else {
            return Vec::new();
        };
        self.run_from_source(frame, |oid| db.get(&oid))
    }

    /// Capture an intermediate event state and the exact Device-clock observation
    /// retained with it. No clockless snapshot falls back to a later live read.
    pub(crate) fn run_from_snapshot(
        self,
        snapshot: &dyn BACnetObject,
        frame: Option<ClockFrame>,
    ) -> Vec<CovSubscriptionKey> {
        let Some(frame) = frame.filter(|frame| frame.is_valid_actual_datetime()) else {
            return Vec::new();
        };
        self.run_from_source(frame, |oid| {
            (snapshot.object_identifier() == oid).then_some(snapshot)
        })
    }

    fn run_from_source<'a>(
        self,
        frame: ClockFrame,
        source: impl Fn(ObjectIdentifier) -> Option<&'a dyn BACnetObject>,
    ) -> Vec<CovSubscriptionKey> {
        let mut reads = MultipleReads::default();
        for sub in &self.refs {
            if let Some(object) = source(sub.monitored_object_identifier) {
                reads.capture_source(object, sub);
            }
        }
        let current: Vec<_> = self
            .refs
            .iter()
            .filter_map(|sub| {
                let object = source(sub.monitored_object_identifier)?;
                Some((sub, reads.read(object, sub)?))
            })
            .collect();
        let mut timed = self.store.lock();
        let mut queued = Vec::new();
        for (sub, prepared) in current {
            let key = sub.key();
            let generation = sub.generation();
            let baseline = timed
                .baseline(key, generation)
                .or(sub.last_notified_observation.as_ref())
                .cloned();
            if !self.force && !reads.reports(&prepared, baseline.as_ref()) {
                // Below the reference's increment: nothing to report, but a
                // sibling may carry the field, which then needs the time of
                // this commit (#987).
                timed.note_field(key, generation, &prepared.values, frame);
                continue;
            }
            let values =
                reads.with_flags_companion(&sub.monitored_object_identifier, prepared.values);
            if timed.push(
                key,
                generation,
                TimedChange::new(frame, values, prepared.observation),
            ) {
                queued.push(key.clone());
            }
        }
        queued
    }
}

/// Per-attempt capture for a write request, composed around its Audit
/// observer. Each successful attempt is captured as it commits, under the
/// request's database guard, so a WritePropertyMultiple that goes A-B-A, or
/// fails after a committed prefix, conveys every change it made. A Life
/// Safety object captures the exact changes of that one attempt.
pub(crate) struct TimedWriteCapture<'a> {
    inner: Option<&'a mut dyn WriteCommitObserver>,
    capture: TimedCapture,
    /// State of the Life Safety object the current attempt writes, from
    /// before the attempt.
    life_safety: Option<LifeSafetyCovSnapshots>,
    /// Life Safety references some attempt queued a change for.
    life_safety_queued: Vec<CovSubscriptionKey>,
}

impl<'a> TimedWriteCapture<'a> {
    pub(crate) fn new(
        capture: TimedCapture,
        inner: Option<&'a mut dyn WriteCommitObserver>,
    ) -> Self {
        Self {
            inner,
            capture,
            life_safety: None,
            life_safety_queued: Vec::new(),
        }
    }

    /// Life Safety references whose attempts queued timestamped changes. The
    /// request's exact fanout compares its end with its start, so it can miss
    /// a reference whose property went out and back, or that only one attempt
    /// selected. Revisiting these after that fanout conveys their changes
    /// without waiting for the Max_Notification_Delay backstop.
    pub(crate) fn life_safety_queued(&self) -> &[CovSubscriptionKey] {
        &self.life_safety_queued
    }
}

impl WriteCommitObserver for TimedWriteCapture<'_> {
    fn before(&mut self, db: &ObjectDatabase, write: WriteTarget<'_>) {
        self.life_safety = (is_life_safety_object(write.oid) && self.capture.watches(write.oid))
            .then(|| LifeSafetyCovSnapshots::capture_oid(db, write.oid));
        if let Some(inner) = self.inner.as_deref_mut() {
            inner.before(db, write);
        }
    }

    fn commit_policy(
        &mut self,
        db: &mut ObjectDatabase,
        write: WriteTarget<'_>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        self.inner.as_deref_mut()?.commit_policy(db, write, value)
    }

    fn committed(&mut self, db: &mut ObjectDatabase) {
        if let Some(inner) = self.inner.as_deref_mut() {
            inner.committed(db);
        }
    }

    fn failed(&mut self, db: &mut ObjectDatabase, error: &Error) {
        self.life_safety = None;
        if let Some(inner) = self.inner.as_deref_mut() {
            inner.failed(db, error);
        }
    }

    fn applied(&mut self, db: &ObjectDatabase, oid: ObjectIdentifier) {
        if let Some(inner) = self.inner.as_deref_mut() {
            inner.applied(db, oid);
        }
        if !is_life_safety_object(oid) {
            self.capture.object(oid).run(db);
            return;
        }
        if let Some(before) = self.life_safety.take() {
            for change in before.changes(db, std::slice::from_ref(&oid)) {
                for key in self.capture.exact(&change).run(db) {
                    if !self.life_safety_queued.contains(&key) {
                        self.life_safety_queued.push(key);
                    }
                }
            }
        }
    }
}
