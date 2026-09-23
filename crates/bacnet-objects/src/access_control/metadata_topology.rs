use super::{AccessDoorObject, AccessPointObject, AccessZoneObject};
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Canonical effective rows for the Access Topology trio (ASHRAE 135-2020; PDF = printed + 2):
// - Access Door (type 30, §12.26 Table 12-30; printed p. 325 / PDF p. 327)
// - Access Point (type 33, §12.31 Table 12-36; printed p. 365 / PDF p. 367)
// - Access Zone (type 36, §12.32 Table 12-37; printed p. 382 / PDF p. 384)
// Order preserves each legacy projection; Door Event_State, Priority_Array,
// and Relinquish_Default (readable but unlisted) are appended after the
// legacy rows (Lift FLOOR_NUMBER precedent) so the served projection gains
// exactly three rows, and Property_List is appended so the projection helper
// omits it while required_properties keeps it. Only implemented rows are
// described: table rows the objects do not serve (Door pulse/unlock timers,
// Current_Command_Priority, Authentication_Status, Occupancy_State,
// event/intrinsic/audit/tag/profile rows) stay absent until dispatch exists.
// Object_Identifier, Object_Name, and Object_Type carry the table R code and
// have no network write route, so RequiredRead/ReadOnly. Object_Name
// explicitly documents the denial: a rename falls through to
// WRITE_ACCESS_DENIED (no object has a write_object_name arm). Description
// carries the table O code with a routed CharacterString write arm, so
// Optional/Always. Out_Of_Service carries the table R code with the routed
// Boolean arm, so RequiredRead/Always.
// Door Present_Value carries the table W code (commandable) with the
// priority-slot write arm, so RequiredWrite/Always. Relinquish_Default
// carries the table R code with the 0..=3 setter arm, so
// RequiredRead/Always. Priority_Array and Event_State are served readable
// rows with no write arm, so RequiredRead/ReadOnly. Door_Status, Lock_Status,
// Secured_Status, Door_Alarm_State, and Door_Members carry the table O code
// with no write arm, so Optional/ReadOnly.
// Point Present_Value is an implementation-extra row (Table 12-36 has no
// Present_Value row) with a routed Enumerated arm, so Optional/Always.
// Access_Event, Access_Event_Tag, Access_Event_Time, Access_Doors, and
// Event_State carry the table R code with no write arm, so
// RequiredRead/ReadOnly.
// Zone Global_Identifier carries the table W code with the routed Unsigned
// arm, so RequiredWrite/Always. Present_Value and Access_Doors are
// implementation-extra rows (Present_Value with a routed arm, so
// Optional/Always; Access_Doors with none, so Optional/ReadOnly).
// Occupancy_Count carries the table O code with no arm, so
// Optional/ReadOnly; Entry_Points and Exit_Points carry the table R code
// with no arm, so RequiredRead/ReadOnly. Status_Flags and Reliability carry
// the table R code with no network write route, so RequiredRead/ReadOnly.
// Writability is Always, never WhenOutOfService: dispatch routes every write
// arm unconditionally and the suites pin in-service writes, so the metadata
// mirrors dispatch. Presence is None throughout: the implementation models
// no commandable, intrinsic-reporting, or paired-text gating on this family.
// The trio is not createable at runtime (the network factory builds only the
// eight analog/binary/multi-state input/output/value types, so the
// is_createable=false default holds) and remains deleteable (delete denies
// only Device and NetworkPort, so the is_deleteable=true default holds);
// neither needs an override. Array gating keeps the default: Priority_Array
// (BACnetARRAY per Table 12-30) and Property_List admit an index while every
// other served row rejects one. COV keeps its override: supports_cov=true on
// Access Door, and the COV gating path (read_property plus
// supports_cov_property to supports_cov) never consults metadata.
const ACCESS_DOOR_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredWrite, None, Always),
    PropertyMetadata::new(P::DOOR_STATUS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::LOCK_STATUS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::SECURED_STATUS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::DOOR_ALARM_STATE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::DOOR_MEMBERS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, RequiredRead, None, Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const ACCESS_POINT_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, Optional, None, Always),
    PropertyMetadata::new(P::ACCESS_EVENT, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ACCESS_EVENT_TAG, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ACCESS_EVENT_TIME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ACCESS_DOORS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const ACCESS_ZONE_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, Optional, None, Always),
    PropertyMetadata::new(P::GLOBAL_IDENTIFIER, RequiredWrite, None, Always),
    PropertyMetadata::new(P::OCCUPANCY_COUNT, Optional, None, ReadOnly),
    PropertyMetadata::new(P::ACCESS_DOORS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::ENTRY_POINTS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EXIT_POINTS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_access_door_object(_object: &AccessDoorObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(ACCESS_DOOR_BASE)
}

pub(super) fn for_access_point_object(_object: &AccessPointObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(ACCESS_POINT_BASE)
}

pub(super) fn for_access_zone_object(_object: &AccessZoneObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(ACCESS_ZONE_BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property_metadata::PropertyWriteCapability;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::{Date, PropertyValue, Time};
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
        for row in metadata.iter() {
            assert_eq!(row.presence_condition, None);
            let expected = if (row.property_identifier == P::PRESENT_VALUE
                && object.object_identifier().object_type() == ObjectType::ACCESS_DOOR)
                || row.property_identifier == P::GLOBAL_IDENTIFIER
            {
                RequiredWrite
            } else if required.contains(&row.property_identifier) {
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
    fn property_metadata_access_door_exact_sets_readable_rows_and_indexed_list() {
        let object = AccessDoorObject::new(1, "DOOR-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::DOOR_STATUS,
            P::LOCK_STATUS,
            P::SECURED_STATUS,
            P::DOOR_ALARM_STATE,
            P::DOOR_MEMBERS,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::EVENT_STATE,
            P::PRIORITY_ARRAY,
            P::RELINQUISH_DEFAULT,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::EVENT_STATE,
            P::PRIORITY_ARRAY,
            P::RELINQUISH_DEFAULT,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        assert!(object.supports_cov());
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::RELINQUISH_DEFAULT, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::EVENT_STATE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::DOOR_MEMBERS, None).unwrap(),
            PropertyValue::List(vec![])
        );
        // Priority_Array is BACnetARRAY (Table 12-30), so the service gate
        // admits an index; Door_Members is BACnetLIST and rejects one.
        assert!(object.is_array_property(P::PRIORITY_ARRAY));
        assert_eq!(
            object.read_property(P::PRIORITY_ARRAY, Some(0)).unwrap(),
            PropertyValue::Unsigned(16)
        );
        assert_eq!(
            object.read_property(P::PRIORITY_ARRAY, Some(1)).unwrap(),
            PropertyValue::Null
        );
        assert!(!object.is_array_property(P::DOOR_MEMBERS));
        assert!(!object.is_array_property(P::DOOR_STATUS));
    }

    #[test]
    fn property_metadata_access_point_exact_sets_readable_rows_and_indexed_list() {
        let object = AccessPointObject::new(1, "AP-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::ACCESS_EVENT,
            P::ACCESS_EVENT_TAG,
            P::ACCESS_EVENT_TIME,
            P::ACCESS_DOORS,
            P::EVENT_STATE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::ACCESS_EVENT,
            P::ACCESS_EVENT_TAG,
            P::ACCESS_EVENT_TIME,
            P::ACCESS_DOORS,
            P::EVENT_STATE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        assert!(!object.supports_cov());
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::ACCESS_EVENT, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::ACCESS_EVENT_TAG, None).unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            object.read_property(P::ACCESS_EVENT_TIME, None).unwrap(),
            PropertyValue::List(vec![
                PropertyValue::Date(Date {
                    year: 0xFF,
                    month: 0xFF,
                    day: 0xFF,
                    day_of_week: 0xFF,
                }),
                PropertyValue::Time(Time {
                    hour: 0xFF,
                    minute: 0xFF,
                    second: 0xFF,
                    hundredths: 0xFF,
                }),
            ])
        );
        assert_eq!(
            object.read_property(P::ACCESS_DOORS, None).unwrap(),
            PropertyValue::List(vec![])
        );
        // Access_Doors and Access_Event_Time are BACnetLIST rows, so an
        // index is rejected even though the values are lists.
        assert!(!object.is_array_property(P::ACCESS_DOORS));
        assert!(!object.is_array_property(P::ACCESS_EVENT_TIME));
    }

    #[test]
    fn property_metadata_access_zone_exact_sets_readable_rows_and_indexed_list() {
        let object = AccessZoneObject::new(1, "ZONE-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::GLOBAL_IDENTIFIER,
            P::OCCUPANCY_COUNT,
            P::ACCESS_DOORS,
            P::ENTRY_POINTS,
            P::EXIT_POINTS,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::GLOBAL_IDENTIFIER,
            P::ENTRY_POINTS,
            P::EXIT_POINTS,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        assert!(!object.supports_cov());
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::GLOBAL_IDENTIFIER, None).unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            object.read_property(P::OCCUPANCY_COUNT, None).unwrap(),
            PropertyValue::Unsigned(0)
        );
        for p in [P::ACCESS_DOORS, P::ENTRY_POINTS, P::EXIT_POINTS] {
            assert_eq!(
                object.read_property(p, None).unwrap(),
                PropertyValue::List(vec![])
            );
            assert!(!object.is_array_property(p));
        }
    }

    #[test]
    fn property_metadata_access_trio_write_capabilities_match_dispatch() {
        let cases: [(fn() -> Box<dyn BACnetObject>, &[P]); 3] = [
            (
                || Box::new(AccessDoorObject::new(1, "DOOR-1").unwrap()),
                &[
                    P::DESCRIPTION,
                    P::OUT_OF_SERVICE,
                    P::PRESENT_VALUE,
                    P::RELINQUISH_DEFAULT,
                ],
            ),
            (
                || Box::new(AccessPointObject::new(1, "AP-1").unwrap()),
                &[P::DESCRIPTION, P::OUT_OF_SERVICE, P::PRESENT_VALUE],
            ),
            (
                || Box::new(AccessZoneObject::new(1, "ZONE-1").unwrap()),
                &[
                    P::DESCRIPTION,
                    P::OUT_OF_SERVICE,
                    P::PRESENT_VALUE,
                    P::GLOBAL_IDENTIFIER,
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
    fn property_metadata_access_door_writes_command_priority_and_gate_relinquish() {
        for out_of_service in [false, true] {
            let mut object = AccessDoorObject::new(1, "DOOR-1").unwrap();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            // A priority write commands Present_Value; relinquishing the slot
            // falls back to Relinquish_Default.
            object
                .write_property(
                    P::PRESENT_VALUE,
                    None,
                    PropertyValue::Enumerated(1),
                    Some(8),
                )
                .unwrap();
            assert_eq!(
                object.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Enumerated(1)
            );
            assert_eq!(
                object.read_property(P::PRIORITY_ARRAY, Some(8)).unwrap(),
                PropertyValue::Enumerated(1)
            );
            object
                .write_property(P::PRESENT_VALUE, None, PropertyValue::Null, Some(8))
                .unwrap();
            assert_eq!(
                object.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Enumerated(0)
            );
            // Relinquish_Default admits the four BACnetDoorValue productions
            // and resolves Present_Value anew from the empty array.
            for raw in [0u32, 1, 2, 3] {
                object
                    .write_property(
                        P::RELINQUISH_DEFAULT,
                        None,
                        PropertyValue::Enumerated(raw),
                        None,
                    )
                    .unwrap();
                assert_eq!(
                    object.read_property(P::RELINQUISH_DEFAULT, None).unwrap(),
                    PropertyValue::Enumerated(raw)
                );
                assert_eq!(
                    object.read_property(P::PRESENT_VALUE, None).unwrap(),
                    PropertyValue::Enumerated(raw)
                );
            }
            object
                .write_property(
                    P::RELINQUISH_DEFAULT,
                    None,
                    PropertyValue::Enumerated(1),
                    None,
                )
                .unwrap();
            assert_error(
                object
                    .write_property(
                        P::RELINQUISH_DEFAULT,
                        None,
                        PropertyValue::Enumerated(4),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            assert_eq!(
                object.read_property(P::RELINQUISH_DEFAULT, None).unwrap(),
                PropertyValue::Enumerated(1)
            );
            // Mistyped values are rejected without changing state.
            for (p, value) in [
                (P::PRESENT_VALUE, PropertyValue::Real(1.0)),
                (P::RELINQUISH_DEFAULT, PropertyValue::Real(1.0)),
                (P::DESCRIPTION, PropertyValue::Unsigned(1)),
                (P::OUT_OF_SERVICE, PropertyValue::Unsigned(1)),
            ] {
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            assert_eq!(
                object.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Enumerated(1)
            );
            // Table-O status rows and the readable-only rows deny even their
            // own readback on write.
            for p in [
                P::DOOR_STATUS,
                P::LOCK_STATUS,
                P::SECURED_STATUS,
                P::DOOR_ALARM_STATE,
                P::DOOR_MEMBERS,
                P::PRIORITY_ARRAY,
                P::EVENT_STATE,
                P::STATUS_FLAGS,
                P::RELIABILITY,
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
    fn property_metadata_access_point_and_zone_writes_store_verbatim() {
        for out_of_service in [false, true] {
            let mut point = AccessPointObject::new(1, "AP-1").unwrap();
            point
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            point
                .write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(5), None)
                .unwrap();
            assert_eq!(
                point.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Enumerated(5)
            );
            assert_error(
                point
                    .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(5.0), None)
                    .unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
            assert_eq!(
                point.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Enumerated(5)
            );
            for p in [
                P::ACCESS_EVENT,
                P::ACCESS_EVENT_TAG,
                P::ACCESS_EVENT_TIME,
                P::ACCESS_DOORS,
                P::EVENT_STATE,
                P::STATUS_FLAGS,
                P::RELIABILITY,
            ] {
                let value = point.read_property(p, None).unwrap();
                assert_error(
                    point.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                assert!(!point.is_writable_property(p));
            }
            let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
            zone.write_property(
                P::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(out_of_service),
                None,
            )
            .unwrap();
            zone.write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(2), None)
                .unwrap();
            assert_eq!(
                zone.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Enumerated(2)
            );
            zone.write_property(
                P::GLOBAL_IDENTIFIER,
                None,
                PropertyValue::Unsigned(99),
                None,
            )
            .unwrap();
            assert_eq!(
                zone.read_property(P::GLOBAL_IDENTIFIER, None).unwrap(),
                PropertyValue::Unsigned(99)
            );
            for (p, value) in [
                (P::PRESENT_VALUE, PropertyValue::Real(2.0)),
                (P::GLOBAL_IDENTIFIER, PropertyValue::Enumerated(99)),
                (P::DESCRIPTION, PropertyValue::Unsigned(1)),
                (P::OUT_OF_SERVICE, PropertyValue::Unsigned(1)),
            ] {
                assert_error(
                    zone.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            assert_eq!(
                zone.read_property(P::GLOBAL_IDENTIFIER, None).unwrap(),
                PropertyValue::Unsigned(99)
            );
            for p in [
                P::OCCUPANCY_COUNT,
                P::ACCESS_DOORS,
                P::ENTRY_POINTS,
                P::EXIT_POINTS,
                P::STATUS_FLAGS,
                P::RELIABILITY,
            ] {
                let value = zone.read_property(p, None).unwrap();
                assert_error(
                    zone.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                assert!(!zone.is_writable_property(p));
            }
        }
    }

    #[test]
    fn property_metadata_access_trio_unserved_rows_stay_unknown() {
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

        // Door pulse/unlock timers and Current_Command_Priority are Table
        // 12-30 rows with no read arm.
        let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
        assert_unserved(&mut door, P::DOOR_PULSE_TIME);
        assert_unserved(&mut door, P::DOOR_EXTENDED_PULSE_TIME);
        assert_unserved(&mut door, P::DOOR_UNLOCK_DELAY_TIME);
        assert_unserved(&mut door, P::CURRENT_COMMAND_PRIORITY);
        // Authentication_Status is the Table 12-36 R row with no read arm.
        let mut point = AccessPointObject::new(1, "AP-1").unwrap();
        assert_unserved(&mut point, P::AUTHENTICATION_STATUS);
        // Occupancy_State is the Table 12-37 row with no read arm.
        let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
        assert_unserved(&mut zone, P::OCCUPANCY_STATE);
    }
}
