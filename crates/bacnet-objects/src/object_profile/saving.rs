//! One shared Tags owner for profile-bearing objects.

use crate::prelude::*;
use alloc::sync::Arc;
#[cfg(feature = "durable-writes")]
use bytes::BytesMut;
#[cfg(feature = "durable-writes")]
use core::time::Duration;

use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

#[cfg(feature = "durable-writes")]
use super::slot;
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
#[cfg(feature = "durable-writes")]
use crate::snapshot::tags::{fits_len, tags_len};
use crate::write_config::description;

/// What a saved write leaves: the property it sets, as storage holds it.
enum Next {
    Tags(Vec<BACnetNameValue>),
    Description(String),
    /// A staged step: nothing beyond its write, which its claim makes again
    /// on the state served (#2546).
    #[cfg(feature = "durable-writes")]
    Staged,
}

/// A request's staged steps hold nothing beyond their writes (#2546). The
/// stage makes each write in turn on one working copy of the Tags and the
/// Description, counting the octets the Tags encode to as it goes, and
/// saves that copy once; each step's claim makes its write again on the
/// state served, which is the state the stage made it on. So a staged
/// write costs what it changes, at its stage and as it is made, and a
/// request holds the state a constant number of times however many writes
/// it makes, a whole array's included (#2526).
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
        Next::Staged => unreachable!("{AGAIN}"),
    }
}

/// Why only a claim sees [`Next::Staged`]: every other write is worked
/// out, and saved, as the whole state it leaves.
#[cfg(feature = "durable-writes")]
const AGAIN: &str = "a staged step's write is made again as it is taken";

/// Whether a staged write of Tags at `index` can't be made on `served`, the
/// Tags served as it is taken: an element past their end. Nothing else a
/// stage took depends on the state, and every change to the served Tags
/// counts as a write, so a step is taken only while none came between: this
/// holds no step back. Should one ever be stale, its write isn't taken and
/// is refused as made on the Tags served, rather than reported made
/// (#2526).
#[cfg(feature = "durable-writes")]
fn unmakeable(property: P, index: Option<u32>, served: Option<&[BACnetNameValue]>) -> bool {
    let len = served.map_or(0, <[_]>::len);
    property == P::TAGS && index.is_some_and(|index| index > 0 && slot(index, len).is_err())
}

/// Without durable-writes no step is staged, so none is stale.
#[cfg(not(feature = "durable-writes"))]
fn unmakeable(_property: P, _index: Option<u32>, _served: Option<&[BACnetNameValue]>) -> bool {
    false
}

/// The Tags a stage works its writes out on, and the octets they encode to
/// ([`tags_len`]), kept current write by write (#2546).
#[cfg(feature = "durable-writes")]
struct Working {
    tags: Vec<BACnetNameValue>,
    len: usize,
}

#[cfg(feature = "durable-writes")]
impl Working {
    /// `tags`, counted.
    fn of(tags: Vec<BACnetNameValue>, scratch: &mut BytesMut) -> Result<Self, Error> {
        let len = tags_len(&tags, scratch)?;
        Ok(Self { tags, len })
    }

    /// Make a write of `value` at `index` in place, as [`write_tags`] makes
    /// it, if the Tags it leaves fit a save beside `description`. A write
    /// refused either way leaves the Tags as they were: [`write_tags`]
    /// changes nothing it refuses, and a write that doesn't fit is undone.
    fn write(
        &mut self,
        index: Option<u32>,
        value: &PropertyValue,
        description: Option<&str>,
        scratch: &mut BytesMut,
    ) -> Result<(), Error> {
        match index {
            None => {
                let mut written = Vec::new();
                write_tags(&mut written, None, value)?;
                let len = tags_len(&written, scratch)?;
                fits_len(description, Some(len))?;
                *self = Self { tags: written, len };
            }
            Some(0) => {
                // A smaller size drops the tags past it: they come out first,
                // to be counted, and to go back should the write be refused.
                let before = self.tags.len();
                let kept = match value {
                    PropertyValue::Unsigned(size) => {
                        usize::try_from(*size).map_or(before, |size| size.min(before))
                    }
                    _ => before,
                };
                let dropped = self.tags.split_off(kept);
                let len = write_tags(&mut self.tags, Some(0), value).and_then(|()| {
                    let added = tags_len(&self.tags[kept..], scratch)?;
                    let len = self.len - tags_len(&dropped, scratch)? + added;
                    fits_len(description, Some(len)).map(|()| len)
                });
                match len {
                    Ok(len) => self.len = len,
                    Err(error) => {
                        self.tags.truncate(kept);
                        self.tags.extend(dropped);
                        return Err(error);
                    }
                }
            }
            Some(index) => {
                // As write_tags checks it first.
                let slot = slot(index, self.tags.len())?;
                let old = self.tags[slot].clone();
                write_tags(&mut self.tags, Some(index), value)?;
                let new = tags_len(core::slice::from_ref(&self.tags[slot]), scratch)?;
                let len = self.len - tags_len(core::slice::from_ref(&old), scratch)? + new;
                if let Err(error) = fits_len(description, Some(len)) {
                    self.tags[slot] = old;
                    return Err(error);
                }
                self.len = len;
            }
        }
        Ok(())
    }
}

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
            #[cfg(feature = "durable-writes")]
            Next::Staged => unreachable!("{AGAIN}"),
        }
        self.writes = self.writes.wrapping_add(1);
    }

    /// Make a staged write of `value` to `property`, Tags at `index` or a
    /// kept Description, whose step a claim has just taken: again, on the
    /// state served, which is the state its stage made it on, so it can't
    /// be refused now; and bring what storage holds for that state, and the
    /// copy a staged write still to be taken keeps of it, up to date in
    /// place (#2546).
    #[cfg(feature = "durable-writes")]
    fn again(&mut self, property: P, index: Option<u32>, value: &PropertyValue) {
        let Some(kept) = self.kept.as_deref_mut() else {
            return;
        };
        let Kept { saved, storage, .. } = kept;
        let follows = |kept: &TagsSnapshot, saved: &TagsSnapshot| {
            debug_assert!(kept == saved, "the staged write keeps the state served");
        };
        if property == P::DESCRIPTION {
            let text = description::written(value);
            debug_assert!(text.is_ok(), "a staged Description its stage took");
            let Ok(text) = text else { return };
            saved.description = Some(text.clone());
            storage.correct_with(
                || saved.clone(),
                |kept| {
                    kept.description = Some(text);
                    follows(kept, saved);
                },
            );
        } else {
            // A write the stage made on these Tags, so one they take again;
            // and the Tags storage holds are these, should it hold any.
            let make = |tags: &mut Vec<BACnetNameValue>| {
                let made = write_tags(tags, index, value);
                debug_assert!(made.is_ok(), "a staged Tags write its stage took");
            };
            let served = self.profile.tags.get_or_insert_default();
            make(served);
            match &mut saved.tags {
                Some(tags) => make(tags),
                None => saved.tags = Some(served.clone()),
            }
            storage.correct_with(
                || saved.clone(),
                |kept| {
                    match &mut kept.tags {
                        Some(tags) => make(tags),
                        None => kept.tags.clone_from(&saved.tags),
                    }
                    follows(kept, saved);
                },
            );
        }
        self.writes = self.writes.wrapping_add(1);
    }

    /// Take the step a claim took for a write of `value` to `property` at
    /// `index`: a staged write is made again (by `again`). A step holding
    /// the state its write leaves installs it, as a write made where it is
    /// does. No stage makes one now; the path keeps `write_kept` the size
    /// the firmware images, which claim nothing, are measured at.
    #[cfg_attr(not(feature = "durable-writes"), expect(unused_variables))]
    fn take(&mut self, next: Next, property: P, index: Option<u32>, value: &PropertyValue) {
        #[cfg(feature = "durable-writes")]
        if let Next::Staged = next {
            return self.again(property, index, value);
        }
        self.install(next);
        self.with_storage(|_| ());
    }

    /// What a write of `value` to `property`, Tags or a kept Description,
    /// leaves on top of `staged_tags` and `staged_text` (`None` for the
    /// Tags served and the Description saved), or its refusal. With
    /// storage, what the write leaves must fit a save beside the rest of
    /// what storage would then hold.
    ///
    /// A write made where it is passes `None` for both, and is the only
    /// caller: a stage works its writes out in place (`Working`, #2546).
    /// The two stay because the firmware images lay `write_kept` out up to
    /// 36 octets longer without them.
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
                Next::Staged => unreachable!("{AGAIN}"),
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
            && let Some(claimed) = kept
                .storage
                .claim_unless(property, index, value, base, |_| {
                    unmakeable(property, index, self.profile.tags.as_deref())
                })
        {
            // A save that failed dropped every step without leaving storage
            // anything to correct.
            return claimed
                .map(|next| self.take(next, property, index, value))
                .map_err(refused);
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
        // The Description the steps so far leave, and the Tags they work on,
        // copied from those served when a step first writes them (#2546):
        // the Tags the steps leave once one of them has been made. And the
        // octets the saved Tags encode to, counted once a step needs them.
        let (mut tags, mut text): (Option<Working>, Option<String>) = (None, None);
        let (mut made, mut saved_len) = (false, None);
        let mut scratch = BytesMut::new();
        let steps = staged::steps(writes, |_, write| {
            if !self.keeps(write.property, &write.value) {
                return Ok(None);
            }
            if write.property == P::DESCRIPTION && write.array_index.is_some() {
                return Err(common::property_is_not_an_array_error());
            }
            let saved = &self.kept.as_ref().expect("a kept write").saved;
            if write.property == P::DESCRIPTION {
                // What the write leaves must fit a save beside the Tags the
                // steps so far leave, or those storage holds.
                let written = description::written(&write.value)?;
                let tags_len = match (tags.as_ref().filter(|_| made), saved_len) {
                    (Some(working), _) => Some(working.len),
                    (None, Some(len)) => len,
                    (None, None) => {
                        let len = saved
                            .tags
                            .as_deref()
                            .map(|tags| tags_len(tags, &mut scratch));
                        *saved_len.insert(len.transpose()?)
                    }
                };
                fits_len(Some(&written), tags_len)?;
                text = Some(written);
            } else {
                let working = match &mut tags {
                    Some(working) => working,
                    None => {
                        let served = self.profile.tags.clone().unwrap_or_default();
                        tags.insert(Working::of(served, &mut scratch)?)
                    }
                };
                let description = text.as_deref().or(saved.description.as_deref());
                working.write(write.array_index, &write.value, description, &mut scratch)?;
                made = true;
            }
            Ok(Some(Next::Staged))
        });
        if steps.is_empty() {
            return StageStep::Skip;
        }
        let base = self.writes;
        let Kept { saved, storage, .. } = &mut **self.kept.as_mut().expect("checked above");
        let snapshot = TagsSnapshot {
            tags: tags
                .filter(|_| made)
                .map(|working| working.tags)
                .or_else(|| saved.tags.clone()),
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

// The stage's writes, at the cap and made one at a time (#2546).
#[cfg(all(test, feature = "durable-writes"))]
#[path = "saving_cost_tests.rs"]
mod cost_tests;
