//! Array-index gating across the object-access services (#190, #260).
//!
//! Every service that carries a `property_array_index` — ReadProperty,
//! ReadPropertyMultiple, WriteProperty, WritePropertyMultiple — gates the
//! index on the object's `is_array_property` classification and rejects an
//! index on a BACnetLIST property with PROPERTY / PROPERTY_IS_NOT_AN_ARRAY
//! (Clause 15.5.1.3 / 15.9.1.3; Clause 12.1.5.2 makes ReadRange the only
//! positional access to a BACnetLIST). Properties that are true BACnetARRAY
//! datatypes (Clause 12.1.5.1) pass the gate unchanged.

use super::*;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject};
use bacnet_objects::command::CommandObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::group::{GlobalGroupObject, GroupObject, StructuredViewObject};
use bacnet_objects::lighting::ChannelObject;
use bacnet_objects::multistate::MultiStateInputObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::schedule::{CalendarObject, ScheduleObject};
use bacnet_objects::staging::StagingObject;
use bacnet_objects::value_types::CharacterStringValueObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// Database holding one instance of every object type the matrix touches.
fn gating_db() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();

    let mut device = DeviceObject::new(DeviceConfig {
        instance: 1,
        name: "GateDevice".into(),
        ..Default::default()
    })
    .unwrap();
    // A deterministic OBJECT_LIST: [device, AI-1].
    device.set_object_list(vec![
        oid(ObjectType::DEVICE, 1),
        oid(ObjectType::ANALOG_INPUT, 1),
    ]);
    db.add(Box::new(device)).unwrap();

    db.add(Box::new(AnalogInputObject::new(1, "AI-1", 62).unwrap()))
        .unwrap();
    db.add(Box::new(AnalogOutputObject::new(1, "AO-1", 62).unwrap()))
        .unwrap();
    db.add(Box::new(MultiStateInputObject::new(1, "MSI-1", 3).unwrap()))
        .unwrap();
    db.add(Box::new(CalendarObject::new(1, "CAL-1").unwrap()))
        .unwrap();
    db.add(Box::new(GroupObject::new(1, "GRP-1").unwrap()))
        .unwrap();
    db.add(Box::new(NotificationClass::new(1, "NC-1").unwrap()))
        .unwrap();
    db.add(Box::new(EventLogObject::new(1, "ELG-1", 16).unwrap()))
        .unwrap();
    db.add(Box::new(
        ScheduleObject::new(1, "SCH-1", PropertyValue::Unsigned(0)).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        CharacterStringValueObject::new(1, "CSV-1").unwrap(),
    ))
    .unwrap();
    db.add(Box::new(ChannelObject::new(1, "CH-1", 7).unwrap()))
        .unwrap();
    db.add(Box::new(GlobalGroupObject::new(1, "GG-1").unwrap()))
        .unwrap();
    db.add(Box::new(StructuredViewObject::new(1, "SV-1").unwrap()))
        .unwrap();
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    command.set_action(vec![vec![1, 2, 3]]);
    db.add(Box::new(command)).unwrap();
    db.add(Box::new(StagingObject::new(1, "STG-1", 3).unwrap()))
        .unwrap();
    db
}

/// The (object, property) rejection matrix: BACnetLIST on the object type
/// that defines it, so an index must be refused on all four services.
const LIST_TARGETS: &[(ObjectType, PropertyIdentifier)] = &[
    (ObjectType::CALENDAR, PropertyIdentifier::DATE_LIST),
    (ObjectType::GROUP, PropertyIdentifier::LIST_OF_GROUP_MEMBERS),
    (
        ObjectType::NOTIFICATION_CLASS,
        PropertyIdentifier::RECIPIENT_LIST,
    ),
    (ObjectType::EVENT_LOG, PropertyIdentifier::LOG_BUFFER),
    (
        ObjectType::DEVICE,
        PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS,
    ),
    (
        ObjectType::SCHEDULE,
        PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
    ),
    // BACnetLIST on the multi-state family (Table 12-21) — array only on
    // CharacterString/BitString Value.
    (
        ObjectType::MULTI_STATE_INPUT,
        PropertyIdentifier::ALARM_VALUES,
    ),
];

fn read_indexed(
    db: &ObjectDatabase,
    target_oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: u32,
) -> Result<(), Error> {
    let request = ReadPropertyRequest {
        object_identifier: target_oid,
        property_identifier: property,
        property_array_index: Some(index),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    let mut ack_buf = BytesMut::new();
    handle_read_property(db, &buf, &mut ack_buf).map(|_| ())
}

fn assert_not_an_array(result: Result<(), Error>, context: &str) {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(
                class,
                ErrorClass::PROPERTY.to_raw() as u32,
                "{context}: wrong error class"
            );
            assert_eq!(
                code,
                ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32,
                "{context}: wrong error code"
            );
        }
        other => panic!("{context}: expected PROPERTY/PROPERTY_IS_NOT_AN_ARRAY, got {other:?}"),
    }
}

fn assert_protocol_error(result: Result<(), Error>, expected_code: ErrorCode, context: &str) {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(
                class,
                ErrorClass::PROPERTY.to_raw() as u32,
                "{context}: wrong error class"
            );
            assert_eq!(
                code,
                expected_code.to_raw() as u32,
                "{context}: wrong error code"
            );
        }
        other => panic!("{context}: expected PROPERTY/{expected_code:?}, got {other:?}"),
    }
}

fn encode_value(value: PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, &value).unwrap();
    buf.to_vec()
}

fn write_indexed(
    db: &mut ObjectDatabase,
    target_oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    value_bytes: Vec<u8>,
) -> Result<(), Error> {
    let request = WritePropertyRequest {
        object_identifier: target_oid,
        property_identifier: property,
        property_array_index: index,
        property_value: value_bytes,
        priority: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    handle_write_property(db, &mut buf).map(|_| ())
}

fn wpm_single(
    db: &mut ObjectDatabase,
    target_oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    value_bytes: Vec<u8>,
) -> Result<(), Error> {
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: target_oid,
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: property,
                property_array_index: index,
                value: value_bytes,
                priority: None,
            }],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    handle_write_property_multiple(db, &buf).map(|_| ())
}

#[test]
fn list_properties_reject_indexed_read_property() {
    let db = gating_db();
    for &(object_type, property) in LIST_TARGETS {
        assert_not_an_array(
            read_indexed(&db, oid(object_type, 1), property, 1),
            &format!("ReadProperty {object_type:?}.{property:?}"),
        );
    }
}

#[test]
fn list_properties_reject_indexed_read_property_multiple_inline() {
    use bacnet_services::common::PropertyReference;
    use bacnet_services::rpm::ReadAccessSpecification;

    let db = gating_db();
    for &(object_type, property) in LIST_TARGETS {
        // Sibling reference (OBJECT_NAME, scalar) succeeds while the indexed
        // list reference fails inline — per-reference isolation.
        let request = ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: oid(object_type, 1),
                list_of_property_references: vec![
                    PropertyReference {
                        property_identifier: PropertyIdentifier::OBJECT_NAME,
                        property_array_index: None,
                    },
                    PropertyReference {
                        property_identifier: property,
                        property_array_index: Some(1),
                    },
                ],
            }],
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        let mut ack_buf = BytesMut::new();
        handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
        let ack = ReadPropertyMultipleACK::decode(&ack_buf.to_vec()).unwrap();
        let results = &ack.list_of_read_access_results[0].list_of_results;
        assert!(
            results[0].property_value.is_some(),
            "{object_type:?}.OBJECT_NAME: sibling read must succeed"
        );
        assert_eq!(
            results[1].error,
            Some((ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)),
            "{object_type:?}.{property:?}: indexed list read must fail inline"
        );
        // The index is echoed back in the error element (Clause 15.8.1.2).
        assert_eq!(results[1].property_array_index, Some(1));
    }
}

#[test]
fn list_properties_reject_indexed_write_property_and_write_property_multiple() {
    for &(object_type, property) in LIST_TARGETS {
        let mut db = gating_db();
        let value = encode_value(PropertyValue::OctetString(vec![0]));
        assert_not_an_array(
            write_indexed(
                &mut db,
                oid(object_type, 1),
                property,
                Some(1),
                value.clone(),
            ),
            &format!("WriteProperty {object_type:?}.{property:?}"),
        );
        assert_not_an_array(
            wpm_single(&mut db, oid(object_type, 1), property, Some(1), value),
            &format!("WritePropertyMultiple {object_type:?}.{property:?}"),
        );
    }
}

#[test]
fn true_arrays_pass_the_gate_on_read_property() {
    let db = gating_db();

    // OBJECT_LIST index 0 = element count, index 1..N = the Nth member.
    let mut buf = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::DEVICE, 1),
        property_identifier: PropertyIdentifier::OBJECT_LIST,
        property_array_index: Some(0),
    }
    .encode(&mut buf);
    let mut ack_buf = BytesMut::new();
    handle_read_property(&db, &buf, &mut ack_buf).unwrap();
    let ack = ReadPropertyACK::decode(&ack_buf.to_vec()).unwrap();
    let (count, _) =
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
    assert_eq!(count, PropertyValue::Unsigned(2));

    read_indexed(
        &db,
        oid(ObjectType::DEVICE, 1),
        PropertyIdentifier::OBJECT_LIST,
        1,
    )
    .unwrap();
    read_indexed(
        &db,
        oid(ObjectType::DEVICE, 1),
        PropertyIdentifier::OBJECT_LIST,
        2,
    )
    .unwrap();

    // PROPERTY_LIST index 0 / 1..N.
    for index in [0, 1, 3] {
        read_indexed(
            &db,
            oid(ObjectType::ANALOG_INPUT, 1),
            PropertyIdentifier::PROPERTY_LIST,
            index,
        )
        .unwrap();
    }

    // STATE_TEXT on a multi-state input.
    for index in [0, 1, 3] {
        read_indexed(
            &db,
            oid(ObjectType::MULTI_STATE_INPUT, 1),
            PropertyIdentifier::STATE_TEXT,
            index,
        )
        .unwrap();
    }

    // WEEKLY_SCHEDULE / EXCEPTION_SCHEDULE on a Schedule.
    read_indexed(
        &db,
        oid(ObjectType::SCHEDULE, 1),
        PropertyIdentifier::WEEKLY_SCHEDULE,
        0,
    )
    .unwrap();
    read_indexed(
        &db,
        oid(ObjectType::SCHEDULE, 1),
        PropertyIdentifier::WEEKLY_SCHEDULE,
        1,
    )
    .unwrap();
    read_indexed(
        &db,
        oid(ObjectType::SCHEDULE, 1),
        PropertyIdentifier::EXCEPTION_SCHEDULE,
        0,
    )
    .unwrap();

    // PRIORITY_ARRAY slots 1..=16 on a commandable analog output.
    for index in [1, 8, 16] {
        read_indexed(
            &db,
            oid(ObjectType::ANALOG_OUTPUT, 1),
            PropertyIdentifier::PRIORITY_ARRAY,
            index,
        )
        .unwrap();
    }
}

#[test]
fn modeled_arrays_on_command_and_staging_pass_the_gate() {
    // Review FIX 1: ACTION (Table 12-12) and STAGES / STAGE_NAMES /
    // TARGET_REFERENCES (Table 12-80) are BACnetARRAY[N]. The objects still
    // return the whole value for any index (same documented residue as
    // Global Group / Structured View) — the pin is that the gate ADMITS the
    // index instead of rejecting PROPERTY_IS_NOT_AN_ARRAY.
    let db = gating_db();
    for &(property, object_type) in &[
        (PropertyIdentifier::ACTION, ObjectType::COMMAND),
        (PropertyIdentifier::STAGES, ObjectType::STAGING),
        (PropertyIdentifier::STAGE_NAMES, ObjectType::STAGING),
        (PropertyIdentifier::TARGET_REFERENCES, ObjectType::STAGING),
    ] {
        for index in [0, 1] {
            read_indexed(&db, oid(object_type, 1), property, index).unwrap_or_else(|e| {
                panic!("{object_type:?}.{property:?} index {index} must pass the gate: {e:?}")
            });
        }
    }
}

#[test]
fn event_time_stamps_accepts_index_range_via_event_history() {
    // EVENT_TIME_STAMPS is BACnetARRAY[3]; element semantics delegate to
    // EventHistory::read (#171 owns the remaining integration — only the
    // gate pass-through is pinned here).
    let db = gating_db();
    let msi = oid(ObjectType::MULTI_STATE_INPUT, 1);

    // No index → whole array; index 0 → count; 1..=3 → element.
    for index in [None, Some(0), Some(1), Some(2), Some(3)] {
        let request = ReadPropertyRequest {
            object_identifier: msi,
            property_identifier: PropertyIdentifier::EVENT_TIME_STAMPS,
            property_array_index: index,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        let mut ack_buf = BytesMut::new();
        handle_read_property(&db, &buf, &mut ack_buf)
            .unwrap_or_else(|e| panic!("EVENT_TIME_STAMPS index {index:?} must be served: {e:?}"));
    }

    // Out of range → INVALID_ARRAY_INDEX from the object layer.
    assert_protocol_error(
        read_indexed(&db, msi, PropertyIdentifier::EVENT_TIME_STAMPS, 4),
        ErrorCode::INVALID_ARRAY_INDEX,
        "EVENT_TIME_STAMPS index 4",
    );
}

#[test]
fn alarm_values_gate_varies_by_object_type() {
    let db = gating_db();

    // Multi-state input: ALARM_VALUES is a BACnetLIST (Table 12-21) → reject.
    assert_not_an_array(
        read_indexed(
            &db,
            oid(ObjectType::MULTI_STATE_INPUT, 1),
            PropertyIdentifier::ALARM_VALUES,
            1,
        ),
        "ReadProperty MULTI_STATE_INPUT.ALARM_VALUES",
    );

    // CharacterString Value: ALARM_VALUES is BACnetARRAY[N] (Table 12-44),
    // so the gate admits the index. The property is not yet modeled on the
    // object, which surfaces UNKNOWN_PROPERTY — a different, later failure
    // stage than the gate's PROPERTY_IS_NOT_AN_ARRAY.
    assert_protocol_error(
        read_indexed(
            &db,
            oid(ObjectType::CHARACTERSTRING_VALUE, 1),
            PropertyIdentifier::ALARM_VALUES,
            1,
        ),
        ErrorCode::UNKNOWN_PROPERTY,
        "ReadProperty CHARACTERSTRING_VALUE.ALARM_VALUES must NOT be gated",
    );
}

#[test]
fn schedule_list_of_object_property_references_rejects_indexed_read_and_write() {
    let db = gating_db();
    let sched = oid(ObjectType::SCHEDULE, 1);

    assert_not_an_array(
        read_indexed(
            &db,
            sched,
            PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            1,
        ),
        "ReadProperty SCHEDULE.LIST_OF_OBJECT_PROPERTY_REFERENCES",
    );

    let mut db = gating_db();
    let value = encode_value(PropertyValue::OctetString(vec![0]));
    assert_not_an_array(
        write_indexed(
            &mut db,
            sched,
            PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            Some(1),
            value,
        ),
        "WriteProperty SCHEDULE.LIST_OF_OBJECT_PROPERTY_REFERENCES",
    );
}

#[test]
fn object_level_classification_matrix() {
    // Classification truth source: identifier-stable arrays (Table 12-17/…),
    // identifier-stable lists, and the type-dependent identifiers.
    use bacnet_objects::traits::BACnetObject;

    let nc = NotificationClass::new(9, "NC-9").unwrap();
    // PRIORITY is BACnetARRAY[3] on Notification Class (Table 12-24).
    assert!(nc.is_array_property(PropertyIdentifier::PRIORITY));
    assert!(!nc.is_array_property(PropertyIdentifier::RECIPIENT_LIST));
    assert!(!nc.is_array_property(PropertyIdentifier::ACK_REQUIRED));

    let channel = ChannelObject::new(9, "CH-9", 7).unwrap();
    // LIST_OF_OBJECT_PROPERTY_REFERENCES: array on Channel (Table 12-62)…
    assert!(channel.is_array_property(PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES));
    let schedule = ScheduleObject::new(9, "SCH-9", PropertyValue::Unsigned(0)).unwrap();
    // …but a BACnetLIST on Schedule (Table 12-28).
    assert!(!schedule.is_array_property(PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES));
    assert!(schedule.is_array_property(PropertyIdentifier::WEEKLY_SCHEDULE));
    assert!(schedule.is_array_property(PropertyIdentifier::EXCEPTION_SCHEDULE));
    assert!(!schedule.is_array_property(PropertyIdentifier::PRESENT_VALUE));

    let calendar = CalendarObject::new(9, "CAL-9").unwrap();
    assert!(!calendar.is_array_property(PropertyIdentifier::DATE_LIST));

    let event_log = EventLogObject::new(9, "ELG-9", 16).unwrap();
    assert!(!event_log.is_array_property(PropertyIdentifier::LOG_BUFFER));

    let gg = GlobalGroupObject::new(9, "GG-9").unwrap();
    // Global Group (Table 12-57): GROUP_MEMBERS, GROUP_MEMBER_NAMES and
    // PRESENT_VALUE are BACnetARRAY — PRESENT_VALUE is scalar elsewhere.
    assert!(gg.is_array_property(PropertyIdentifier::GROUP_MEMBERS));
    assert!(gg.is_array_property(PropertyIdentifier::GROUP_MEMBER_NAMES));
    assert!(gg.is_array_property(PropertyIdentifier::PRESENT_VALUE));

    let sv = StructuredViewObject::new(9, "SV-9").unwrap();
    // Structured View (Table 12-34): BACnetARRAY subordinate properties.
    assert!(sv.is_array_property(PropertyIdentifier::SUBORDINATE_LIST));
    assert!(sv.is_array_property(PropertyIdentifier::SUBORDINATE_ANNOTATIONS));

    let ai = AnalogInputObject::new(9, "AI-9", 62).unwrap();
    assert!(!ai.is_array_property(PropertyIdentifier::PRESENT_VALUE));
    assert!(ai.is_array_property(PropertyIdentifier::PROPERTY_LIST));

    let csv = CharacterStringValueObject::new(9, "CSV-9").unwrap();
    assert!(csv.is_array_property(PropertyIdentifier::ALARM_VALUES));
    assert!(csv.is_array_property(PropertyIdentifier::FAULT_VALUES));
    assert!(csv.is_array_property(PropertyIdentifier::PRIORITY_ARRAY));

    let ao = AnalogOutputObject::new(9, "AO-9", 62).unwrap();
    assert!(ao.is_array_property(PropertyIdentifier::PRIORITY_ARRAY));
    assert!(!ao.is_array_property(PropertyIdentifier::PRESENT_VALUE));

    let cmd = CommandObject::new(9, "CMD-9").unwrap();
    // Command (Table 12-12): ACTION is BACnetARRAY[N]; ACTION_TEXT is an
    // array in the standard but not modeled in-tree, so it stays rejected
    // until its object-side modeling lands.
    assert!(cmd.is_array_property(PropertyIdentifier::ACTION));
    assert!(!cmd.is_array_property(PropertyIdentifier::ACTION_TEXT));
    assert!(!cmd.is_array_property(PropertyIdentifier::PRESENT_VALUE));

    let stg = StagingObject::new(9, "STG-9", 3).unwrap();
    // Staging (Table 12-80): all three collection properties are arrays.
    assert!(stg.is_array_property(PropertyIdentifier::STAGES));
    assert!(stg.is_array_property(PropertyIdentifier::STAGE_NAMES));
    assert!(stg.is_array_property(PropertyIdentifier::TARGET_REFERENCES));
    assert!(!stg.is_array_property(PropertyIdentifier::PRESENT_STAGE));
}

// ── #266: omitted-index PRIORITY_ARRAY writes are protocol errors ─────────

#[test]
fn wpm_omitted_index_priority_array_write_is_write_access_denied() {
    // Clause 12.1.5.1: an omitted index means whole-array access; commandable
    // objects do not support whole-array writes, so Result(-) carries
    // PROPERTY / WRITE_ACCESS_DENIED (Clause 15.9.1.3) — not an opaque
    // encoding error the service layer cannot map.
    let mut db = gating_db();
    let ao = oid(ObjectType::ANALOG_OUTPUT, 1);
    let value = encode_value(PropertyValue::Real(1.0));
    assert_protocol_error(
        wpm_single(&mut db, ao, PropertyIdentifier::PRIORITY_ARRAY, None, value),
        ErrorCode::WRITE_ACCESS_DENIED,
        "WPM PRIORITY_ARRAY without index",
    );
}

#[test]
fn write_property_omitted_index_priority_array_is_write_access_denied() {
    let mut db = gating_db();
    let ao = oid(ObjectType::ANALOG_OUTPUT, 1);
    let value = encode_value(PropertyValue::Real(1.0));
    assert_protocol_error(
        write_indexed(&mut db, ao, PropertyIdentifier::PRIORITY_ARRAY, None, value),
        ErrorCode::WRITE_ACCESS_DENIED,
        "WriteProperty PRIORITY_ARRAY without index",
    );
}

#[test]
fn priority_array_out_of_range_index_stays_invalid_array_index() {
    // Indexes 0 and 17 are outside the fixed 1..=16 array: INVALID_ARRAY_INDEX.
    let mut db = gating_db();
    let ao = oid(ObjectType::ANALOG_OUTPUT, 1);
    for index in [Some(0), Some(17)] {
        let value = encode_value(PropertyValue::Real(1.0));
        assert_protocol_error(
            wpm_single(
                &mut db,
                ao,
                PropertyIdentifier::PRIORITY_ARRAY,
                index,
                value.clone(),
            ),
            ErrorCode::INVALID_ARRAY_INDEX,
            &format!("WPM PRIORITY_ARRAY index {index:?}"),
        );
        assert_protocol_error(
            write_indexed(
                &mut db,
                ao,
                PropertyIdentifier::PRIORITY_ARRAY,
                index,
                value,
            ),
            ErrorCode::INVALID_ARRAY_INDEX,
            &format!("WriteProperty PRIORITY_ARRAY index {index:?}"),
        );
    }
}

#[test]
fn indexed_access_to_recipient_list_rejected_with_not_an_array() {
    // Recipient_List is a BACnetLIST (Table 12-24): indexed reads AND writes
    // hit the same PROPERTY / PROPERTY_IS_NOT_AN_ARRAY classification
    // (Tranche J's INVALID_DATA_TYPE stopgap is replaced by the gate).
    let db = gating_db();
    let nc = oid(ObjectType::NOTIFICATION_CLASS, 1);
    assert_not_an_array(
        read_indexed(&db, nc, PropertyIdentifier::RECIPIENT_LIST, 2),
        "ReadProperty NOTIFICATION_CLASS.RECIPIENT_LIST",
    );

    let mut db = gating_db();
    let value = encode_value(PropertyValue::OctetString(vec![0]));
    assert_not_an_array(
        write_indexed(
            &mut db,
            nc,
            PropertyIdentifier::RECIPIENT_LIST,
            Some(2),
            value,
        ),
        "WriteProperty NOTIFICATION_CLASS.RECIPIENT_LIST",
    );

    // The unindexed whole-list write is unaffected: Recipient_List stays
    // writable at the list level (tranche J's framed tests pin the wire form).
    let mut framed = BytesMut::new();
    bacnet_encoding::constructed::encode_destination_list(&mut framed, &[]);
    let mut db = gating_db();
    write_indexed(
        &mut db,
        nc,
        PropertyIdentifier::RECIPIENT_LIST,
        None,
        framed.to_vec(),
    )
    .unwrap();
}

#[test]
fn notification_class_priority_accepts_index_range() {
    // PRIORITY is BACnetARRAY[3] of Unsigned (Table 12-24). The old
    // identifier whitelist wrongly rejected it; the trait gate must admit
    // index 0 (count) and 1..=3 (elements).
    let db = gating_db();
    let nc = oid(ObjectType::NOTIFICATION_CLASS, 1);

    let mut buf = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: nc,
        property_identifier: PropertyIdentifier::PRIORITY,
        property_array_index: Some(0),
    }
    .encode(&mut buf);
    let mut ack_buf = BytesMut::new();
    handle_read_property(&db, &buf, &mut ack_buf).unwrap();
    let ack = ReadPropertyACK::decode(&ack_buf.to_vec()).unwrap();
    assert_eq!(ack.property_array_index, Some(0));
    let (count, _) =
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
    assert_eq!(count, PropertyValue::Unsigned(3));

    for index in 1..=3 {
        read_indexed(&db, nc, PropertyIdentifier::PRIORITY, index).unwrap();
    }
}

#[test]
fn wpm_gate_rejection_commits_nothing() {
    // Atomicity proof: one request carrying a valid write plus a gated
    // indexed write fails as a whole with PROPERTY_IS_NOT_AN_ARRAY, and the
    // valid property is untouched — the validation-phase gate fires before
    // the commit loop starts (§19.1.2-level atomicity).
    let mut db = gating_db();
    let nc = oid(ObjectType::NOTIFICATION_CLASS, 1);
    let original = db
        .get(&nc)
        .unwrap()
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap();

    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: nc,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::DESCRIPTION,
                    property_array_index: None,
                    value: encode_value(PropertyValue::CharacterString("MUTATED".into())),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::RECIPIENT_LIST,
                    property_array_index: Some(1),
                    value: encode_value(PropertyValue::OctetString(vec![0])),
                    priority: None,
                },
            ],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    assert_not_an_array(
        handle_write_property_multiple(&mut db, &buf).map(|_| ()),
        "WPM with one gated reference",
    );

    // The preceding valid write in the same request left no trace.
    assert_eq!(
        db.get(&nc)
            .unwrap()
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        original,
        "the valid reference must NOT be applied when a sibling hits the gate"
    );
}
