//! Property metadata of the Elevator trio (Clauses 12.58-12.60, Tables 12-76
//! to 12-78): the exact Property_List, required and optional sets, and the
//! write capability of every served row, each checked against dispatch.

use super::super::*;
use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyWriteCapability,
};
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::enums::{ErrorClass, ErrorCode, EscalatorMode};
use bacnet_types::error::RemoteError;

/// The class and code only: a refused list element is also named (#1048),
/// which the Fault_Signals suites check.
pub(super) fn assert_error(error: Error, expected: ErrorCode) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code }) | Error::Structured { class, code, .. }
            if class == ErrorClass::PROPERTY
                && code == expected),
        "expected {expected:?}, got {error:?}"
    );
}

fn assert_exact_sets(object: &dyn BACnetObject, all: &[P], required: &[P]) {
    let metadata = object.property_metadata();
    assert!(matches!(metadata, Cow::Borrowed(_)));
    assert_eq!(metadata.len(), all.len() + 1);
    assert_eq!(object.property_list().as_ref(), all);
    assert_eq!(object.required_properties().as_ref(), required);
    assert_eq!(
        metadata
            .iter()
            .map(|row| row.property_identifier)
            .collect::<HashSet<_>>()
            .len(),
        metadata.len()
    );
    assert!(!object.is_createable());
    assert!(object.is_deleteable());
    assert!(!object.supports_cov());
    for row in metadata.iter() {
        assert_eq!(row.presence_condition, None);
        let expected = if required.contains(&row.property_identifier) {
            RequiredRead
        } else {
            Optional
        };
        assert_eq!(row.conformance, expected, "{:?}", row.property_identifier);
        object.read_property(row.property_identifier, None).unwrap();
    }
}

fn assert_indexed_property_list(object: &dyn BACnetObject, all: &[P]) {
    let wire: Vec<_> = all
        .iter()
        .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
        .map(|p| PropertyValue::Enumerated(p.to_raw()))
        .collect();
    assert!(object.is_array_property(P::PROPERTY_LIST));
    assert_eq!(
        object.read_property(P::PROPERTY_LIST, None).unwrap(),
        PropertyValue::List(wire.clone())
    );
    assert_eq!(
        object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
        PropertyValue::Unsigned(wire.len() as u64)
    );
    for (index, value) in wire.iter().enumerate() {
        assert_eq!(
            object
                .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                .unwrap(),
            *value
        );
    }
    for index in [wire.len() as u32 + 1, u32::MAX] {
        assert_error(
            object
                .read_property(P::PROPERTY_LIST, Some(index))
                .unwrap_err(),
            ErrorCode::INVALID_ARRAY_INDEX,
        );
    }
}

#[test]
fn property_metadata_elevator_group_exact_sets_readable_rows_and_indexed_list() {
    let object = ElevatorGroupObject::new(1, "EG-1").unwrap();
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::MACHINE_ROOM_ID,
        P::GROUP_ID,
        P::GROUP_MEMBERS,
        P::GROUP_MODE,
        P::LANDING_CALLS,
        P::LANDING_CALL_CONTROL,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::MACHINE_ROOM_ID,
        P::GROUP_ID,
        P::GROUP_MEMBERS,
        P::PROPERTY_LIST,
    ];
    assert_exact_sets(&object, &all, &required);
    assert_indexed_property_list(&object, &all);
    assert_eq!(
        object.read_property(P::GROUP_MEMBERS, None).unwrap(),
        PropertyValue::List(vec![])
    );
    // Group_Members is BACnetARRAY (Table 12-76), so the service gate
    // admits an index; Landing_Calls is BACnetLIST and rejects one.
    assert!(object.is_array_property(P::GROUP_MEMBERS));
    assert!(!object.is_array_property(P::LANDING_CALLS));
}

#[test]
fn property_metadata_escalator_exact_sets_readable_rows_and_indexed_list() {
    let object = EscalatorObject::new(1, "ESC-1").unwrap();
    // Table 12-78 order.
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::DESCRIPTION,
        P::STATUS_FLAGS,
        P::ELEVATOR_GROUP,
        P::GROUP_ID,
        P::INSTALLATION_ID,
        P::POWER_MODE,
        P::OPERATION_DIRECTION,
        P::ESCALATOR_MODE,
        P::ENERGY_METER,
        P::ENERGY_METER_REF,
        P::RELIABILITY,
        P::OUT_OF_SERVICE,
        P::FAULT_SIGNALS,
        P::PASSENGER_ALARM,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::STATUS_FLAGS,
        P::ELEVATOR_GROUP,
        P::GROUP_ID,
        P::INSTALLATION_ID,
        P::OPERATION_DIRECTION,
        P::OUT_OF_SERVICE,
        P::PASSENGER_ALARM,
        P::PROPERTY_LIST,
    ];
    assert_exact_sets(&object, &all, &required);
    assert_indexed_property_list(&object, &all);
    assert_eq!(
        object.read_property(P::ESCALATOR_MODE, None).unwrap(),
        PropertyValue::Enumerated(EscalatorMode::UNKNOWN.to_raw())
    );
    // An uninitialized BACnetDeviceObjectReference: object-identifier [1]
    // Accumulator (23) instance 4194303, no device-identifier.
    assert_eq!(
        object.read_property(P::ENERGY_METER_REF, None).unwrap(),
        PropertyValue::ApplicationData(vec![0x1C, 0x05, 0xFF, 0xFF, 0xFF])
    );
    // Fault_Signals is BACnetLIST (Table 12-78), so an index is rejected.
    for p in all {
        assert!(!object.is_array_property(p), "{p:?}");
    }
}

#[test]
fn property_metadata_lift_exact_sets_readable_rows_and_indexed_list() {
    let object = LiftObject::new(1, "LIFT-1", 3).unwrap();
    // Table 12-77 order.
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::DESCRIPTION,
        P::STATUS_FLAGS,
        P::ELEVATOR_GROUP,
        P::GROUP_ID,
        P::INSTALLATION_ID,
        P::FLOOR_TEXT,
        P::ASSIGNED_LANDING_CALLS,
        P::MAKING_CAR_CALL,
        P::REGISTERED_CAR_CALL,
        P::CAR_POSITION,
        P::CAR_MOVING_DIRECTION,
        P::CAR_ASSIGNED_DIRECTION,
        P::CAR_DOOR_STATUS,
        P::CAR_DOOR_COMMAND,
        P::CAR_DOOR_ZONE,
        P::CAR_MODE,
        P::CAR_LOAD,
        P::CAR_LOAD_UNITS,
        P::NEXT_STOPPING_FLOOR,
        P::PASSENGER_ALARM,
        P::ENERGY_METER,
        P::ENERGY_METER_REF,
        P::RELIABILITY,
        P::OUT_OF_SERVICE,
        P::CAR_DRIVE_STATUS,
        P::FAULT_SIGNALS,
        P::LANDING_DOOR_STATUS,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::STATUS_FLAGS,
        P::ELEVATOR_GROUP,
        P::GROUP_ID,
        P::INSTALLATION_ID,
        P::CAR_POSITION,
        P::CAR_MOVING_DIRECTION,
        P::CAR_DOOR_STATUS,
        P::PASSENGER_ALARM,
        P::OUT_OF_SERVICE,
        P::FAULT_SIGNALS,
        P::PROPERTY_LIST,
    ];
    assert_exact_sets(&object, &all, &required);
    assert_indexed_property_list(&object, &all);
    // Floor_Text and the six per-door arrays are the BACnetARRAYs of
    // Table 12-77 the object serves; every other row rejects an index.
    let arrays = [
        P::FLOOR_TEXT,
        P::ASSIGNED_LANDING_CALLS,
        P::MAKING_CAR_CALL,
        P::REGISTERED_CAR_CALL,
        P::CAR_DOOR_STATUS,
        P::CAR_DOOR_COMMAND,
        P::LANDING_DOOR_STATUS,
    ];
    for p in all {
        assert_eq!(object.is_array_property(p), arrays.contains(&p), "{p:?}");
    }
    for p in arrays {
        assert!(matches!(
            object.read_property(p, Some(0)).unwrap(),
            PropertyValue::Unsigned(_)
        ));
    }
}

#[test]
fn property_metadata_elevator_trio_write_capabilities_match_dispatch() {
    // Constructor paired with the properties it must accept writes for,
    // always and only while Out_Of_Service is TRUE.
    type WriteCase = (fn() -> Box<dyn BACnetObject>, &'static [P], &'static [P]);
    let cases: [WriteCase; 3] = [
        (
            || Box::new(ElevatorGroupObject::new(1, "EG-1").unwrap()),
            &[
                P::DESCRIPTION,
                P::GROUP_ID,
                P::GROUP_MODE,
                P::LANDING_CALL_CONTROL,
            ],
            &[],
        ),
        (
            || Box::new(EscalatorObject::new(1, "ESC-1").unwrap()),
            &[
                P::DESCRIPTION,
                P::OUT_OF_SERVICE,
                P::POWER_MODE,
                P::OPERATION_DIRECTION,
                P::ESCALATOR_MODE,
                P::ENERGY_METER,
                P::FAULT_SIGNALS,
                P::PASSENGER_ALARM,
            ],
            &[],
        ),
        (
            || Box::new(LiftObject::new(1, "LIFT-1", 3).unwrap()),
            &[
                P::DESCRIPTION,
                P::OUT_OF_SERVICE,
                P::CAR_POSITION,
                P::CAR_MOVING_DIRECTION,
                P::CAR_LOAD,
                P::PASSENGER_ALARM,
                P::ENERGY_METER,
                P::FAULT_SIGNALS,
            ],
            // Items (c) and (d) of the Lift's Out_Of_Service description
            // (#1035, #1052).
            &[
                P::ASSIGNED_LANDING_CALLS,
                P::MAKING_CAR_CALL,
                P::REGISTERED_CAR_CALL,
                P::CAR_ASSIGNED_DIRECTION,
                P::CAR_DOOR_STATUS,
                P::CAR_DOOR_COMMAND,
                P::CAR_DOOR_ZONE,
                P::CAR_MODE,
                P::NEXT_STOPPING_FLOOR,
                P::CAR_DRIVE_STATUS,
                P::LANDING_DOOR_STATUS,
            ],
        ),
    ];
    for (make, writable, out_of_service_only) in cases {
        for out_of_service in [false, true] {
            let mut object = make();
            // Elevator Group has no Out_Of_Service (Table 12-76).
            if object.property_list().contains(&P::OUT_OF_SERVICE) {
                object
                    .write_property(
                        P::OUT_OF_SERVICE,
                        None,
                        PropertyValue::Boolean(out_of_service),
                        None,
                    )
                    .unwrap();
            }
            let original = object.property_metadata().into_owned();
            for row in &original {
                let p = row.property_identifier;
                let (capability, accepted) = if writable.contains(&p) {
                    (PropertyWriteCapability::Always, true)
                } else if out_of_service_only.contains(&p) {
                    (PropertyWriteCapability::WhenOutOfService, out_of_service)
                } else {
                    (PropertyWriteCapability::ReadOnly, false)
                };
                assert_eq!(row.write_capability, capability, "{p:?}");
                assert_eq!(
                    object.is_writable_property(p),
                    capability.is_writable(),
                    "{p:?}"
                );
                let value = object.read_property(p, None).unwrap();
                let result = object.write_property(p, None, value, None);
                if accepted {
                    result.unwrap();
                } else {
                    assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
                }
            }
            // Object_Name has no network write route: a rename falls
            // through to WRITE_ACCESS_DENIED even with a well-formed value.
            assert!(!object.is_writable_property(P::OBJECT_NAME));
            assert_error(
                object
                    .write_property(
                        P::OBJECT_NAME,
                        None,
                        PropertyValue::CharacterString("renamed".into()),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert_eq!(object.property_metadata().as_ref(), original);
        }
    }
}

#[test]
fn property_metadata_elevator_trio_unserved_rows_stay_unknown() {
    fn assert_unserved(object: &mut dyn BACnetObject, p: P) {
        assert!(!object.is_writable_property(p));
        assert_error(
            object.read_property(p, None).unwrap_err(),
            ErrorCode::UNKNOWN_PROPERTY,
        );
        assert_error(
            object
                .write_property(p, None, PropertyValue::Null, None)
                .unwrap_err(),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }

    // Table 12-76 defines no Status_Flags, Out_Of_Service or Reliability.
    let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    assert_unserved(&mut group, P::STATUS_FLAGS);
    assert_unserved(&mut group, P::OUT_OF_SERVICE);
    assert_unserved(&mut group, P::RELIABILITY);
    // Escalator: a Lift-only row, and Table 12-78 rows it doesn't serve.
    let mut escalator = EscalatorObject::new(1, "ESC-1").unwrap();
    for p in [P::CAR_POSITION, P::EVENT_STATE, P::TIME_DELAY] {
        assert_unserved(&mut escalator, p);
    }
    // Lift: Tracking_Value and Floor_Number aren't Table 12-77 rows
    // (#1021), and Car_Door_Text and the deck rows are optional rows it
    // doesn't serve.
    let mut lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
    for p in [
        P::TRACKING_VALUE,
        P::FLOOR_NUMBER,
        P::CAR_DOOR_TEXT,
        P::HIGHER_DECK,
        P::LOWER_DECK,
    ] {
        assert_unserved(&mut lift, p);
    }
}
