//! Writes of List_Of_Object_Property_References and Priority_For_Writing
//! (#1088): the checks, the read-back shape, and what a change owes the
//! targets: the current value to the new list, and a NULL to each slot it
//! leaves behind while the Schedule holds it.

use super::*;
use bacnet_encoding::constructed::encode_object_property_reference;
use bacnet_types::calendar::SpecificDate;

type P = PropertyIdentifier;

/// Monday 14 September 2026.
fn monday() -> SpecificDate {
    SpecificDate::new(2026, 9, 14).unwrap()
}

fn noon() -> Time {
    Time {
        hour: 12,
        minute: 0,
        second: 0,
        hundredths: 0,
    }
}

fn reference(object_type: ObjectType, instance: u32) -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference::new(
        ObjectIdentifier::new(object_type, instance).unwrap(),
        P::PRESENT_VALUE.to_raw(),
    )
}

fn a() -> BACnetObjectPropertyReference {
    reference(ObjectType::ANALOG_VALUE, 1)
}

fn b() -> BACnetObjectPropertyReference {
    reference(ObjectType::ANALOG_OUTPUT, 2)
}

fn encoded(references: &[BACnetObjectPropertyReference]) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    for reference in references {
        encode_object_property_reference(&mut bytes, reference);
    }
    bytes.to_vec()
}

fn write_references(
    sched: &mut ScheduleObject,
    references: &[BACnetObjectPropertyReference],
) -> Result<(), Error> {
    sched.write_property(
        P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
        None,
        PropertyValue::ApplicationData(encoded(references)),
        None,
    )
}

fn write_priority(sched: &mut ScheduleObject, priority: u64) -> Result<(), Error> {
    sched.write_property(
        P::PRIORITY_FOR_WRITING,
        None,
        PropertyValue::Unsigned(priority),
        None,
    )
}

fn tick(sched: &mut ScheduleObject) -> Option<ScheduleWrite> {
    sched.tick_schedule(monday(), noon(), &|_| false)
}

fn write(
    value: PropertyValue,
    priority: u8,
    references: Vec<BACnetObjectPropertyReference>,
) -> ScheduleWrite {
    ScheduleWrite {
        value,
        priority,
        references,
        retry: false,
    }
}

/// Default 10.0, commanding A and B at priority 16, after the first tick.
fn commanding() -> ScheduleObject {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(10.0)).unwrap();
    sched
        .set_object_property_references(vec![a(), b()])
        .unwrap();
    assert_eq!(
        tick(&mut sched),
        Some(write(PropertyValue::Real(10.0), 16, vec![a(), b()]))
    );
    sched
}

fn assert_code(result: Result<(), Error>, class: ErrorClass, code: ErrorCode, what: &str) {
    match result {
        Err(Error::Protocol { class: c, code: e }) => {
            assert_eq!(c, class, "{what}: class");
            assert_eq!(e, code, "{what}: expected {code:?}");
        }
        other => panic!("{what}: expected {code:?}, got {other:?}"),
    }
}

#[test]
fn priority_for_writing_write_takes_an_unsigned_from_1_to_16() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(10.0)).unwrap();
    for priority in [1, 16, 9] {
        write_priority(&mut sched, priority).unwrap();
    }
    for priority in [0, 17, 256, u64::MAX] {
        assert_code(
            write_priority(&mut sched, priority),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            "out of range",
        );
    }
    assert_code(
        sched.write_property(
            P::PRIORITY_FOR_WRITING,
            None,
            PropertyValue::Real(8.0),
            None,
        ),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
        "a Real",
    );
    assert_code(
        sched.write_property(
            P::PRIORITY_FOR_WRITING,
            Some(1),
            PropertyValue::Unsigned(8),
            None,
        ),
        ErrorClass::PROPERTY,
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        "an array index",
    );
    assert_eq!(
        sched.read_property(P::PRIORITY_FOR_WRITING, None).unwrap(),
        PropertyValue::Unsigned(9)
    );
}

#[test]
fn reference_list_write_takes_the_wire_bytes_and_reads_back_unchanged() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(10.0)).unwrap();
    let indexed = BACnetObjectPropertyReference::new_indexed(
        ObjectIdentifier::new(ObjectType::MULTI_STATE_OUTPUT, 7).unwrap(),
        P::STATE_TEXT.to_raw(),
        2,
    );
    let references = vec![a(), indexed.clone()];
    write_references(&mut sched, &references).unwrap();
    let read = sched
        .read_property(P::LIST_OF_OBJECT_PROPERTY_REFERENCES, None)
        .unwrap();
    assert_eq!(read, PropertyValue::ApplicationData(encoded(&references)));
    // What a read returns writes back unchanged, and so does one element per
    // ApplicationData.
    sched
        .write_property(
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            None,
            read.clone(),
            None,
        )
        .unwrap();
    sched
        .write_property(
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            None,
            PropertyValue::List(vec![
                PropertyValue::ApplicationData(encoded(std::slice::from_ref(&indexed))),
                PropertyValue::ApplicationData(encoded(&[a()])),
            ]),
            None,
        )
        .unwrap();
    assert_eq!(sched.list_of_object_property_references, [indexed, a()]);
    // An empty list empties it.
    write_references(&mut sched, &[]).unwrap();
    assert!(sched.list_of_object_property_references.is_empty());
}

#[test]
fn reference_list_refusals_leave_it_unchanged() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(10.0)).unwrap();
    write_references(&mut sched, &[a()]).unwrap();
    let before = sched.list_of_object_property_references.clone();
    let mut remote = encoded(&[b()]);
    // [3] Device 9: an object in another device.
    remote.extend([0x3C, 0x02, 0x00, 0x00, 0x09]);
    let local_then_remote = [encoded(&[a()]), remote.clone()].concat();
    // [3] naming analog-value 9, which is no Device (#1308).
    let mut not_a_device = encoded(&[b()]);
    not_a_device.extend([0x3C, 0x00, 0x80, 0x00, 0x09]);
    let local_then_not_a_device = [encoded(&[a()]), not_a_device].concat();
    // A refusal of one member names its position in the list, from 1
    // (#1121); a refusal of the whole value names none.
    let cases = [
        (
            None,
            PropertyValue::ApplicationData(local_then_remote),
            ErrorClass::PROPERTY,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            Some(2),
            "a member in another device",
        ),
        (
            None,
            PropertyValue::ApplicationData(local_then_not_a_device),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            Some(2),
            "a member whose device identifier is no Device",
        ),
        (
            None,
            PropertyValue::ApplicationData(vec![0x44, 0x41, 0xA8, 0, 0]),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
            Some(1),
            "an application Real",
        ),
        (
            None,
            PropertyValue::List(vec![
                PropertyValue::ApplicationData(encoded(&[a()])),
                PropertyValue::ApplicationData(encoded(&[b()])[..4].to_vec()),
            ]),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_ENCODING,
            Some(2),
            "a truncated member in the second element",
        ),
        (
            None,
            PropertyValue::Real(1.0),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
            None,
            "a Real value",
        ),
        (
            Some(1),
            PropertyValue::ApplicationData(encoded(&[b()])),
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
            None,
            "an array index",
        ),
        (
            None,
            PropertyValue::ApplicationData(encoded(&vec![b(); targets::MAX_REFERENCES + 1])),
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
            Some(1025),
            "past the cap",
        ),
    ];
    for (index, value, class, code, member, what) in cases {
        let result =
            sched.write_property(P::LIST_OF_OBJECT_PROPERTY_REFERENCES, index, value, None);
        match member {
            Some(position) => {
                common::assert_list_element_refused(result, class, code, position, what)
            }
            None => assert_code(result, class, code, what),
        }
        assert_eq!(sched.list_of_object_property_references, before, "{what}");
    }
    // The local setters share the cap.
    sched
        .set_object_property_references(vec![b(); targets::MAX_REFERENCES])
        .unwrap();
    assert_code(
        sched.add_object_property_reference(a()),
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        "add past the cap",
    );
    assert_code(
        sched.set_object_property_references(vec![b(); targets::MAX_REFERENCES + 1]),
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        "set past the cap",
    );
    assert_eq!(
        sched.list_of_object_property_references.len(),
        targets::MAX_REFERENCES
    );
}

#[test]
fn a_new_reference_gets_the_current_value_at_once() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(10.0)).unwrap();
    // Without references the value is calculated but goes nowhere.
    assert_eq!(tick(&mut sched), None);
    write_references(&mut sched, &[a()]).unwrap();
    assert!(sched.take_owed_schedule_writes().is_empty());
    assert_eq!(
        tick(&mut sched),
        Some(write(PropertyValue::Real(10.0), 16, vec![a()]))
    );
    assert_eq!(tick(&mut sched), None);
    // Appending sends the unchanged value to the whole list.
    sched.add_object_property_reference(b()).unwrap();
    assert_eq!(
        tick(&mut sched),
        Some(write(PropertyValue::Real(10.0), 16, vec![a(), b()]))
    );
    assert_eq!(tick(&mut sched), None);
}

#[test]
fn a_dropped_reference_is_relinquished_at_the_priority_it_holds() {
    let mut sched = commanding();
    write_references(&mut sched, &[a()]).unwrap();
    assert_eq!(
        sched.take_owed_schedule_writes(),
        [write(PropertyValue::Null, 16, vec![b()])]
    );
    assert_eq!(
        tick(&mut sched),
        Some(write(PropertyValue::Real(10.0), 16, vec![a()]))
    );
    // B is no longer held: dropping A as well relinquishes A alone.
    write_references(&mut sched, &[]).unwrap();
    assert_eq!(
        sched.take_owed_schedule_writes(),
        [write(PropertyValue::Null, 16, vec![a()])]
    );
    assert_eq!(tick(&mut sched), None);
}

#[test]
fn a_priority_change_moves_every_target_to_the_new_slot() {
    let mut sched = commanding();
    write_priority(&mut sched, 9).unwrap();
    assert_eq!(
        sched.take_owed_schedule_writes(),
        [write(PropertyValue::Null, 16, vec![a(), b()])]
    );
    assert_eq!(
        tick(&mut sched),
        Some(write(PropertyValue::Real(10.0), 9, vec![a(), b()]))
    );
    // The same priority again changes no slot and resends the value.
    write_priority(&mut sched, 9).unwrap();
    assert!(sched.take_owed_schedule_writes().is_empty());
    assert_eq!(
        tick(&mut sched),
        Some(write(PropertyValue::Real(10.0), 9, vec![a(), b()]))
    );
}

#[test]
fn two_changes_before_a_pass_owe_each_slot_once() {
    // A WritePropertyMultiple of both properties: drop B, then move to 9.
    let mut sched = commanding();
    write_references(&mut sched, &[a()]).unwrap();
    write_priority(&mut sched, 9).unwrap();
    assert_eq!(
        sched.take_owed_schedule_writes(),
        [
            write(PropertyValue::Null, 16, vec![b()]),
            write(PropertyValue::Null, 16, vec![a()]),
        ]
    );
    assert_eq!(
        tick(&mut sched),
        Some(write(PropertyValue::Real(10.0), 9, vec![a()]))
    );
}

#[test]
fn a_null_value_or_an_inactive_schedule_holds_no_slot() {
    // A NULL Present_Value has relinquished already: nothing more to clear.
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Null).unwrap();
    sched.add_object_property_reference(a()).unwrap();
    assert_eq!(
        tick(&mut sched),
        Some(write(PropertyValue::Null, 16, vec![a()]))
    );
    write_references(&mut sched, &[b()]).unwrap();
    assert!(sched.take_owed_schedule_writes().is_empty());
    assert_eq!(
        tick(&mut sched),
        Some(write(PropertyValue::Null, 16, vec![b()]))
    );

    // Out of season: an Effective_Period that ended yesterday. Another
    // Schedule may hold the slots now, so nothing is relinquished and
    // nothing written.
    let mut sched = commanding();
    sched
        .set_effective_period(BACnetDateRange {
            start_date: SpecificDate::new(2026, 9, 1).unwrap().to_date(),
            end_date: SpecificDate::new(2026, 9, 13).unwrap().to_date(),
        })
        .unwrap();
    assert_eq!(tick(&mut sched), None);
    write_references(&mut sched, &[a()]).unwrap();
    write_priority(&mut sched, 9).unwrap();
    assert!(sched.take_owed_schedule_writes().is_empty());
    assert_eq!(tick(&mut sched), None);
}

#[test]
fn out_of_service_a_change_owes_the_current_value() {
    let mut sched = commanding();
    sched
        .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
        .unwrap();
    sched
        .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(30.0), None)
        .unwrap();
    assert_eq!(
        sched.take_owed_schedule_writes(),
        [write(PropertyValue::Real(30.0), 16, vec![a(), b()])]
    );
    // The calculation stays suspended; the change itself owes the value.
    write_references(&mut sched, &[b()]).unwrap();
    assert_eq!(
        sched.take_owed_schedule_writes(),
        [
            write(PropertyValue::Null, 16, vec![a()]),
            write(PropertyValue::Real(30.0), 16, vec![b()]),
        ]
    );
    assert_eq!(tick(&mut sched), None);
    assert!(sched.take_owed_schedule_writes().is_empty());
}
