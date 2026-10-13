//! One shared Tags owner for profile-bearing objects.

use std::sync::Arc;
use std::time::Duration;

use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::persistence::encode_snapshot;
use super::{write_tags, ObjectProfile, TagsPersistence, TagsSnapshot};
use crate::common;
use crate::durable::staged::{self, StagedSaves};
use crate::durable::{DurableWrites, PendingWrite, SaveWait, SaveWriter, StageStep};
use crate::property_metadata::PropertyMetadata;

type Storage = StagedSaves<TagsSnapshot, Vec<BACnetNameValue>>;

/// Provisioned served rows, independent saved override, and sole writer.
#[derive(Default)]
pub(crate) struct ProfileState {
    profile: ObjectProfile,
    saved: Option<Vec<BACnetNameValue>>,
    writes: u64,
    storage: Option<Storage>,
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

impl ProfileState {
    pub(crate) fn persistent(
        oid: ObjectIdentifier,
        persistence: Arc<dyn TagsPersistence>,
    ) -> Result<Self, Error> {
        let snapshot = persistence.load(oid)?.unwrap_or_default();
        encode_snapshot(&snapshot)?;
        let writer = SaveWriter::new(
            format!("bacnet-tags-{oid}-save"),
            move |snapshot: &TagsSnapshot| persistence.save(oid, snapshot),
            move |_, result| {
                if let Err(error) = result {
                    tracing::warn!(object = %oid, %error, "Failed to save object Tags");
                }
            },
        );
        Ok(Self {
            saved: snapshot.tags,
            storage: Some(Storage::new(writer)),
            ..Self::default()
        })
    }

    pub(crate) fn provision(&mut self, mut profile: ObjectProfile) -> Result<(), Error> {
        profile.check()?;
        // Validate first: refusal must not release another request's stage.
        self.with_storage(Storage::drop_staged);
        if profile.tags.is_some() {
            if let Some(saved) = &self.saved {
                profile.tags = Some(saved.clone());
            }
        }
        self.profile = profile;
        self.writes = self.writes.wrapping_add(1);
        Ok(())
    }

    pub(crate) fn profile(&self) -> &ObjectProfile {
        &self.profile
    }

    pub(crate) fn metadata(&self) -> impl Iterator<Item = PropertyMetadata> {
        self.profile.metadata()
    }

    pub(crate) fn read(
        &self,
        property: P,
        index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        self.profile.read(property, index)
    }

    pub(crate) fn capability(&mut self) -> Option<&mut dyn DurableWrites> {
        self.storage.is_some().then_some(self)
    }

    pub(crate) fn wait_for_saves(&self) {
        if let Some(storage) = &self.storage {
            storage.wait_idle();
        }
    }

    fn snapshot(&self) -> TagsSnapshot {
        TagsSnapshot {
            tags: self.saved.clone(),
        }
    }

    fn install(&mut self, tags: Vec<BACnetNameValue>) {
        if self.storage.is_some() {
            self.saved = Some(tags.clone());
        }
        self.profile.tags = Some(tags);
        self.writes = self.writes.wrapping_add(1);
    }

    fn next(
        &self,
        current: &[BACnetNameValue],
        index: Option<u32>,
        value: &PropertyValue,
    ) -> Result<Vec<BACnetNameValue>, Error> {
        let mut tags = if index.is_some() {
            current.to_vec()
        } else {
            Vec::new()
        };
        write_tags(&mut tags, index, value)?;
        if self.storage.is_some() {
            encode_snapshot(&TagsSnapshot {
                tags: Some(tags.clone()),
            })?;
        }
        Ok(tags)
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
        Some(self.write_tags(index, value))
    }

    fn write_tags(&mut self, index: Option<u32>, value: &PropertyValue) -> Result<(), Error> {
        if self.profile.tags.is_none() {
            return Err(common::unknown_property_error());
        }
        let refused =
            |_| common::protocol_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM);
        if let Some(mut storage) = self.storage.take() {
            let claimed = storage.claim(P::TAGS, index, value, self.writes);
            // Restore attachment before install so it records saved precedence.
            self.storage = Some(storage);
            if let Some(claimed) = claimed {
                let result = claimed.map(|tags| self.install(tags)).map_err(refused);
                self.with_storage(|_| ());
                return result;
            }
        }
        let next = self.next(
            self.profile.tags.as_deref().expect("checked above"),
            index,
            value,
        )?;
        self.with_storage(Storage::drop_staged);
        if let Some(storage) = &mut self.storage {
            storage
                .save_now(TagsSnapshot {
                    tags: Some(next.clone()),
                })
                .map_err(refused)?;
        }
        self.install(next);
        Ok(())
    }

    fn with_storage<R>(&mut self, f: impl FnOnce(&mut Storage) -> R) -> Option<R> {
        let mut storage = self.storage.take()?;
        let result = f(&mut storage);
        storage.correct(|| self.snapshot());
        self.storage = Some(storage);
        Some(result)
    }

    pub(crate) fn expire(&mut self, now: Duration) {
        self.with_storage(|storage| storage.expire(now));
    }
}

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

    fn stage_writes(&mut self, writes: &[PendingWrite]) -> StageStep {
        if self.storage.is_none()
            || self.profile.tags.is_none()
            || !writes.iter().any(|write| write.property == P::TAGS)
        {
            return StageStep::Skip;
        }
        if let Some(wait) = self.with_storage(Storage::busy).flatten() {
            return StageStep::Busy(wait);
        }
        let steps = staged::steps::<Vec<BACnetNameValue>>(writes, |earlier, write| {
            if write.property != P::TAGS {
                return Ok(None);
            }
            let current = earlier
                .last()
                .map(|step| step.next.as_slice())
                .unwrap_or_else(|| self.profile.tags.as_deref().expect("checked above"));
            self.next(current, write.array_index, &write.value)
                .map(Some)
        });
        let served = self.snapshot();
        self.storage.as_mut().expect("checked above").stage(
            steps,
            self.writes,
            served,
            |snapshot, tags| snapshot.tags = Some(tags.clone()),
        )
    }

    fn release_staged_write(&mut self, staged: &SaveWait) {
        self.with_storage(|storage| storage.release(staged));
    }

    fn has_staged_write(&self) -> bool {
        self.storage.as_ref().is_some_and(Storage::is_staged)
    }

    fn settle_forgotten_writes(&mut self) -> Option<SaveWait> {
        let mut storage = self.storage.take()?;
        let wait = storage.drop_forgotten(|| self.snapshot());
        self.storage = Some(storage);
        Some(wait)
    }
}
