//! AddListElement and RemoveListElement refuse every target that is not a
//! BACnetLIST with SERVICES / PROPERTY_IS_NOT_A_LIST, before they decode its
//! elements (#999; Clauses 15.1.1.3.1 and 15.2.1.3.1). The decision follows the
//! property's datatype, not the shape of the value read: a whole array and a
//! BACnetDateTime both read as lists, and a constructed value reads as framed
//! bytes.

use super::*;
use bacnet_objects::elevator::ElevatorGroupObject;
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::schedule::ScheduleObject;
use bacnet_objects::value_types::DateTimeValueObject;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::primitives::{Date, Time};

type ListService = fn(&mut ObjectDatabase, &[u8]) -> Result<(), Error>;

const LIST_SERVICES: [(&str, ListService); 2] = [
    ("AddListElement", handle_add_list_element),
    ("RemoveListElement", handle_remove_list_element),
];

/// One element followed by a truncated application tag.
const MALFORMED: &[u8] = &[0x21, 2, 0xD1, 0];

/// A raw request, so malformed element lists reach the handler unchanged.
fn request(
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    elements: &[u8],
) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_ctx_object_id(&mut encoded, 0, &oid);
    bacnet_encoding::primitives::encode_ctx_enumerated(&mut encoded, 1, property.to_raw());
    if let Some(index) = index {
        bacnet_encoding::primitives::encode_ctx_unsigned(&mut encoded, 2, u64::from(index));
    }
    encoded.extend_from_slice(&[0x3e]);
    encoded.extend_from_slice(elements);
    encoded.extend_from_slice(&[0x3f]);
    encoded.to_vec()
}

fn encode_value(value: PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, &value).unwrap();
    buf.to_vec()
}

fn read(db: &ObjectDatabase, oid: ObjectIdentifier, property: PropertyIdentifier) -> PropertyValue {
    db.get(&oid).unwrap().read_property(property, None).unwrap()
}

/// Run both services over every case; each must refuse with `class`/`code`
/// and leave the target property as it was. `element` is the First Failed
/// Element Number: zero for every refusal of the target (#1026).
fn assert_both_services_refuse(
    db: &mut ObjectDatabase,
    cases: &[(ObjectIdentifier, PropertyIdentifier, Option<u32>, Vec<u8>)],
    class: ErrorClass,
    code: ErrorCode,
    element: u32,
) {
    for (name, service) in LIST_SERVICES {
        for (oid, property, index, elements) in cases {
            let context = format!("{name} {oid:?} {property:?} index {index:?}");
            let before = db
                .get(oid)
                .and_then(|object| object.read_property(*property, None).ok());
            let result = service(db, &request(*oid, *property, *index, elements));
            assert_eq!(list_refusal(result), (class, code, element), "{context}");
            let after = db
                .get(oid)
                .and_then(|object| object.read_property(*property, None).ok());
            assert_eq!(after, before, "{context}: target changed");
        }
    }
}

fn msv_db() -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    let oid = msv.object_identifier();
    db.add(Box::new(msv)).unwrap();
    (db, oid)
}

#[test]
fn list_services_refuse_scalar_properties_as_not_a_list() {
    let (mut db, msv) = msv_db();
    let ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let ai = {
        let oid = ai.object_identifier();
        db.add(Box::new(ai)).unwrap();
        oid
    };
    // Each element matches the property's own datatype, so only the target's
    // kind can refuse the request.
    let cases = [
        (
            ai,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            encode_value(PropertyValue::Real(1.0)),
        ),
        (
            msv,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            encode_value(PropertyValue::Unsigned(2)),
        ),
        (
            msv,
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            encode_value(PropertyValue::Boolean(true)),
        ),
        (
            msv,
            PropertyIdentifier::OBJECT_NAME,
            None,
            encode_value(PropertyValue::CharacterString("MSV-1".into())),
        ),
    ];
    assert_both_services_refuse(
        &mut db,
        &cases,
        ErrorClass::SERVICES,
        ErrorCode::PROPERTY_IS_NOT_A_LIST,
        0,
    );
}

#[test]
fn list_services_refuse_constructed_single_values_as_not_a_list() {
    let mut db = ObjectDatabase::new();
    let group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    let group_oid = group.object_identifier();
    db.add(Box::new(group)).unwrap();
    let dtv = DateTimeValueObject::new(1, "DTV-1").unwrap();
    let dtv_oid = dtv.object_identifier();
    db.add(Box::new(dtv)).unwrap();
    let current = encode_value(read(&db, dtv_oid, PropertyIdentifier::PRESENT_VALUE));
    let other = encode_value(PropertyValue::List(vec![
        PropertyValue::Date(Date {
            year: 0x7E,
            month: 8,
            day: 12,
            day_of_week: 3,
        }),
        PropertyValue::Time(Time {
            hour: 9,
            minute: 41,
            second: 30,
            hundredths: 0,
        }),
    ]));
    let cases = [
        // Landing_Call_Control holds one framed BACnetLandingCallStatus; the
        // element is a well-formed call (floor 5, direction UP).
        (
            group_oid,
            PropertyIdentifier::LANDING_CALL_CONTROL,
            None,
            vec![0x09, 0x05, 0x19, 0x03],
        ),
        // A BACnetDateTime reads as its two members; neither the current
        // value nor another one may be edited element by element.
        (dtv_oid, PropertyIdentifier::PRESENT_VALUE, None, current),
        (dtv_oid, PropertyIdentifier::PRESENT_VALUE, None, other),
    ];
    let commands = read(&db, dtv_oid, PropertyIdentifier::PRIORITY_ARRAY);
    assert_both_services_refuse(
        &mut db,
        &cases,
        ErrorClass::SERVICES,
        ErrorCode::PROPERTY_IS_NOT_A_LIST,
        0,
    );
    // Removing an absent member once wrote the unchanged pair back as a
    // priority-16 command; a refusal leaves Priority_Array as it was.
    assert_eq!(
        read(&db, dtv_oid, PropertyIdentifier::PRIORITY_ARRAY),
        commands
    );
}

#[test]
fn list_services_refuse_arrays_and_array_elements_as_not_a_list() {
    let (mut db, msv) = msv_db();
    let text = encode_value(PropertyValue::CharacterString("Extra".into()));
    let cases = [
        // Whole arrays read as lists but are not BACnetLISTs.
        (msv, PropertyIdentifier::STATE_TEXT, None, text.clone()),
        (
            msv,
            PropertyIdentifier::PROPERTY_LIST,
            None,
            encode_value(PropertyValue::Enumerated(
                PropertyIdentifier::DESCRIPTION.to_raw(),
            )),
        ),
        (
            msv,
            PropertyIdentifier::PRIORITY_ARRAY,
            None,
            encode_value(PropertyValue::Null),
        ),
        // A writable array element is not a list either.
        (msv, PropertyIdentifier::STATE_TEXT, Some(1), text),
    ];
    assert_both_services_refuse(
        &mut db,
        &cases,
        ErrorClass::SERVICES,
        ErrorCode::PROPERTY_IS_NOT_A_LIST,
        0,
    );
}

#[test]
fn list_target_errors_precede_element_errors_in_clause_order() {
    let (mut db, msv) = msv_db();
    let missing = ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 99).unwrap();
    let vendor = PropertyIdentifier::from_raw(9999);
    // Object, then property, then the array index, then the list kind, and
    // only then the elements' datatype. Every case carries a malformed tail.
    // Only the last refusal is about an element, the second (#1026).
    let ladder: [(
        ObjectIdentifier,
        PropertyIdentifier,
        Option<u32>,
        ErrorClass,
        ErrorCode,
    ); 8] = [
        (
            missing,
            PropertyIdentifier::ALARM_VALUES,
            None,
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ),
        (
            msv,
            vendor,
            None,
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ),
        (
            msv,
            vendor,
            Some(1),
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ),
        (
            msv,
            PropertyIdentifier::PRESENT_VALUE,
            Some(1),
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            msv,
            PropertyIdentifier::ALARM_VALUES,
            Some(1),
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            msv,
            PropertyIdentifier::STATE_TEXT,
            Some(99),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_ARRAY_INDEX,
        ),
        (
            msv,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            ErrorClass::SERVICES,
            ErrorCode::PROPERTY_IS_NOT_A_LIST,
        ),
        (
            msv,
            PropertyIdentifier::ALARM_VALUES,
            None,
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ];
    for (oid, property, index, class, code) in ladder {
        let element = if code == ErrorCode::INVALID_DATA_TYPE {
            2
        } else {
            0
        };
        assert_both_services_refuse(
            &mut db,
            &[(oid, property, index, MALFORMED.to_vec())],
            class,
            code,
            element,
        );
    }
    // The same list accepts well-formed elements, so the last rung is about
    // the elements alone.
    handle_add_list_element(
        &mut db,
        &request(msv, PropertyIdentifier::ALARM_VALUES, None, &[0x21, 2]),
    )
    .unwrap();
    assert_eq!(
        read(&db, msv, PropertyIdentifier::ALARM_VALUES),
        PropertyValue::List(vec![PropertyValue::Unsigned(2)])
    );
}

#[test]
fn framed_lists_of_other_elements_never_take_the_destination_codec() {
    let mut db = ObjectDatabase::new();
    let mut schedule = ScheduleObject::new(1, "SCH-1", PropertyValue::Real(0.0)).unwrap();
    schedule
        .add_object_property_reference(BACnetObjectPropertyReference::new(
            ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ))
        .unwrap();
    let oid = schedule.object_identifier();
    db.add(Box::new(schedule)).unwrap();
    let destination = BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        to_time: Time {
            hour: 23,
            minute: 59,
            second: 0,
            hundredths: 0,
        },
        recipient: BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 7).unwrap()),
        process_identifier: 7,
        issue_confirmed_notifications: false,
        transitions: EventTransitionBits::all(),
    };
    let mut framed = BytesMut::new();
    bacnet_encoding::constructed::encode_destination_list(&mut framed, &[destination]).unwrap();
    // Schedule's list of BACnetDeviceObjectPropertyReference is held as raw
    // bytes, like Recipient_List, but takes its own reference codec (#1121):
    // a destination is not a reference, so the first element is refused.
    assert_both_services_refuse(
        &mut db,
        &[(
            oid,
            PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            None,
            framed.to_vec(),
        )],
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
        1,
    );
}
