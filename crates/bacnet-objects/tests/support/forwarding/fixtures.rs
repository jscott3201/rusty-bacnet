//! Values the probe and the rows both use: the day and time the schedule
//! rows pass, object identifiers, the probe's property error, one instance
//! of each clock, the address a capability renders as, and the probe's
//! property metadata.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::property_metadata::{
    PropertyConformance, PropertyMetadata, PropertyWriteCapability,
};
use bacnet_objects::schedule::ScheduleWrite;
use bacnet_objects::traits::{DeadlineWaker, MonotonicClock};
use bacnet_types::calendar::SpecificDate;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, Time};

/// The day the probe's calendar is active, and the day schedule rows pass.
pub fn day() -> SpecificDate {
    SpecificDate::new(2026, 10, 2).unwrap()
}

pub fn noon() -> Time {
    Time {
        hour: 12,
        minute: 0,
        second: 0,
        hundredths: 0,
    }
}

pub fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

pub fn property_error(code: ErrorCode) -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY.to_raw() as u32,
        code: code.to_raw() as u32,
    }
}

struct NoClock;

impl ClockReader for NoClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        None
    }
}

// One instance of each clock, so a row passes the same handle to the probe
// and to an adapter, and the log can name it by address.
static CLOCK: LazyLock<Arc<dyn ClockReader>> = LazyLock::new(|| Arc::new(NoClock));
static MONOTONIC: LazyLock<Arc<MonotonicClock>> =
    LazyLock::new(|| Arc::new(|| Duration::from_secs(99)));

pub fn clock() -> Arc<dyn ClockReader> {
    CLOCK.clone()
}

pub fn monotonic() -> Arc<MonotonicClock> {
    MONOTONIC.clone()
}

static WAKER: LazyLock<Arc<DeadlineWaker>> = LazyLock::new(|| Arc::new(|| {}));

pub fn waker() -> Arc<DeadlineWaker> {
    WAKER.clone()
}

/// A capability or handle, rendered as the address it points at.
pub fn address<T: ?Sized>(reference: &T) -> String {
    format!("{:p}", std::ptr::from_ref(reference).cast::<()>())
}

pub fn schedule_write(value: u64) -> ScheduleWrite {
    ScheduleWrite {
        value: PropertyValue::Unsigned(value),
        priority: 16,
        references: Vec::new(),
        retry: false,
    }
}

/// DESCRIPTION is writable here, and CUSTOM is absent, unlike the probe's
/// own answers, so defaults derived from these rows give themselves away.
pub const METADATA: [PropertyMetadata; 5] = [
    PropertyMetadata::new(
        P::OBJECT_IDENTIFIER,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        P::OBJECT_NAME,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        P::OBJECT_TYPE,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        P::DESCRIPTION,
        PropertyConformance::Optional,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        P::PROPERTY_LIST,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
];
