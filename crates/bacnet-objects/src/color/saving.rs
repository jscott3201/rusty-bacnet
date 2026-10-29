//! How a colour object saves its Present_Value (#1526); see
//! [`persistence`](super::persistence) for what is kept and why a change
//! doesn't wait for its save.
//!
//! The saves go through a [`SaveWriter`], the writer type the durable
//! objects share ([`crate::durable`]); each object with storage owns one,
//! with its own thread for as long as the object lives. A save is queued
//! under the database guard, runs on that thread, and is coalesced while one
//! waits to start. Nothing is staged. A failed save isn't retried at once:
//! the next change, the server's stop or the object's drop tries again. The server's `stop()` asks the object to settle
//! (`settle_forgotten_writes`): it queues the value again if the last save
//! failed and hands back the wait for its saves, joined to the Tags
//! storage's wait when the object has one. Dropping the object does the
//! same, and the writer's drop waits for those saves.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::sync::AtomicU64;

use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::persistence::{ColorOutputPersistence, ColorOutputSnapshot, ColorOutputValue};
use crate::durable::{SaveWait, SaveWriter};

/// A colour object's Present_Value storage, if it has one.
///
/// A clone, as an application clone or a COV snapshot makes, has none: the
/// storage stays with the original, as its Tags storage does.
#[derive(Default)]
pub(super) struct OutputStorage(Option<OutputStore>);

impl Clone for OutputStorage {
    fn clone(&self) -> Self {
        Self(None)
    }
}

/// Lifetime totals of one colour object's Present_Value saves, shared with
/// the object: take it with `output_save_counters` (on
/// [`ColorObject`](super::ColorObject) or
/// [`ColorTemperatureObject`](super::ColorTemperatureObject)) before the
/// object joins a database, and read it at any time after. A failed save
/// refuses nothing, so this is where one shows besides the log: until a
/// retry lands, storage holds an older value than the object serves.
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
    writer: SaveWriter<ColorOutputSnapshot>,
    counters: ColorOutputSaveCounters,
    /// The value of the last save queued, or the one loaded.
    queued: Option<ColorOutputValue>,
    /// The value the object holds now, as it last reported it.
    current: ColorOutputValue,
    /// Whether the last save the writer ran failed. Its done hook sets it,
    /// so it tells the outcome of the latest save.
    last_failed: Arc<AtomicBool>,
}

impl OutputStorage {
    /// Take `persistence` for the object `oid`, which holds `current`, and
    /// load what it saved. Returns the saved value, if any; the caller checks
    /// it fits the object before taking it.
    pub(super) fn open(
        &mut self,
        oid: ObjectIdentifier,
        current: ColorOutputValue,
        persistence: Arc<dyn ColorOutputPersistence>,
    ) -> Result<Option<ColorOutputValue>, Error> {
        let loaded = persistence.load(oid)?.map(|saved| saved.present_value);
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
                        "Failed to save a colour Present_Value; the next change or stop tries again"
                    );
                }
            },
        );
        self.0 = Some(OutputStore {
            writer,
            counters,
            queued: loaded,
            // The object takes a loaded value as Present_Value (a Color
            // Temperature clamps it, and its next `note` saves the clamped
            // one). `current` is only saved again after a failed save, and
            // every save follows a `note` that sets it, so this is a
            // starting point, not a value storage can receive unnoted.
            current: loaded.unwrap_or(current),
            last_failed,
        });
        Ok(loaded)
    }

    /// The save counters, once the object has storage.
    pub(super) fn counters(&self) -> Option<ColorOutputSaveCounters> {
        self.0.as_ref().map(|store| store.counters.clone())
    }

    /// Whether the object has storage.
    pub(super) fn is_some(&self) -> bool {
        self.0.is_some()
    }

    /// The object holds `value` now: queue a save of it when it differs from
    /// the last one queued, or the last save failed.
    pub(super) fn note(&mut self, value: ColorOutputValue) {
        let Some(store) = &mut self.0 else {
            return;
        };
        store.current = value;
        if store.queued != Some(value) || store.last_failed.load(Ordering::Acquire) {
            store.queue();
        }
    }

    /// Settle, as the server stops: queue the value again if the last save
    /// failed, and return the wait for every save queued so far. `None`
    /// without storage.
    pub(super) fn settle(&mut self) -> Option<SaveWait> {
        let store = self.0.as_mut()?;
        store.retry_failed();
        Some(store.writer.idle())
    }

    /// Block until the saves queued so far have run.
    pub(super) fn wait_for_saves(&self) {
        if let Some(store) = &self.0 {
            store.writer.wait_idle();
        }
    }
}

impl OutputStore {
    fn queue(&mut self) {
        self.writer.submit_coalescing(ColorOutputSnapshot {
            present_value: self.current,
        });
        self.queued = Some(self.current);
    }

    fn retry_failed(&mut self) {
        if self.last_failed.load(Ordering::Acquire) {
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
/// object with `profile: ProfileState` and `output: OutputStorage` fields.
/// Tags writes stage on the profile as before; Present_Value never stages,
/// and settling joins its saves to the profile's.
macro_rules! impl_color_durable {
    ($object:ty) => {
        impl $crate::durable::DurableWrites for $object {
            fn stage_write(
                &mut self,
                property: bacnet_types::enums::PropertyIdentifier,
                array_index: Option<u32>,
                value: &bacnet_types::primitives::PropertyValue,
            ) -> $crate::durable::StageStep {
                self.profile.stage_write(property, array_index, value)
            }

            fn stage_writes(
                &mut self,
                writes: &[$crate::durable::PendingWrite],
            ) -> $crate::durable::StageStep {
                self.profile.stage_writes(writes)
            }

            fn release_staged_write(&mut self, staged: &$crate::durable::SaveWait) {
                self.profile.release_staged_write(staged);
            }

            fn has_staged_write(&self) -> bool {
                self.profile.has_staged_write()
            }

            fn settle_forgotten_writes(&mut self) -> Option<$crate::durable::SaveWait> {
                let tags = self.profile.settle_forgotten_writes();
                let output = self.output.settle();
                match (tags, output) {
                    (Some(tags), Some(output)) => Some(tags.and(output)),
                    (tags, output) => tags.or(output),
                }
            }
        }
    };
}

pub(super) use impl_color_durable;
