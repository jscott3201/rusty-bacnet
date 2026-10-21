//! Access Zone Occupancy_Count and Reliability writes while Out_Of_Service is
//! TRUE (Clauses 12.32.9, 12.32.10 and 12.32.11, Table 12-37 footnote 1,
//! #1247).

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::enums::{PropertyIdentifier as P, Reliability};

use super::*;

/// A zone whose own count is 12, with no fault.
fn zone() -> AccessZoneObject {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.set_occupancy_count(12);
    zone
}

fn read(zone: &AccessZoneObject, property: P) -> PropertyValue {
    zone.read_property(property, None).unwrap()
}

fn write(zone: &mut AccessZoneObject, property: P, value: PropertyValue) -> Result<(), Error> {
    zone.write_property(property, None, value, None)
}

fn set_out_of_service(zone: &mut AccessZoneObject, out_of_service: bool) {
    write(
        zone,
        P::OUT_OF_SERVICE,
        PropertyValue::Boolean(out_of_service),
    )
    .unwrap();
}

/// Occupancy_Count, Reliability and Status_Flags as served.
fn served(zone: &AccessZoneObject) -> [PropertyValue; 3] {
    [P::OCCUPANCY_COUNT, P::RELIABILITY, P::STATUS_FLAGS].map(|property| read(zone, property))
}

fn status_flags(fault: bool, out_of_service: bool) -> PropertyValue {
    let mut flags = StatusFlags::empty();
    flags.set(StatusFlags::FAULT, fault);
    flags.set(StatusFlags::OUT_OF_SERVICE, out_of_service);
    PropertyValue::BitString {
        unused_bits: 4,
        data: vec![flags.bits() << 4],
    }
}

/// The values a zone with count `count` and the given Reliability serves.
fn values(count: u64, reliability: Reliability, out_of_service: bool) -> [PropertyValue; 3] {
    [
        PropertyValue::Unsigned(count),
        PropertyValue::Enumerated(reliability.to_raw()),
        status_flags(
            reliability != Reliability::NO_FAULT_DETECTED,
            out_of_service,
        ),
    ]
}

/// The zone's own values: count 12, no fault.
fn own(out_of_service: bool) -> [PropertyValue; 3] {
    values(12, Reliability::NO_FAULT_DETECTED, out_of_service)
}

fn assert_property_error(result: Result<(), Error>, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code: actual })
            if class == ErrorClass::PROPERTY
                && actual == code),
        "expected PROPERTY / {code:?}, got {result:?}"
    );
}

#[test]
fn access_zone_refuses_simulated_rows_in_service() {
    let mut zone = zone();
    for (property, value) in [
        (P::OCCUPANCY_COUNT, PropertyValue::Unsigned(40)),
        (
            P::RELIABILITY,
            PropertyValue::Enumerated(Reliability::UNRELIABLE_OTHER.to_raw()),
        ),
        // In service the refusal comes before any datatype check.
        (P::OCCUPANCY_COUNT, PropertyValue::Real(1.0)),
        (P::RELIABILITY, PropertyValue::Null),
    ] {
        assert_property_error(
            write(&mut zone, property, value),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_eq!(served(&zone), own(false), "{property:?}");
    }
    // Both rows have a write route, open only out of service.
    assert!(zone.is_writable_property(P::OCCUPANCY_COUNT));
    assert!(zone.is_writable_property(P::RELIABILITY));
}

#[test]
fn access_zone_takes_simulated_rows_out_of_service() {
    let mut zone = zone();
    set_out_of_service(&mut zone, true);
    assert_eq!(served(&zone), own(true));

    // Any Unsigned count is taken, zero and the top of the range included.
    for count in [40, 0, u64::MAX] {
        write(
            &mut zone,
            P::OCCUPANCY_COUNT,
            PropertyValue::Unsigned(count),
        )
        .unwrap();
        assert_eq!(
            served(&zone),
            values(count, Reliability::NO_FAULT_DETECTED, true)
        );
    }

    // A simulated fault sets the FAULT flag; a proprietary value is taken too.
    for reliability in [Reliability::UNRELIABLE_OTHER, Reliability::from_raw(64)] {
        write(
            &mut zone,
            P::RELIABILITY,
            PropertyValue::Enumerated(reliability.to_raw()),
        )
        .unwrap();
        assert_eq!(served(&zone), values(u64::MAX, reliability, true));
    }
}

#[test]
fn access_zone_simulated_rows_outside_their_datatypes_are_refused_unchanged() {
    let mut zone = zone();
    set_out_of_service(&mut zone, true);
    for (property, value, code) in [
        (
            P::OCCUPANCY_COUNT,
            PropertyValue::Signed(5),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            P::OCCUPANCY_COUNT,
            PropertyValue::Real(5.0),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            P::OCCUPANCY_COUNT,
            PropertyValue::Enumerated(5),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            P::OCCUPANCY_COUNT,
            PropertyValue::Null,
            ErrorCode::INVALID_DATA_TYPE,
        ),
        // 11 is reserved for ASHRAE, 65536 past the datatype.
        (
            P::RELIABILITY,
            PropertyValue::Enumerated(11),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            P::RELIABILITY,
            PropertyValue::Enumerated(65_536),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            P::RELIABILITY,
            PropertyValue::Unsigned(7),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            P::RELIABILITY,
            PropertyValue::Null,
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ] {
        assert_property_error(write(&mut zone, property, value), code);
        assert_eq!(served(&zone), own(true), "{property:?} {code:?}");
    }
}

#[test]
fn access_zone_return_to_service_serves_the_zone_again() {
    let mut zone = zone();
    set_out_of_service(&mut zone, true);
    write(&mut zone, P::OCCUPANCY_COUNT, PropertyValue::Unsigned(40)).unwrap();
    write(
        &mut zone,
        P::RELIABILITY,
        PropertyValue::Enumerated(Reliability::UNRELIABLE_OTHER.to_raw()),
    )
    .unwrap();
    let simulated = served(&zone);

    // Out of service the application's count goes aside, and its Reliability
    // is refused as on the other Reliability carriers.
    zone.set_occupancy_count(13);
    assert_property_error(
        zone.set_reliability_internal(Reliability::COMMUNICATION_FAILURE),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(served(&zone), simulated);

    // The return to service serves the application's count and the
    // Reliability from before, dropping the simulation.
    set_out_of_service(&mut zone, false);
    assert_eq!(
        served(&zone),
        values(13, Reliability::NO_FAULT_DETECTED, false)
    );

    // In service the application's values are served at once.
    zone.set_reliability_internal(Reliability::COMMUNICATION_FAILURE)
        .unwrap();
    zone.set_occupancy_count(12);
    assert_eq!(
        served(&zone),
        values(12, Reliability::COMMUNICATION_FAILURE, false)
    );
    zone.set_reliability_internal(Reliability::NO_FAULT_DETECTED)
        .unwrap();

    // A second period out of service starts from the zone's values.
    set_out_of_service(&mut zone, true);
    assert_eq!(served(&zone), own(true));
    set_out_of_service(&mut zone, false);
    assert_eq!(served(&zone), own(false));
}

#[test]
fn access_zone_null_out_of_service_write_keeps_the_simulation() {
    let mut zone = zone();
    set_out_of_service(&mut zone, true);
    write(&mut zone, P::OCCUPANCY_COUNT, PropertyValue::Unsigned(3)).unwrap();
    let simulated = served(&zone);
    write(&mut zone, P::OUT_OF_SERVICE, PropertyValue::Null).unwrap();
    assert_eq!(served(&zone), simulated);
    // Writing TRUE again isn't a new entry: the zone's values stay aside.
    set_out_of_service(&mut zone, true);
    assert_eq!(served(&zone), simulated);
    set_out_of_service(&mut zone, false);
    assert_eq!(served(&zone), own(false));
}
