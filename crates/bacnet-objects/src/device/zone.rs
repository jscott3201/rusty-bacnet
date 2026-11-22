//! A peer's write of the Device's UTC_Offset or Daylight_Savings_Status
//! (#2395), which the host opts in to through its clock
//! ([`ClockReader::takes_zone_writes`]). Without the opt-in both stay
//! read-only, and a write of either is refused as before.
//!
//! The written value moves the clock as the host's own zone change does
//! ([`ClockReader::write_zone`]): the other half of the zone stays as the
//! clock holds it, the clock keeps its UTC instant, and a synchronized clock
//! stays synchronized. UTC_Offset takes an INTEGER, a multiple of 15 minutes
//! within -1440 to 1440 (Clause 12.11.25 asks for at least those);
//! Daylight_Savings_Status takes a BOOLEAN. A value of another datatype is
//! refused with INVALID_DATA_TYPE, a NULL included, which the write services
//! then answer as a relinquish that changes nothing (Clause 15.9); an offset
//! off the 15-minute grid or past a day either way with VALUE_OUT_OF_RANGE.
//!
//! A Device built with Tags storage
//! ([`with_tags_persistence`](DeviceObject::with_tags_persistence)) keeps
//! each half a peer writes there, as it keeps a written Description: saved
//! before the write is answered, and refused with DEVICE /
//! OPERATIONAL_PROBLEM, the clock left as it was, when the save fails. A
//! saved half wins over the zone the host gives the clock until
//! [`clear_written_zone`](DeviceObject::clear_written_zone); the host reads
//! it with [`written_zone`](DeviceObject::written_zone), as the bundled
//! server does at its start and at each zone change of its own.
use super::*;
use crate::object_profile::WrittenZone;

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
    ///
    /// The clock moves first and storage saves after: a save that fails
    /// puts the clock back, under the same database guard, so no reader sees
    /// the zone it refused. A staged save (the server's) has run already,
    /// and the clock refusing the zone leaves its step to be released.
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
        clock.write_zone(offset, summer)?;
        if let Some(Err(error)) = self.profile.write_zone(property, value) {
            // The zone it had was in effect a moment ago, so the clock takes
            // it back.
            let _ = clock.write_zone(served.0, served.1);
            return Err(error);
        }
        Ok(())
    }

    /// The halves of the clock's zone storage holds because a peer wrote
    /// them, now or before a restart (#2395); none without storage. A host
    /// that builds the clock puts each held half over the zone it gives it.
    pub fn written_zone(&self) -> WrittenZone {
        self.profile.saved_zone()
    }

    /// Drop the zone halves a write saved (#2395): storage keeps none from
    /// now on, so the zone the host gives the clock applies in full again
    /// from its next start. The clock stays as it is. Saves before it
    /// returns; a save that fails keeps the halves and returns storage's
    /// error. Does nothing without storage or a saved half. Call it before
    /// the object joins the database, as the setters are: it waits for its
    /// save.
    pub fn clear_written_zone(&mut self) -> Result<(), Error> {
        self.profile.clear_written_zone()
    }
}
