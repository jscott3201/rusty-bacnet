//! One shared Tags owner for profile-bearing objects.

use crate::prelude::*;
use alloc::sync::Arc;
#[cfg(feature = "durable-writes")]
use core::time::Duration;

use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::{ObjectProfile, TagsPersistence, TagsSnapshot, write_tags};
use crate::common;
use crate::durable::SaveWriter;
#[cfg(feature = "durable-writes")]
use crate::durable::staged;
use crate::durable::staged::StagedSaves;
#[cfg(feature = "durable-writes")]
use crate::durable::{DurableWrites, PendingWrite, SaveWait, StageStep};
use crate::property_metadata::PropertyMetadata;
use crate::snapshot::tags::fits;
use crate::write_config::description;

/// What a saved write leaves: the property it sets, as storage holds it.
enum Next {
    Tags(Vec<BACnetNameValue>),
    Description(String),
    /// A staged step's Tags write with an index: the change it makes to the
    /// Tags served as it is taken (#2526). A staged step holds only what its
    /// write sets; for a whole array or a Description, that is what it
    /// leaves.
    #[cfg(feature = "durable-writes")]
    Staged(TagsChange),
}

/// What a Tags write with an index sets (#2526).
#[cfg(feature = "durable-writes")]
enum TagsChange {
    /// The size an index-0 write gives the array.
    Size(usize),
    /// The tag an element write puts at its zero-based slot.
    Tag(usize, BACnetNameValue),
}

#[cfg(feature = "durable-writes")]
impl TagsChange {
    /// The change a write at `index` makes, given `tags`, the array it
    /// leaves: `None` for a whole write.
    fn of(index: Option<u32>, tags: &[BACnetNameValue]) -> Option<Self> {
        match index? {
            0 => Some(Self::Size(tags.len())),
            index => {
                let slot = usize::try_from(index.checked_sub(1)?).ok()?;
                Some(Self::Tag(slot, tags.get(slot)?.clone()))
            }
        }
    }

    /// Make the change on `tags`, the array the write was worked out on.
    fn make(self, tags: &mut Vec<BACnetNameValue>) {
        match self {
            Self::Size(size) => tags.resize_with(size, || BACnetNameValue::semantic("")),
            Self::Tag(slot, tag) => {
                debug_assert!(slot < tags.len(), "a staged element past the Tags served");
                if let Some(served) = tags.get_mut(slot) {
                    *served = tag;
                }
            }
        }
    }
}

/// A request's staged steps each hold what their write sets, not the state
/// it leaves, so its stage holds the Tags a constant number of times however
/// many writes it makes (#2526): the stage works the steps out on one
/// working copy, and each write is made on the Tags served as it is taken.
type Storage = StagedSaves<TagsSnapshot, Next>;

/// Provisioned served rows, independent saved override, and sole writer.
#[derive(Default)]
pub(crate) struct ProfileState {
    profile: ObjectProfile,
    writes: u64,
    /// The storage, on an object built with one: boxed, so an object
    /// without storage pays one pointer for it.
    kept: Option<Box<Kept>>,
}

/// An object's storage and what it holds.
struct Kept {
    /// What storage holds for the state the object serves: the Tags a write
    /// set, and the Description on an object that keeps it here.
    saved: TagsSnapshot,
    /// Whether the object keeps a written Description here (#2364): a
    /// Device, a Lighting Output and a Binary Lighting Output do.
    keeps_description: bool,
    storage: Storage,
}

/// Both application clones and COV copies retain served data but have no
/// persistence owner, saved override authority, pending stage or settlement.
impl Clone for ProfileState {
    fn clone(&self) -> Self {
        Self {
            profile: self.profile.clone(),
            ..Self::default()
        }
    }
}

/// Put what a write leaves into `snapshot`.
fn apply(snapshot: &mut TagsSnapshot, next: &Next) {
    match next {
        Next::Tags(tags) => snapshot.tags = Some(tags.clone()),
        Next::Description(text) => snapshot.description = Some(text.clone()),
        #[cfg(feature = "durable-writes")]
        Next::Staged(_) => unreachable!("{WHOLE}"),
    }
}

/// Why only a staged step holds a [`TagsChange`]: every other write is
/// worked out, and saved, as the whole state it leaves.
#[cfg(feature = "durable-writes")]
const WHOLE: &str = "a write is worked out as the whole state it leaves";

impl ProfileState {
    /// A profile that keeps written Tags in `persistence`, and with
    /// `keeps_description` a written Description too (#2364), starting from
    /// what is saved there for `oid`. An object that doesn't keep its
    /// Description here refuses storage holding one, and never saves one,
    /// so its file stays version 1.
    pub(crate) fn persistent(
        oid: ObjectIdentifier,
        persistence: Arc<dyn TagsPersistence>,
        keeps_description: bool,
    ) -> Result<Self, Error> {
        let snapshot = persistence.load(oid)?.unwrap_or_default();
        // As the file backend refuses the same Tags in a file.
        crate::snapshot::tags::check_loaded(&snapshot)
            .map_err(|error| Error::from(error.in_format("Tags snapshot")))?;
        if !keeps_description && snapshot.description.is_some() {
            return Err(Error::Encoding(
                "saved Tags hold a Description, which this object type doesn't keep there".into(),
            ));
        }
        let writer = SaveWriter::new(
            format!("bacnet-tags-{oid}-save"),
            move |snapshot: &TagsSnapshot| persistence.save(oid, snapshot),
            move |_, result| {
                if let Err(error) = result {
                    log_warn!(object = %oid, %error, "Failed to save object Tags");
                }
            },
        );
        Ok(Self {
            kept: Some(Box::new(Kept {
                saved: snapshot,
                keeps_description,
                storage: Storage::new(writer),
            })),
            ..Self::default()
        })
    }

    pub(crate) fn provision(&mut self, mut profile: ObjectProfile) -> Result<(), Error> {
        profile.check()?;
        // Validate first: refusal must not release another request's stage.
        self.with_storage(Storage::drop_staged);
        if profile.tags.is_some()
            && let Some(saved) = self.kept.as_ref().and_then(|kept| kept.saved.tags.as_ref())
        {
            profile.tags = Some(saved.clone());
        }
        self.profile = profile;
        self.writes = self.writes.wrapping_add(1);
        Ok(())
    }

    pub(crate) fn profile(&self) -> &ObjectProfile {
        &self.profile
    }

    pub(crate) fn metadata(&self) -> impl Iterator<Item = PropertyMetadata> + use<> {
        self.profile.metadata()
    }

    pub(crate) fn read(
        &self,
        property: P,
        index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        self.profile.read(property, index)
    }

    #[cfg(feature = "durable-writes")]
    pub(crate) fn capability(&mut self) -> Option<&mut dyn DurableWrites> {
        self.kept.is_some().then_some(self)
    }

    pub(crate) fn wait_for_saves(&self) {
        if let Some(kept) = &self.kept {
            kept.storage.wait_idle();
        }
    }

    /// The Description storage holds, one a write set now or before a
    /// restart (#2364): the object serves it, and its setter leaves it
    /// alone. `None` without storage, and on an object that doesn't keep
    /// its Description here.
    pub(crate) fn saved_description(&self) -> Option<&str> {
        self.kept.as_ref()?.saved.description.as_deref()
    }

    /// Whether storage keeps a write of `value` to `property`: Tags while
    /// the row is provisioned, and, on an object that keeps it here, a
    /// Description other than a NULL, which changes nothing.
    fn keeps(&self, property: P, value: &PropertyValue) -> bool {
        let Some(kept) = &self.kept else {
            return false;
        };
        match property {
            P::TAGS => self.profile.tags.is_some(),
            P::DESCRIPTION => kept.keeps_description && !description::null_write(property, value),
            _ => false,
        }
    }

    fn install(&mut self, next: Next) {
        match next {
            Next::Tags(tags) => {
                if let Some(kept) = &mut self.kept {
                    kept.saved.tags = Some(tags.clone());
                }
                self.profile.tags = Some(tags);
            }
            // Only an object with storage keeps one.
            Next::Description(text) => {
                if let Some(kept) = &mut self.kept {
                    kept.saved.description = Some(text);
                }
            }
            // A staged step's change, made on the Tags served: the array the
            // stage made it on, since no other write came between (#2526).
            #[cfg(feature = "durable-writes")]
            Next::Staged(change) => {
                let mut tags = self.profile.tags.take().unwrap_or_default();
                change.make(&mut tags);
                return self.install(Next::Tags(tags));
            }
        }
        self.writes = self.writes.wrapping_add(1);
    }

    /// What a write of `value` to `property`, Tags or a kept Description,
    /// leaves on top of `staged_tags` and `staged_text`, the Tags and the
    /// Description the earlier steps of its request leave (`None` for one
    /// they don't set), or its refusal. With storage, what the write leaves
    /// must fit a save beside the rest of what storage would then hold.
    fn next(
        &self,
        staged_tags: Option<&[BACnetNameValue]>,
        staged_text: Option<&str>,
        property: P,
        index: Option<u32>,
        value: &PropertyValue,
    ) -> Result<Next, Error> {
        let next = if property == P::DESCRIPTION {
            Next::Description(description::written(value)?)
        } else {
            let current = staged_tags.or(self.profile.tags.as_deref());
            let mut tags = match index {
                Some(_) => current.unwrap_or_default().to_vec(),
                None => Vec::new(),
            };
            write_tags(&mut tags, index, value)?;
            Next::Tags(tags)
        };
        if let Some(kept) = &self.kept {
            let saved = &kept.saved;
            match &next {
                Next::Tags(tags) => fits(
                    staged_text.or(saved.description.as_deref()),
                    Some(tags.as_slice()),
                ),
                Next::Description(text) => {
                    fits(Some(text.as_str()), staged_tags.or(saved.tags.as_deref()))
                }
                #[cfg(feature = "durable-writes")]
                Next::Staged(_) => unreachable!("{WHOLE}"),
            }?;
        }
        Ok(next)
    }

    pub(crate) fn write(
        &mut self,
        property: P,
        index: Option<u32>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        if property != P::TAGS {
            return self.profile.write(property, index, value);
        }
        if self.profile.tags.is_none() {
            return Some(Err(common::unknown_property_error()));
        }
        Some(self.write_kept(P::TAGS, index, value))
    }

    /// A write of `value` to Description on an object that keeps it here
    /// (#2364): saved before `description` takes it, and refused with
    /// DEVICE / OPERATIONAL_PROBLEM, the old text kept, when the save
    /// fails. `None` when storage keeps nothing for the write (no storage,
    /// another kind of object, or a NULL): the object then makes it as it
    /// always has.
    pub(crate) fn write_description(
        &mut self,
        description: &mut String,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        if !self.keeps(P::DESCRIPTION, value) {
            return None;
        }
        let result = self.write_kept(P::DESCRIPTION, None, value);
        if result.is_ok()
            && let Some(text) = self.saved_description()
        {
            *description = text.to_owned();
        }
        Some(result)
    }

    /// Make a kept write, saving it before the object serves it. A staged
    /// write takes the state already saved; any other saves now, where it
    /// is. A write that cannot be saved is refused with DEVICE /
    /// OPERATIONAL_PROBLEM and the old value stays.
    fn write_kept(
        &mut self,
        property: P,
        index: Option<u32>,
        value: &PropertyValue,
    ) -> Result<(), Error> {
        let refused =
            |_| common::protocol_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM);
        let base = self.writes;
        if let Some(kept) = &mut self.kept
            && let Some(claimed) = kept.storage.claim(property, index, value, base)
        {
            // Install before correcting, so the steps the request has still
            // to take keep the state served now.
            let result = claimed.map(|next| self.install(next)).map_err(refused);
            self.with_storage(|_| ());
            return result;
        }
        // A write the object refuses leaves a staged one alone (#1424).
        let next = self.next(None, None, property, index, value)?;
        self.with_storage(Storage::drop_staged);
        if let Some(kept) = &mut self.kept {
            let mut snapshot = kept.saved.clone();
            apply(&mut snapshot, &next);
            kept.storage.save_now(snapshot).map_err(refused)?;
        }
        self.install(next);
        Ok(())
    }

    /// Drop a Description a write saved (#2364), so storage keeps none and
    /// the object's setter applies again; the object goes on serving the
    /// text until then. Saves where it is and waits for the save; one that
    /// fails keeps the written Description and returns storage's error.
    /// Nothing to do without storage or a saved Description.
    pub(crate) fn clear_written_description(&mut self) -> Result<(), Error> {
        let Some(kept) = &mut self.kept else {
            return Ok(());
        };
        if kept.saved.description.is_none() {
            return Ok(());
        }
        kept.storage.save_now(TagsSnapshot {
            tags: kept.saved.tags.clone(),
            description: None,
        })?;
        kept.saved.description = None;
        // A write staged before this gives way, and keeps the cleared state
        // as what is served.
        self.writes = self.writes.wrapping_add(1);
        self.with_storage(|_| ());
        Ok(())
    }

    /// Run `f` on storage; then, if `f` dropped a staged write, queue the
    /// save that puts storage back to the served state. `None` without
    /// storage.
    fn with_storage<R>(&mut self, f: impl FnOnce(&mut Storage) -> R) -> Option<R> {
        let Kept { saved, storage, .. } = &mut **self.kept.as_mut()?;
        let result = f(storage);
        storage.correct(|| saved.clone());
        Some(result)
    }
}

#[cfg(feature = "durable-writes")]
impl DurableWrites for ProfileState {
    fn stage_write(
        &mut self,
        property: P,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> StageStep {
        self.stage_writes(&[PendingWrite {
            property,
            array_index,
            value: value.clone(),
        }])
    }

    /// Fold the request's Tags and kept Description writes into one staged
    /// save; each leaves its property on top of the ones before it.
    fn stage_writes(&mut self, writes: &[PendingWrite]) -> StageStep {
        let kept = |write: &PendingWrite| self.keeps(write.property, &write.value);
        if !writes.iter().any(kept) {
            return StageStep::Skip;
        }
        if let Some(wait) = self.with_storage(Storage::busy).flatten() {
            return StageStep::Busy(wait);
        }
        // One working copy of the Tags and the Description the steps so far
        // leave; a step holds only what its write sets (#2526).
        let (mut tags, mut text) = (None, None);
        let steps = staged::steps(writes, |_, write| {
            if !self.keeps(write.property, &write.value) {
                return Ok(None);
            }
            if write.property == P::DESCRIPTION && write.array_index.is_some() {
                return Err(common::property_is_not_an_array_error());
            }
            let (property, index, value) = (write.property, write.array_index, &write.value);
            Ok(Some(
                match self.next(tags.as_deref(), text.as_deref(), property, index, value)? {
                    Next::Tags(next) => {
                        let step = match TagsChange::of(index, &next) {
                            Some(change) => Next::Staged(change),
                            None => Next::Tags(next.clone()),
                        };
                        tags = Some(next);
                        step
                    }
                    Next::Description(next) => {
                        text = Some(next.clone());
                        Next::Description(next)
                    }
                    Next::Staged(_) => unreachable!("{WHOLE}"),
                },
            ))
        });
        if steps.is_empty() {
            return StageStep::Skip;
        }
        let base = self.writes;
        let Kept { saved, storage, .. } = &mut **self.kept.as_mut().expect("checked above");
        let snapshot = TagsSnapshot {
            tags: tags.or_else(|| saved.tags.clone()),
            description: text.or_else(|| saved.description.clone()),
        };
        storage.stage_built(steps, base, snapshot, saved.clone())
    }

    fn release_staged_write(&mut self, staged: &SaveWait) {
        self.with_storage(|storage| storage.release(staged));
    }

    fn has_staged_write(&self) -> bool {
        self.kept
            .as_ref()
            .is_some_and(|kept| kept.storage.is_staged())
    }

    fn settle_forgotten_writes(&mut self) -> Option<SaveWait> {
        let Kept { saved, storage, .. } = &mut **self.kept.as_mut()?;
        Some(storage.drop_forgotten(|| saved.clone()))
    }

    fn expire_staged(&mut self, now: Duration) {
        self.with_storage(|storage| storage.expire(now));
    }
}
