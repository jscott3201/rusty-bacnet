//! Capture of timestamped COV-multiple changes at the mutation boundary.
//!
//! A producer captures while it still holds the database borrow that
//! committed the change, so each captured value is paired with the Device
//! clock frame of the commit rather than the later time of notification
//! preparation.
//!
//! A capture takes the table and the histories apart, in three steps a host
//! runs under its own guards:
//!
//! 1. [`TimedCapture::object`], [`TimedCapture::exact`], [`TimedCapture::all`]
//!    or [`TimedCapture::initial`] collects the references from the table.
//! 2. [`TimedCapture::read`] reads their current values from the database,
//!    and one Device clock frame for all of them.
//! 3. [`CaptureRead::queue`] decides, against the histories, which values
//!    are changes, and queues those.
//!
//! A host that locks takes the table only for the first step and the
//! histories only for the third, in that order after the database, so no
//! object is read under either.
//!
//! One capture has one instant on the host's monotonic timeline, the `now`
//! its queueing is given: every change it queues counts its context's
//! Max_Notification_Delay from there. The time a subscriber sees is the
//! clock frame's, which is one per capture too.
//!
//! Ordinary objects capture by each reference's reporting criterion. Life
//! Safety objects report exactly the properties a mutation changed, so they
//! capture through [`TimedCapture::exact`] with the same selection as their
//! exact fanout. WritePropertyMultiple captures every successful attempt as
//! it commits, through [`TimedWriteCapture`].
use bacnet_objects::clock::ClockFrame;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_time::Instant;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use bacnet_device_core::cov::life_safety::{
    LifeSafetyCovChange, LifeSafetyCovSnapshots, is_life_safety_object,
};
use bacnet_device_core::cov::multiple_reads::{MultipleReads, PreparedReference};
use bacnet_device_core::cov::timed::{TimedChange, TimedHistories};
use bacnet_device_core::cov::{
    CovNotificationKind, CovSubscriptionKey, CovSubscriptionSnapshot, CovTable,
};
use bacnet_device_core::write::{WriteCommitObserver, WriteTarget};

/// Live timestamped references whose changes a producer must capture.
pub struct TimedCapture {
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

impl TimedCapture {
    /// The timestamped Multiple references of `table`, live at `now`, that
    /// `select` keeps.
    fn of(
        table: &CovTable,
        now: Instant,
        select: impl Fn(&CovSubscriptionSnapshot) -> bool,
    ) -> Self {
        Self {
            refs: table
                .entries()
                .filter(|sub| {
                    sub.timestamped
                        && sub.notification_kind == CovNotificationKind::Multiple
                        && select(sub)
                        && table.is_current(sub, now)
                })
                .cloned()
                .collect(),
            force: false,
            frame: None,
        }
    }

    /// References of `oid` an ordinary committing mutation must capture. A
    /// Life Safety object captures its exact changes instead.
    pub fn object(table: &CovTable, oid: ObjectIdentifier, now: Instant) -> Self {
        let ordinary = !is_life_safety_object(oid);
        Self::of(table, now, |sub| {
            ordinary && sub.monitored_object_identifier == oid
        })
    }

    /// References the exact Life Safety fanout of `changes` reports: Life
    /// Safety mutations, and LifeSafetyOperation changes on any object.
    pub fn exact(table: &CovTable, changes: &[LifeSafetyCovChange], now: Instant) -> Self {
        Self::of(table, now, |sub| {
            changes.iter().any(|change| exact_selects(sub, change))
        })
    }

    /// Every live timestamped reference, for a producer that commits several
    /// objects one after another under one guard.
    pub fn all(table: &CovTable, now: Instant) -> Self {
        Self::of(table, now, |_| true)
    }

    /// Initial reports of newly (re)subscribed timestamped references. No
    /// change has been observed yet, so the local convention stamps the
    /// current value with the Device time of admission: `frame`, the sample
    /// the admission check validated.
    pub fn initial(
        table: &CovTable,
        snapshots: &[CovSubscriptionSnapshot],
        frame: ClockFrame,
        now: Instant,
    ) -> Self {
        Self {
            refs: snapshots
                .iter()
                .filter(|sub| sub.timestamped && table.is_current(sub, now))
                .cloned()
                .collect(),
            force: true,
            frame: Some(frame),
        }
    }

    /// The kept references only: this runs per write attempt under the
    /// database guard, so nothing else is cloned.
    fn select(&self, keep: impl Fn(&CovSubscriptionSnapshot) -> bool) -> Self {
        Self {
            refs: self.refs.iter().filter(|sub| keep(sub)).cloned().collect(),
            force: self.force,
            frame: self.frame,
        }
    }

    /// The ordinary references of one written object.
    fn of_object(&self, oid: ObjectIdentifier) -> Self {
        let ordinary = !is_life_safety_object(oid);
        self.select(|sub| ordinary && sub.monitored_object_identifier == oid)
    }

    /// The references one Life Safety exact change selects.
    fn of_exact(&self, change: &LifeSafetyCovChange) -> Self {
        self.select(|sub| exact_selects(sub, change))
    }

    fn watches(&self, oid: ObjectIdentifier) -> bool {
        self.refs
            .iter()
            .any(|sub| sub.monitored_object_identifier == oid)
    }

    /// Read each reference's current values under the committing mutation's
    /// database borrow, with the Device clock frame they share. `None` when
    /// there is nothing to queue: no reference, or a missing or invalid
    /// Device clock, which captures nothing; the notification builder then
    /// applies its clockless policy.
    pub fn read(self, db: &ObjectDatabase) -> Option<CaptureRead> {
        if self.refs.is_empty() {
            return None;
        }
        let frame = self.frame.or_else(|| {
            db.clock_frame()
                .filter(|frame| frame.is_valid_actual_datetime())
        })?;
        Some(self.read_source(frame, |oid| db.get(&oid)))
    }

    /// Read an intermediate event state and the exact Device-clock
    /// observation retained with it. No clockless snapshot falls back to a
    /// later live read.
    pub fn read_snapshot(
        self,
        snapshot: &dyn BACnetObject,
        frame: Option<ClockFrame>,
    ) -> Option<CaptureRead> {
        let frame = frame.filter(|frame| frame.is_valid_actual_datetime())?;
        Some(self.read_source(frame, |oid| {
            (snapshot.object_identifier() == oid).then_some(snapshot)
        }))
    }

    fn read_source<'a>(
        self,
        frame: ClockFrame,
        source: impl Fn(ObjectIdentifier) -> Option<&'a dyn BACnetObject>,
    ) -> CaptureRead {
        let mut reads = MultipleReads::default();
        for sub in &self.refs {
            if let Some(object) = source(sub.monitored_object_identifier) {
                reads.capture_source(object, sub);
            }
        }
        let current = self
            .refs
            .iter()
            .enumerate()
            .filter_map(|(at, sub)| {
                let object = source(sub.monitored_object_identifier)?;
                Some((at, reads.read(object, sub)?))
            })
            .collect();
        CaptureRead {
            refs: self.refs,
            force: self.force,
            frame,
            reads,
            current,
        }
    }
}

/// What a capture read from the database: each readable reference's current
/// values, all at one Device clock frame. Nothing is decided yet; see
/// [`CaptureRead::queue`].
pub struct CaptureRead {
    refs: Vec<CovSubscriptionSnapshot>,
    force: bool,
    frame: ClockFrame,
    reads: MultipleReads,
    /// The position among `refs` of each reference that was read, with what
    /// the read prepared.
    current: Vec<(usize, PreparedReference)>,
}

impl CaptureRead {
    /// Queue each qualifying change in `histories`, captured at `now` on the
    /// host's monotonic timeline, and return the references that queued one.
    pub fn queue(self, histories: &mut TimedHistories, now: Instant) -> Vec<CovSubscriptionKey> {
        let Self {
            refs,
            force,
            frame,
            reads,
            current,
        } = self;
        let mut queued = Vec::new();
        for (at, prepared) in current {
            let sub = &refs[at];
            let key = sub.key();
            let generation = sub.generation();
            let baseline = histories
                .baseline(key, generation)
                .or(sub.last_notified_observation.as_ref())
                .cloned();
            if !force && !reads.reports(&prepared, baseline.as_ref()) {
                // Below the reference's increment: nothing to report, but a
                // sibling may carry the field, which then needs the time of
                // this commit (#987).
                histories.note_field(key, generation, &prepared.values, frame);
                continue;
            }
            let values =
                reads.with_flags_companion(&sub.monitored_object_identifier, prepared.values);
            if histories.push(
                key,
                generation,
                TimedChange::new(frame, values, prepared.observation, now),
            ) {
                queued.push(key.clone());
            }
        }
        queued
    }
}

/// Where a write request's captures are queued: the host's histories, as it
/// reaches them. [`TimedWriteCapture`] hands each attempt's read over as the
/// attempt commits; the host queues it ([`CaptureRead::queue`]) with its
/// histories and the time, behind whatever guards them. `Send`, as the
/// write observer it serves is.
pub trait CaptureQueue: Send {
    /// Queue what `read` holds, and return the references that queued a
    /// change.
    fn queue(&mut self, read: CaptureRead) -> Vec<CovSubscriptionKey>;
}

/// Histories a host holds outright, with the instant its request runs at.
impl CaptureQueue for (&mut TimedHistories, Instant) {
    fn queue(&mut self, read: CaptureRead) -> Vec<CovSubscriptionKey> {
        read.queue(self.0, self.1)
    }
}

/// Per-attempt capture for a write request, composed around its Audit
/// observer. Each successful attempt is captured as it commits, under the
/// request's database guard, so a WritePropertyMultiple that goes A-B-A, or
/// fails after a committed prefix, conveys every change it made. A Life
/// Safety object captures the exact changes of that one attempt, and keeps
/// its state from before the request's first attempt on it, for the
/// request's exact fanout.
pub struct TimedWriteCapture<'a, Q> {
    inner: Option<&'a mut dyn WriteCommitObserver>,
    capture: TimedCapture,
    queue: Q,
    /// Each Life Safety object's state from before the request's first
    /// attempt on it.
    request_life_safety: LifeSafetyCovSnapshots,
    /// State of the Life Safety object the current attempt writes, from
    /// before the attempt.
    life_safety: Option<LifeSafetyCovSnapshots>,
    /// Life Safety references some attempt queued a change for.
    life_safety_queued: Vec<CovSubscriptionKey>,
}

impl<'a, Q: CaptureQueue> TimedWriteCapture<'a, Q> {
    /// Capture the attempts of one write request from `capture`, which
    /// [`TimedCapture::all`] collected, into `queue`, around the request's
    /// own observer `inner`.
    pub fn new(
        capture: TimedCapture,
        queue: Q,
        inner: Option<&'a mut dyn WriteCommitObserver>,
    ) -> Self {
        Self {
            inner,
            capture,
            queue,
            request_life_safety: LifeSafetyCovSnapshots::default(),
            life_safety: None,
            life_safety_queued: Vec::new(),
        }
    }

    /// Life Safety references whose attempts queued timestamped changes. The
    /// request's exact fanout compares its end with its start, so it can miss
    /// a reference whose property went out and back, or that only one attempt
    /// selected. Revisiting these after that fanout conveys their changes
    /// without waiting for the Max_Notification_Delay backstop.
    pub fn life_safety_queued(&self) -> &[CovSubscriptionKey] {
        &self.life_safety_queued
    }

    /// What the request changed on each Life Safety object among
    /// `committed`, against its state before the request's first attempt
    /// on it.
    pub fn life_safety_changes(
        &self,
        db: &ObjectDatabase,
        committed: &[ObjectIdentifier],
    ) -> Vec<LifeSafetyCovChange> {
        self.request_life_safety.changes(db, committed)
    }

    /// Read and queue one attempt's capture.
    fn run(&mut self, capture: TimedCapture, db: &ObjectDatabase) -> Vec<CovSubscriptionKey> {
        match capture.read(db) {
            Some(read) => self.queue.queue(read),
            None => Vec::new(),
        }
    }
}

impl<Q: CaptureQueue> WriteCommitObserver for TimedWriteCapture<'_, Q> {
    fn before(&mut self, db: &ObjectDatabase, write: WriteTarget<'_>) {
        self.request_life_safety.capture_before_write(db, write.oid);
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
            let capture = self.capture.of_object(oid);
            self.run(capture, db);
            return;
        }
        if let Some(before) = self.life_safety.take() {
            for change in before.changes(db, core::slice::from_ref(&oid)) {
                let capture = self.capture.of_exact(&change);
                for key in self.run(capture, db) {
                    if !self.life_safety_queued.contains(&key) {
                        self.life_safety_queued.push(key);
                    }
                }
            }
        }
    }
}
