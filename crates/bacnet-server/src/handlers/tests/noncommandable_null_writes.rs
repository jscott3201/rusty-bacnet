//! A NULL written to a property that isn't commandable and has no NULL in
//! its datatype succeeds and leaves the property as it is, over
//! WriteProperty and WritePropertyMultiple (Clauses 15.9.2 and 15.10.2,
//! #1396). The checks made before the value still answer first, a
//! commandable Present_Value still relinquishes, and a property whose
//! datatype has a NULL still stores it.

use super::*;
use crate::handlers::relinquish::null_in_datatype;
use bacnet_objects::access_control::{AccessCredentialObject, AccessRightsObject};
use bacnet_objects::analog::{AnalogOutputObject, AnalogValueObject};
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::multistate::MultiStateInputObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::notification_forwarder::NotificationForwarderObject;
use bacnet_objects::schedule::{CalendarObject, ScheduleObject};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::constructed::BACnetPortPermission;
use bacnet_types::enums::EngineeringUnits;
use PropertyIdentifier as P;

/// One application NULL, the whole value.
const NULL: &[u8] = &[0x00];
/// A rule in force always and everywhere, disabled.
const ANYWHERE_OFF: &[u8] = &[0x09, 0x01, 0x29, 0x01, 0x49, 0x00];

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn ar() -> ObjectIdentifier {
    oid(ObjectType::ACCESS_RIGHTS, 1)
}
fn cred() -> ObjectIdentifier {
    oid(ObjectType::ACCESS_CREDENTIAL, 1)
}
fn ai() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, 1)
}
fn ao() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_OUTPUT, 1)
}
fn cal() -> ObjectIdentifier {
    oid(ObjectType::CALENDAR, 1)
}
fn nc() -> ObjectIdentifier {
    oid(ObjectType::NOTIFICATION_CLASS, 1)
}
fn msi() -> ObjectIdentifier {
    oid(ObjectType::MULTI_STATE_INPUT, 1)
}
fn nf() -> ObjectIdentifier {
    oid(ObjectType::NOTIFICATION_FORWARDER, 1)
}
fn sch() -> ObjectIdentifier {
    oid(ObjectType::SCHEDULE, 1)
}
/// The object a WritePropertyMultiple writes after the NULL, to show the
/// request goes on.
fn witness() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_VALUE, 99)
}

fn encoded(value: PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_property_value(&mut bytes, &value).unwrap();
    bytes.to_vec()
}

fn db() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(AccessRightsObject::new(1, "AR-1").unwrap()))
        .unwrap();
    let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
    credential.set_global_identifier(7);
    db.add(Box::new(credential)).unwrap();
    let mut input = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    input.set_description("kept");
    db.add(Box::new(input)).unwrap();
    db.add(Box::new(
        AnalogOutputObject::new(1, "AO-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(CalendarObject::new(1, "CAL-1").unwrap()))
        .unwrap();
    db.add(Box::new(NotificationClass::new(1, "NC-1").unwrap()))
        .unwrap();
    let mut states = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    states.set_alarm_values(vec![2]);
    db.add(Box::new(states)).unwrap();
    let mut forwarder = NotificationForwarderObject::new(1, "NF-1").unwrap();
    forwarder.set_port_filter(Some(vec![
        BACnetPortPermission {
            port_id: 0,
            enabled: true,
        },
        BACnetPortPermission {
            port_id: 1,
            enabled: false,
        },
    ]));
    db.add(Box::new(forwarder)).unwrap();
    db.add(Box::new(
        ScheduleObject::new(1, "SCH-1", PropertyValue::Real(1.0)).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogValueObject::new(99, "AV-99", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    // One positive rule, so the array has an element 1.
    wp(
        &mut db,
        ar(),
        P::POSITIVE_ACCESS_RULES,
        None,
        ANYWHERE_OFF,
        None,
    )
    .unwrap();
    db
}

fn request(
    oid: ObjectIdentifier,
    property: P,
    index: Option<u32>,
    value: &[u8],
    priority: Option<u8>,
) -> BytesMut {
    let mut bytes = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: index,
        property_value: value.to_vec(),
        priority,
    }
    .encode(&mut bytes)
    .unwrap();
    bytes
}

/// WriteProperty from `origin`, and what the handler says it did.
fn wp_from(
    db: &mut ObjectDatabase,
    origin: &CommandOrigin,
    oid: ObjectIdentifier,
    property: P,
    index: Option<u32>,
    value: &[u8],
    priority: Option<u8>,
) -> Result<Applied, Error> {
    let request = request(oid, property, index, value, priority);
    handle_write_property_observed(db, &request, None, None, Some(origin)).map(
        |(written, applied)| {
            assert_eq!(written, oid);
            applied
        },
    )
}

fn wp(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: P,
    index: Option<u32>,
    value: &[u8],
    priority: Option<u8>,
) -> Result<Applied, Error> {
    let origin = crate::command_source::test_origin();
    wp_from(db, &origin, oid, property, index, value, priority)
}

/// WritePropertyMultiple of `attempts` in order, one object each.
fn wpm(
    db: &mut ObjectDatabase,
    attempts: Vec<(ObjectIdentifier, P, Option<u32>, Vec<u8>)>,
) -> Result<Vec<ObjectIdentifier>, Error> {
    let mut bytes = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: attempts
            .into_iter()
            .map(|(oid, property, index, value)| WriteAccessSpecification {
                object_identifier: oid,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: property,
                    property_array_index: index,
                    value,
                    priority: None,
                }],
            })
            .collect(),
    }
    .encode(&mut bytes)
    .unwrap();
    sourced_wpm(db, &bytes)
}

/// The value ReadProperty serves for the whole property, in wire octets.
fn read(db: &ObjectDatabase, oid: ObjectIdentifier, property: P) -> Vec<u8> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut request);
    let mut ack = BytesMut::new();
    handle_read_property(db, &request, &mut ack).unwrap();
    ReadPropertyACK::decode(&ack).unwrap().property_value
}

fn assert_error(result: Result<impl std::fmt::Debug, Error>, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(&result, Err(Error::Protocol { class: c, code: e })
            if *c == class && *e == code),
        "expected {class:?} / {code:?}, got {result:?}"
    );
}

/// Non-commandable properties without a NULL in their datatype: scalars,
/// raw-octet properties, BACnetLISTs written whole, array elements and an
/// array size. `true` marks the two the objects already take a NULL for
/// themselves, which the handler therefore reports as written.
fn cases() -> Vec<(ObjectIdentifier, P, Option<u32>, bool)> {
    vec![
        // Access Rights Enable, a rule array whole, by element and resized.
        (ar(), P::LOG_ENABLE, None, false),
        (ar(), P::POSITIVE_ACCESS_RULES, None, false),
        (ar(), P::POSITIVE_ACCESS_RULES, Some(1), false),
        (ar(), P::NEGATIVE_ACCESS_RULES, Some(0), false),
        (cred(), P::GLOBAL_IDENTIFIER, None, false),
        (ai(), P::DESCRIPTION, None, true),
        (ai(), P::OUT_OF_SERVICE, None, true),
        (ai(), P::OBJECT_NAME, None, false),
        (ai(), P::COV_INCREMENT, None, false),
        // BACnetLISTs: one of dates, one of raw destinations, one of states.
        (cal(), P::DATE_LIST, None, false),
        (nc(), P::RECIPIENT_LIST, None, false),
        (msi(), P::ALARM_VALUES, None, false),
        (msi(), P::STATE_TEXT, Some(2), false),
        // State_Text written whole, which sets the state count (#1443).
        (msi(), P::STATE_TEXT, None, false),
        // An array of constructed elements the handler splits itself.
        (nf(), P::PORT_FILTER, None, false),
        (nf(), P::PORT_FILTER, Some(1), false),
        (sch(), P::WEEKLY_SCHEDULE, Some(1), false),
        (sch(), P::EFFECTIVE_PERIOD, None, false),
    ]
}

#[test]
fn null_to_a_noncommandable_property_succeeds_unchanged_over_wp() {
    let mut db = db();
    for (oid, property, index, object_takes_it) in cases() {
        for priority in [None, Some(8)] {
            let before = read(&db, oid, property);
            let applied = wp(&mut db, oid, property, index, NULL, priority)
                .unwrap_or_else(|e| panic!("{property:?} {index:?}: {e:?}"));
            let expected = if object_takes_it {
                Applied::Written
            } else {
                Applied::Unchanged
            };
            assert_eq!(applied, expected, "{property:?} {index:?}");
            assert_eq!(read(&db, oid, property), before, "{property:?} {index:?}");
        }
    }
}

#[test]
fn null_to_a_noncommandable_property_succeeds_unchanged_over_wpm() {
    let mut db = db();
    for (n, (oid, property, index, object_takes_it)) in cases().into_iter().enumerate() {
        let before = read(&db, oid, property);
        let after = encoded(PropertyValue::CharacterString(format!("after {n}")));
        let committed = wpm(
            &mut db,
            vec![
                (oid, property, index, NULL.to_vec()),
                (witness(), P::DESCRIPTION, None, after.clone()),
            ],
        )
        .unwrap_or_else(|e| panic!("{property:?} {index:?}: {e:?}"));
        assert_eq!(read(&db, oid, property), before, "{property:?} {index:?}");
        assert_eq!(read(&db, witness(), P::DESCRIPTION), after);
        // Only a write that changed its object commits it, so the NULL adds
        // no post-write work.
        let expected = if object_takes_it {
            vec![oid, witness()]
        } else {
            vec![witness()]
        };
        assert_eq!(committed, expected, "{property:?} {index:?}");
    }
}

#[test]
fn null_keeps_the_errors_its_checks_give() {
    let mut db = db();
    let cases: [(ObjectIdentifier, P, Option<u32>, ErrorClass, ErrorCode); 8] = [
        (
            oid(ObjectType::ANALOG_INPUT, 2),
            P::DESCRIPTION,
            None,
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ),
        (
            ai(),
            P::from_raw(5555),
            None,
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ),
        (
            ai(),
            P::DESCRIPTION,
            Some(1),
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            ar(),
            P::POSITIVE_ACCESS_RULES,
            Some(5),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_ARRAY_INDEX,
        ),
        // Read-only properties.
        (
            ai(),
            P::STATUS_FLAGS,
            None,
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
        (
            cred(),
            P::OBJECT_TYPE,
            None,
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
        // Writable only out of service, and the input is in service.
        (
            ai(),
            P::PRESENT_VALUE,
            None,
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
        // Weekly_Schedule always has seven days.
        (
            sch(),
            P::WEEKLY_SCHEDULE,
            Some(0),
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
    ];
    for (oid, property, index, class, code) in cases {
        assert_error(wp(&mut db, oid, property, index, NULL, None), class, code);
        // WritePropertyMultiple stops at the NULL, after the write before it.
        let prefix = encoded(PropertyValue::CharacterString(format!("{property:?}")));
        assert_error(
            wpm(
                &mut db,
                vec![
                    (witness(), P::DESCRIPTION, None, prefix.clone()),
                    (oid, property, index, NULL.to_vec()),
                ],
            ),
            class,
            code,
        );
        assert_eq!(read(&db, witness(), P::DESCRIPTION), prefix);
    }
}

#[test]
fn null_still_relinquishes_a_commandable_present_value() {
    let mut db = db();
    for (value, priority) in [(10.0, 8), (20.0, 4)] {
        let real = encoded(PropertyValue::Real(value));
        wp(&mut db, ao(), P::PRESENT_VALUE, None, &real, Some(priority)).unwrap();
    }
    assert_eq!(
        wp(&mut db, ao(), P::PRESENT_VALUE, None, NULL, Some(4)).unwrap(),
        Applied::Written
    );
    assert_eq!(
        read(&db, ao(), P::PRESENT_VALUE),
        encoded(PropertyValue::Real(10.0))
    );
    // WritePropertyMultiple writes at the default priority, 16.
    let real = encoded(PropertyValue::Real(30.0));
    wp(&mut db, ao(), P::PRESENT_VALUE, None, &real, Some(16)).unwrap();
    wp(&mut db, ao(), P::PRESENT_VALUE, None, NULL, Some(8)).unwrap();
    assert_eq!(
        wpm(&mut db, vec![(ao(), P::PRESENT_VALUE, None, NULL.to_vec())]).unwrap(),
        vec![ao()]
    );
    assert_eq!(
        read(&db, ao(), P::PRESENT_VALUE),
        encoded(PropertyValue::Real(0.0))
    );
}

#[test]
fn null_is_stored_where_the_datatype_has_one() {
    let mut db = db();
    for (oid, property, value) in [
        (sch(), P::SCHEDULE_DEFAULT, PropertyValue::Real(5.0)),
        (
            nf(),
            P::PROCESS_IDENTIFIER_FILTER,
            PropertyValue::Unsigned(7),
        ),
    ] {
        wp(&mut db, oid, property, None, &encoded(value.clone()), None).unwrap();
        assert_eq!(
            wp(&mut db, oid, property, None, NULL, None).unwrap(),
            Applied::Written
        );
        assert_eq!(read(&db, oid, property), NULL, "{property:?}");
        wp(&mut db, oid, property, None, &encoded(value), None).unwrap();
        assert_eq!(
            wpm(&mut db, vec![(oid, property, None, NULL.to_vec())]).unwrap(),
            vec![oid]
        );
        assert_eq!(read(&db, oid, property), NULL, "{property:?}");
    }
}

#[test]
fn null_to_value_source_needs_the_command_owner() {
    let mut db = db();
    let real = encoded(PropertyValue::Real(10.0));
    wp(&mut db, ao(), P::PRESENT_VALUE, None, &real, Some(8)).unwrap();
    let before = read(&db, ao(), P::VALUE_SOURCE);
    // Value_Source has no application NULL: its "none" member is
    // context-tagged. The owner's NULL succeeds unchanged.
    assert_eq!(
        wp(&mut db, ao(), P::VALUE_SOURCE, None, NULL, Some(8)).unwrap(),
        Applied::Unchanged
    );
    // Anyone else may not correct the slot, whatever they write.
    let stranger = CommandOrigin::Local {
        owner_device: oid(ObjectType::DEVICE, 2),
        initiating_object: None,
    };
    assert_error(
        wp_from(
            &mut db,
            &stranger,
            ao(),
            P::VALUE_SOURCE,
            None,
            NULL,
            Some(8),
        ),
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(read(&db, ao(), P::VALUE_SOURCE), before);
}

#[test]
fn null_in_datatype_names_the_choices_with_an_application_null() {
    for (object_type, property, index, expected) in [
        (ObjectType::SCHEDULE, P::PRESENT_VALUE, None, true),
        (ObjectType::CHANNEL, P::PRESENT_VALUE, None, true),
        (ObjectType::ANALOG_VALUE, P::PRESENT_VALUE, None, false),
        (ObjectType::SCHEDULE, P::SCHEDULE_DEFAULT, None, true),
        (
            ObjectType::CHARACTERSTRING_VALUE,
            P::ALARM_VALUES,
            Some(1),
            true,
        ),
        (ObjectType::MULTI_STATE_INPUT, P::ALARM_VALUES, None, false),
        (ObjectType::TIMER, P::STATE_CHANGE_VALUES, Some(3), true),
        // Index 0 is the array's size.
        (ObjectType::TIMER, P::STATE_CHANGE_VALUES, Some(0), false),
        (
            ObjectType::NOTIFICATION_FORWARDER,
            P::PROCESS_IDENTIFIER_FILTER,
            None,
            true,
        ),
        (ObjectType::TREND_LOG, P::CLIENT_COV_INCREMENT, None, true),
        (ObjectType::LOOP, P::LOW_DIFF_LIMIT, None, true),
        (
            ObjectType::AUDIT_REPORTER,
            P::MONITORED_OBJECTS,
            Some(1),
            true,
        ),
        (
            ObjectType::ANALOG_OUTPUT,
            P::AUDIT_PRIORITY_FILTER,
            None,
            true,
        ),
        (ObjectType::ANALOG_OUTPUT, P::PRIORITY_ARRAY, Some(1), true),
        // Context-tagged NULL members are no application NULL.
        (ObjectType::ANALOG_OUTPUT, P::VALUE_SOURCE, None, false),
        (
            ObjectType::EVENT_ENROLLMENT,
            P::FAULT_PARAMETERS,
            None,
            false,
        ),
        (
            ObjectType::ACCESS_CREDENTIAL,
            P::GLOBAL_IDENTIFIER,
            None,
            false,
        ),
    ] {
        assert_eq!(
            null_in_datatype(object_type, property, index),
            expected,
            "{object_type:?} {property:?} {index:?}"
        );
    }
}
