use super::*;
use bacnet_objects::{
    access_control::{AccessDoorObject, AccessPointObject, AccessZoneObject},
    traits::BACnetObject,
};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn access_objects(configured: bool) -> [Box<dyn BACnetObject>; 3] {
    let mut door = AccessDoorObject::new(7, "DOOR-7").unwrap();
    let point = AccessPointObject::new(7, "AP-7").unwrap();
    let mut zone = AccessZoneObject::new(7, "ZONE-7").unwrap();
    if configured {
        door.write_property(
            P::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(8),
        )
        .unwrap();
        door.write_property(
            P::RELINQUISH_DEFAULT,
            None,
            PropertyValue::Enumerated(1),
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
    }
    let mut objects: [Box<dyn BACnetObject>; 3] = [Box::new(door), Box::new(point), Box::new(zone)];
    for object in &mut objects {
        object
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString("long access label".repeat(100)),
                None,
            )
            .unwrap();
        // Exercise the unconditional write route so large encodings persist.
        object
            .write_property(
                P::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(configured),
                None,
            )
            .unwrap();
    }
    objects
}

/// The zone's intrinsic-reporting rows (#1305), in metadata order. All but
/// [`PERMITTED_EVENT_ROWS`] are required of a zone that reports
/// intrinsically (#1485).
const ZONE_EVENT_ROWS: [P; 13] = [
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

/// The event rows Tables 12-30 and 12-37 only permit (#1485).
const PERMITTED_EVENT_ROWS: [P; 5] = [
    P::EVENT_MESSAGE_TEXTS,
    P::EVENT_MESSAGE_TEXTS_CONFIG,
    P::EVENT_ALGORITHM_INHIBIT_REF,
    P::EVENT_ALGORITHM_INHIBIT,
    P::TIME_DELAY_NORMAL,
];

/// The door's rows #1149 added, in metadata order: the masked list, then
/// the zone's event rows with Fault_Values after Alarm_Values.
fn door_alarm_rows() -> impl Iterator<Item = P> {
    [P::MASKED_ALARM_VALUES]
        .into_iter()
        .chain(ZONE_EVENT_ROWS[..3].iter().copied())
        .chain([P::FAULT_VALUES])
        .chain(ZONE_EVENT_ROWS[3..].iter().copied())
}

fn expected_lists(kind: ObjectType) -> (Vec<P>, Vec<P>, Vec<P>) {
    let all = match kind {
        ObjectType::ACCESS_DOOR => vec![
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
            // The Table 12-30 required rows #1073 added.
            P::DOOR_PULSE_TIME,
            P::DOOR_EXTENDED_PULSE_TIME,
            P::DOOR_OPEN_TOO_LONG_TIME,
            P::CURRENT_COMMAND_PRIORITY,
        ]
        .into_iter()
        .chain(door_alarm_rows())
        .collect(),
        ObjectType::ACCESS_POINT => vec![
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
            // The optional row #1132 added.
            P::ACCESS_EVENT_AUTHENTICATION_FACTOR,
        ],
        _ => vec![
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
        .chain(ZONE_EVENT_ROWS)
        .collect(),
    };
    // Door_Alarm_State, and the zone's Occupancy_Count,
    // Occupancy_Count_Enable and Adjust_Value, carry the footnote that
    // requires them of an intrinsic reporter, so none of them is here.
    let optional_rows = match kind {
        ObjectType::ACCESS_DOOR => vec![
            P::DESCRIPTION,
            P::DOOR_STATUS,
            P::LOCK_STATUS,
            P::SECURED_STATUS,
            P::DOOR_MEMBERS,
            P::MASKED_ALARM_VALUES,
            P::FAULT_VALUES,
        ],
        // Tables 12-36 and 12-37 have no Present_Value, and Table 12-37 no
        // Access_Doors (#1064).
        ObjectType::ACCESS_POINT => vec![P::DESCRIPTION, P::ACCESS_EVENT_AUTHENTICATION_FACTOR],
        _ => vec![
            P::DESCRIPTION,
            P::OCCUPANCY_UPPER_LIMIT,
            P::OCCUPANCY_LOWER_LIMIT,
        ],
    };
    let optional: Vec<_> = all
        .iter()
        .copied()
        .filter(|p| optional_rows.contains(p) || PERMITTED_EVENT_ROWS.contains(p))
        .collect();
    let required: Vec<_> = all
        .iter()
        .copied()
        .filter(|p| !optional.contains(p))
        .collect();
    (all, required, optional)
}

#[test]
fn rpm_access_topology_metadata_selectors_preserve_bytes_and_budgets() {
    for configured in [false, true] {
        for object in access_objects(configured) {
            let oid = object.object_identifier();
            let (all, required, optional) = expected_lists(oid.object_type());
            let mut db = ObjectDatabase::new();
            db.add(object).unwrap();
            for (selector, expected) in [
                (P::ALL, all.as_slice()),
                (P::REQUIRED, required.as_slice()),
                (P::OPTIONAL, optional.as_slice()),
                (P::PROPERTY_LIST, &[P::PROPERTY_LIST]),
            ] {
                assert_rpm_selector_bytes(&db, oid, selector, expected);
            }
        }
    }
}

#[test]
fn rpm_access_topology_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let cases = [
        (
            ObjectType::ACCESS_DOOR,
            ObjectIdentifier::new(ObjectType::ACCESS_DOOR, 7).unwrap(),
        ),
        (
            ObjectType::ACCESS_POINT,
            ObjectIdentifier::new(ObjectType::ACCESS_POINT, 7).unwrap(),
        ),
        (
            ObjectType::ACCESS_ZONE,
            ObjectIdentifier::new(ObjectType::ACCESS_ZONE, 7).unwrap(),
        ),
    ];
    for (kind, oid) in cases {
        for object_specifier in [
            ObjectSpecifier::Type(kind),
            ObjectSpecifier::Identifier(oid),
        ] {
            let mut db = ObjectDatabase::new();
            let mut request = BytesMut::new();
            CreateObjectRequest {
                object_specifier,
                list_of_initial_values: vec![],
            }
            .encode(&mut request);
            let mut response = BytesMut::new();
            let result = handle_create_object(&mut db, &request, &mut response);
            assert!(matches!(result, Err(Error::Protocol { class, code })
                if class == ErrorClass::OBJECT.to_raw() as u32
                    && code == ErrorCode::UNSUPPORTED_OBJECT_TYPE.to_raw() as u32));
            assert!(response.is_empty());
            assert!(db.is_empty());
        }
    }
}

#[test]
fn access_topology_delete_object_removes_each_trio_member() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;

    for object in access_objects(false) {
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        let mut request = BytesMut::new();
        DeleteObjectRequest {
            object_identifier: oid,
        }
        .encode(&mut request);
        handle_delete_object(&mut db, &request).unwrap();
        assert!(db.get(&oid).is_none());
    }
}
