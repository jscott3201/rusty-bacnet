//! Access Zone occupancy counting: Occupancy_State, Occupancy_Count_Enable,
//! Adjust_Value and the occupancy limits (Clauses 12.32.6 and 12.32.10 to
//! 12.32.15, #1284).

use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::enums::{AccessZoneOccupancyState as S, ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

use super::*;

fn read(zone: &AccessZoneObject, property: P) -> PropertyValue {
    zone.read_property(property, None).unwrap()
}

fn write(zone: &mut AccessZoneObject, property: P, value: PropertyValue) -> Result<(), Error> {
    zone.write_property(property, None, value, None)
}

fn adjust(zone: &mut AccessZoneObject, value: i32) {
    write(zone, P::ADJUST_VALUE, PropertyValue::Signed(value)).unwrap();
}

fn set_out_of_service(zone: &mut AccessZoneObject, out_of_service: bool) {
    write(
        zone,
        P::OUT_OF_SERVICE,
        PropertyValue::Boolean(out_of_service),
    )
    .unwrap();
}

fn assert_property_error(result: Result<(), Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == expected),
        "expected PROPERTY / {expected:?}, got {result:?}"
    );
}

/// Occupancy_Count, Occupancy_State and Adjust_Value as served.
fn counting(zone: &AccessZoneObject) -> [PropertyValue; 3] {
    [P::OCCUPANCY_COUNT, P::OCCUPANCY_STATE, P::ADJUST_VALUE].map(|p| read(zone, p))
}

fn counted(count: u64, state: S, adjust: i32) -> [PropertyValue; 3] {
    [
        PropertyValue::Unsigned(count),
        PropertyValue::Enumerated(state.to_raw()),
        PropertyValue::Signed(adjust),
    ]
}

#[test]
fn access_zone_occupancy_state_follows_count_and_limits() {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.set_occupancy_limits(2, 5).unwrap();
    for (count, state) in [
        (0, S::BELOW_LOWER_LIMIT),
        (1, S::BELOW_LOWER_LIMIT),
        (2, S::AT_LOWER_LIMIT),
        (3, S::NORMAL),
        (4, S::NORMAL),
        (5, S::AT_UPPER_LIMIT),
        (6, S::ABOVE_UPPER_LIMIT),
        (u64::MAX, S::ABOVE_UPPER_LIMIT),
    ] {
        zone.set_occupancy_count(count);
        assert_eq!(counting(&zone), counted(count, state, 0), "count {count}");
        assert_eq!(zone.occupancy_state(), state);
    }
    // A limit of zero is no limit: with neither, every count is NORMAL.
    zone.set_occupancy_limits(0, 0).unwrap();
    for count in [0, 1, u64::MAX] {
        zone.set_occupancy_count(count);
        assert_eq!(zone.occupancy_state(), S::NORMAL, "count {count}");
    }
    // An upper limit alone, then a lower limit alone.
    zone.set_occupancy_limits(0, 3).unwrap();
    zone.set_occupancy_count(0);
    assert_eq!(zone.occupancy_state(), S::NORMAL);
    zone.set_occupancy_limits(3, 0).unwrap();
    zone.set_occupancy_count(1_000);
    assert_eq!(zone.occupancy_state(), S::NORMAL);
    assert_eq!(
        [P::OCCUPANCY_LOWER_LIMIT, P::OCCUPANCY_UPPER_LIMIT].map(|p| read(&zone, p)),
        [PropertyValue::Unsigned(3), PropertyValue::Unsigned(0)]
    );
}

#[test]
fn access_zone_occupancy_limits_refuse_an_upper_limit_not_above_the_lower() {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.set_occupancy_limits(2, 5).unwrap();
    for (lower, upper) in [(5, 5), (5, 3), (u64::MAX, 1)] {
        assert_property_error(
            zone.set_occupancy_limits(lower, upper),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(
            [P::OCCUPANCY_LOWER_LIMIT, P::OCCUPANCY_UPPER_LIMIT].map(|p| read(&zone, p)),
            [PropertyValue::Unsigned(2), PropertyValue::Unsigned(5)]
        );
    }
}

#[test]
fn access_zone_counting_off_reads_disabled_and_zero() {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.set_occupancy_limits(2, 5).unwrap();
    zone.set_occupancy_count(4);
    adjust(&mut zone, 2);
    assert_eq!(counting(&zone), counted(6, S::ABOVE_UPPER_LIMIT, 2));

    zone.set_occupancy_count_enable(false);
    assert_eq!(
        read(&zone, P::OCCUPANCY_COUNT_ENABLE),
        PropertyValue::Boolean(false)
    );
    assert_eq!(counting(&zone), counted(0, S::DISABLED, 0));
    // The application's counts are dropped, and an Adjust_Value write is
    // taken but leaves the value at zero.
    zone.set_occupancy_count(7);
    adjust(&mut zone, 3);
    assert_eq!(counting(&zone), counted(0, S::DISABLED, 0));

    // Counting on again starts from zero.
    zone.set_occupancy_count_enable(true);
    assert_eq!(counting(&zone), counted(0, S::BELOW_LOWER_LIMIT, 0));
    zone.set_occupancy_count(7);
    assert_eq!(counting(&zone), counted(7, S::ABOVE_UPPER_LIMIT, 0));
}

#[test]
fn access_zone_adjust_value_moves_the_count() {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.set_occupancy_count(5);
    adjust(&mut zone, 3);
    assert_eq!(counting(&zone), counted(8, S::NORMAL, 3));
    // A negative sum stops at zero.
    adjust(&mut zone, -10);
    assert_eq!(counting(&zone), counted(0, S::NORMAL, -10));
    // Zero clears the count.
    zone.set_occupancy_count(4);
    adjust(&mut zone, 0);
    assert_eq!(counting(&zone), counted(0, S::NORMAL, 0));
    // The sum stops at the top of the Unsigned range too.
    zone.set_occupancy_count(u64::MAX - 1);
    adjust(&mut zone, i32::MAX);
    assert_eq!(counting(&zone), counted(u64::MAX, S::NORMAL, i32::MAX));
    zone.set_occupancy_count(1);
    adjust(&mut zone, i32::MIN);
    assert_eq!(counting(&zone), counted(0, S::NORMAL, i32::MIN));
}

#[test]
fn access_zone_adjust_value_refuses_other_datatypes() {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.set_occupancy_count(5);
    adjust(&mut zone, 1);
    for value in [
        PropertyValue::Unsigned(1),
        PropertyValue::Real(1.0),
        PropertyValue::Null,
    ] {
        assert_property_error(
            write(&mut zone, P::ADJUST_VALUE, value),
            ErrorCode::INVALID_DATA_TYPE,
        );
        assert_eq!(counting(&zone), counted(6, S::NORMAL, 1));
    }
}

#[test]
fn access_zone_out_of_service_adjust_value_leaves_the_counts() {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.set_occupancy_limits(0, 5).unwrap();
    zone.set_occupancy_count(3);
    set_out_of_service(&mut zone, true);
    // A simulated count moves Occupancy_State as a counted one would.
    write(&mut zone, P::OCCUPANCY_COUNT, PropertyValue::Unsigned(9)).unwrap();
    assert_eq!(counting(&zone), counted(9, S::ABOVE_UPPER_LIMIT, 0));
    // Adjust_Value is kept, but neither the simulated count nor the zone's
    // own count moves (12.32.10).
    adjust(&mut zone, -4);
    assert_eq!(counting(&zone), counted(9, S::ABOVE_UPPER_LIMIT, -4));
    set_out_of_service(&mut zone, false);
    assert_eq!(counting(&zone), counted(3, S::NORMAL, -4));
}

#[test]
fn access_zone_counting_off_out_of_service_takes_only_a_zero_count() {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.set_occupancy_count(4);
    set_out_of_service(&mut zone, true);
    write(&mut zone, P::OCCUPANCY_COUNT, PropertyValue::Unsigned(6)).unwrap();
    // Turning counting off zeroes the simulated count and the count set
    // aside, so the return to service serves zero too.
    zone.set_occupancy_count_enable(false);
    assert_eq!(counting(&zone), counted(0, S::DISABLED, 0));
    assert_property_error(
        write(&mut zone, P::OCCUPANCY_COUNT, PropertyValue::Unsigned(3)),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    write(&mut zone, P::OCCUPANCY_COUNT, PropertyValue::Unsigned(0)).unwrap();
    assert_eq!(counting(&zone), counted(0, S::DISABLED, 0));
    set_out_of_service(&mut zone, false);
    assert_eq!(counting(&zone), counted(0, S::DISABLED, 0));
}
