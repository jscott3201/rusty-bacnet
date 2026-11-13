//! How a colour object saves its Present_Value (#1526) and a written default
//! (#1956); see [`persistence`](super::persistence) for what is kept and why
//! a Present_Value change doesn't wait for its save.
//!
//! The saves go through a staged writer, the one the durable objects share
//! ([`crate::durable`]); each object with storage owns one, with a thread of
//! its own while it saves. A Present_Value save is queued under the database
//! guard, runs on that thread, and is coalesced while one waits to start. A
//! failed one isn't retried at once: the next change, the server's stop or
//! the object's drop tries again.
//!
//! A default write stages, as a Notification Class's Recipient_List write
//! does: the save of the new default with the Present_Value served now runs
//! while the database guard is dropped, the write then takes it, and a write
//! whose save failed is refused. While it is staged, Present_Value changes
//! queue no save of their own, which would land after the staged one with
//! the old default; the next save after the write, or the correcting one if
//! the write never comes, carries them.
//!
//! A written Description is kept the same way, in the same file (#2364):
//! staged with the request's default write, one save for both, refused when
//! its save fails, and never saved for a NULL, which changes nothing.
//!
//! The server's `stop()` asks the object to settle
//! (`settle_forgotten_writes`): it drops a forgotten staged default, queues
//! the served state again if the last save failed, and hands back the wait
//! for its saves, joined to the Tags storage's wait when the object has one.
//! Dropping the object does the same, and the writer's drop waits for those
//! saves.

use crate::prelude::*;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use core::time::Duration;

use crate::sync::AtomicU64;

use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::persistence::{ColorOutputPersistence, ColorOutputSnapshot, ColorOutputValue};
use crate::common;
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::durable::staged::{self, StagedSaves};
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::durable::{PendingWrite, SaveWait, SaveWriter, StageStep};
use crate::write_config::description;

/// What a saved write leaves: the property it sets, as storage holds it.
enum Next {
    Default(ColorOutputValue),
    Description(String),
}

/// A snapshot's Present_Value and default.
type Values = (Option<ColorOutputValue>, Option<ColorOutputValue>);

fn values(snapshot: &ColorOutputSnapshot) -> Values {
    (snapshot.present_value, snapshot.default)
}

/// Put what a write leaves into `snapshot`.
fn apply(snapshot: &mut ColorOutputSnapshot, next: &Next) {
    match next {
        Next::Default(default) => snapshot.default = Some(*default),
        Next::Description(text) => snapshot.description = Some(text.clone()),
    }
}

/// A colour object's output storage, if it has one: boxed, so an object
/// without storage pays one pointer for it.
///
/// A clone, as an application clone or a COV snapshot makes, has none: the
/// storage stays with the original, as its Tags storage does.
#[derive(Default)]
pub(super) struct OutputStorage(Option<Box<OutputStore>>);

impl Clone for OutputStorage {
    fn clone(&self) -> Self {
        Self(None)
    }
}

/// Lifetime totals of one colour object's output saves, shared with the
/// object: take it with `output_save_counters` (on
/// [`ColorObject`](super::ColorObject) or
/// [`ColorTemperatureObject`](super::ColorTemperatureObject)) before the
/// object joins a database, and read it at any time after. A failed
/// Present_Value save refuses nothing, so this is where one shows besides
/// the log: until a retry lands, storage holds an older value than the
/// object serves.
#[derive(Clone, Debug, Default)]
pub struct ColorOutputSaveCounters {
    failed_saves: Arc<AtomicU64>,
}

impl ColorOutputSaveCounters {
    /// Saves the persistence refused, each retry that failed included.
    /// Saturates at `u64::MAX`.
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

struct OutputStore {
    /// The writer, and a default or Description write staged on it.
    saves: StagedSaves<ColorOutputSnapshot, Next>,
    counters: ColorOutputSaveCounters,
    /// The Present_Value and default the last save queued holds, or those
    /// loaded: what a change is compared with to tell whether it needs a
    /// save. A Description changes only through a save of its own, so it
    /// isn't copied here.
    queued: Option<Values>,
    /// What the object serves now, as storage keeps it.
    current: ColorOutputSnapshot,
    /// Whether the last save the writer ran failed. Its done hook sets it,
    /// so it tells the outcome of the latest save.
    last_failed: Arc<AtomicBool>,
    /// Default and Description writes taken, so a staged one can tell
    /// whether another came between.
    kept_writes: u64,
}

impl OutputStorage {
    /// Take `persistence` for the object `oid` and load what it saved. The
    /// caller checks that the loaded values fit the object before taking
    /// them; a Description past 16 KiB is refused here.
    pub(super) fn open(
        &mut self,
        oid: ObjectIdentifier,
        persistence: Arc<dyn ColorOutputPersistence>,
    ) -> Result<Option<ColorOutputSnapshot>, Error> {
        let loaded = persistence.load(oid)?;
        description::check_saved(
            loaded
                .as_ref()
                .and_then(|saved| saved.description.as_deref()),
        )?;
        let last_failed = Arc::new(AtomicBool::new(false));
        let failed = Arc::clone(&last_failed);
        let counters = ColorOutputSaveCounters::default();
        let counted = counters.clone();
        let writer = SaveWriter::new(
            format!(
                "bacnet-color-{}-{}-save",
                oid.object_type().to_raw(),
                oid.instance_number()
            ),
            move |snapshot: &ColorOutputSnapshot| persistence.save(oid, snapshot),
            move |_, result| {
                failed.store(result.is_err(), Ordering::Release);
                if let Err(error) = result {
                    counted.record_failure();
                    log_warn!(
                        object = %oid,
                        %error,
                        "Failed to save a colour output; the next change or stop tries again"
                    );
                }
            },
        );
        self.0 = Some(Box::new(OutputStore {
            saves: StagedSaves::new(writer),
            counters,
            queued: loaded.as_ref().map(values),
            // The object takes a loaded Present_Value (a Color Temperature
            // clamps it, and its next `note` saves the clamped one).
            current: loaded.clone().unwrap_or_default(),
            last_failed,
            kept_writes: 0,
        }));
        Ok(loaded)
    }

    /// The save counters, once the object has storage.
    pub(super) fn counters(&self) -> Option<ColorOutputSaveCounters> {
        self.0.as_ref().map(|store| store.counters.clone())
    }

    /// Whether the object has storage.
    #[cfg(feature = "durable-writes")]
    pub(super) fn is_some(&self) -> bool {
        self.0.is_some()
    }

    /// Whether storage holds a default a write set, now or before a
    /// restart: the object's default setter then leaves it alone.
    pub(super) fn default_saved(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(|store| store.current.default.is_some())
    }

    /// The object holds `value` now: queue a save of it when it differs from
    /// the last one queued, or the last save failed. While a default write
    /// is staged, the save waits for it.
    pub(super) fn note(&mut self, value: ColorOutputValue) {
        let Some(store) = self.0.as_deref_mut() else {
            return;
        };
        store.current.present_value = Some(value);
        if store.saves.is_staged() {
            store.saves.correct(|| store.current.clone());
        } else if store.queued != Some(values(&store.current))
            || store.last_failed.load(Ordering::Acquire)
        {
            store.queue();
        }
    }

    /// Stage the writes storage keeps among a request's `writes` (see
    /// [`DurableWrites::stage_writes`](crate::durable::DurableWrites::stage_writes)):
    /// those to `property`, the object's default, for which `decode` gives
    /// the default each leaves or the object's refusal, and those of a
    /// Description other than a NULL (#2364).
    #[cfg(feature = "durable-writes")]
    pub(super) fn stage_kept(
        &mut self,
        writes: &[PendingWrite],
        property: PropertyIdentifier,
        decode: impl Fn(&PropertyValue) -> Result<ColorOutputValue, Error>,
    ) -> StageStep {
        let Some(store) = self.0.as_deref_mut() else {
            return StageStep::Skip;
        };
        let kept = |write: &PendingWrite| {
            write.property == property
                || (write.property == PropertyIdentifier::DESCRIPTION
                    && !description::null_write(write.property, &write.value))
        };
        if !writes.iter().any(kept) {
            return StageStep::Skip;
        }
        let busy = store.saves.busy();
        store.after_staging();
        if let Some(wait) = busy {
            return StageStep::Busy(wait);
        }
        let steps = staged::steps(writes, |_, write| {
            if !kept(write) {
                return Ok(None);
            }
            if write.array_index.is_some() {
                return Err(common::property_is_not_an_array_error());
            }
            if write.property == property {
                return decode(&write.value).map(|default| Some(Next::Default(default)));
            }
            description::written(&write.value).map(|text| Some(Next::Description(text)))
        });
        // The default the staged save holds: the last one written.
        let mut saved = values(&store.current);
        for step in &steps {
            if let Next::Default(default) = step.next {
                saved.1 = Some(default);
            }
        }
        let step = store
            .saves
            .stage(steps, store.kept_writes, store.current.clone(), apply);
        if matches!(step, StageStep::Staged(_)) {
            store.queued = Some(saved);
        }
        step
    }

    /// A default write of `value` to `property`: take its staged step if it
    /// is the next one, or else save the default `decode` gives with the
    /// Present_Value served now. Returns the default to serve. A save that
    /// fails refuses the write with DEVICE / OPERATIONAL_PROBLEM. Without
    /// storage the default is just decoded.
    pub(super) fn write_default(
        &mut self,
        property: PropertyIdentifier,
        value: &PropertyValue,
        decode: impl FnOnce() -> Result<ColorOutputValue, Error>,
    ) -> Result<ColorOutputValue, Error> {
        let Some(store) = self.0.as_deref_mut() else {
            return decode();
        };
        match store.write_kept(property, value, || decode().map(Next::Default))? {
            Next::Default(default) => Ok(default),
            Next::Description(_) => unreachable!("a default write leaves a default"),
        }
    }

    /// A write of `value` to Description on an object with storage (#2364):
    /// saved, as a default write is, before `served` takes it. `None`
    /// without storage and for a NULL, which storage keeps nothing for: the
    /// object then makes the write as it always has.
    pub(super) fn write_description(
        &mut self,
        served: &mut String,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        let store = self.0.as_deref_mut()?;
        if *value == PropertyValue::Null {
            return None;
        }
        let written = || description::written(value).map(Next::Description);
        let kept = store.write_kept(PropertyIdentifier::DESCRIPTION, value, written);
        Some(kept.map(|next| {
            if let Next::Description(text) = next {
                *served = text;
            }
        }))
    }

    /// Drop a default a write saved, so storage keeps none and the object's
    /// default setter applies again. The default served now stays until the
    /// setter or a write changes it. Saves where it is and waits for the
    /// save; one that fails keeps the written default and returns storage's
    /// error. Nothing to do without storage or a written default.
    pub(super) fn clear_written_default(&mut self) -> Result<(), Error> {
        self.clear(|saved| saved.default.take().is_some())
    }

    /// Drop a Description a write saved (#2364), as
    /// [`clear_written_default`](Self::clear_written_default) drops a
    /// default: storage keeps none, and the text served now stays.
    pub(super) fn clear_written_description(&mut self) -> Result<(), Error> {
        self.clear(|saved| saved.description.take().is_some())
    }

    /// Save what storage holds without what `drop` takes out of it, if it
    /// takes anything.
    fn clear(&mut self, drop: impl FnOnce(&mut ColorOutputSnapshot) -> bool) -> Result<(), Error> {
        let Some(store) = self.0.as_deref_mut() else {
            return Ok(());
        };
        let mut cleared = store.current.clone();
        if !drop(&mut cleared) {
            return Ok(());
        }
        store.saves.save_now(cleared.clone())?;
        store.queued = Some(values(&cleared));
        store.current = cleared;
        // A write staged before this gives way, and keeps the cleared state
        // as what is served.
        store.kept_writes = store.kept_writes.wrapping_add(1);
        store.after_staging();
        Ok(())
    }

    /// See [`DurableWrites::release_staged_write`](crate::durable::DurableWrites::release_staged_write).
    #[cfg(feature = "durable-writes")]
    pub(super) fn release(&mut self, staged: &SaveWait) {
        if let Some(store) = self.0.as_deref_mut() {
            store.saves.release(staged);
            store.after_staging();
        }
    }

    /// Whether a default or Description write is staged.
    #[cfg(feature = "durable-writes")]
    pub(super) fn has_staged(&self) -> bool {
        self.0.as_ref().is_some_and(|store| store.saves.is_staged())
    }

    /// See [`DurableWrites::expire_staged`](crate::durable::DurableWrites::expire_staged).
    #[cfg(feature = "durable-writes")]
    pub(super) fn expire(&mut self, now: Duration) {
        if let Some(store) = self.0.as_deref_mut() {
            store.saves.expire(now);
            store.after_staging();
        }
    }

    /// Settle, as the server stops: drop a forgotten staged default, queue
    /// the served state again if the last save failed, and return the wait
    /// for every save queued so far. `None` without storage.
    #[cfg(feature = "durable-writes")]
    pub(super) fn settle(&mut self) -> Option<SaveWait> {
        let store = self.0.as_deref_mut()?;
        store.saves.drop_staged();
        store.after_staging();
        store.retry_failed();
        Some(store.saves.idle())
    }

    /// Block until the saves queued so far have run.
    pub(super) fn wait_for_saves(&self) {
        if let Some(store) = &self.0 {
            store.saves.wait_idle();
        }
    }

    /// Whether the writer's thread has ended (waiting for it a while), for
    /// the tests that count threads.
    // std: looks at the writer thread.
    #[cfg(all(test, feature = "std"))]
    pub(super) fn writer_thread_ended(&self) -> bool {
        self.0
            .as_ref()
            .is_none_or(|store| store.saves.writer().thread_ended())
    }
}

impl OutputStore {
    fn queue(&mut self) {
        self.saves.submit_coalescing(self.current.clone());
        self.queued = Some(values(&self.current));
    }

    fn retry_failed(&mut self) {
        if self.last_failed.load(Ordering::Acquire) {
            self.queue();
        }
    }

    /// After a call that may have dropped a staged default: put storage back
    /// to the served state, or keep a staged write's copy of it current.
    fn after_staging(&mut self) {
        if self.saves.take_correction() {
            self.queue();
        } else {
            self.saves.correct(|| self.current.clone());
        }
    }

    /// A kept write of `value` to `property`, the default or Description:
    /// take its staged step if it is the next one, or else save what
    /// `decode` gives with the rest of what is served now. Returns what the
    /// write leaves, for the object to serve. A save that fails refuses the
    /// write with DEVICE / OPERATIONAL_PROBLEM.
    fn write_kept(
        &mut self,
        property: PropertyIdentifier,
        value: &PropertyValue,
        decode: impl FnOnce() -> Result<Next, Error>,
    ) -> Result<Next, Error> {
        let refused =
            |_| common::protocol_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM);
        if let Some(claimed) = self.saves.claim(property, None, value, self.kept_writes) {
            match &claimed {
                Ok(next) => self.take(next),
                // Storage keeps what it held: save the Present_Value changes
                // that waited for the refused write.
                Err(_) => self.queue(),
            }
            self.after_staging();
            return claimed.map_err(refused);
        }
        // A write the object refuses leaves a staged one alone (#1424).
        let next = decode()?;
        self.saves.drop_staged();
        self.after_staging();
        let mut saved = self.current.clone();
        apply(&mut saved, &next);
        let queued = values(&saved);
        self.saves.save_now(saved).map_err(refused)?;
        self.queued = Some(queued);
        self.take(&next);
        Ok(next)
    }

    /// Serve what a kept write leaves, and save the Present_Value changes
    /// that waited for it. While the request has steps still to take, the
    /// staged save holds them, so nothing is queued until the last.
    fn take(&mut self, next: &Next) {
        apply(&mut self.current, next);
        self.kept_writes = self.kept_writes.wrapping_add(1);
        if !self.saves.is_staged() && self.queued != Some(values(&self.current)) {
            self.queue();
        }
    }
}

impl Drop for OutputStore {
    /// One more try at a failed save; the writer's own drop, which follows,
    /// waits for it.
    fn drop(&mut self) {
        self.retry_failed();
    }
}

/// Implement [`DurableWrites`](crate::durable::DurableWrites) for a colour
/// object with `profile: ProfileState` and `output: OutputStorage` fields
/// and a `stage_default` method. Tags writes stage on the profile, default
/// and Description writes on the output storage; a request that writes to
/// both gets one wait for both saves. Present_Value never stages, and settling joins the
/// output's saves to the profile's.
macro_rules! impl_color_durable {
    ($object:ty) => {
        #[cfg(feature = "durable-writes")]
        impl $crate::durable::DurableWrites for $object {
            fn stage_write(
                &mut self,
                property: bacnet_types::enums::PropertyIdentifier,
                array_index: Option<u32>,
                value: &bacnet_types::primitives::PropertyValue,
            ) -> $crate::durable::StageStep {
                self.stage_writes(&[$crate::durable::PendingWrite {
                    property,
                    array_index,
                    value: value.clone(),
                }])
            }

            fn stage_writes(
                &mut self,
                writes: &[$crate::durable::PendingWrite],
            ) -> $crate::durable::StageStep {
                use $crate::durable::StageStep;
                let tags = self.profile.stage_writes(writes);
                if matches!(tags, StageStep::Busy(_)) {
                    return tags;
                }
                match (tags, self.stage_default(writes)) {
                    // Wait for the other request without holding the Tags.
                    (StageStep::Staged(tags), StageStep::Busy(wait)) => {
                        self.profile.release_staged_write(&tags);
                        StageStep::Busy(wait)
                    }
                    (StageStep::Staged(tags), StageStep::Staged(default)) => {
                        StageStep::Staged(tags.and(default))
                    }
                    (StageStep::Skip, step) | (step, StageStep::Skip) => step,
                    (_, step) => step,
                }
            }

            fn release_staged_write(&mut self, staged: &$crate::durable::SaveWait) {
                self.profile.release_staged_write(staged);
                self.output.release(staged);
            }

            fn has_staged_write(&self) -> bool {
                self.profile.has_staged_write() || self.output.has_staged()
            }

            fn settle_forgotten_writes(&mut self) -> Option<$crate::durable::SaveWait> {
                let tags = self.profile.settle_forgotten_writes();
                let output = self.output.settle();
                match (tags, output) {
                    (Some(tags), Some(output)) => Some(tags.and(output)),
                    (tags, output) => tags.or(output),
                }
            }

            fn expire_staged(&mut self, now: core::time::Duration) {
                $crate::durable::DurableWrites::expire_staged(&mut self.profile, now);
                self.output.expire(now);
            }
        }
    };
}

pub(super) use impl_color_durable;
