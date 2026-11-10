//! The Device's wall clock, for firmware with no clock of its own (#2321):
//! TimeSynchronization and UTCTimeSynchronization set it, and the Device's
//! Local_Date and Local_Time, timestamps and Notification Class recipient
//! windows read it.
//!
//! - [`SyncedClock`] is the clock as plain data: an instant on the host's
//!   monotonic timeline, the UTC date and time it stood for, and the
//!   Device's [`Zone`]. It allocates nothing and divides nothing in 64
//!   bits. Until the first [`set`](SyncedClock::set) it is unsynchronized
//!   and counts from local midnight, 1990-01-01, as a device with no time
//!   source does after a restart (Clauses 12.11.23, 12.11.24).
//! - [`DeviceClock`] is that clock with the host's monotonic timer
//!   ([`MonotonicNow`]), lent to the database as its `ClockReader`
//!   ([`DeviceClock::lend`]), so the Device serves Local_Date, Local_Time,
//!   UTC_Offset and Daylight_Savings_Status from it, and lists the two
//!   services in Protocol_Services_Supported when its identity names them
//!   ([`SERVICES`]).
//! - The [`Responder`](crate::responder::Responder) given one
//!   ([`with_clock`](crate::responder::Responder::with_clock)) sets it from
//!   each TimeSynchronization and UTCTimeSynchronization the Device lists,
//!   and lends the set clock to the database again. A local
//!   TimeSynchronization's date and time become Local_Date and Local_Time
//!   as given (Clause 16.7); a UTCTimeSynchronization's are UTC, which
//!   reads locally as less UTC_Offset, plus an hour while
//!   Daylight_Savings_Status is set (Clause 16.8). Either way the clock
//!   keeps the UTC instant, as the server's does, so the two read the same
//!   for the same request in the same zone. Admission (which sources may
//!   set the clock, how often, by how much) is the host's: it hands the
//!   core only the requests it accepts.
//! - A host with a real-time clock on the board sets it directly, at boot
//!   and whenever it likes: [`DeviceClock::set`] before lending,
//!   [`Responder::set_time`](crate::responder::Responder::set_time) or
//!   [`Reporter::set_time`](crate::report::Reporter::set_time) after.
//!   That counts as synchronized: the host vouches for its clock.
//! - [`destination_open`] judges a Notification Class destination's valid
//!   days and time window against [`SyncedClock::window`]: by the server's
//!   own test once the clock is synchronized, and failing closed before,
//!   when only a destination open every day, all day, is.
//!
//! - The zone can change while the device runs: daylight saving starts or
//!   ends, or the host corrects its UTC_Offset. [`DeviceClock::set_zone`]
//!   before lending, [`Responder::set_zone`](crate::responder::Responder::set_zone)
//!   or [`Reporter::set_zone`](crate::report::Reporter::set_zone) after. A
//!   synchronized clock keeps its UTC instant and stays synchronized, so
//!   local time, and with it the recipient windows, moves by the difference
//!   ([`SyncedClock::set_zone`]). Only the host changes it: UTC_Offset and
//!   Daylight_Savings_Status are read-only to a peer.

mod synced;

use alloc::sync::Arc;

use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::notification_class::{always_in_window, in_window};
use bacnet_services::device_mgmt::TimeSynchronizationRequest;
use bacnet_time::Instant;
use bacnet_types::bitstring::{DaysOfWeek, ServicesSupported};
use bacnet_types::constructed::BACnetDestination;
use bacnet_types::enums::{PropertyIdentifier, ServiceSupported, UnconfirmedServiceChoice};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};

pub use synced::{Basis, SyncedClock, Zone};

#[cfg(test)]
mod tests;

/// The services a [`DeviceClock`] lets a [`Responder`](crate::responder)
/// run: add them to the Device's services
/// (`DeviceIdentity::with_services`) for it to act on them.
pub const SERVICES: &[ServiceSupported] = &[
    ServiceSupported::TIME_SYNCHRONIZATION,
    ServiceSupported::UTC_TIME_SYNCHRONIZATION,
];

/// The host's monotonic timer: the instant now, on the timeline it passes
/// the core as `now`.
pub type MonotonicNow = dyn Fn() -> Instant + Send + Sync;

/// A [`SyncedClock`] read at the host's timer, which the database borrows
/// as its `ClockReader` ([`lend`](Self::lend)).
#[derive(Clone)]
pub struct DeviceClock {
    clock: SyncedClock,
    now: Arc<MonotonicNow>,
}

impl core::fmt::Debug for DeviceClock {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DeviceClock")
            .field("clock", &self.clock)
            .finish_non_exhaustive()
    }
}

impl DeviceClock {
    /// An unsynchronized clock in `zone`, starting at local midnight,
    /// 1990-01-01, as `now` reads it now.
    pub fn new(zone: Zone, now: Arc<MonotonicNow>) -> Self {
        let clock = SyncedClock::new(zone, now());
        Self { clock, now }
    }

    /// The clock.
    pub fn clock(&self) -> &SyncedClock {
        &self.clock
    }

    /// The host's timer, read now.
    pub fn now(&self) -> Instant {
        (self.now)()
    }

    /// Set the clock to `date` and `time` on `basis`, now, as
    /// [`SyncedClock::set`] does. A database this clock was lent to before
    /// still reads the clock as it was: lend it again.
    pub fn set(&mut self, basis: Basis, date: Date, time: Time) -> Result<(), Error> {
        let now = self.now();
        self.clock.set(basis, date, time, now)
    }

    /// Move the clock to `zone`, now, as [`SyncedClock::set_zone`] does. A
    /// database this clock was lent to before still reads the old zone: lend
    /// it again.
    pub fn set_zone(&mut self, zone: Zone) -> Result<(), Error> {
        let now = self.now();
        self.clock.set_zone(zone, now)
    }

    /// Bind a copy of this clock to `db` and every object in it, in place
    /// of the clock it had. Each lending allocates the copy the database
    /// shares.
    pub fn lend(&self, db: &mut ObjectDatabase) {
        db.set_clock_reader(Some(Arc::new(self.clone())));
    }
}

impl ClockReader for DeviceClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        self.clock.frame(self.now())
    }
}

/// Whether a Notification Class `destination` takes a notification sent
/// when the clock's [`window`](SyncedClock::window) is `window`: with a
/// day and time, when that day is one of its valid days and the time falls
/// in its window, as the server selects; with none (an unsynchronized
/// clock), only when it is open every day, all day. Its transitions aren't
/// looked at.
pub fn destination_open(
    destination: &BACnetDestination,
    window: Option<(DaysOfWeek, Time)>,
) -> bool {
    match window {
        Some((today, now)) => in_window(destination, today, &now),
        None => always_in_window(destination),
    }
}

/// Run a TimeSynchronization or UTCTimeSynchronization, `service`, whose
/// parameters are `parameters`, against `clock`, for `device`: `false`
/// when it is neither. One that `device` doesn't list in its
/// Protocol_Services_Supported, that doesn't decode, or whose value
/// [`SyncedClock::set`] refuses, changes nothing; one it takes is lent to
/// `db`.
pub(crate) fn synchronize(
    clock: &mut DeviceClock,
    db: &mut ObjectDatabase,
    device: ObjectIdentifier,
    service: UnconfirmedServiceChoice,
    parameters: &[u8],
) -> bool {
    let (basis, listed) = if service == UnconfirmedServiceChoice::TIME_SYNCHRONIZATION {
        (Basis::Local, ServiceSupported::TIME_SYNCHRONIZATION)
    } else if service == UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION {
        (Basis::Utc, ServiceSupported::UTC_TIME_SYNCHRONIZATION)
    } else {
        return false;
    };
    let lists = db.get(&device).is_some_and(|device| {
        matches!(
            device.read_property(PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED, None),
            Ok(PropertyValue::BitString { data, .. })
                if ServicesSupported::from_bacnet(&data).contains(listed)
        )
    });
    if lists
        && let Ok(request) = TimeSynchronizationRequest::decode(parameters)
        && clock.set(basis, request.date, request.time).is_ok()
    {
        clock.lend(db);
    }
    true
}
