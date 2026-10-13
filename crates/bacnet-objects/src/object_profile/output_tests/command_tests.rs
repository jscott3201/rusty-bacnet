use super::*;
use crate::command_source::CommandOrigin;
use bacnet_encoding::constructed::decode_value_source;
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetValueSource};
use bacnet_types::enums::{EventState, EventType, Reliability};
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier};

fn origin(device: u32) -> CommandOrigin {
    CommandOrigin::Local {
        owner_device: ObjectIdentifier::new(ObjectType::DEVICE, device).unwrap(),
        initiating_object: None,
    }
}

pub(super) fn values(kind: ObjectType) -> [PropertyValue; 3] {
    match kind {
        ObjectType::ANALOG_OUTPUT => [
            PropertyValue::Real(2.5),
            PropertyValue::Real(3.5),
            PropertyValue::Real(0.0),
        ],
        ObjectType::BINARY_OUTPUT => [
            PropertyValue::Enumerated(1),
            PropertyValue::Enumerated(0),
            PropertyValue::Enumerated(0),
        ],
        ObjectType::MULTI_STATE_OUTPUT => [
            PropertyValue::Unsigned(2),
            PropertyValue::Unsigned(3),
            PropertyValue::Unsigned(1),
        ],
        _ => unreachable!(),
    }
}

pub(super) fn state(object: &dyn ProfileOutput) -> Vec<(P, PropertyValue)> {
    [
        P::PRESENT_VALUE,
        P::PRIORITY_ARRAY,
        P::CURRENT_COMMAND_PRIORITY,
        P::VALUE_SOURCE,
        P::VALUE_SOURCE_ARRAY,
        P::LAST_COMMAND_TIME,
        P::RELINQUISH_DEFAULT,
        P::FEEDBACK_VALUE,
        P::STATUS_FLAGS,
        P::RELIABILITY,
        P::EVENT_STATE,
        P::EVENT_TIME_STAMPS,
        P::EVENT_MESSAGE_TEXTS,
        P::ACKED_TRANSITIONS,
    ]
    .into_iter()
    .filter(|p| object.property_list().contains(p))
    .map(|p| (p, object.read_property(p, None).unwrap()))
    .collect()
}

fn projected(object: &dyn ProfileOutput, priority: Option<u8>, device: Option<u32>, time: u16) {
    assert_eq!(
        object
            .read_property(P::CURRENT_COMMAND_PRIORITY, None)
            .unwrap(),
        priority.map_or(PropertyValue::Null, |p| PropertyValue::Unsigned(p.into()))
    );
    let PropertyValue::ApplicationData(bytes) =
        object.read_property(P::VALUE_SOURCE, None).unwrap()
    else {
        panic!("encoded source")
    };
    let (source, used) = decode_value_source(&bytes, 0).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(
        source,
        device.map_or(BACnetValueSource::None, |n| {
            BACnetValueSource::Object(BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, n).unwrap(),
            })
        })
    );
    let PropertyValue::ApplicationData(bytes) =
        object.read_property(P::LAST_COMMAND_TIME, None).unwrap()
    else {
        panic!("encoded timestamp")
    };
    let (timestamp, used) = decode_timestamp_choice(&bytes, 0).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(timestamp, BACnetTimeStamp::SequenceNumber(time));
}

fn tags_leave_state(object: &mut dyn ProfileOutput) {
    let before = state(object);
    object
        .write_property_from(P::TAGS, None, framed(&tags("written")), Some(4), &origin(2))
        .unwrap();
    assert_eq!(state(object), before);
    assert_eq!(
        object.read_property(P::TAGS, Some(1)).unwrap(),
        framed(&tags("written"))
    );
}

#[test]
fn tags_do_not_steal_priorities_source_correction_or_command_time() {
    for kind in KINDS {
        for persistent in [false, true] {
            let storage = Arc::new(Memory::default());
            let mut profiled = object(kind, persistent.then(|| storage.clone())).unwrap();
            let mut control = object(kind, None).unwrap();
            profiled.provision(optional(7)).unwrap();
            let [a, b, default] = values(kind);
            let mut priority_array = vec![PropertyValue::Null; 16];
            for (step, priority, value, writer, active, source, expected) in [
                (1, 8, a.clone(), 1, Some(8), Some(1), a.clone()),
                (2, 4, b, 2, Some(4), Some(2), values(kind)[1].clone()),
                (3, 4, PropertyValue::Null, 2, Some(8), Some(1), a.clone()),
                (4, 8, PropertyValue::Null, 1, None, None, default),
            ] {
                for output in [&mut profiled, &mut control] {
                    output
                        .write_property_from(
                            P::PRESENT_VALUE,
                            None,
                            value.clone(),
                            Some(priority),
                            &origin(writer),
                        )
                        .unwrap();
                }
                priority_array[usize::from(priority - 1)] = value;
                assert_eq!(
                    profiled.read_property(P::PRESENT_VALUE, None).unwrap(),
                    expected
                );
                assert_eq!(
                    profiled.read_property(P::PRIORITY_ARRAY, None).unwrap(),
                    PropertyValue::List(priority_array.clone())
                );
                projected(profiled.as_ref(), active, source, step);
                tags_leave_state(profiled.as_mut());
                assert_eq!(state(profiled.as_ref()), state(control.as_ref()));
                if step == 1 {
                    let claim = profiled.read_property(P::VALUE_SOURCE, None).unwrap();
                    assert_code(
                        profiled.write_property_from(
                            P::VALUE_SOURCE,
                            None,
                            claim.clone(),
                            Some(8),
                            &origin(2),
                        ),
                        ErrorCode::WRITE_ACCESS_DENIED,
                    );
                    profiled
                        .write_property_from(P::VALUE_SOURCE, None, claim, Some(8), &origin(1))
                        .unwrap();
                    assert_eq!(state(profiled.as_ref()), state(control.as_ref()));
                }
            }
            for output in [&mut profiled, &mut control] {
                output
                    .write_property(P::RELINQUISH_DEFAULT, None, a.clone(), None)
                    .unwrap();
            }
            assert_eq!(profiled.read_property(P::PRESENT_VALUE, None).unwrap(), a);
            projected(profiled.as_ref(), None, None, 4);
            assert_eq!(state(profiled.as_ref()), state(control.as_ref()));
            let before = state(profiled.as_ref());
            assert_code(
                profiled.write_property(P::PRESENT_VALUE, None, a.clone(), Some(8)),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert_code(
                profiled.write_property(P::VALUE_SOURCE, None, PropertyValue::Null, Some(8)),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert_code(
                profiled.write_property_from(
                    P::PRESENT_VALUE,
                    None,
                    PropertyValue::Boolean(true),
                    Some(8),
                    &origin(1),
                ),
                ErrorCode::INVALID_DATA_TYPE,
            );
            let invalid = match kind {
                ObjectType::ANALOG_OUTPUT => PropertyValue::Real(f32::NAN),
                ObjectType::BINARY_OUTPUT => PropertyValue::Enumerated(2),
                _ => PropertyValue::Unsigned(0),
            };
            assert_code(
                profiled.write_property_from(P::PRESENT_VALUE, None, invalid, Some(8), &origin(1)),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            assert_eq!(state(profiled.as_ref()), before);
            assert_eq!(storage.attempts(), if persistent { 4 } else { 0 });
        }
    }
}

#[test]
fn profile_writes_preserve_command_failure_and_out_of_range_transitions() {
    for kind in KINDS {
        let mut profiled = object(kind, None).unwrap();
        let mut control = object(kind, None).unwrap();
        profiled.provision(optional(7)).unwrap();
        for output in [&mut profiled, &mut control] {
            output
                .write_property(
                    P::EVENT_DETECTION_ENABLE,
                    None,
                    PropertyValue::Boolean(true),
                    None,
                )
                .unwrap();
            output
                .write_property(P::TIME_DELAY, None, PropertyValue::Unsigned(0), None)
                .unwrap();
            if kind == ObjectType::ANALOG_OUTPUT {
                for (p, v) in [
                    (P::HIGH_LIMIT, 1.0),
                    (P::LOW_LIMIT, -1.0),
                    (P::DEADBAND, 0.0),
                ] {
                    output
                        .write_property(p, None, PropertyValue::Real(v), None)
                        .unwrap();
                }
                output
                    .write_property(
                        P::LIMIT_ENABLE,
                        None,
                        PropertyValue::BitString {
                            unused_bits: 6,
                            data: vec![0xc0],
                        },
                        None,
                    )
                    .unwrap();
            }
            output
                .write_property_from(
                    P::PRESENT_VALUE,
                    None,
                    values(kind)[0].clone(),
                    Some(8),
                    &origin(1),
                )
                .unwrap();
        }
        tags_leave_state(profiled.as_mut());
        let target = if kind == ObjectType::ANALOG_OUTPUT {
            EventState::HIGH_LIMIT
        } else {
            EventState::OFFNORMAL
        };
        let algorithm = if kind == ObjectType::ANALOG_OUTPUT {
            EventType::OUT_OF_RANGE
        } else {
            EventType::COMMAND_FAILURE
        };
        for output in [&mut profiled, &mut control] {
            let proposal = output.evaluate_intrinsic_reporting().unwrap();
            assert_eq!(proposal.change.to, target);
            assert_eq!(proposal.event_type, algorithm);
            crate::event::commit_test_proposal(output.as_mut(), proposal);
        }
        assert_eq!(state(profiled.as_ref()), state(control.as_ref()));
        tags_leave_state(profiled.as_mut());
        for output in [&mut profiled, &mut control] {
            if kind == ObjectType::ANALOG_OUTPUT {
                output
                    .write_property_from(
                        P::PRESENT_VALUE,
                        None,
                        PropertyValue::Real(0.0),
                        Some(8),
                        &origin(1),
                    )
                    .unwrap();
            } else {
                output
                    .write_property(P::FEEDBACK_VALUE, None, values(kind)[0].clone(), None)
                    .unwrap();
            }
            let proposal = output.evaluate_intrinsic_reporting().unwrap();
            assert_eq!(proposal.change.from, target);
            assert_eq!(proposal.change.to, EventState::NORMAL);
            crate::event::commit_test_proposal(output.as_mut(), proposal);
        }
        assert_eq!(state(profiled.as_ref()), state(control.as_ref()));
        if kind == ObjectType::MULTI_STATE_OUTPUT {
            profiled
                .write_property(P::FEEDBACK_VALUE, None, PropertyValue::Unsigned(7), None)
                .unwrap();
            tags_leave_state(profiled.as_mut());
            assert_eq!(
                profiled.read_property(P::RELIABILITY, None).unwrap(),
                PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
            );
        }
    }
}

#[test]
fn profile_writes_preserve_output_engineering_polarity_and_state_configuration() {
    let mut analog = AnalogOutputObject::new(1, "AO", 62).unwrap();
    analog.set_min_pres_value(-5.0);
    analog.set_max_pres_value(5.0);
    let mut binary = BinaryOutputObject::new(1, "BO").unwrap();
    binary
        .write_property(
            P::ACTIVE_TEXT,
            None,
            PropertyValue::CharacterString("Run".into()),
            None,
        )
        .unwrap();
    let mut multistate = MultiStateOutputObject::new(1, "MSO", 3).unwrap();
    multistate.set_number_of_states(5).unwrap();
    for (mut output, rows) in [
        (
            Box::new(analog) as Box<dyn ProfileOutput>,
            vec![
                P::MIN_PRES_VALUE,
                P::MAX_PRES_VALUE,
                P::UNITS,
                P::COV_INCREMENT,
            ],
        ),
        (
            Box::new(binary),
            vec![P::POLARITY, P::ACTIVE_TEXT, P::INACTIVE_TEXT],
        ),
        (
            Box::new(multistate),
            vec![P::NUMBER_OF_STATES, P::STATE_TEXT],
        ),
    ] {
        let before: Vec<_> = rows
            .iter()
            .map(|p| output.read_property(*p, None).unwrap())
            .collect();
        output.provision(optional(7)).unwrap();
        tags_leave_state(output.as_mut());
        assert_eq!(
            rows.iter()
                .map(|p| output.read_property(*p, None).unwrap())
                .collect::<Vec<_>>(),
            before
        );
    }
}
