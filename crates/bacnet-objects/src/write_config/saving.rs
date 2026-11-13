//! The staged saves a Timer and a Channel share (#2243), on the pattern the
//! [durable module](crate::durable) describes: a write to a property the
//! object keeps stages one save of the configuration the request's writes
//! leave, takes it once the save has landed, and is refused with DEVICE /
//! OPERATIONAL_PROBLEM, keeping the old value, when the save failed. A
//! write nobody staged saves where it is.
//!
//! The object holds its configuration as a [`ConfigWrites::Config`], which
//! also says which parts a write has set; storage keeps just those
//! ([`ConfigWrites::snapshot`]). A step keeps nothing but its write, so a
//! request's stage holds the configuration a constant number of times
//! however many writes it makes (#2526): the stage works the writes out on
//! one working copy and saves the last, and each write is worked out again
//! on the configuration served as it is made.

use alloc::sync::Arc;
use core::borrow::Borrow;
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use core::time::Duration;

#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use super::{WriteConfigPersistence, WriteConfigSnapshot, description};
use crate::common;
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::durable::staged::{self, StagedSaves};
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::durable::{PendingWrite, SaveWait, SaveWriter, StageStep};

/// An object's writer and the write staged on it. A step keeps only its
/// write (#2526).
pub(crate) type Storage = StagedSaves<WriteConfigSnapshot, ()>;

/// The writer that saves `oid`'s configuration in `persistence`, handed
/// it as `S`: the snapshot itself, or one shared with what keeps it.
pub(crate) fn writer<S: Borrow<WriteConfigSnapshot> + Send + 'static>(
    oid: ObjectIdentifier,
    persistence: Arc<dyn WriteConfigPersistence>,
) -> SaveWriter<S> {
    SaveWriter::new(
        format!(
            "bacnet-config-{}-{}-save",
            oid.object_type().to_raw(),
            oid.instance_number()
        ),
        move |snapshot: &S| persistence.save(oid, snapshot.borrow()),
        move |_, result| {
            if let Err(error) = result {
                log_warn!(object = %oid, %error, "Failed to save a written configuration");
            }
        },
    )
}

/// An object whose written configuration storage keeps.
pub(crate) trait ConfigWrites {
    /// The configuration as the object serves it, with which parts a write
    /// has set.
    type Config: Clone + Send + 'static;

    /// The storage, once the object has it.
    fn storage(&mut self) -> &mut Option<Storage>;

    /// The storage, for a look.
    #[cfg(feature = "durable-writes")]
    fn storage_ref(&self) -> Option<&Storage>;

    /// The configuration served now; `None` while the object serves none
    /// (a Timer before its set).
    fn config(&self) -> Option<&Self::Config>;

    /// Serve `config`, what a write leaves, and count the write.
    fn install(&mut self, config: Self::Config);

    /// Configuration changes counted so far, so a staged step is taken only
    /// if nothing changed it since the stage. Every change to what storage
    /// keeps is counted: a write ([`install`](Self::install)) and each
    /// setter. A Channel's learned member datatypes, a cache storage doesn't
    /// keep, are not.
    fn writes(&self) -> u64;

    /// What storage keeps of `config`: the parts a write has set.
    fn snapshot(config: Option<&Self::Config>) -> WriteConfigSnapshot;

    /// Whether storage keeps a write of `property`.
    #[cfg(feature = "durable-writes")]
    fn saves(property: PropertyIdentifier) -> bool;

    /// The configuration `write` leaves on top of `config`, or the
    /// object's refusal of it.
    fn next(config: &Self::Config, write: &PendingWrite) -> Result<Self::Config, Error>;

    /// Mark every part of `config` as one no write has set.
    fn unwrite(config: &mut Self::Config);
}

/// Run `f` on `object`'s storage; then, if `f` dropped a staged write,
/// queue the save that puts storage back to the served configuration.
fn with_storage<T: ConfigWrites, R>(
    object: &mut T,
    f: impl FnOnce(&mut Storage) -> R,
) -> Option<R> {
    let mut storage = object.storage().take()?;
    let result = f(&mut storage);
    storage.correct(|| T::snapshot(object.config()));
    *object.storage() = Some(storage);
    Some(result)
}

/// Make `write`, one storage keeps: work out what it leaves, then take its
/// staged step if it is the next one, or else save that and serve it. A
/// save that fails refuses the write with DEVICE / OPERATIONAL_PROBLEM, and
/// the object keeps serving what it did.
pub(crate) fn write<T: ConfigWrites>(object: &mut T, write: PendingWrite) -> Result<(), Error> {
    let refused = |_| common::protocol_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM);
    let Some(config) = object.config() else {
        return Err(common::unknown_property_error());
    };
    // Worked out on the configuration served now, as a step keeps only its
    // write (#2526). A step is taken only while nothing counted changed the
    // configuration since its stage ([`ConfigWrites::writes`]), so this is
    // what the stage saved for it. A write the object refuses takes no step
    // and leaves another request's staged write alone (#1424).
    let next = T::next(config, &write)?;
    let base = object.writes();
    let claimed = object
        .storage()
        .as_mut()
        .and_then(|storage| storage.claim(write.property, write.array_index, &write.value, base));
    if let Some(claimed) = claimed {
        let taken = claimed.map(|()| object.install(next));
        // Steps the request has still to take keep the configuration served now.
        with_storage(object, |_| ());
        return taken.map_err(refused);
    }
    if object.storage().is_some() {
        // This write will be made, so it supersedes a staged one: storage
        // goes back to the served configuration ahead of its save.
        with_storage(object, Storage::drop_staged);
        let snapshot = T::snapshot(Some(&next));
        let storage = object.storage().as_mut().expect("checked above");
        storage.save_now(snapshot).map_err(refused)?;
    }
    object.install(next);
    Ok(())
}

/// Drop every part a write saved, so storage keeps none and the setters
/// apply again; the configuration served stays until they change it. Saves
/// where it is and waits for the save; one that fails keeps the written
/// parts and returns storage's error. Nothing to do when nothing is written.
pub(crate) fn clear<T: ConfigWrites>(object: &mut T) -> Result<(), Error> {
    let Some(config) = object.config() else {
        return Ok(());
    };
    let mut next = config.clone();
    T::unwrite(&mut next);
    let cleared = T::snapshot(Some(&next));
    if cleared == T::snapshot(object.config()) {
        return Ok(());
    }
    if let Some(storage) = object.storage().as_mut() {
        storage.save_now(cleared)?;
    }
    // Counted, so a step staged from the written configuration gives way.
    object.install(next);
    // A step still staged keeps the cleared configuration as what is served.
    with_storage(object, |_| ());
    Ok(())
}

/// The application changed the configuration: a staged step, worked out
/// from the old one, gives way, and storage goes back to what is served.
pub(crate) fn changed<T: ConfigWrites>(object: &mut T) {
    with_storage(object, Storage::drop_staged);
}

/// Stage the writes a request makes to `object` (see
/// [`DurableWrites::stage_writes`](crate::durable::DurableWrites::stage_writes)):
/// one save of what they leave together (#1423).
#[cfg(feature = "durable-writes")]
pub(crate) fn stage_writes<T: ConfigWrites>(object: &mut T, writes: &[PendingWrite]) -> StageStep {
    // A NULL to Description changes nothing, so it stages nothing (#2364).
    let kept = |write: &PendingWrite| {
        T::saves(write.property) && !description::null_write(write.property, &write.value)
    };
    if object.storage_ref().is_none() || object.config().is_none() || !writes.iter().any(kept) {
        return StageStep::Skip;
    }
    if let Some(wait) = with_storage(object, Storage::busy).flatten() {
        return StageStep::Busy(wait);
    }
    let current = object.config().expect("checked above");
    // One working copy of the configuration the steps so far leave; a step
    // keeps only its write (#2526).
    let mut working = None;
    let steps = staged::steps(writes, |_, write| {
        if !kept(write) {
            return Ok(None);
        }
        working = Some(T::next(working.as_ref().unwrap_or(current), write)?);
        Ok(Some(()))
    });
    // A step was taken exactly when the working copy was set.
    let Some(last) = working else {
        return StageStep::Skip;
    };
    let (snapshot, served) = (T::snapshot(Some(&last)), T::snapshot(object.config()));
    let base = object.writes();
    let storage = object.storage().as_mut().expect("checked above");
    storage.stage_built(steps, base, snapshot, served)
}

/// See [`DurableWrites::release_staged_write`](crate::durable::DurableWrites::release_staged_write).
#[cfg(feature = "durable-writes")]
pub(crate) fn release<T: ConfigWrites>(object: &mut T, staged: &SaveWait) {
    with_storage(object, |storage| storage.release(staged));
}

/// See [`DurableWrites::has_staged_write`](crate::durable::DurableWrites::has_staged_write).
#[cfg(feature = "durable-writes")]
pub(crate) fn has_staged<T: ConfigWrites>(object: &T) -> bool {
    object.storage_ref().is_some_and(Storage::is_staged)
}

/// See [`DurableWrites::settle_forgotten_writes`](crate::durable::DurableWrites::settle_forgotten_writes).
#[cfg(feature = "durable-writes")]
pub(crate) fn settle<T: ConfigWrites>(object: &mut T) -> Option<SaveWait> {
    let mut storage = object.storage().take()?;
    let wait = storage.drop_forgotten(|| T::snapshot(object.config()));
    *object.storage() = Some(storage);
    Some(wait)
}

/// See [`DurableWrites::expire_staged`](crate::durable::DurableWrites::expire_staged).
#[cfg(feature = "durable-writes")]
pub(crate) fn expire<T: ConfigWrites>(object: &mut T, now: Duration) {
    with_storage(object, |storage| storage.expire(now));
}

/// Implement [`DurableWrites`](crate::durable::DurableWrites) for an object
/// that implements [`ConfigWrites`].
macro_rules! impl_config_durable {
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
                $crate::write_config::saving::stage_writes(self, writes)
            }

            fn release_staged_write(&mut self, staged: &$crate::durable::SaveWait) {
                $crate::write_config::saving::release(self, staged);
            }

            fn has_staged_write(&self) -> bool {
                $crate::write_config::saving::has_staged(self)
            }

            fn settle_forgotten_writes(&mut self) -> Option<$crate::durable::SaveWait> {
                $crate::write_config::saving::settle(self)
            }

            fn expire_staged(&mut self, now: core::time::Duration) {
                $crate::write_config::saving::expire(self, now);
            }
        }
    };
}

pub(crate) use impl_config_durable;
