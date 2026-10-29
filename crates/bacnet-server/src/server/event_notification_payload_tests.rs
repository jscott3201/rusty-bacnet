use super::*;
use bacnet_encoding::constructed::decode_bacnet_property_value;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use std::borrow::Cow;

struct BuiltInProjectionObject {
    oid: ObjectIdentifier,
    present_value: PropertyValue,
    feedback_value: Option<PropertyValue>,
    reliability: PropertyValue,
    status_flags: PropertyValue,
}

impl BuiltInProjectionObject {
    fn new(
        instance: u32,
        object_type: ObjectType,
        present_value: PropertyValue,
        feedback_value: Option<PropertyValue>,
    ) -> Self {
        Self {
            oid: ObjectIdentifier::new(object_type, instance).unwrap(),
            present_value,
            feedback_value,
            reliability: PropertyValue::Enumerated(2),
            status_flags: PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0xc0],
            },
        }
    }
}

impl BACnetObject for BuiltInProjectionObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "projection-source"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, bacnet_types::error::Error> {
        let object_type = self.oid.object_type();
        match property {
            // An Access Zone watches Occupancy_State and serves no
            // Present_Value, and an Access Door watches Door_Alarm_State; the
            // fixture's value stands for the watched one. The door's own
            // Present_Value reads LOCK.
            p if p == PropertyIdentifier::OCCUPANCY_STATE
                && object_type == ObjectType::ACCESS_ZONE =>
            {
                Ok(self.present_value.clone())
            }
            p if p == PropertyIdentifier::DOOR_ALARM_STATE
                && object_type == ObjectType::ACCESS_DOOR =>
            {
                Ok(self.present_value.clone())
            }
            p if p == PropertyIdentifier::PRESENT_VALUE
                && object_type == ObjectType::ACCESS_DOOR =>
            {
                Ok(PropertyValue::Enumerated(0))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE
                && object_type != ObjectType::ACCESS_ZONE =>
            {
                Ok(self.present_value.clone())
            }
            p if p == PropertyIdentifier::FEEDBACK_VALUE => {
                self.feedback_value
                    .clone()
                    .ok_or(bacnet_types::error::Error::protocol(
                        ErrorClass::PROPERTY,
                        ErrorCode::UNKNOWN_PROPERTY,
                    ))
            }
            p if p == PropertyIdentifier::RELIABILITY => Ok(self.reliability.clone()),
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(self.status_flags.clone()),
            p if p == PropertyIdentifier::HIGH_LIMIT => Ok(PropertyValue::Real(80.0)),
            p if p == PropertyIdentifier::LOW_LIMIT => Ok(PropertyValue::Real(20.0)),
            p if p == PropertyIdentifier::DEADBAND => Ok(PropertyValue::Real(2.0)),
            _ => Err(bacnet_types::error::Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), bacnet_types::error::Error> {
        Err(bacnet_types::error::Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
}

fn normal_payload(object: &BuiltInProjectionObject) -> NotificationParameters {
    let (event_type, to) = match object.oid.object_type() {
        ObjectType::ANALOG_INPUT | ObjectType::ANALOG_OUTPUT | ObjectType::ANALOG_VALUE => {
            (EventType::OUT_OF_RANGE, EventState::HIGH_LIMIT)
        }
        ObjectType::BINARY_INPUT
        | ObjectType::BINARY_VALUE
        | ObjectType::MULTI_STATE_INPUT
        | ObjectType::MULTI_STATE_VALUE
        | ObjectType::ACCESS_ZONE
        | ObjectType::ACCESS_DOOR => (EventType::CHANGE_OF_STATE, EventState::OFFNORMAL),
        ObjectType::BINARY_OUTPUT | ObjectType::MULTI_STATE_OUTPUT => {
            (EventType::COMMAND_FAILURE, EventState::OFFNORMAL)
        }
        other => panic!("unexpected built-in type {other:?}"),
    };
    project_intrinsic_payload(
        object,
        &EventStateChange {
            from: EventState::NORMAL,
            to,
        },
        event_type,
        None,
    )
    .unwrap()
    .0
}

fn all_eleven_sources() -> Vec<(BuiltInProjectionObject, NotificationParameters)> {
    vec![
        (
            BuiltInProjectionObject::new(
                1,
                ObjectType::ANALOG_INPUT,
                PropertyValue::Real(85.0),
                None,
            ),
            NotificationParameters::OutOfRange {
                exceeding_value: 85.0,
                status_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
                deadband: 2.0,
                exceeded_limit: 80.0,
            },
        ),
        (
            BuiltInProjectionObject::new(
                2,
                ObjectType::ANALOG_OUTPUT,
                PropertyValue::Real(85.0),
                None,
            ),
            NotificationParameters::OutOfRange {
                exceeding_value: 85.0,
                status_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
                deadband: 2.0,
                exceeded_limit: 80.0,
            },
        ),
        (
            BuiltInProjectionObject::new(
                3,
                ObjectType::ANALOG_VALUE,
                PropertyValue::Real(85.0),
                None,
            ),
            NotificationParameters::OutOfRange {
                exceeding_value: 85.0,
                status_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
                deadband: 2.0,
                exceeded_limit: 80.0,
            },
        ),
        (
            BuiltInProjectionObject::new(
                4,
                ObjectType::BINARY_INPUT,
                PropertyValue::Enumerated(1),
                None,
            ),
            NotificationParameters::ChangeOfState {
                new_state: BACnetPropertyStates::BinaryValue(1),
                status_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
            },
        ),
        (
            BuiltInProjectionObject::new(
                5,
                ObjectType::BINARY_VALUE,
                PropertyValue::Enumerated(1),
                None,
            ),
            NotificationParameters::ChangeOfState {
                new_state: BACnetPropertyStates::BinaryValue(1),
                status_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
            },
        ),
        (
            BuiltInProjectionObject::new(
                6,
                ObjectType::MULTI_STATE_INPUT,
                PropertyValue::Unsigned(3),
                None,
            ),
            NotificationParameters::ChangeOfState {
                new_state: BACnetPropertyStates::UnsignedValue(3),
                status_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
            },
        ),
        (
            BuiltInProjectionObject::new(
                7,
                ObjectType::MULTI_STATE_VALUE,
                PropertyValue::Unsigned(3),
                None,
            ),
            NotificationParameters::ChangeOfState {
                new_state: BACnetPropertyStates::UnsignedValue(3),
                status_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
            },
        ),
        (
            BuiltInProjectionObject::new(
                8,
                ObjectType::BINARY_OUTPUT,
                PropertyValue::Enumerated(1),
                Some(PropertyValue::Enumerated(0)),
            ),
            NotificationParameters::CommandFailure {
                command_value: vec![0x91, 0x01],
                status_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
                feedback_value: vec![0x91, 0x00],
            },
        ),
        (
            BuiltInProjectionObject::new(
                9,
                ObjectType::MULTI_STATE_OUTPUT,
                PropertyValue::Unsigned(3),
                Some(PropertyValue::Unsigned(2)),
            ),
            NotificationParameters::CommandFailure {
                command_value: vec![0x21, 0x03],
                status_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
                feedback_value: vec![0x21, 0x02],
            },
        ),
        (
            // Occupancy_State ABOVE_UPPER_LIMIT (4), as zone-occupancy-state.
            BuiltInProjectionObject::new(
                10,
                ObjectType::ACCESS_ZONE,
                PropertyValue::Enumerated(4),
                None,
            ),
            NotificationParameters::ChangeOfState {
                new_state: BACnetPropertyStates::ZoneOccupancyState(4),
                status_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
            },
        ),
        (
            // Door_Alarm_State FORCED_OPEN (3), as door-alarm-state.
            BuiltInProjectionObject::new(
                11,
                ObjectType::ACCESS_DOOR,
                PropertyValue::Enumerated(3),
                None,
            ),
            NotificationParameters::ChangeOfState {
                new_state: BACnetPropertyStates::DoorAlarmState(3),
                status_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
            },
        ),
    ]
}

#[test]
fn all_eleven_builtin_normal_families_project_exact_typed_values() {
    for (source, expected) in all_eleven_sources() {
        assert_eq!(normal_payload(&source), expected, "source {}", source.oid);
    }
}

#[test]
fn builtin_fault_projection_is_tag_19_with_explicit_property_order() {
    for (source, _) in all_eleven_sources() {
        let payload = project_intrinsic_payload(
            &source,
            &EventStateChange {
                from: EventState::NORMAL,
                to: EventState::FAULT,
            },
            EventType::CHANGE_OF_RELIABILITY,
            None,
        )
        .unwrap()
        .0;
        let NotificationParameters::ChangeOfReliability {
            reliability,
            status_flags,
            property_values,
        } = payload
        else {
            panic!("{} did not project CHANGE_OF_RELIABILITY", source.oid);
        };
        assert_eq!(reliability, Reliability::OVER_RANGE);
        assert_eq!(status_flags, StatusFlags::IN_ALARM | StatusFlags::FAULT);

        let mut decoded = Vec::new();
        let mut offset = 0;
        while offset < property_values.len() {
            let (entry, next) = decode_bacnet_property_value(&property_values, offset).unwrap();
            assert!(next > offset);
            decoded.push(entry);
            offset = next;
        }
        // Table 13-5: the zone reports Occupancy_State in place of
        // Present_Value, and the door Door_Alarm_State before it.
        let expected_properties = match source.oid.object_type() {
            ObjectType::BINARY_OUTPUT | ObjectType::MULTI_STATE_OUTPUT => vec![
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::FEEDBACK_VALUE,
            ],
            ObjectType::ACCESS_ZONE => vec![PropertyIdentifier::OCCUPANCY_STATE],
            ObjectType::ACCESS_DOOR => vec![
                PropertyIdentifier::DOOR_ALARM_STATE,
                PropertyIdentifier::PRESENT_VALUE,
            ],
            _ => vec![PropertyIdentifier::PRESENT_VALUE],
        };
        assert_eq!(
            decoded
                .iter()
                .map(|entry| entry.property_identifier)
                .collect::<Vec<_>>(),
            expected_properties,
            "{} property order",
            source.oid
        );
        assert_eq!(
            decoded[0].value,
            encode_abstract_value(&source.present_value).unwrap()
        );
        if let Some(feedback) = &source.feedback_value {
            assert_eq!(decoded[1].value, encode_abstract_value(feedback).unwrap());
        }
        if source.oid.object_type() == ObjectType::ACCESS_DOOR {
            // The door's Present_Value, LOCK.
            assert_eq!(decoded[1].value, [0x91, 0x00]);
        }
    }
}

#[test]
fn fault_recovery_requires_effective_reliability_type_and_none_is_not_projected() {
    let source =
        BuiltInProjectionObject::new(1, ObjectType::ANALOG_INPUT, PropertyValue::Real(50.0), None);
    let recovery = EventStateChange {
        from: EventState::FAULT,
        to: EventState::NORMAL,
    };
    assert!(
        project_intrinsic_payload(&source, &recovery, EventType::CHANGE_OF_RELIABILITY, None)
            .is_some()
    );
    assert!(project_intrinsic_payload(&source, &recovery, EventType::OUT_OF_RANGE, None).is_none());
    assert!(
        project_intrinsic_payload(
            &source,
            &EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            EventType::NONE,
            None,
        )
        .is_none()
    );
}

#[test]
fn malformed_required_builtin_data_fails_closed() {
    let mut source =
        BuiltInProjectionObject::new(1, ObjectType::ANALOG_INPUT, PropertyValue::Real(85.0), None);
    source.status_flags = PropertyValue::Unsigned(0);
    assert!(
        project_intrinsic_payload(
            &source,
            &EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            EventType::OUT_OF_RANGE,
            None,
        )
        .is_none()
    );
}

#[test]
fn limit_selection_covers_entries_crossings_and_normal_recovery() {
    let low = 20.0;
    let high = 80.0;
    for (from, to, expected) in [
        (EventState::NORMAL, EventState::LOW_LIMIT, low),
        (EventState::HIGH_LIMIT, EventState::LOW_LIMIT, low),
        (EventState::LOW_LIMIT, EventState::NORMAL, low),
        (EventState::NORMAL, EventState::HIGH_LIMIT, high),
        (EventState::LOW_LIMIT, EventState::HIGH_LIMIT, high),
        (EventState::HIGH_LIMIT, EventState::NORMAL, high),
    ] {
        assert_eq!(
            selected_limit(&EventStateChange { from, to }, low, high),
            Some(expected)
        );
    }
}
