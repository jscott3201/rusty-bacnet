//! When a Load Control saves its shed request (Clause 12.28, #2479).
//!
//! An object built with [`LoadControlObject::with_persistence`] keeps its
//! request in the storage it is given: Requested_Shed_Level, Start_Time,
//! Shed_Duration and Duty_Window, the agreed Duty_Window and the shed state,
//! with Enable and a written Description beside them.
//!
//! - A write of one of the four request properties, of Enable or of a
//!   Description is saved before it is answered, on the pattern of the
//!   [durable module](crate::durable): the bundled server stages it, the
//!   save runs on the object's writer thread with the database guard
//!   dropped, and the write then takes effect. One that can't be saved is
//!   refused with DEVICE / OPERATIONAL_PROBLEM and changes nothing. A write
//!   nobody staged saves and waits where it is. A NULL to Description
//!   changes nothing, so nothing is saved for it, and one past 16 KiB is
//!   refused with RESOURCES / NO_SPACE_TO_WRITE_PROPERTY.
//! - What the state machine changes on its own (a request starting to shed,
//!   or ending: at its time, cancelled, by Enable FALSE, or as the object is
//!   removed from its database) and what the application's setters change
//!   are saved with nobody waiting, a newer change replacing one not yet
//!   begun.
//!
//! A staged save holds the request as the writes leave it once the state
//! machine's own rules have run on it: a disabled object holds none, and a
//! request cancelled, or whose time has run out on the Device clock, ends.
//! What the application answers can't be known ahead, so whether a request
//! that goes on then sheds, and so is SHED_COMPLIANT or SHED_NON_COMPLIANT
//! rather than pending, follows in a save of its own. Until that save lands
//! storage may show such a request pending, which a restart decides again
//! all the same.
//!
//! A rebuilt object restores the request. One that was pending or being
//! shed comes back SHED_REQUEST_PENDING, both levels at the requested
//! choice's default, and the next look decides it as a Start_Time write
//! would: the server's start ([`Timed::start_output`]), a write, or the
//! once-a-second pass. One whose time ran out while the device was down
//! then ends at once. If it was being shed when it was saved, its end calls
//! the application's [`stop`](super::LoadShedder::stop), so loads still shed
//! from before the restart are let go, though `shed` was never asked about
//! it in this process.
//!
//! [`Timed::start_output`]: crate::traits::Timed::start_output

use crate::prelude::*;
use alloc::sync::Arc;
use core::time::Duration;

use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier, ShedState};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::persistence::{LoadControlPersistence, LoadControlSnapshot};
use super::shed::request_end;
use super::{
    LoadControlObject, boolean_write, check_requested_level, check_start_time, choice_default,
    decode_date_time_write, decode_shed_level_write, unsigned_write,
};
use crate::clock::{UNSPECIFIED_DATETIME, local_moment};
use crate::common;
use crate::durable::staged::{self, StagedSaves};
use crate::durable::{DurableWrites, PendingWrite, SaveWait, SaveWriter, StageStep};
use crate::write_config::description;

/// The object's writer and the write staged on it; each step holds the
/// request a write leaves.
type Storage = StagedSaves<LoadControlSnapshot, LoadControlSnapshot>;

/// Where a Load Control keeps its request, with persistence. A copy of the
/// object keeps none: storage belongs to one object.
#[derive(Default)]
pub(super) struct Durable {
    storage: Option<Storage>,
    /// What the object last handed storage: what it holds once the saves
    /// queued so far have run.
    saved: LoadControlSnapshot,
    /// Kept writes made so far, so a staged write is taken only if no other
    /// came between.
    writes: u64,
    /// A write set Description, now or before a restart: storage keeps it.
    description_written: bool,
}

impl Clone for Durable {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl Durable {
    /// Whether the object has storage.
    pub(super) fn is_kept(&self) -> bool {
        self.storage.is_some()
    }

    /// A write set Description: storage keeps it from now on.
    pub(super) fn description_written(&mut self) {
        self.description_written = true;
    }

    /// Take storage away for a while, so nothing the object does meanwhile
    /// is saved; [`attach`](Self::attach) puts it back.
    pub(super) fn detach(&mut self) -> Option<Storage> {
        self.storage.take()
    }

    /// Put back what [`detach`](Self::detach) took.
    pub(super) fn attach(&mut self, storage: Option<Storage>) {
        self.storage = storage;
    }
}

/// Whether storage keeps a write of `value` to `property`: one of the four
/// request properties, Enable, or a Description other than a NULL, which
/// changes nothing.
pub(super) fn kept(property: PropertyIdentifier, value: &PropertyValue) -> bool {
    matches!(
        property,
        PropertyIdentifier::REQUESTED_SHED_LEVEL
            | PropertyIdentifier::START_TIME
            | PropertyIdentifier::SHED_DURATION
            | PropertyIdentifier::DUTY_WINDOW
            | PropertyIdentifier::ENABLE
    ) || (property == PropertyIdentifier::DESCRIPTION && !description::null_write(property, value))
}

impl LoadControlObject {
    /// A Load Control that keeps its shed request in `persistence`,
    /// starting from the request saved there for this object, if any (see
    /// the `saving` module for when it saves and how a request comes back).
    /// Configure it as one from [`new`](Self::new); the setters' changes
    /// are saved too.
    ///
    /// Give each object its own storage, and don't put one object built on
    /// a storage in place of another built on the same one while the first
    /// still runs: each saves what it serves. A replaced object leaves the
    /// storage alone as it goes ([`Retirement::Released`]), so building the
    /// replacement first, from the request the first one saved, is safe.
    ///
    /// Fails when the saved request can't be loaded, or holds what a write
    /// would refuse: a percent over 100, a negative or non-finite amount, a
    /// partly unspecified Start_Time, a state that isn't a shed state, or a
    /// Description past 16 KiB.
    ///
    /// [`Retirement::Released`]: crate::traits::Retirement::Released
    pub fn with_persistence(
        instance: u32,
        name: impl Into<String>,
        persistence: Arc<dyn LoadControlPersistence>,
    ) -> Result<Self, Error> {
        let mut object = Self::new(instance, name)?;
        let oid = object.oid;
        if let Some(saved) = persistence.load(oid)? {
            object.restore(saved)?;
        }
        let writer = SaveWriter::new(
            format!("bacnet-lc-{instance}-save"),
            move |snapshot: &LoadControlSnapshot| persistence.save(oid, snapshot),
            move |_, result| {
                if let Err(error) = result {
                    log_warn!(object = %oid, %error, "Failed to save a Load Control's shed request");
                }
            },
        );
        object.durable.storage = Some(StagedSaves::new(writer));
        object.durable.saved = object.snapshot();
        Ok(object)
    }

    /// Block until the saves queued so far have run. Dropping the object
    /// waits for them too.
    pub fn wait_for_saves(&self) {
        if let Some(storage) = &self.durable.storage {
            storage.wait_idle();
        }
    }

    /// Take back a saved request: one pending or being shed waits to be
    /// decided again, as after a Start_Time write.
    fn restore(&mut self, saved: LoadControlSnapshot) -> Result<(), Error> {
        check_requested_level(&saved.requested_shed_level)?;
        check_start_time(saved.start_time)?;
        description::check_saved(saved.description.as_deref())?;
        if saved.state.to_raw() > ShedState::SHED_NON_COMPLIANT.to_raw() {
            return Err(common::value_out_of_range_error());
        }
        if saved.state != ShedState::SHED_INACTIVE {
            self.state = ShedState::SHED_REQUEST_PENDING;
        }
        if matches!(
            saved.state,
            ShedState::SHED_COMPLIANT | ShedState::SHED_NON_COMPLIANT
        ) {
            self.shed_before_restart = Some(saved.state);
        }
        let none = choice_default(&saved.requested_shed_level);
        self.expected_shed_level = none.clone();
        self.actual_shed_level = none;
        self.requested_shed_level = saved.requested_shed_level;
        self.start_time = saved.start_time;
        self.shed_duration = saved.shed_duration;
        self.duty_window = saved.duty_window;
        self.agreed_duty_window = saved.agreed_duty_window;
        // Restored as a write leaves it: the first look holds a disabled
        // object to no request, as it does after a written FALSE.
        self.enable = saved.enable;
        if let Some(text) = saved.description {
            self.description = text;
            self.durable.description_written = true;
        }
        Ok(())
    }

    /// What storage keeps of the object as it serves it. A restored request
    /// that was being shed keeps its saved state until it is decided, so a
    /// second restart still owes the stop.
    fn snapshot(&self) -> LoadControlSnapshot {
        LoadControlSnapshot {
            state: self.shed_before_restart.unwrap_or(self.state),
            requested_shed_level: self.requested_shed_level.clone(),
            start_time: self.start_time,
            shed_duration: self.shed_duration,
            duty_window: self.duty_window,
            agreed_duty_window: self.agreed_duty_window,
            enable: self.enable,
            description: self
                .durable
                .description_written
                .then(|| self.description.clone()),
        }
    }

    /// After a change to what storage keeps, or to what is staged: queue a
    /// save of what the object serves, unless a staged write holds storage,
    /// in which case it keeps that to put back should it be dropped. A
    /// dropped staged write is corrected the same way.
    pub(super) fn follow_storage(&mut self) {
        let Some(mut storage) = self.durable.storage.take() else {
            return;
        };
        let served = self.snapshot();
        if storage.take_correction() || (!storage.is_staged() && served != self.durable.saved) {
            storage.submit_coalescing(served.clone());
            self.durable.saved = served;
        } else {
            storage.correct(|| served);
        }
        self.durable.storage = Some(storage);
    }

    /// Run `f` on storage, then follow what it did ([`follow_storage`]).
    ///
    /// [`follow_storage`]: Self::follow_storage
    fn with_storage<R>(&mut self, f: impl FnOnce(&mut Storage) -> R) -> Option<R> {
        let result = f(self.durable.storage.as_mut()?);
        self.follow_storage();
        Some(result)
    }

    /// Make a kept write with storage, saving what it leaves before the
    /// object takes it: a staged write takes what is already saved; any
    /// other saves now. One that can't be saved is refused with DEVICE /
    /// OPERATIONAL_PROBLEM and changes nothing.
    pub(super) fn write_kept(
        &mut self,
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        let refused =
            |_| common::protocol_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM);
        let storage = self
            .durable
            .storage
            .as_mut()
            .expect("a kept write has storage");
        match storage.claim(property, None, &value, self.durable.writes) {
            Some(Err(error)) => {
                self.follow_storage();
                return Err(refused(error));
            }
            Some(Ok(_)) => {}
            None => {
                // A write the object refuses leaves a write staged for
                // another request alone (#1424).
                let next = next_request(&self.snapshot(), self.now(), property, &value)?;
                self.with_storage(Storage::drop_staged);
                let storage = self.durable.storage.as_mut().expect("checked above");
                storage.save_now(next.clone()).map_err(refused)?;
                self.durable.saved = next;
            }
        }
        self.durable.writes = self.durable.writes.wrapping_add(1);
        self.write_request(property, value)
    }
}

/// What a write of `value` to `property` leaves of `request`, the object's
/// refusal of it checked as the setters check it, once the state machine's
/// own rules have run ([`decided`]). While Enable is FALSE a Start_Time or
/// Shed_Duration is taken but not kept. `now` is the Device clock, if it
/// can be read.
fn next_request(
    request: &LoadControlSnapshot,
    now: Option<i64>,
    property: PropertyIdentifier,
    value: &PropertyValue,
) -> Result<LoadControlSnapshot, Error> {
    let mut next = request.clone();
    let enable = next.enable;
    match property {
        PropertyIdentifier::REQUESTED_SHED_LEVEL => {
            let level = decode_shed_level_write(value.clone())?;
            check_requested_level(&level)?;
            next.requested_shed_level = level;
        }
        PropertyIdentifier::START_TIME => {
            let start = decode_date_time_write(value)?;
            check_start_time(start)?;
            if enable {
                next.start_time = start;
                // ReceiveShedRequest: a Start_Time starts a request.
                if next.state == ShedState::SHED_INACTIVE && start != UNSPECIFIED_DATETIME {
                    next.state = ShedState::SHED_REQUEST_PENDING;
                }
            }
        }
        PropertyIdentifier::SHED_DURATION => {
            let minutes = unsigned_write(value.clone())?;
            if enable {
                next.shed_duration = minutes;
            }
        }
        PropertyIdentifier::DUTY_WINDOW => next.duty_window = unsigned_write(value.clone())?,
        PropertyIdentifier::ENABLE => next.enable = boolean_write(value.clone())?,
        _ => next.description = Some(description::written(value)?),
    }
    decided(&mut next, now);
    Ok(next)
}

/// Run on `request` what the state machine decides without the application
/// (`shed::step`): a disabled object holds no request and no Shed_Duration;
/// a request whose level is its choice's default, whose Start_Time names no
/// moment, or whose end has come at `now`, ends, its properties back at
/// their idle values.
fn decided(request: &mut LoadControlSnapshot, now: Option<i64>) {
    if request.state == ShedState::SHED_INACTIVE {
        if !request.enable {
            request.shed_duration = 0;
        }
        return;
    }
    let start = local_moment(request.start_time);
    let cancelled = request.requested_shed_level == choice_default(&request.requested_shed_level);
    let over = match (start, now) {
        (Some(start), Some(now)) => now >= request_end(start, request.shed_duration),
        (Some(_), None) => false,
        (None, _) => true,
    };
    if !request.enable || cancelled || over {
        request.state = ShedState::SHED_INACTIVE;
        request.requested_shed_level = choice_default(&request.requested_shed_level);
        request.start_time = UNSPECIFIED_DATETIME;
        request.shed_duration = 0;
        request.duty_window = request.agreed_duty_window;
    }
}

impl DurableWrites for LoadControlObject {
    fn stage_write(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> StageStep {
        self.stage_writes(&[PendingWrite {
            property,
            array_index,
            value: value.clone(),
        }])
    }

    /// Fold the request's kept writes into one staged save (#1423), each
    /// leaving what it does on top of the ones before it.
    fn stage_writes(&mut self, writes: &[PendingWrite]) -> StageStep {
        let listed = |write: &PendingWrite| kept(write.property, &write.value);
        if self.durable.storage.is_none() || !writes.iter().any(listed) {
            return StageStep::Skip;
        }
        if let Some(wait) = self.with_storage(Storage::busy).flatten() {
            return StageStep::Busy(wait);
        }
        let served = self.snapshot();
        let now = self.now();
        let steps = staged::steps(writes, |earlier, write| {
            if !listed(write) {
                return Ok(None);
            }
            if write.array_index.is_some() {
                return Err(common::property_is_not_an_array_error());
            }
            let request = earlier.last().map_or(&served, |step| &step.next);
            next_request(request, now, write.property, &write.value).map(Some)
        });
        let Some(last) = steps.last().map(|step| step.next.clone()) else {
            return StageStep::Skip;
        };
        let base = self.durable.writes;
        let storage = self.durable.storage.as_mut().expect("checked above");
        let step = storage.stage(steps, base, served, |snapshot, next| {
            *snapshot = next.clone();
        });
        self.durable.saved = last;
        step
    }

    fn release_staged_write(&mut self, staged: &SaveWait) {
        self.with_storage(|storage| storage.release(staged));
    }

    fn has_staged_write(&self) -> bool {
        self.durable
            .storage
            .as_ref()
            .is_some_and(Storage::is_staged)
    }

    fn settle_forgotten_writes(&mut self) -> Option<SaveWait> {
        self.with_storage(Storage::drop_staged)?;
        self.durable.storage.as_ref().map(Storage::idle)
    }

    fn expire_staged(&mut self, now: Duration) {
        self.with_storage(|storage| storage.expire(now));
    }
}
