//! Wire pins for the object fields stored as their `bacnet_types::enums`
//! newtypes (#932).
//!
//! Each newtype wraps the raw `u32`, so storing it instead of a bare integer
//! must not change what a read returns, which values a write accepts, or how
//! a vendor-proprietary or not-yet-named value round-trips.

use crate::access_control::{AccessDoorObject, AccessPointObject, AccessUserObject};
use crate::analog::AnalogInputObject;
use crate::elevator::ElevatorGroupObject;
use crate::life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject};
use crate::loop_obj::LoopObject;
use crate::schedule::ScheduleObject;
use crate::staging::{StagingConfig, StagingObject};
use crate::traits::BACnetObject;
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, LifeSafetyMode, LifeSafetyOperation, LifeSafetyState,
    ObjectType, PropertyIdentifier, Reliability, SilencedState,
};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

/// Values no write path range-checks today: an unnamed small value, a value
/// past the 16-bit vendor range, and the largest encodable Enumerated.
const UNCHECKED_ENUMERATED: [u32; 3] = [200, 70_000, u32::MAX];

fn read(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyValue {
    object
        .read_property(property, None)
        .expect("test property must be readable")
}

fn write(
    object: &mut dyn BACnetObject,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    object.write_property(property, None, value, None)
}

fn assert_protocol_error(result: Result<(), Error>, class: ErrorClass, code: ErrorCode) {
    match result {
        Err(Error::Remote(RemoteError::Error {
            class: actual_class,
            code: actual_code,
        })) => {
            assert_eq!(actual_class, class);
            assert_eq!(actual_code, code);
        }
        other => panic!("expected {class:?} / {code:?}, got {other:?}"),
    }
}

fn status_flags_fault(object: &dyn BACnetObject) -> bool {
    match read(object, PropertyIdentifier::STATUS_FLAGS) {
        PropertyValue::BitString { data, .. } => data[0] & (StatusFlags::FAULT.bits() << 4) != 0,
        other => panic!("Status_Flags must be a bit string, got {other:?}"),
    }
}

/// An Enumerated write of `property` accepts every value, stores it
/// unchanged, and refuses a non-Enumerated datatype.
fn assert_unchecked_enumerated_round_trip(
    object: &mut dyn BACnetObject,
    property: PropertyIdentifier,
) {
    for raw in UNCHECKED_ENUMERATED {
        write(object, property, PropertyValue::Enumerated(raw))
            .expect("this Enumerated write has no range check");
        assert_eq!(read(object, property), PropertyValue::Enumerated(raw));
    }
    assert_protocol_error(
        write(object, property, PropertyValue::Unsigned(1)),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_eq!(
        read(object, property),
        PropertyValue::Enumerated(u32::MAX),
        "a refused write must leave the stored value alone"
    );
}

fn assert_fault_flag(object: &dyn BACnetObject, expected: bool) {
    assert_eq!(
        status_flags_fault(object),
        expected,
        "FAULT is set exactly when Reliability is not NO_FAULT_DETECTED"
    );
}

/// While Out_Of_Service is TRUE, a client Reliability write accepts the named
/// set plus 64..=65535 and stores vendor values verbatim; a refused value
/// leaves the stored value alone.
fn assert_client_reliability_round_trip(object: &mut dyn BACnetObject) {
    write(
        object,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .expect("Out_Of_Service must be writable");
    for vendor in [64, 1_000, 65_535] {
        write(
            object,
            PropertyIdentifier::RELIABILITY,
            PropertyValue::Enumerated(vendor),
        )
        .expect("a vendor Reliability value must be accepted");
        assert_eq!(
            read(object, PropertyIdentifier::RELIABILITY),
            PropertyValue::Enumerated(vendor)
        );
        assert_fault_flag(object, true);
    }
    for refused in [11, 26, 63, 65_536, u32::MAX] {
        assert_protocol_error(
            write(
                object,
                PropertyIdentifier::RELIABILITY,
                PropertyValue::Enumerated(refused),
            ),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(
            read(object, PropertyIdentifier::RELIABILITY),
            PropertyValue::Enumerated(65_535)
        );
    }
    assert_protocol_error(
        write(
            object,
            PropertyIdentifier::RELIABILITY,
            PropertyValue::Unsigned(1),
        ),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_eq!(
        read(object, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(65_535),
        "a wrong-datatype write must leave the stored value alone"
    );
}

/// A vendor value from evaluation is saved on entering Out_Of_Service and
/// restored on leaving it, replacing the client's simulated vendor value.
fn assert_vendor_reliability_survives_out_of_service(object: &mut dyn BACnetObject) {
    object
        .set_reliability_internal(Reliability::from_raw(65_535))
        .expect("a vendor Reliability value must be accepted internally");
    write(
        object,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    write(
        object,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(1_000),
    )
    .expect("a vendor Reliability value must be accepted from a client");
    assert_eq!(
        read(object, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(1_000)
    );
    write(
        object,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(false),
    )
    .unwrap();
    assert_eq!(
        read(object, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(65_535),
        "leaving Out_Of_Service must restore the saved vendor value"
    );
}

/// In service, the internal route applies the same value domain as the
/// client route.
fn assert_internal_reliability_round_trip(object: &mut dyn BACnetObject) {
    object
        .set_reliability_internal(Reliability::from_raw(65_535))
        .expect("a vendor Reliability value must be accepted internally");
    assert_eq!(
        read(object, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(65_535)
    );
    assert_fault_flag(object, true);
    for refused in [11, 26, 63, 65_536, u32::MAX] {
        assert_protocol_error(
            object.set_reliability_internal(Reliability::from_raw(refused)),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_eq!(
        read(object, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(65_535)
    );
    object
        .set_reliability_internal(Reliability::NO_FAULT_DETECTED)
        .unwrap();
    assert_fault_flag(object, false);
}

fn staging() -> StagingObject {
    let stage = |limit| BACnetStageLimitValue {
        limit,
        values: vec![true],
        deadband: 1.0,
    };
    StagingObject::new(
        1,
        "STG-1",
        StagingConfig {
            present_value: 5.0,
            min_present_value: 0.0,
            units: EngineeringUnits::DEGREES_CELSIUS,
            priority_for_writing: 8,
            stages: vec![stage(10.0), stage(20.0)],
            target_references: vec![BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1).unwrap(),
            }],
            stage_names: None,
        },
    )
    .unwrap()
}

#[test]
fn reliability_vendor_values_round_trip_on_every_write_route() {
    // One carrier per distinct write arm: the shared inhibit route, Loop's,
    // Schedule's and Staging's own arms.
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    assert_internal_reliability_round_trip(&mut ai);
    assert_client_reliability_round_trip(&mut ai);

    let mut lp = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    assert_internal_reliability_round_trip(&mut lp);
    assert_client_reliability_round_trip(&mut lp);

    let mut schedule = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(0.0)).unwrap();
    assert_internal_reliability_round_trip(&mut schedule);
    assert_client_reliability_round_trip(&mut schedule);

    assert_client_reliability_round_trip(&mut staging());
}

#[test]
fn vendor_reliability_is_saved_and_restored_across_out_of_service() {
    // The shared inhibit route, and the common save/restore helper that Loop
    // and Schedule use.
    assert_vendor_reliability_survives_out_of_service(
        &mut AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    );
    assert_vendor_reliability_survives_out_of_service(
        &mut LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    );
    assert_vendor_reliability_survives_out_of_service(
        &mut ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(0.0)).unwrap(),
    );
}

#[test]
fn life_safety_listed_mode_writes_store_any_enumerated() {
    // A Mode write must name a mode in Accepted_Modes (#1092); once the
    // application lists these values, Mode stores each one verbatim.
    let listed = UNCHECKED_ENUMERATED.map(LifeSafetyMode::from_raw);
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    point.set_accepted_modes(listed);
    assert_unchecked_enumerated_round_trip(&mut point, PropertyIdentifier::MODE);
    let mut zone = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    zone.set_accepted_modes(listed);
    assert_unchecked_enumerated_round_trip(&mut zone, PropertyIdentifier::MODE);
}

#[test]
fn life_safety_proprietary_states_read_back_verbatim() {
    let proprietary = LifeSafetyState::from_raw(300);
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    point.set_present_value(proprietary);
    point.set_tracking_value(proprietary);
    point.set_silenced(SilencedState::from_raw(300));
    point.set_operation_expected(LifeSafetyOperation::from_raw(300));
    for property in [
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::TRACKING_VALUE,
        PropertyIdentifier::SILENCED,
        PropertyIdentifier::OPERATION_EXPECTED,
    ] {
        assert_eq!(read(&point, property), PropertyValue::Enumerated(300));
    }

    let mut zone = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    zone.set_present_value(proprietary);
    zone.set_tracking_value(proprietary);
    zone.set_silenced(SilencedState::from_raw(300));
    for property in [
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::TRACKING_VALUE,
        PropertyIdentifier::SILENCED,
    ] {
        assert_eq!(read(&zone, property), PropertyValue::Enumerated(300));
    }
}

#[test]
fn silence_operation_refuses_a_reserved_or_proprietary_silenced_state_and_keeps_it() {
    // 4 is the first reserved BACnetSilencedState value and 64 the first
    // proprietary one; neither decomposes into audible/visible bits.
    for raw in [4, 64] {
        let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        point.set_silenced(SilencedState::from_raw(raw));
        point.set_operation_expected(LifeSafetyOperation::SILENCE_AUDIBLE);
        match point.apply_life_safety_operation(LifeSafetyOperation::SILENCE_AUDIBLE) {
            Err(Error::Remote(RemoteError::Error { class, code })) => {
                assert_eq!(class, ErrorClass::OBJECT);
                assert_eq!(code, ErrorCode::INVALID_OPERATION_IN_THIS_STATE);
            }
            other => panic!("expected OBJECT / INVALID_OPERATION_IN_THIS_STATE, got {other:?}"),
        }
        assert_eq!(
            read(&point, PropertyIdentifier::SILENCED),
            PropertyValue::Enumerated(raw)
        );
        assert_eq!(
            read(&point, PropertyIdentifier::OPERATION_EXPECTED),
            PropertyValue::Enumerated(LifeSafetyOperation::SILENCE_AUDIBLE.to_raw())
        );
    }
}

#[test]
fn access_and_elevator_enumerated_writes_store_any_value() {
    // The Access Point, Access User and Access Zone Present_Value rows these
    // pins once covered are gone: their tables define none (#1064).
    assert_unchecked_enumerated_round_trip(
        &mut AccessUserObject::new(1, "AU-1").unwrap(),
        PropertyIdentifier::USER_TYPE,
    );
    assert_unchecked_enumerated_round_trip(
        &mut ElevatorGroupObject::new(1, "EG-1").unwrap(),
        PropertyIdentifier::GROUP_MODE,
    );
}

#[test]
fn retyped_access_and_elevator_defaults_keep_their_wire_values() {
    let door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    for property in [
        PropertyIdentifier::DOOR_STATUS,
        PropertyIdentifier::LOCK_STATUS,
        PropertyIdentifier::SECURED_STATUS,
        PropertyIdentifier::DOOR_ALARM_STATE,
    ] {
        assert_eq!(read(&door, property), PropertyValue::Enumerated(0));
    }
    let point = AccessPointObject::new(1, "AP-1").unwrap();
    assert_eq!(
        read(&point, PropertyIdentifier::ACCESS_EVENT),
        PropertyValue::Enumerated(0)
    );
    let user = AccessUserObject::new(1, "AU-1").unwrap();
    assert_eq!(
        read(&user, PropertyIdentifier::USER_TYPE),
        PropertyValue::Enumerated(0)
    );
    let group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    assert_eq!(
        read(&group, PropertyIdentifier::GROUP_MODE),
        PropertyValue::Enumerated(0)
    );
}
