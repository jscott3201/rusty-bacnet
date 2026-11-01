//! State_Change_Values, List_Of_Object_Property_References and
//! Priority_For_Writing: present as a set, their writes, the run each
//! transition queues and Reliability (#2206).

use super::*;
use crate::command::{RunPlan, WriteFailure};
use crate::traits::CommandRunner;
use bacnet_encoding::constructed::encode_device_object_property_reference;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference as Ref, BACnetTimerStateChangeValue as V,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::RemoteError;
use bytes::BytesMut;

const ROWS: [P; 3] = [
    P::STATE_CHANGE_VALUES,
    P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
    P::PRIORITY_FOR_WRITING,
];

fn code(error: Error) -> (ErrorClass, ErrorCode) {
    match error {
        Error::Remote(RemoteError::Error { class, code }) => (class, code),
        other => panic!("not a protocol error: {other:?}"),
    }
}

fn property(code: ErrorCode) -> (ErrorClass, ErrorCode) {
    (ErrorClass::PROPERTY, code)
}

fn av(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap()
}

fn reference(instance: u32) -> Ref {
    Ref::new_local(av(instance), P::PRESENT_VALUE.to_raw())
}

fn real(value: f32) -> V {
    V::Primitive(PropertyValue::Real(value))
}

/// Transition N writes N.0, except RUNNING_TO_IDLE (2), which writes nothing.
fn values() -> StateChangeValues {
    core::array::from_fn(|index| match index + 1 {
        2 => V::NoValue,
        n => real(n as f32),
    })
}

fn timer() -> TimerObject {
    let mut timer = TimerObject::new(1, "TMR-1").unwrap();
    timer
        .set_state_change(values(), vec![reference(1), reference(2)], 9)
        .unwrap();
    timer
}

fn reliability(timer: &TimerObject) -> Reliability {
    match timer.read_property(P::RELIABILITY, None).unwrap() {
        PropertyValue::Enumerated(raw) => Reliability::from_raw(raw),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_three_rows_come_together_or_not_at_all() {
    let mut plain = TimerObject::new(1, "TMR-1").unwrap();
    let listed = crate::test_rows::listed(&plain.property_metadata());
    for p in ROWS {
        assert!(!listed.contains(&p));
        assert!(!plain.is_writable_property(p));
        assert_eq!(
            code(plain.read_property(p, None).unwrap_err()),
            property(ErrorCode::UNKNOWN_PROPERTY)
        );
    }
    assert!(plain.command_runner_mut().is_none());

    let timer = timer();
    let listed = crate::test_rows::listed(&timer.property_metadata());
    // In Table 12-75's order, last before Property_List (which isn't listed).
    assert_eq!(listed[listed.len() - 3..], ROWS);
    assert!(!crate::test_rows::required(&timer.property_metadata()).contains(&ROWS[0]));
    for (p, writable) in ROWS.into_iter().zip([true, true, false]) {
        assert_eq!(timer.is_writable_property(p), writable, "{p:?}");
    }
    assert_eq!(
        timer
            .read_property(P::STATE_CHANGE_VALUES, Some(0))
            .unwrap(),
        PropertyValue::Unsigned(7)
    );
    assert_eq!(
        timer
            .read_property(P::STATE_CHANGE_VALUES, Some(1))
            .unwrap(),
        PropertyValue::Real(1.0)
    );
    // no-value goes out as its context-tagged NULL.
    assert_eq!(
        timer
            .read_property(P::STATE_CHANGE_VALUES, Some(2))
            .unwrap(),
        PropertyValue::ApplicationData(vec![0x08])
    );
    assert_eq!(
        timer.read_property(P::PRIORITY_FOR_WRITING, None).unwrap(),
        PropertyValue::Unsigned(9)
    );
    let PropertyValue::List(references) = timer
        .read_property(P::LIST_OF_OBJECT_PROPERTY_REFERENCES, None)
        .unwrap()
    else {
        panic!("a list");
    };
    assert_eq!(references.len(), 2);
}

#[test]
fn the_setter_refuses_what_it_cant_serve_and_changes_nothing() {
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap();
    let not_a_device = Ref {
        device_identifier: Some(av(9)),
        ..reference(1)
    };
    let mut no_encoding = values();
    no_encoding[0] = V::Primitive(PropertyValue::List(vec![]));
    let range = property(ErrorCode::VALUE_OUT_OF_RANGE);
    for (values, references, priority, expected) in [
        (values(), vec![], 0, range),
        (values(), vec![], 17, range),
        (no_encoding, vec![], 9, range),
        (values(), vec![not_a_device], 9, range),
        (
            values(),
            vec![reference(1); MAX_TIMER_REFERENCES + 1],
            9,
            (ErrorClass::RESOURCES, ErrorCode::NO_SPACE_TO_WRITE_PROPERTY),
        ),
    ] {
        let mut timer = TimerObject::new(1, "TMR-1").unwrap();
        let error = timer.set_state_change(values, references, priority);
        assert_eq!(code(error.unwrap_err()), expected);
        assert!(timer.command_runner().is_none());
    }
    // A reference into another Device is taken.
    let remote = Ref {
        device_identifier: Some(device),
        ..reference(1)
    };
    TimerObject::new(1, "TMR-1")
        .unwrap()
        .set_state_change(values(), vec![remote], 16)
        .unwrap();
}

fn octets(values: &[V]) -> PropertyValue {
    let mut encoded = BytesMut::new();
    for value in values {
        bacnet_encoding::constructed::encode_timer_state_change_value(&mut encoded, value).unwrap();
    }
    PropertyValue::ApplicationData(encoded.to_vec())
}

#[test]
fn state_change_values_take_whole_and_element_writes() {
    let mut timer = timer();
    let scv = P::STATE_CHANGE_VALUES;
    let enumerated: [V; 7] =
        core::array::from_fn(|n| V::Primitive(PropertyValue::Enumerated(n as u32)));
    timer
        .write_property(scv, None, octets(&enumerated), None)
        .unwrap();
    assert_eq!(
        timer.read_property(scv, Some(7)).unwrap(),
        PropertyValue::Enumerated(6)
    );
    // A whole read writes back unchanged.
    let read = timer.read_property(scv, None).unwrap();
    timer.write_property(scv, None, read.clone(), None).unwrap();
    assert_eq!(timer.read_property(scv, None).unwrap(), read);
    timer
        .write_property(scv, Some(3), octets(&[V::NoValue]), None)
        .unwrap();
    timer
        .write_property(scv, Some(4), PropertyValue::Enumerated(9), None)
        .unwrap();
    assert_eq!(
        timer.read_property(scv, Some(4)).unwrap(),
        PropertyValue::Enumerated(9)
    );
    for (index, value, expected) in [
        (
            None,
            octets(&enumerated[..6]),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            Some(0),
            PropertyValue::Unsigned(7),
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
        (Some(8), PropertyValue::Null, ErrorCode::INVALID_ARRAY_INDEX),
        (
            Some(1),
            PropertyValue::ApplicationData(vec![0x4E, 0x00, 0x4F]),
            ErrorCode::DATATYPE_NOT_SUPPORTED,
        ),
        (
            Some(1),
            PropertyValue::ApplicationData(vec![0x1E, 0x1F]),
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        (
            Some(1),
            octets(&[V::NoValue, V::NoValue]),
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        (
            Some(1),
            PropertyValue::List(vec![]),
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ] {
        let error = timer.write_property(scv, index, value, None).unwrap_err();
        assert_eq!(code(error), property(expected), "{index:?}");
    }
    assert_eq!(
        timer.read_property(scv, Some(4)).unwrap(),
        PropertyValue::Enumerated(9)
    );
}

#[test]
fn references_take_whole_list_writes_checked_as_a_channel_checks_them() {
    let mut timer = timer();
    let lopr = P::LIST_OF_OBJECT_PROPERTY_REFERENCES;
    let encode = |references: &[Ref]| {
        let mut encoded = BytesMut::new();
        for reference in references {
            encode_device_object_property_reference(&mut encoded, reference);
        }
        PropertyValue::ApplicationData(encoded.to_vec())
    };
    timer
        .write_property(lopr, None, encode(&[reference(5)]), None)
        .unwrap();
    let read = timer.read_property(lopr, None).unwrap();
    assert_eq!(read, PropertyValue::List(vec![encode(&[reference(5)])]));
    let not_a_device = Ref {
        device_identifier: Some(av(9)),
        ..reference(1)
    };
    for (index, value, expected) in [
        (
            None,
            encode(&[reference(1), not_a_device]),
            property(ErrorCode::VALUE_OUT_OF_RANGE),
        ),
        (
            Some(1),
            encode(&[reference(1)]),
            property(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        (
            None,
            encode(&vec![reference(1); MAX_TIMER_REFERENCES + 1]),
            (ErrorClass::RESOURCES, ErrorCode::NO_SPACE_TO_WRITE_PROPERTY),
        ),
        (
            None,
            PropertyValue::Real(1.0),
            property(ErrorCode::INVALID_DATA_TYPE),
        ),
    ] {
        let error = timer.write_property(lopr, index, value, None).unwrap_err();
        assert_eq!(code(error), expected);
    }
    assert_eq!(timer.read_property(lopr, None).unwrap(), read);
}

/// The run TMR-1 has queued: each write's target, value and priority.
fn queued(timer: &mut TimerObject) -> Option<Vec<(ObjectIdentifier, PropertyValue, Option<u8>)>> {
    let run = timer.take_command_run()?;
    assert_eq!(run.generation, timer.command_generation());
    let RunPlan::Actions(commands) = run.plan else {
        panic!("a timer queues actions");
    };
    Some(
        commands
            .into_iter()
            .map(|command| {
                assert!(!command.quit_on_failure && command.post_delay.is_none());
                (
                    command.object_identifier,
                    command.property_value,
                    command.priority,
                )
            })
            .collect(),
    )
}

fn writes(value: f32) -> Vec<(ObjectIdentifier, PropertyValue, Option<u8>)> {
    [1, 2]
        .map(|n| (av(n), PropertyValue::Real(value), Some(9)))
        .to_vec()
}

#[test]
fn each_transition_queues_its_value_for_every_reference() {
    let mut timer = timer();
    assert_eq!(queued(&mut timer), None);
    timer
        .write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(5_000), None)
        .unwrap();
    assert_eq!(queued(&mut timer), Some(writes(1.0)));
    let generation = timer.command_generation();
    // RUNNING_TO_IDLE has no value: nothing queued, and the run before is
    // superseded.
    let idle = PropertyValue::Enumerated(TimerState::IDLE.to_raw());
    timer
        .write_property(P::TIMER_STATE, None, idle, None)
        .unwrap();
    assert_eq!(queued(&mut timer), None);
    assert!(!timer.record_command_write(generation, 0, true));
    assert!(!timer.complete_command_run(generation, Err(WriteFailure::Process)));
    // Two transitions under one guard queue one run, in order.
    timer
        .write_property(P::TIMER_RUNNING, None, PropertyValue::Boolean(true), None)
        .unwrap();
    timer
        .write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(0), None)
        .unwrap();
    let mut both = writes(1.0);
    both.extend(writes(5.0));
    assert_eq!(queued(&mut timer), Some(both));
    // The run ends with its first failure, as Reliability.
    let generation = timer.command_generation();
    assert!(timer.record_command_write(generation, 3, false));
    assert!(!timer.record_command_write(generation, 4, false));
    assert!(timer.complete_command_run(generation, Err(WriteFailure::Communication)));
    assert_eq!(reliability(&timer), Reliability::COMMUNICATION_FAILURE);
    assert!(!timer.complete_command_run(generation, Ok(())));
}

#[test]
fn reliability_flags_mixed_datatypes_and_is_simulated_out_of_service() {
    let mut timer = timer();
    let scv = P::STATE_CHANGE_VALUES;
    assert_eq!(reliability(&timer), Reliability::NO_FAULT_DETECTED);
    // A NULL relinquish mixes with any datatype.
    timer
        .write_property(scv, Some(1), PropertyValue::Null, None)
        .unwrap();
    assert_eq!(reliability(&timer), Reliability::NO_FAULT_DETECTED);
    timer
        .write_property(scv, Some(1), PropertyValue::Boolean(true), None)
        .unwrap();
    assert_eq!(reliability(&timer), Reliability::CONFIGURATION_ERROR);
    assert_eq!(
        timer.read_property(P::STATUS_FLAGS, None).unwrap(),
        common::compute_status_flags(
            StatusFlags::empty(),
            Reliability::CONFIGURATION_ERROR,
            false,
            EventState::NORMAL,
        )
    );
    // Out of service the client's value holds, starting from the served one.
    let oos = P::OUT_OF_SERVICE;
    timer
        .write_property(oos, None, PropertyValue::Boolean(true), None)
        .unwrap();
    assert_eq!(reliability(&timer), Reliability::CONFIGURATION_ERROR);
    let fault = PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw());
    timer
        .write_property(P::RELIABILITY, None, fault, None)
        .unwrap();
    timer
        .write_property(scv, Some(1), PropertyValue::Real(1.0), None)
        .unwrap();
    assert_eq!(reliability(&timer), Reliability::OVER_RANGE);
    timer
        .write_property(oos, None, PropertyValue::Boolean(false), None)
        .unwrap();
    assert_eq!(reliability(&timer), Reliability::NO_FAULT_DETECTED);
}
