//! A peer's write of the Device's UTC_Offset or Daylight_Savings_Status
//! (#2395), which the host opts in to through its clock
//! ([`ClockReader::takes_zone_writes`]) in a build with the `zone-writes`
//! feature. Without either both stay read-only, and a write of either is
//! refused as before.
//!
//! The written value moves the clock as the host's own zone change does:
//! the other half of the zone stays as the clock holds it, the clock keeps
//! its UTC instant, and a synchronized clock stays synchronized. UTC_Offset
//! takes an INTEGER, a multiple of 15 minutes within -1440 to 1440 (Clause
//! 12.11.25 asks for at least those); Daylight_Savings_Status takes a
//! BOOLEAN. A value of another datatype is refused with INVALID_DATA_TYPE,
//! a NULL included, which the write services then answer as a relinquish
//! that changes nothing (Clause 15.9); an offset off the 15-minute grid or
//! past a day either way with VALUE_OUT_OF_RANGE.
//!
//! A Device built with Tags storage
//! ([`with_tags_persistence`](DeviceObject::with_tags_persistence)) keeps
//! each half a peer writes there, as it keeps a written Description: saved
//! before the write is answered, and refused with DEVICE /
//! OPERATIONAL_PROBLEM when the save fails. The clock is asked first
//! ([`ClockReader::check_zone`]) and moved only once storage has the half
//! ([`ClockReader::write_zone`]), so a refused write never reaches it. The
//! host reads what is kept with [`written_zone`](DeviceObject::written_zone):
//! the bundled server puts it over the zone it starts the clock in, and its
//! own later zone change, being newer, drops it. While the clock takes no
//! zone writes a half held stays in storage, unapplied, and every later
//! save keeps it.
use super::*;
use crate::object_profile::WrittenZone;

#[cfg(feature = "zone-writes")]
impl DeviceObject {
    /// Whether the clock takes a peer's write of the zone.
    pub(super) fn zone_writable(&self) -> bool {
        self.clock
            .as_ref()
            .is_some_and(|clock| clock.takes_zone_writes())
    }

    /// A write of UTC_Offset or Daylight_Savings_Status, when the clock
    /// takes one; `None` for any other property, or while it takes none, so
    /// the write is refused as a read-only property's.
    pub(super) fn write_zone(
        &mut self,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        if !matches!(
            property,
            PropertyIdentifier::UTC_OFFSET | PropertyIdentifier::DAYLIGHT_SAVINGS_STATUS
        ) || !self.zone_writable()
        {
            return None;
        }
        let clock = Arc::clone(self.clock.as_ref()?);
        Some(self.write_zone_on(&*clock, property, value))
    }

    /// Check the zone the write leaves with the clock, save the half, then
    /// move the clock, which took the zone a moment ago under the same
    /// database guard.
    fn write_zone_on(
        &mut self,
        clock: &dyn ClockReader,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Result<(), Error> {
        let frame = clock.read_clock().ok_or(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ))?;
        let served = (frame.utc_offset, frame.daylight_savings_status);
        let (offset, summer) = WrittenZone::default()
            .with_write(property, value)?
            .over(served);
        clock.check_zone(offset, summer)?;
        if let Some(saved) = self.profile.write_zone(property, value) {
            saved?;
        }
        clock.write_zone(offset, summer)
    }
}

impl DeviceObject {
    /// The halves of the clock's zone storage holds because a peer wrote
    /// them, now or before a restart (#2395); none without storage. A host
    /// that builds the clock puts each held half over the zone it gives it,
    /// while the clock takes zone writes.
    pub fn written_zone(&self) -> WrittenZone {
        self.profile.saved_zone()
    }
}
