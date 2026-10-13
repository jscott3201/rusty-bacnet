use super::*;
use crate::property_metadata::{PropertyPresenceCondition, PropertyWriteCapability};
use crate::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
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

/// The intrinsic-reporting rows of the zone (#1305) and the door (#1149),
/// in metadata order.
const EVENT_ROWS: [P; 13] = [
    P::TIME_DELAY,
    P::NOTIFICATION_CLASS,
    P::ALARM_VALUES,
    P::EVENT_ENABLE,
    P::ACKED_TRANSITIONS,
    P::NOTIFY_TYPE,
    P::EVENT_TIME_STAMPS,
    P::EVENT_MESSAGE_TEXTS,
    P::EVENT_MESSAGE_TEXTS_CONFIG,
    P::EVENT_ALGORITHM_INHIBIT_REF,
    P::EVENT_ALGORITHM_INHIBIT,
    P::EVENT_DETECTION_ENABLE,
    P::TIME_DELAY_NORMAL,
];

/// The event rows Tables 12-30 and 12-37 only permit, by footnote 5 or 7
/// alone, rather than require of an object that reports intrinsically.
const PERMITTED_EVENT_ROWS: [P; 5] = [
    P::EVENT_MESSAGE_TEXTS,
    P::EVENT_MESSAGE_TEXTS_CONFIG,
    P::EVENT_ALGORITHM_INHIBIT_REF,
    P::EVENT_ALGORITHM_INHIBIT,
    P::TIME_DELAY_NORMAL,
];

/// The rows the table requires of an object that reports intrinsically:
/// the event rows but the permitted ones, and `extra`.
fn intrinsic_required(extra: &[P]) -> Vec<P> {
    EVENT_ROWS
        .iter()
        .copied()
        .filter(|p| !PERMITTED_EVENT_ROWS.contains(p))
        .chain(extra.iter().copied())
        .collect()
}

/// `all` kept to the rows `base` or `intrinsic` names, in metadata order,
/// then Property_List: the required set RPM and the PICS list.
fn required_set(all: &[P], base: &[P], intrinsic: &[P]) -> Vec<P> {
    all.iter()
        .copied()
        .filter(|p| base.contains(p) || intrinsic.contains(p))
        .chain([P::PROPERTY_LIST])
        .collect()
}

fn assert_exact_sets(object: &dyn BACnetObject, all: &[P], required: &[P], intrinsic: &[P]) {
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
        let condition = if PERMITTED_EVENT_ROWS.contains(&row.property_identifier) {
            Some(PropertyPresenceCondition::IntrinsicReportingOptional)
        } else if intrinsic.contains(&row.property_identifier) {
            Some(PropertyPresenceCondition::IntrinsicReportingRequired)
        } else {
            None
        };
        assert_eq!(
            row.presence_condition, condition,
            "{:?}",
            row.property_identifier
        );
        let expected = if condition.is_some() {
            Optional
        } else if (row.property_identifier == P::PRESENT_VALUE
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
        P::DOOR_PULSE_TIME,
        P::DOOR_EXTENDED_PULSE_TIME,
        P::DOOR_OPEN_TOO_LONG_TIME,
        P::CURRENT_COMMAND_PRIORITY,
        // The rows #1149 added: the masked list, the event rows and, after
        // Alarm_Values, Fault_Values.
        P::MASKED_ALARM_VALUES,
    ]
    .into_iter()
    .chain(EVENT_ROWS[..3].iter().copied())
    .chain([P::FAULT_VALUES])
    .chain(EVENT_ROWS[3..].iter().copied())
    .collect::<Vec<_>>();
    let base_required = [
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
        P::DOOR_PULSE_TIME,
        P::DOOR_EXTENDED_PULSE_TIME,
        P::DOOR_OPEN_TOO_LONG_TIME,
        P::CURRENT_COMMAND_PRIORITY,
    ];
    // Table 12-30 footnote 3 also requires Door_Alarm_State (#1485).
    let intrinsic = intrinsic_required(&[P::DOOR_ALARM_STATE]);
    let required = required_set(&all, &base_required, &intrinsic);
    assert_exact_sets(&object, &all, &required, &intrinsic);
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
    // The three alarm lists start empty and are BACnetLISTs, not arrays.
    for p in [P::MASKED_ALARM_VALUES, P::ALARM_VALUES, P::FAULT_VALUES] {
        assert_eq!(
            object.read_property(p, None).unwrap(),
            PropertyValue::List(vec![])
        );
        assert!(!object.is_array_property(p), "{p:?}");
        assert!(object.is_list_property(p), "{p:?}");
    }
    // Priority_Array and Door_Members are BACnetARRAYs (Table 12-30), so
    // the service gate admits an index on both (#1169).
    assert!(object.is_array_property(P::PRIORITY_ARRAY));
    assert_eq!(
        object.read_property(P::PRIORITY_ARRAY, Some(0)).unwrap(),
        PropertyValue::Unsigned(16)
    );
    assert_eq!(
        object.read_property(P::PRIORITY_ARRAY, Some(1)).unwrap(),
        PropertyValue::Null
    );
    assert!(object.is_array_property(P::DOOR_MEMBERS));
    assert!(!object.is_array_property(P::DOOR_STATUS));
    // The #1073 rows: the three times in tenths of a second, and no
    // command priority while Present_Value is the default.
    for (p, tenths) in [
        (P::DOOR_PULSE_TIME, 50),
        (P::DOOR_EXTENDED_PULSE_TIME, 150),
        (P::DOOR_OPEN_TOO_LONG_TIME, 300),
    ] {
        assert_eq!(
            object.read_property(p, None).unwrap(),
            PropertyValue::Unsigned(tenths)
        );
        assert!(!object.is_array_property(p));
    }
    assert_eq!(
        object
            .read_property(P::CURRENT_COMMAND_PRIORITY, None)
            .unwrap(),
        PropertyValue::Null
    );
}

#[test]
fn property_metadata_access_point_exact_sets_readable_rows_and_indexed_list() {
    let object = AccessPointObject::new(1, "AP-1").unwrap();
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::ACCESS_EVENT,
        P::ACCESS_EVENT_TAG,
        P::ACCESS_EVENT_TIME,
        P::ACCESS_DOORS,
        P::EVENT_STATE,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
        // The Table 12-36 required rows #1284 added.
        P::AUTHENTICATION_STATUS,
        P::ACCESS_EVENT_CREDENTIAL,
        // The Table 12-36 required rows #1307 added.
        P::ACTIVE_AUTHENTICATION_POLICY,
        P::NUMBER_OF_AUTHENTICATION_POLICIES,
        P::AUTHORIZATION_MODE,
        P::PRIORITY_FOR_WRITING,
        // The optional factor of each event (#1132).
        P::ACCESS_EVENT_AUTHENTICATION_FACTOR,
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
        P::AUTHENTICATION_STATUS,
        P::ACCESS_EVENT_CREDENTIAL,
        P::ACTIVE_AUTHENTICATION_POLICY,
        P::NUMBER_OF_AUTHENTICATION_POLICIES,
        P::AUTHORIZATION_MODE,
        P::PRIORITY_FOR_WRITING,
        P::PROPERTY_LIST,
    ];
    assert_exact_sets(&object, &all, &required, &[]);
    assert_indexed_property_list(&object, &all);
    // Table 13-1 has an Access Point row (#1061).
    assert!(object.supports_cov());
    // Table 12-36 has no Present_Value row (#1064).
    assert_error(
        object.read_property(P::PRESENT_VALUE, None).unwrap_err(),
        ErrorCode::UNKNOWN_PROPERTY,
    );
    assert_eq!(
        object.read_property(P::ACCESS_EVENT, None).unwrap(),
        PropertyValue::Enumerated(0)
    );
    assert_eq!(
        object.read_property(P::ACCESS_EVENT_TAG, None).unwrap(),
        PropertyValue::Unsigned(0)
    );
    // The unspecified date and time as the datetime [2] choice (#1133).
    assert_eq!(
        object.read_property(P::ACCESS_EVENT_TIME, None).unwrap(),
        PropertyValue::ApplicationData(vec![
            0x2E, 0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xB4, 0xFF, 0xFF, 0xFF, 0xFF, 0x2F
        ])
    );
    assert_eq!(
        object.read_property(P::ACCESS_DOORS, None).unwrap(),
        PropertyValue::List(vec![])
    );
    // READY until the application reports otherwise, and before any event
    // the no-credential reference: Access Credential 4194303 under [1].
    assert_eq!(
        object
            .read_property(P::AUTHENTICATION_STATUS, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
    assert_eq!(
        object
            .read_property(P::ACCESS_EVENT_CREDENTIAL, None)
            .unwrap(),
        PropertyValue::ApplicationData(vec![0x1C, 0x08, 0x3F, 0xFF, 0xFF])
    );
    // And the UNDEFINED factor: format type [0] 0, class [1] 0, no value
    // [2] (Clause 12.31.31).
    assert_eq!(
        object
            .read_property(P::ACCESS_EVENT_AUTHENTICATION_FACTOR, None)
            .unwrap(),
        PropertyValue::ApplicationData(vec![0x09, 0x00, 0x19, 0x00, 0x28])
    );
    // Access_Event_Time is one BACnetTimeStamp, so an index is rejected;
    // Access_Doors is a BACnetARRAY and takes one (#1169).
    assert!(object.is_array_property(P::ACCESS_DOORS));
    assert!(!object.is_array_property(P::ACCESS_EVENT_TIME));
    assert!(!object.is_array_property(P::ACCESS_EVENT_CREDENTIAL));
}

#[test]
fn property_metadata_access_zone_exact_sets_readable_rows_and_indexed_list() {
    let object = AccessZoneObject::new(1, "ZONE-1").unwrap();
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::GLOBAL_IDENTIFIER,
        P::OCCUPANCY_COUNT,
        P::ENTRY_POINTS,
        P::EXIT_POINTS,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
        // The Table 12-37 rows #1284 added.
        P::OCCUPANCY_STATE,
        P::EVENT_STATE,
        P::OCCUPANCY_COUNT_ENABLE,
        P::ADJUST_VALUE,
        P::OCCUPANCY_UPPER_LIMIT,
        P::OCCUPANCY_LOWER_LIMIT,
    ]
    .into_iter()
    .chain(EVENT_ROWS)
    .collect::<Vec<_>>();
    let base_required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::GLOBAL_IDENTIFIER,
        P::ENTRY_POINTS,
        P::EXIT_POINTS,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
        P::OCCUPANCY_STATE,
        P::EVENT_STATE,
    ];
    // Table 12-37 footnote 3 also requires the occupancy-counting rows
    // of a zone that reports intrinsically (#1485).
    let intrinsic = intrinsic_required(&[
        P::OCCUPANCY_COUNT,
        P::OCCUPANCY_COUNT_ENABLE,
        P::ADJUST_VALUE,
    ]);
    let required = required_set(&all, &base_required, &intrinsic);
    assert_exact_sets(&object, &all, &required, &intrinsic);
    assert_indexed_property_list(&object, &all);
    assert!(!object.supports_cov());
    // Table 12-37 has neither of these rows (#1064).
    for property in [P::PRESENT_VALUE, P::ACCESS_DOORS] {
        assert_error(
            object.read_property(property, None).unwrap_err(),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
    assert_eq!(
        object.read_property(P::GLOBAL_IDENTIFIER, None).unwrap(),
        PropertyValue::Unsigned(0)
    );
    // A new zone counts, from zero, with no limits and nothing adjusted, so
    // Occupancy_State is NORMAL and Event_State NORMAL.
    for (p, value) in [
        (P::OCCUPANCY_COUNT, PropertyValue::Unsigned(0)),
        (P::OCCUPANCY_STATE, PropertyValue::Enumerated(0)),
        (P::EVENT_STATE, PropertyValue::Enumerated(0)),
        (P::OCCUPANCY_COUNT_ENABLE, PropertyValue::Boolean(true)),
        (P::ADJUST_VALUE, PropertyValue::Signed(0)),
        (P::OCCUPANCY_UPPER_LIMIT, PropertyValue::Unsigned(0)),
        (P::OCCUPANCY_LOWER_LIMIT, PropertyValue::Unsigned(0)),
    ] {
        assert_eq!(object.read_property(p, None).unwrap(), value, "{p:?}");
        assert!(!object.is_array_property(p));
    }
    for p in [P::ENTRY_POINTS, P::EXIT_POINTS] {
        assert_eq!(
            object.read_property(p, None).unwrap(),
            PropertyValue::List(vec![])
        );
        assert!(!object.is_array_property(p));
    }
}

#[test]
fn property_metadata_access_trio_write_capabilities_match_dispatch() {
    // Constructor paired with the properties it must always accept writes
    // for, and those it must accept only while Out_Of_Service is TRUE.
    type WriteCase = (fn() -> Box<dyn BACnetObject>, &'static [P], &'static [P]);
    let cases: [WriteCase; 3] = [
        (
            || Box::new(AccessDoorObject::new(1, "DOOR-1").unwrap()),
            &[
                P::DESCRIPTION,
                P::OUT_OF_SERVICE,
                P::PRESENT_VALUE,
                P::RELINQUISH_DEFAULT,
                P::DOOR_PULSE_TIME,
                P::DOOR_EXTENDED_PULSE_TIME,
                P::DOOR_OPEN_TOO_LONG_TIME,
                // The lists and the event configuration (#1149).
                P::MASKED_ALARM_VALUES,
                P::TIME_DELAY,
                P::NOTIFICATION_CLASS,
                P::ALARM_VALUES,
                P::FAULT_VALUES,
                P::EVENT_ENABLE,
                P::NOTIFY_TYPE,
                P::EVENT_DETECTION_ENABLE,
                P::TIME_DELAY_NORMAL,
                // The message texts and the inhibit pair (#1329).
                P::EVENT_MESSAGE_TEXTS_CONFIG,
                P::EVENT_ALGORITHM_INHIBIT_REF,
                P::EVENT_ALGORITHM_INHIBIT,
            ],
            // Table 12-30 footnote 1 (#1131), and Reliability, which the
            // FAULT_STATE check can move (Clause 12.26.9, #1149).
            &[
                P::DOOR_STATUS,
                P::LOCK_STATUS,
                P::DOOR_ALARM_STATE,
                P::RELIABILITY,
            ],
        ),
        (
            || Box::new(AccessPointObject::new(1, "AP-1").unwrap()),
            // The policy in effect and the mode (#1307).
            &[
                P::DESCRIPTION,
                P::OUT_OF_SERVICE,
                P::ACTIVE_AUTHENTICATION_POLICY,
                P::AUTHORIZATION_MODE,
            ],
            // Reliability, which a zero active policy moves (Clause 12.31.8,
            // #1325).
            &[P::RELIABILITY],
        ),
        (
            || Box::new(AccessZoneObject::new(1, "ZONE-1").unwrap()),
            // Adjust_Value: Table 12-37 footnote 5 (#1284); the event
            // configuration (#1305).
            &[
                P::DESCRIPTION,
                P::OUT_OF_SERVICE,
                P::GLOBAL_IDENTIFIER,
                P::ADJUST_VALUE,
                P::TIME_DELAY,
                P::NOTIFICATION_CLASS,
                P::ALARM_VALUES,
                P::EVENT_ENABLE,
                P::NOTIFY_TYPE,
                P::EVENT_DETECTION_ENABLE,
                P::TIME_DELAY_NORMAL,
                // The message texts and the inhibit pair (#1329).
                P::EVENT_MESSAGE_TEXTS_CONFIG,
                P::EVENT_ALGORITHM_INHIBIT_REF,
                P::EVENT_ALGORITHM_INHIBIT,
            ],
            // Table 12-37 footnote 1 (#1247).
            &[P::OCCUPANCY_COUNT, P::RELIABILITY],
        ),
    ];
    for (make, writable, when_out_of_service) in cases {
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
                } else if when_out_of_service.contains(&p) {
                    PropertyWriteCapability::WhenOutOfService
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
                if capability == PropertyWriteCapability::Always
                    || (capability == PropertyWriteCapability::WhenOutOfService && out_of_service)
                {
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
        for p in [
            P::ACCESS_EVENT,
            P::ACCESS_EVENT_TAG,
            P::ACCESS_EVENT_TIME,
            P::ACCESS_DOORS,
            P::EVENT_STATE,
            P::STATUS_FLAGS,
            P::AUTHENTICATION_STATUS,
            P::ACCESS_EVENT_CREDENTIAL,
            P::NUMBER_OF_AUTHENTICATION_POLICIES,
            P::PRIORITY_FOR_WRITING,
        ] {
            let value = point.read_property(p, None).unwrap();
            assert_error(
                point.write_property(p, None, value, None).unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert!(!point.is_writable_property(p));
        }
        // Reliability takes its own readback only while out of service
        // (#1325).
        let value = point.read_property(P::RELIABILITY, None).unwrap();
        let result = point.write_property(P::RELIABILITY, None, value, None);
        if out_of_service {
            result.unwrap();
        } else {
            assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
        }
        assert!(point.is_writable_property(P::RELIABILITY));
        let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
        zone.write_property(
            P::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(out_of_service),
            None,
        )
        .unwrap();
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
        // The footnote-1 rows take their own readback only while out of
        // service (#1247).
        for p in [P::OCCUPANCY_COUNT, P::RELIABILITY] {
            let value = zone.read_property(p, None).unwrap();
            let result = zone.write_property(p, None, value, None);
            if out_of_service {
                result.unwrap();
            } else {
                assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
            }
            assert!(zone.is_writable_property(p));
        }
        for p in [
            P::ENTRY_POINTS,
            P::EXIT_POINTS,
            P::STATUS_FLAGS,
            P::OCCUPANCY_STATE,
            P::EVENT_STATE,
            P::OCCUPANCY_COUNT_ENABLE,
            P::OCCUPANCY_UPPER_LIMIT,
            P::OCCUPANCY_LOWER_LIMIT,
            P::ACKED_TRANSITIONS,
            P::EVENT_TIME_STAMPS,
            P::EVENT_MESSAGE_TEXTS,
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
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }

    // Door_Unlock_Delay_Time and Maintenance_Required are Table 12-30 O
    // rows with no read arm; Masked_Alarm_Values is served (#1149).
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    assert_unserved(&mut door, P::DOOR_UNLOCK_DELAY_TIME);
    assert_unserved(&mut door, P::MAINTENANCE_REQUIRED);
    // Present_Value is no Table 12-36 row (#1064). Authentication_Status
    // (#1284) and Access_Event_Authentication_Factor (#1132) are served. The
    // two policy arrays are served only once the application sets them
    // (#1325).
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    assert_unserved(&mut point, P::PRESENT_VALUE);
    assert_unserved(&mut point, P::AUTHENTICATION_POLICY_LIST);
    assert_unserved(&mut point, P::AUTHENTICATION_POLICY_NAMES);
    // Credentials_In_Zone is a Table 12-37 O row with no read arm;
    // Present_Value and Access_Doors are no Table 12-37 rows (#1064).
    // Occupancy_State is served (#1284).
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    assert_unserved(&mut zone, P::CREDENTIALS_IN_ZONE);
    assert_unserved(&mut zone, P::PRESENT_VALUE);
    assert_unserved(&mut zone, P::ACCESS_DOORS);
}

#[path = "metadata_topology_door_tests.rs"]
mod door;
