use super::{ElevatorGroupObject, EscalatorObject, LiftObject};
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Canonical effective rows for the Elevator trio (ASHRAE 135-2020; PDF = printed + 2):
// - ElevatorGroup (type 57, §12.58 Table 12-76; printed p. 578 / PDF p. 580)
// - Lift (type 59, §12.59 Table 12-77; printed pp. 582-583 / PDF pp. 584-585)
// - Escalator (type 58, §12.60 Table 12-78; printed p. 594 / PDF p. 596)
// Order preserves each legacy projection; PROPERTY_LIST is appended so the
// projection helper omits it while required_properties keeps it. Lift
// FLOOR_NUMBER (readable but unlisted, aliasing Tracking_Value) is appended
// after the legacy rows (LoadControl EVENT_STATE precedent) so the served
// projection gains exactly one row. Only implemented rows are described:
// table rows the objects do not serve (ElevatorGroup Machine_Room_ID and
// audit/tag/profile rows; Escalator Elevator_Group, Group_ID,
// Installation_ID, event/intrinsic/audit/tag/profile rows; Lift
// Elevator_Group, Group_ID, Installation_ID, Passenger_Alarm, Fault_Signals,
// door/deck/call/event/intrinsic/audit/tag/profile rows) stay absent until
// dispatch exists.
// Object_Identifier, Object_Name, and Object_Type carry the table R code and
// have no network write route, so RequiredRead/ReadOnly. Object_Name
// explicitly documents the denial: a rename falls through to
// WRITE_ACCESS_DENIED. Description carries the table O code with a routed
// CharacterString write arm, so Optional/Always. Table-R served rows with no
// network write route stay RequiredRead/ReadOnly; table-R rows with a write
// arm are RequiredRead/Always. Table-O served rows are Optional, with Always
// exactly where dispatch accepts the write.
// Status_Flags, Out_Of_Service, and Reliability on ElevatorGroup are served
// through the shared common read arms but Table 12-76 carries no such rows,
// so the NetworkPort precedent keeps them RequiredRead/ReadOnly and
// RequiredRead/Always (Out_Of_Service has the routed Boolean arm). Lift and
// Escalator Tables 12-77/12-78 do list them (Status_Flags R, Out_Of_Service
// R, Reliability O), which agrees with the same capabilities. Tracking_Value
// is served with a write arm but appears in neither Table 12-77 production
// nor the Lift property descriptions, so it stays Optional/Always rather
// than advertising a required row the table does not define; Floor_Number
// mirrors its Tracking_Value readback with no write arm, so
// Optional/ReadOnly.
// Writability is Always, never WhenOutOfService: the Lift §12.59 and
// Escalator §12.60 Out_Of_Service descriptions gate simulation writes behind
// Out_Of_Service TRUE (items (c)-(e)), but dispatch routes every write arm
// unconditionally and the writability suites pin in-service writes, so the
// metadata mirrors dispatch rather than the OOS-gate paragraph.
// Presence is None throughout: the implementation models no
// lift-group-conditional, intrinsic-reporting, or paired-text gating on this
// family. The trio is not createable at runtime (the network factory builds
// only the eight analog/binary/multi-state input/output/value types, so the
// is_createable=false default holds) and remains deleteable (delete denies
// only Device and NetworkPort, so the is_deleteable=true default holds);
// neither needs an override. Array gating, writability, and COV also keep
// their defaults: Group_Members admits an index (BACnetARRAY per Tables
// 12-57/12-76) while every other served row rejects one.
const ELEVATOR_GROUP_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::GROUP_ID, RequiredRead, None, Always),
    PropertyMetadata::new(P::GROUP_MEMBERS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::GROUP_MODE, Optional, None, Always),
    PropertyMetadata::new(P::LANDING_CALLS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::LANDING_CALL_CONTROL, Optional, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const ESCALATOR_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ESCALATOR_MODE, Optional, None, Always),
    PropertyMetadata::new(P::FAULT_SIGNALS, Optional, None, Always),
    PropertyMetadata::new(P::ENERGY_METER, Optional, None, Always),
    PropertyMetadata::new(P::ENERGY_METER_REF, Optional, None, ReadOnly),
    PropertyMetadata::new(P::POWER_MODE, Optional, None, Always),
    PropertyMetadata::new(P::OPERATION_DIRECTION, RequiredRead, None, Always),
    PropertyMetadata::new(P::PASSENGER_ALARM, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const LIFT_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::TRACKING_VALUE, Optional, None, Always),
    PropertyMetadata::new(P::CAR_POSITION, RequiredRead, None, Always),
    PropertyMetadata::new(P::CAR_MOVING_DIRECTION, RequiredRead, None, Always),
    PropertyMetadata::new(P::CAR_DOOR_STATUS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::CAR_LOAD, Optional, None, Always),
    PropertyMetadata::new(P::LANDING_DOOR_STATUS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::FLOOR_TEXT, Optional, None, ReadOnly),
    PropertyMetadata::new(P::ENERGY_METER, Optional, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::FLOOR_NUMBER, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_elevator_group_object(
    _object: &ElevatorGroupObject,
) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(ELEVATOR_GROUP_BASE)
}

pub(super) fn for_escalator_object(_object: &EscalatorObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(ESCALATOR_BASE)
}

pub(super) fn for_lift_object(_object: &LiftObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(LIFT_BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property_metadata::PropertyWriteCapability;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{
        ErrorClass, ErrorCode, EscalatorFault, EscalatorMode, EscalatorOperationDirection,
    };
    use bacnet_types::error::Error;
    use bacnet_types::primitives::PropertyValue;
    use std::collections::HashSet;

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Protocol { class, code }
                if class == ErrorClass::PROPERTY.to_raw() as u32
                    && code == expected.to_raw() as u32),
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
            P::GROUP_ID,
            P::GROUP_MEMBERS,
            P::GROUP_MODE,
            P::LANDING_CALLS,
            P::LANDING_CALL_CONTROL,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::GROUP_ID,
            P::GROUP_MEMBERS,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
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
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::ESCALATOR_MODE,
            P::FAULT_SIGNALS,
            P::ENERGY_METER,
            P::ENERGY_METER_REF,
            P::POWER_MODE,
            P::OPERATION_DIRECTION,
            P::PASSENGER_ALARM,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::OPERATION_DIRECTION,
            P::PASSENGER_ALARM,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        assert_eq!(
            object.read_property(P::ESCALATOR_MODE, None).unwrap(),
            PropertyValue::Enumerated(EscalatorMode::UNKNOWN.to_raw())
        );
        assert_eq!(
            object.read_property(P::ENERGY_METER_REF, None).unwrap(),
            PropertyValue::OctetString(vec![])
        );
        // Fault_Signals is BACnetLIST (Table 12-78), so an index is rejected.
        assert!(!object.is_array_property(P::FAULT_SIGNALS));
    }

    #[test]
    fn property_metadata_lift_exact_sets_readable_rows_and_indexed_list() {
        let object = LiftObject::new(1, "LIFT-1", 3).unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::TRACKING_VALUE,
            P::CAR_POSITION,
            P::CAR_MOVING_DIRECTION,
            P::CAR_DOOR_STATUS,
            P::CAR_LOAD,
            P::LANDING_DOOR_STATUS,
            P::FLOOR_TEXT,
            P::ENERGY_METER,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::FLOOR_NUMBER,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::CAR_POSITION,
            P::CAR_MOVING_DIRECTION,
            P::CAR_DOOR_STATUS,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        // Floor_Number aliases Tracking_Value from a fresh object.
        assert_eq!(
            object.read_property(P::FLOOR_NUMBER, None).unwrap(),
            PropertyValue::Unsigned(1)
        );
        assert_eq!(
            object.read_property(P::LANDING_DOOR_STATUS, None).unwrap(),
            PropertyValue::Unsigned(3)
        );
        // Floor_Text, Car_Door_Status, and Landing_Door_Status are
        // BACnetARRAY in Table 12-77 but keep the default index rejection
        // (no array override on this family).
        assert!(!object.is_array_property(P::FLOOR_TEXT));
        assert!(!object.is_array_property(P::CAR_DOOR_STATUS));
        assert!(!object.is_array_property(P::LANDING_DOOR_STATUS));
    }

    #[test]
    fn property_metadata_elevator_trio_write_capabilities_match_dispatch() {
        let cases: [(fn() -> Box<dyn BACnetObject>, &[P]); 3] = [
            (
                || Box::new(ElevatorGroupObject::new(1, "EG-1").unwrap()),
                &[
                    P::DESCRIPTION,
                    P::OUT_OF_SERVICE,
                    P::GROUP_ID,
                    P::GROUP_MODE,
                    P::LANDING_CALL_CONTROL,
                ],
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
            ),
            (
                || Box::new(LiftObject::new(1, "LIFT-1", 3).unwrap()),
                &[
                    P::DESCRIPTION,
                    P::OUT_OF_SERVICE,
                    P::TRACKING_VALUE,
                    P::CAR_POSITION,
                    P::CAR_MOVING_DIRECTION,
                    P::CAR_LOAD,
                ],
            ),
        ];
        for (make, writable) in cases {
            for out_of_service in [false, true] {
                let mut object = make();
                object
                    .write_property(
                        P::OUT_OF_SERVICE,
                        None,
                        PropertyValue::Boolean(out_of_service),
                        None,
                    )
                    .unwrap();
                let original = object.property_metadata().into_owned();
                for row in &original {
                    let p = row.property_identifier;
                    let capability = if writable.contains(&p) {
                        PropertyWriteCapability::Always
                    } else {
                        PropertyWriteCapability::ReadOnly
                    };
                    assert_eq!(row.write_capability, capability, "{p:?}");
                    assert_eq!(
                        object.is_writable_property(p),
                        capability.is_writable(),
                        "{p:?}"
                    );
                    let value = object.read_property(p, None).unwrap();
                    let result = object.write_property(p, None, value, None);
                    if capability.is_writable() {
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
    fn property_metadata_elevator_group_writes_store_verbatim() {
        for out_of_service in [false, true] {
            let mut object = ElevatorGroupObject::new(1, "EG-1").unwrap();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            // Routed arms store verbatim in both states.
            for (p, value, expected) in [
                (
                    P::GROUP_ID,
                    PropertyValue::Unsigned(47),
                    PropertyValue::Unsigned(47),
                ),
                (
                    P::GROUP_MODE,
                    PropertyValue::Enumerated(2),
                    PropertyValue::Enumerated(2),
                ),
                (
                    P::LANDING_CALL_CONTROL,
                    PropertyValue::Enumerated(1),
                    PropertyValue::Enumerated(1),
                ),
            ] {
                object.write_property(p, None, value, None).unwrap();
                assert_eq!(object.read_property(p, None).unwrap(), expected);
            }
            // Mistyped values are rejected without changing state.
            for (p, value) in [
                (P::GROUP_ID, PropertyValue::Enumerated(47)),
                (P::GROUP_MODE, PropertyValue::Unsigned(2)),
                (P::LANDING_CALL_CONTROL, PropertyValue::Unsigned(1)),
                (P::DESCRIPTION, PropertyValue::Null),
                (P::OUT_OF_SERVICE, PropertyValue::Null),
            ] {
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            // Group_Members and Landing_Calls have no network write route:
            // even their read-back values are denied on write.
            for p in [P::GROUP_MEMBERS, P::LANDING_CALLS] {
                let value = object.read_property(p, None).unwrap();
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                assert!(!object.is_writable_property(p));
            }
        }
    }

    #[test]
    fn property_metadata_escalator_domain_validation_matches_dispatch() {
        for out_of_service in [false, true] {
            let mut object = EscalatorObject::new(1, "ESC-1").unwrap();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            // Escalator_Mode and Operation_Direction share the Clause 23.1
            // domain: named values plus 1024..=65535 round-trip, while the
            // reserved gap and oversized values fail atomically.
            for (p, named, prior) in [
                (
                    P::ESCALATOR_MODE,
                    EscalatorMode::UP.to_raw(),
                    EscalatorMode::STOP.to_raw(),
                ),
                (
                    P::OPERATION_DIRECTION,
                    EscalatorOperationDirection::DOWN_REDUCED_SPEED.to_raw(),
                    EscalatorOperationDirection::UP_RATED_SPEED.to_raw(),
                ),
            ] {
                for raw in [named, 1024, 65535] {
                    object
                        .write_property(p, None, PropertyValue::Enumerated(raw), None)
                        .unwrap_or_else(|e| panic!("{p:?} {raw} must be accepted: {e:?}"));
                    assert_eq!(
                        object.read_property(p, None).unwrap(),
                        PropertyValue::Enumerated(raw)
                    );
                }
                object
                    .write_property(p, None, PropertyValue::Enumerated(prior), None)
                    .unwrap();
                for raw in [6u32, 1023, 65536, u32::MAX] {
                    assert_error(
                        object
                            .write_property(p, None, PropertyValue::Enumerated(raw), None)
                            .unwrap_err(),
                        ErrorCode::VALUE_OUT_OF_RANGE,
                    );
                }
                assert_eq!(
                    object.read_property(p, None).unwrap(),
                    PropertyValue::Enumerated(prior)
                );
                assert_error(
                    object
                        .write_property(p, None, PropertyValue::Unsigned(prior as u64), None)
                        .unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            // Energy_Meter stores finite values and refuses the rest.
            object
                .write_property(P::ENERGY_METER, None, PropertyValue::Real(42.0), None)
                .unwrap();
            for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert_error(
                    object
                        .write_property(P::ENERGY_METER, None, PropertyValue::Real(value), None)
                        .unwrap_err(),
                    ErrorCode::VALUE_OUT_OF_RANGE,
                );
            }
            assert_eq!(
                object.read_property(P::ENERGY_METER, None).unwrap(),
                PropertyValue::Real(42.0)
            );
            // Fault_Signals dedups atomically: duplicates, reserved values,
            // and mistyped shapes fail without touching the stored set.
            let prior = PropertyValue::List(vec![
                PropertyValue::Enumerated(EscalatorFault::CONTROLLER_FAULT.to_raw()),
                PropertyValue::Enumerated(1024),
            ]);
            object
                .write_property(P::FAULT_SIGNALS, None, prior.clone(), None)
                .unwrap();
            for (values, expected) in [
                (
                    vec![
                        PropertyValue::Enumerated(1024),
                        PropertyValue::Enumerated(1024),
                    ],
                    ErrorCode::VALUE_OUT_OF_RANGE,
                ),
                (
                    vec![PropertyValue::Enumerated(9)],
                    ErrorCode::VALUE_OUT_OF_RANGE,
                ),
                (
                    vec![PropertyValue::Enumerated(1023)],
                    ErrorCode::VALUE_OUT_OF_RANGE,
                ),
                (
                    vec![PropertyValue::Unsigned(8)],
                    ErrorCode::INVALID_DATA_TYPE,
                ),
            ] {
                assert_error(
                    object
                        .write_property(P::FAULT_SIGNALS, None, PropertyValue::List(values), None)
                        .unwrap_err(),
                    expected,
                );
                assert_eq!(object.read_property(P::FAULT_SIGNALS, None).unwrap(), prior);
            }
            // Energy_Meter_Ref has no network write route; Power_Mode and
            // Passenger_Alarm store Booleans verbatim.
            let energy_ref = object.read_property(P::ENERGY_METER_REF, None).unwrap();
            assert_error(
                object
                    .write_property(P::ENERGY_METER_REF, None, energy_ref, None)
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            for p in [P::POWER_MODE, P::PASSENGER_ALARM] {
                object
                    .write_property(p, None, PropertyValue::Boolean(true), None)
                    .unwrap();
                assert_eq!(
                    object.read_property(p, None).unwrap(),
                    PropertyValue::Boolean(true)
                );
                assert_error(
                    object
                        .write_property(p, None, PropertyValue::Enumerated(1), None)
                        .unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
        }
    }

    #[test]
    fn property_metadata_lift_writes_store_verbatim_with_range_gates() {
        for out_of_service in [false, true] {
            let mut object = LiftObject::new(1, "LIFT-1", 3).unwrap();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            // Tracking_Value and Car_Position store Unsigned verbatim, and
            // Floor_Number keeps aliasing Tracking_Value after the write.
            for (p, raw) in [(P::TRACKING_VALUE, 5), (P::CAR_POSITION, 2)] {
                object
                    .write_property(p, None, PropertyValue::Unsigned(raw), None)
                    .unwrap();
                assert_eq!(
                    object.read_property(p, None).unwrap(),
                    PropertyValue::Unsigned(raw)
                );
            }
            assert_eq!(
                object.read_property(P::FLOOR_NUMBER, None).unwrap(),
                PropertyValue::Unsigned(5)
            );
            // Car_Moving_Direction admits 0..=3 and refuses the rest.
            object
                .write_property(
                    P::CAR_MOVING_DIRECTION,
                    None,
                    PropertyValue::Enumerated(2),
                    None,
                )
                .unwrap();
            assert_error(
                object
                    .write_property(
                        P::CAR_MOVING_DIRECTION,
                        None,
                        PropertyValue::Enumerated(4),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            assert_eq!(
                object.read_property(P::CAR_MOVING_DIRECTION, None).unwrap(),
                PropertyValue::Enumerated(2)
            );
            // Car_Load admits 0..=100 percent and refuses the rest.
            object
                .write_property(P::CAR_LOAD, None, PropertyValue::Unsigned(50), None)
                .unwrap();
            assert_error(
                object
                    .write_property(P::CAR_LOAD, None, PropertyValue::Unsigned(101), None)
                    .unwrap_err(),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            assert_eq!(
                object.read_property(P::CAR_LOAD, None).unwrap(),
                PropertyValue::Unsigned(50)
            );
            // Mistyped values are rejected without changing state.
            for (p, value) in [
                (P::TRACKING_VALUE, PropertyValue::Enumerated(5)),
                (P::CAR_POSITION, PropertyValue::Enumerated(2)),
                (P::CAR_MOVING_DIRECTION, PropertyValue::Unsigned(2)),
                (P::CAR_LOAD, PropertyValue::Real(50.0)),
                (P::DESCRIPTION, PropertyValue::Null),
                (P::OUT_OF_SERVICE, PropertyValue::Null),
            ] {
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            // Floor_Number is readable but has no write arm: even its own
            // readback is denied on write.
            assert!(!object.is_writable_property(P::FLOOR_NUMBER));
            assert_error(
                object
                    .write_property(P::FLOOR_NUMBER, None, PropertyValue::Unsigned(5), None)
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            // Car_Door_Status, Landing_Door_Status, Floor_Text, and
            // Energy_Meter have no network write route.
            for p in [
                P::CAR_DOOR_STATUS,
                P::LANDING_DOOR_STATUS,
                P::FLOOR_TEXT,
                P::ENERGY_METER,
            ] {
                let value = object.read_property(p, None).unwrap();
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                assert!(!object.is_writable_property(p));
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
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }

        // Machine_Room_ID is the Table 12-76 R row with no read arm.
        let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
        assert_unserved(&mut group, P::MACHINE_ROOM_ID);
        // Elevator_Group, Group_ID, and Installation_ID are Table 12-78 R
        // rows with no read arm on Escalator.
        let mut escalator = EscalatorObject::new(1, "ESC-1").unwrap();
        assert_unserved(&mut escalator, P::ELEVATOR_GROUP);
        assert_unserved(&mut escalator, P::GROUP_ID);
        assert_unserved(&mut escalator, P::INSTALLATION_ID);
        // Elevator_Group, Group_ID, and Installation_ID are Table 12-77 R
        // rows with no read arm on Lift; Passenger_Alarm (R) and
        // Fault_Signals (R) likewise have no Lift arm.
        let mut lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
        assert_unserved(&mut lift, P::ELEVATOR_GROUP);
        assert_unserved(&mut lift, P::GROUP_ID);
        assert_unserved(&mut lift, P::INSTALLATION_ID);
        assert_unserved(&mut lift, P::PASSENGER_ALARM);
        assert_unserved(&mut lift, P::FAULT_SIGNALS);
    }
}
