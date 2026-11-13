//! An object's save writer together with the writes staged on it (#1270).
//!
//! The Notification Forwarder, the Notification Class and the Access Rights
//! object share this part of the pattern the [module documentation](super)
//! describes: the writes a request stages keep the state they leave aside
//! while their save runs, each write then takes its state or the save's
//! error, and a staged write its request never makes is dropped and
//! corrected.
//!
//! # One save per request (#1423)
//!
//! A request can make several writes to one object: a WritePropertyMultiple
//! that provisions an Access Rights object writes both rule arrays and
//! Enable. The object folds them into one staged save. `steps` works out,
//! in the request's order, the state each write leaves on top of the ones
//! before it; a write the object will refuse ends the list, since the
//! request makes no write after it, and a NULL the object refuses as the
//! wrong datatype changes nothing, so the list goes on past it (#1396). One
//! save holds the state the last write leaves. Each write then takes its own
//! step as the request makes it, so the object serves the last step only
//! once every write has been made. A request that stops part way keeps the
//! steps it took, and the release puts storage back to that served state,
//! as for a staged write never made.
//!
//! # A write that won't be made (#1424)
//!
//! A staged write gives way to another write only once that write is sure to
//! be made. [`claim`](StagedSaves::claim) never drops a staged write; the
//! object first works out the state the incoming write leaves, and
//! [drops](StagedSaves::drop_staged) a staged write it doesn't take only
//! when it does. A write the object refuses, and a NULL the server answers
//! as a success that changes nothing, leave another request's staged write
//! in place.
//!
//! A request can vanish between its stage and its write: the server's
//! `stop()` aborts requests, and an application can drop a local write's
//! future. Its staged write then lingers, and storage may hold a state the
//! object never served. Two checks drop it once
//! `STAGED_WRITE_LIFETIME` has passed since
//! its save finished: the next stage of a write to the object
//! ([`busy`](StagedSaves::busy)), and the object's call from the server's
//! once-a-second operation task ([`expire`](StagedSaves::expire)), which
//! measures the lifetime on that task's monotonic clock. Either way the
//! object then saves the state it serves.
//!
//! Neither check runs once the server has stopped, so two more cover the
//! end of an object's life (#1363). The server's `stop()`, once it has
//! joined every request, drops what is still staged
//! (`drop_forgotten`) and waits for the
//! correcting save. And an object dropped with a write still staged puts
//! storage back as it goes: `StagedSaves` keeps, beside the staged write,
//! the state the object served at its latest storage call, and its `Drop`
//! saves that state, unless the staged save failed, before the writer's own
//! drop waits for its thread. An object that stages through `StagedSaves` gets
//! this without a `Drop` of its own; `stage` takes
//! the served state, and [`correct`](StagedSaves::correct) keeps it current.

#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::prelude::*;
#[cfg(feature = "durable-writes")]
use alloc::collections::VecDeque;
#[cfg(feature = "durable-writes")]
use alloc::sync::Arc;
use core::time::Duration;
#[cfg(all(test, feature = "std"))]
use std::time::Instant;

#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::PropertyValue;

#[cfg(feature = "durable-writes")]
use super::{Event, STAGED_WRITE_LIFETIME, SaveTicket};
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use super::{PendingWrite, SaveWait, SaveWriter, StageStep};

/// One write of a staged save, and the state `N` it leaves.
pub(crate) struct Step<N> {
    /// The write as its request makes it. Its array index is part of it, so
    /// a write of the same value to another element of an array can't take
    /// this one's state.
    #[cfg(feature = "durable-writes")]
    write: PendingWrite,
    /// The state the write leaves, on top of the steps before it, in the
    /// form the object holds it.
    pub(crate) next: N,
}

/// The steps a request's `writes` take on an object, in order (#1423).
///
/// `next` gives the state a write leaves on top of the `earlier` steps,
/// `Ok(None)` for a write the object saves nothing for, or the object's
/// refusal. A refused write ends the steps: the request makes no write after
/// it. A NULL refused as the wrong datatype is no refusal, since the bundled
/// server answers it as a success that leaves the property as it is
/// (#1396); none of the properties these objects save is commandable or
/// takes a NULL. So the steps go on past it.
#[cfg(feature = "durable-writes")]
pub(crate) fn steps<N>(
    writes: &[PendingWrite],
    mut next: impl FnMut(&[Step<N>], &PendingWrite) -> Result<Option<N>, Error>,
) -> Vec<Step<N>> {
    let mut steps = Vec::new();
    for write in writes {
        match next(&steps, write) {
            Ok(Some(state)) => steps.push(Step {
                write: write.clone(),
                next: state,
            }),
            Ok(None) => {}
            Err(error) if relinquishes_nothing(&write.value, &error) => {}
            Err(_) => break,
        }
    }
    steps
}

/// Whether `error`, an object's refusal of `value`, is a NULL refused as
/// the wrong datatype: a value that encodes as one application NULL,
/// refused with PROPERTY / INVALID_DATA_TYPE.
#[cfg(feature = "durable-writes")]
fn relinquishes_nothing(value: &PropertyValue, error: &Error) -> bool {
    let (Error::Remote(RemoteError::Error { class, code }) | Error::Structured { class, code, .. }) =
        error
    else {
        return false;
    };
    is_null(value) && *class == ErrorClass::PROPERTY && *code == ErrorCode::INVALID_DATA_TYPE
}

/// Whether `value` encodes as one application NULL: `Null` itself, the raw
/// octets a raw-octet property receives for one, or a list of one NULL.
/// Whether `staged` and `written` are one written value: the same
/// datatype holding the same bits, element by element through a list.
///
/// `==` isn't that test. A REAL or Double that is no number doesn't equal
/// itself, so a request's own write would pass for another request's and
/// be saved again where it stands, under the caller's lock (#2331); and
/// the two zeros are equal, though a step worked out from one isn't the
/// state a write of the other leaves.
#[cfg(feature = "durable-writes")]
fn same_written(staged: &PropertyValue, written: &PropertyValue) -> bool {
    match (staged, written) {
        (PropertyValue::Real(staged), PropertyValue::Real(written)) => {
            staged.to_bits() == written.to_bits()
        }
        (PropertyValue::Double(staged), PropertyValue::Double(written)) => {
            staged.to_bits() == written.to_bits()
        }
        (PropertyValue::List(staged), PropertyValue::List(written)) => {
            staged.len() == written.len()
                && staged
                    .iter()
                    .zip(written)
                    .all(|(staged, written)| same_written(staged, written))
        }
        // No other datatype holds a floating-point number.
        _ => staged == written,
    }
}

#[cfg(feature = "durable-writes")]
fn is_null(value: &PropertyValue) -> bool {
    match value {
        PropertyValue::Null => true,
        PropertyValue::ApplicationData(octets) => octets.as_slice() == [0x00],
        PropertyValue::List(values) => matches!(values.as_slice(), [only] if is_null(only)),
        _ => false,
    }
}

/// The writes staged for a request: the steps it has still to take, saving
/// on the writer as one, and the snapshot `S` of the state the object serves
/// meanwhile.
#[cfg(feature = "durable-writes")]
struct StagedWrite<S, N> {
    /// The steps not taken yet, in the request's order; never empty.
    steps: VecDeque<Step<N>>,
    /// The object's write count when staged. Each step is taken only if no
    /// other write came between: the first at this count, each later one at
    /// one more than the step before it.
    base: u64,
    /// Steps taken so far.
    taken: u64,
    ticket: SaveTicket,
    /// The state the object served at its latest storage call, a step it
    /// took included: what storage goes back to if the staged write is
    /// dropped before [`correct`](StagedSaves::correct) can ask for a fresh
    /// one, as when the object itself is dropped. `None` for a write staged
    /// with [`stage_snapshot`](StagedSaves::stage_snapshot), whose object
    /// puts storage back itself.
    served: Option<S>,
    /// Set when the staged write is taken or dropped, for a request that
    /// found the object busy.
    released: Arc<Event>,
    /// The operation task's monotonic time when it first found the save
    /// finished; [`StagedSaves::expire`] counts the lifetime from it.
    finished_seen_at: Option<Duration>,
}

/// One object's save writer, saving snapshots `S`, and the writes staged on
/// it, each leaving a state `N`.
///
/// The object owns the state it serves, so whatever needs it comes in as a
/// closure: [`correct`](Self::correct) asks for the snapshot of the served
/// state when a dropped staged write may have left storage ahead of it, or
/// to keep the copy a staged write holds current.
pub(crate) struct StagedSaves<S: Send + 'static, N> {
    writer: SaveWriter<S>,
    #[cfg(feature = "durable-writes")]
    staged: Option<StagedWrite<S, N>>,
    /// A staged write was dropped whose save did not fail, so storage may
    /// hold a state the object never served: the served state that write
    /// kept, if it kept one. [`correct`](Self::correct) saves a fresh
    /// snapshot instead; a drop saves this one.
    #[cfg(feature = "durable-writes")]
    correction: Option<Option<S>>,
    #[cfg(not(feature = "durable-writes"))]
    _staged: core::marker::PhantomData<N>,
}

impl<S: Send + 'static, N> StagedSaves<S, N> {
    /// Saves through `writer`, with nothing staged.
    pub(crate) fn new(writer: SaveWriter<S>) -> Self {
        Self {
            writer,
            #[cfg(feature = "durable-writes")]
            staged: None,
            #[cfg(feature = "durable-writes")]
            correction: None,
            #[cfg(not(feature = "durable-writes"))]
            _staged: core::marker::PhantomData,
        }
    }

    /// The wait for a staged write that still holds the object: one whose
    /// save is running, or finished within
    /// [`STAGED_WRITE_LIFETIME`]. An older one
    /// is dropped.
    /// Without std there is no clock to tell, so only the operation task's
    /// call ([`expire`](Self::expire)) drops one.
    #[cfg(feature = "durable-writes")]
    pub(crate) fn busy(&mut self) -> Option<SaveWait> {
        self.busy_unless(SaveTicket::outlived)
    }

    /// [`busy`](Self::busy), judged at `now`.
    // std: judged on the wall clock.
    #[cfg(all(test, feature = "std", feature = "durable-writes"))]
    pub(crate) fn busy_at(&mut self, now: Instant) -> Option<SaveWait> {
        self.busy_unless(|ticket| ticket.outlived_at(now))
    }

    /// The wait for a staged write that holds the object, unless `outlived`
    /// says it has gone on too long; then it is dropped.
    #[cfg(feature = "durable-writes")]
    fn busy_unless(&mut self, outlived: impl FnOnce(&SaveTicket) -> bool) -> Option<SaveWait> {
        let staged = self.staged.as_ref()?;
        if !outlived(&staged.ticket) {
            return Some(SaveWait::new(Arc::clone(&staged.released)));
        }
        self.drop_staged();
        None
    }

    /// Stage `steps`, a request's writes made from write count `base` on,
    /// and keep them aside until the writes arrive: queue one save of
    /// `served`, the state the object serves now, with every step's state
    /// put into it by `apply`. `served` stays with the steps, as what storage
    /// goes back to should they be dropped. No steps stage nothing.
    #[cfg(feature = "durable-writes")]
    pub(crate) fn stage(
        &mut self,
        steps: Vec<Step<N>>,
        base: u64,
        served: S,
        apply: impl Fn(&mut S, &N),
    ) -> StageStep
    where
        S: Clone,
    {
        if steps.is_empty() {
            return StageStep::Skip;
        }
        let mut snapshot = served.clone();
        for step in &steps {
            apply(&mut snapshot, &step.next);
        }
        self.stage_saving(steps, base, snapshot, Some(served))
    }

    /// Stage `steps` as [`stage`](Self::stage) does, saving `snapshot`,
    /// which the caller built with every step's state in it, and keeping
    /// `served` beside them: for steps that hold less than the state they
    /// leave, so a stage keeps one copy of it whatever the steps' number
    /// (the reporting rows, #2414; Tags, #2526).
    #[cfg(feature = "durable-writes")]
    pub(crate) fn stage_built(
        &mut self,
        steps: Vec<Step<N>>,
        base: u64,
        snapshot: S,
        served: S,
    ) -> StageStep {
        if steps.is_empty() {
            return StageStep::Skip;
        }
        self.stage_saving(steps, base, snapshot, Some(served))
    }

    /// Stage `steps` as [`stage`](Self::stage) does, saving `snapshot`,
    /// which the caller built with every step's state in it, and keeping no
    /// served state beside them. The caller puts storage back itself once a
    /// dropped staged write leaves a correction due
    /// ([`take_correction`](Self::take_correction)): a durable log commits
    /// the state it serves then, records included, rather than have a copy
    /// of them kept for the whole stage (#1952).
    #[cfg(feature = "durable-writes")]
    pub(crate) fn stage_snapshot(
        &mut self,
        steps: Vec<Step<N>>,
        base: u64,
        snapshot: S,
    ) -> StageStep {
        if steps.is_empty() {
            return StageStep::Skip;
        }
        self.stage_saving(steps, base, snapshot, None)
    }

    #[cfg(feature = "durable-writes")]
    fn stage_saving(
        &mut self,
        steps: Vec<Step<N>>,
        base: u64,
        snapshot: S,
        served: Option<S>,
    ) -> StageStep {
        // A correction still due lands first, should this save fail. Today's
        // callers can't reach this with one due: each stage follows a `busy`
        // check through `correct`, or the log's own correction, which queues
        // it. The check stays for a caller that stages without one.
        if let Some(Some(correction)) = self.correction.take() {
            self.writer.submit_coalescing(correction);
        }
        let ticket = self.writer.submit(snapshot);
        let wait = ticket.wait();
        self.staged = Some(StagedWrite {
            steps: steps.into(),
            base,
            taken: 0,
            ticket,
            served,
            released: Arc::default(),
            finished_seen_at: None,
        });
        StageStep::Staged(wait)
    }

    /// From the object's operation-task call at monotonic `now`: drop a
    /// staged write whose request is gone. The first call that finds its
    /// save finished starts the count, and a call
    /// [`STAGED_WRITE_LIFETIME`] or more after that drops it, so on the
    /// server's once-a-second task a forgotten write goes within the lifetime
    /// and one tick of its save. A save still running never expires. A clock
    /// that moved back, such as a newly bound one, starts the count again.
    #[cfg(feature = "durable-writes")]
    pub(crate) fn expire(&mut self, now: Duration) {
        let Some(staged) = self.staged.as_mut() else {
            return;
        };
        if !staged.ticket.is_done() {
            return;
        }
        let seen = match staged.finished_seen_at {
            Some(seen) if seen <= now => seen,
            _ => *staged.finished_seen_at.insert(now),
        };
        if now - seen >= STAGED_WRITE_LIFETIME {
            self.drop_staged();
        }
    }

    /// Take the next staged step for a write of `value` to `property` (at
    /// `array_index`) made at write count `base`: its state if the save
    /// succeeded, the save's error if not, which drops every step. The last
    /// step taken ends the staged write. `None` when the next step is not
    /// this write; a staged write is left as it is then, for the object to
    /// [drop](Self::drop_staged) only once it knows the write will be made
    /// (#1424).
    ///
    /// The step is this write when its value is the staged one bit for bit
    /// ([`same_written`]), so a value that isn't equal to itself, a REAL
    /// that is no number, is still taken by the request that staged it
    /// (#2331).
    #[cfg(feature = "durable-writes")]
    pub(crate) fn claim(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &PropertyValue,
        base: u64,
    ) -> Option<Result<N, Error>> {
        self.claim_unless(property, array_index, value, base, |_| false)
    }

    /// [`claim`](Self::claim), for an object whose step keeps what its stage
    /// saved: once the save has succeeded, a step that `stale` says the
    /// object would no longer save isn't taken. That is `None`, as for
    /// another write, so the object drops the staged write and saves where
    /// it is (#2431).
    #[cfg(feature = "durable-writes")]
    pub(crate) fn claim_unless(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &PropertyValue,
        base: u64,
        stale: impl FnOnce(&N) -> bool,
    ) -> Option<Result<N, Error>> {
        let staged = self.staged.as_mut()?;
        let next = &staged.steps.front()?.write;
        if next.property != property
            || next.array_index != array_index
            || staged.base.wrapping_add(staged.taken) != base
            || !same_written(&next.value, value)
        {
            return None;
        }
        if let Err(error) = staged.ticket.take_outcome() {
            self.staged.take().expect("matched above").released.set();
            return Some(Err(error));
        }
        if stale(&staged.steps.front().expect("matched above").next) {
            return None;
        }
        let step = staged.steps.pop_front().expect("matched above");
        staged.taken = staged.taken.wrapping_add(1);
        if staged.steps.is_empty() {
            self.staged.take().expect("matched above").released.set();
        }
        Some(Ok(step.next))
    }

    /// Whether writes are staged that their request has still to take or
    /// release.
    #[cfg(feature = "durable-writes")]
    pub(crate) fn is_staged(&self) -> bool {
        self.staged.is_some()
    }

    /// Save `snapshot` and wait for the outcome, for a write nobody staged.
    pub(crate) fn save_now(&mut self, snapshot: S) -> Result<(), Error> {
        self.writer.submit(snapshot).take_outcome()
    }

    /// Queue a save nobody waits for; see
    /// [`SaveWriter::submit_coalescing`].
    pub(crate) fn submit_coalescing(&mut self, snapshot: S) {
        self.writer.submit_coalescing(snapshot);
    }

    /// The request that staged a write and was given `wait` is done. A
    /// staged write another request made is left alone.
    #[cfg(feature = "durable-writes")]
    pub(crate) fn release(&mut self, wait: &SaveWait) {
        if self
            .staged
            .as_ref()
            .is_some_and(|staged| staged.ticket.issued(wait))
        {
            self.drop_staged();
        }
    }

    /// Drop the staged write, if any, waking the requests that wait for it.
    #[cfg(feature = "durable-writes")]
    pub(crate) fn drop_staged(&mut self) {
        let Some(staged) = self.staged.take() else {
            return;
        };
        staged.released.set();
        // Unless its save failed, storage holds, or is about to hold, a state
        // the object never served.
        if staged.ticket.succeeded() != Some(false) {
            self.correction = Some(staged.served);
        }
    }

    /// Whether a dropped staged write left a correction due, clearing it:
    /// for a caller that queues the correcting save its own way.
    #[cfg(feature = "durable-writes")]
    pub(crate) fn take_correction(&mut self) -> bool {
        self.correction.take().is_some()
    }

    /// After a storage call, with `served` giving the state the object
    /// serves. If the call dropped a staged write, queue a save of that
    /// state at once; it lands after the dropped write's save, since saves
    /// run in order. If a write is still staged, keep the state with it
    /// instead, for a drop of the object to put back.
    #[cfg(feature = "durable-writes")]
    pub(crate) fn correct(&mut self, served: impl FnOnce() -> S) {
        if self.take_correction() {
            self.writer.submit_coalescing(served());
        } else if let Some(staged) = self.staged.as_mut() {
            staged.served = Some(served());
        }
    }

    /// No request is left to take or release the staged write, as once the
    /// server's `stop()` has joined its requests: drop it, queue the save of
    /// `served` that leaves due, and return a wait that ends once every save
    /// queued so far has run.
    #[cfg(feature = "durable-writes")]
    pub(crate) fn drop_forgotten(&mut self, served: impl FnOnce() -> S) -> SaveWait {
        self.drop_staged();
        self.correct(served);
        self.writer.idle()
    }

    /// A wait that ends once every save queued so far has run.
    pub(crate) fn idle(&self) -> SaveWait {
        self.writer.idle()
    }

    /// Block until every queued save has run.
    pub(crate) fn wait_idle(&self) {
        self.writer.wait_idle();
    }

    /// The owner is going away; see `SaveWriter::closing`.
    #[cfg(feature = "test-support")]
    pub(crate) fn closing(&mut self) {
        self.writer.closing();
    }

    /// The writer, for tests that look at its thread.
    // std: for tests that look at the writer thread.
    #[cfg(all(test, feature = "std"))]
    pub(crate) fn writer(&self) -> &SaveWriter<S> {
        &self.writer
    }
}

#[cfg(feature = "durable-writes")]
impl<S: Send + 'static, N> Drop for StagedSaves<S, N> {
    /// The object is going away with nothing left to take a staged write.
    /// Drop it as a release would, and put storage back to the served state
    /// it kept before the writer, dropped next, waits for its thread (#1363).
    fn drop(&mut self) {
        self.drop_staged();
        if let Some(Some(served)) = self.correction.take() {
            self.writer.put_back(served);
        }
    }
}

/// Without `durable-writes` no object stages: the built-in objects don't
/// implement `DurableWrites` and nothing here queues a staged save, so the
/// only staged state is none. These stand in for the calls the objects'
/// inline saving paths make, so that code compiles unchanged and none of the
/// staging code is linked. A write to an object with storage saves on the
/// writer's own path, as one nobody staged does.
#[cfg(not(feature = "durable-writes"))]
impl<S: Send + 'static, N> StagedSaves<S, N> {
    pub(crate) fn busy(&mut self) -> Option<SaveWait> {
        None
    }

    pub(crate) fn expire(&mut self, _now: Duration) {}

    pub(crate) fn claim(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: &PropertyValue,
        _base: u64,
    ) -> Option<Result<N, Error>> {
        None
    }

    pub(crate) fn is_staged(&self) -> bool {
        false
    }

    pub(crate) fn drop_staged(&mut self) {}

    pub(crate) fn take_correction(&mut self) -> bool {
        false
    }

    pub(crate) fn correct(&mut self, _served: impl FnOnce() -> S) {}
}

#[cfg(all(test, feature = "durable-writes"))]
mod tests;
