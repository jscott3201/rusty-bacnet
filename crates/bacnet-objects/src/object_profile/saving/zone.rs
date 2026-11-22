//! The halves of the Device clock's zone a peer writes, which a Device's
//! Tags storage keeps (#2395): the write, and the host's reset, which is
//! newer than any peer's write and drops them.

use super::*;

impl ProfileState {
    /// Whether the object's clock takes a peer's zone writes now (#2395), so
    /// storage keeps the halves written.
    pub(crate) fn take_zone_writes(&mut self, on: bool) {
        if let Some(kept) = &mut self.kept {
            kept.zone_writes = on;
        }
    }

    /// A write of a half of the Device clock's zone on a Device that keeps
    /// it here (#2395): saved before the write is answered, and refused
    /// with DEVICE / OPERATIONAL_PROBLEM when the save fails. `None` when
    /// storage keeps nothing for the write (no storage, another kind of
    /// object, a clock that takes no zone writes, or a NULL).
    pub(crate) fn write_zone(
        &mut self,
        property: P,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        self.keeps(property, value)
            .then(|| self.write_kept(property, None, value))
    }

    /// The zone halves a write of `value` to `property` leaves, if it is a
    /// half; `None` for any other property.
    pub(super) fn zone_next(
        &self,
        property: P,
        value: &PropertyValue,
    ) -> Option<Result<Next, Error>> {
        zone_half(property).then(|| {
            self.saved_zone()
                .with_write(property, value)
                .map(Next::Zone)
        })
    }

    /// Stage the drop of the zone halves storage keeps, for the host's own
    /// zone change, which is newer than any peer's write (#2395): one save
    /// of the state served without them, queued to run off the database
    /// guard. `Skip` when storage keeps none, or the clock takes no zone
    /// writes (a half held then stays unapplied, and kept).
    /// [`reset_written_zone`](Self::reset_written_zone) then takes it.
    ///
    /// The step is keyed on a NULL to UTC_Offset, a write that never
    /// reaches storage (it is refused as the wrong datatype before), so no
    /// other write takes it.
    #[cfg(feature = "durable-writes")]
    pub(crate) fn stage_zone_reset(&mut self) -> StageStep {
        if !self.holds_zone() {
            return StageStep::Skip;
        }
        if let Some(wait) = self.with_storage(Storage::busy).flatten() {
            return StageStep::Busy(wait);
        }
        let base = self.writes;
        let Kept { saved, storage, .. } = &mut **self.kept.as_mut().expect("checked above");
        let steps = staged::steps(&[zone_reset()], |_, _| {
            Ok(Some(Next::Zone(WrittenZone::default())))
        });
        let snapshot = TagsSnapshot {
            zone: WrittenZone::default(),
            ..saved.clone()
        };
        storage.stage_built(steps, base, snapshot, saved.clone())
    }

    /// Drop the zone halves storage keeps (#2395): take the save
    /// `stage_zone_reset` staged, or save where it
    /// is and wait for it when nothing is staged. A save that fails keeps
    /// the halves and returns storage's error. Nothing to do without storage,
    /// without a half held, or while the clock takes no zone writes.
    pub(crate) fn reset_written_zone(&mut self) -> Result<(), Error> {
        if !self.holds_zone() {
            return Ok(());
        }
        #[cfg(feature = "durable-writes")]
        {
            let base = self.writes;
            let reset = zone_reset();
            let kept = self.kept.as_mut().expect("holds a zone");
            if let Some(claimed) = kept.storage.claim(reset.property, None, &reset.value, base) {
                let next = claimed?;
                self.take(next, reset.property, None, &reset.value);
                return Ok(());
            }
        }
        let kept = self.kept.as_mut().expect("holds a zone");
        kept.storage.save_now(TagsSnapshot {
            zone: WrittenZone::default(),
            ..kept.saved.clone()
        })?;
        kept.saved.zone = WrittenZone::default();
        // A write staged before this gives way, as for the Description.
        self.writes = self.writes.wrapping_add(1);
        self.with_storage(|_| ());
        Ok(())
    }

    /// Whether storage holds a zone half the clock applies.
    fn holds_zone(&self) -> bool {
        self.kept
            .as_ref()
            .is_some_and(|kept| kept.zone_writes && !kept.saved.zone.is_empty())
    }
}

/// The key a host's zone reset is staged under: a NULL to UTC_Offset, which
/// no write that reaches storage carries.
#[cfg(feature = "durable-writes")]
fn zone_reset() -> PendingWrite {
    PendingWrite {
        property: P::UTC_OFFSET,
        array_index: None,
        value: PropertyValue::Null,
    }
}
