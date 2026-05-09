use super::*;

fn make_event_req(event_values: Option<NotificationParameters>) -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(7),
        notification_class: 5,
        priority: 100,
        event_type: 5,
        message_text: None,
        notify_type: 0,
        ack_required: true,
        from_state: 0,
        to_state: 3,
        event_values,
    }
}

#[test]
fn notification_params_out_of_range_round_trip() {
    let params = NotificationParameters::OutOfRange {
        exceeding_value: 85.5,
        status_flags: 0b1000, // IN_ALARM
        deadband: 1.0,
        exceeded_limit: 80.0,
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::OutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        } => {
            assert_eq!(exceeding_value, 85.5);
            assert_eq!(status_flags, 0b1000);
            assert_eq!(deadband, 1.0);
            assert_eq!(exceeded_limit, 80.0);
        }
        other => panic!("expected OutOfRange, got {:?}", other),
    }
}

#[test]
fn notification_params_change_of_state_boolean_round_trip() {
    let params = NotificationParameters::ChangeOfState {
        new_state: BACnetPropertyStates::BooleanValue(true),
        status_flags: 0b1100, // IN_ALARM + FAULT
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::ChangeOfState {
            new_state,
            status_flags,
        } => {
            assert_eq!(new_state, BACnetPropertyStates::BooleanValue(true));
            assert_eq!(status_flags, 0b1100);
        }
        other => panic!("expected ChangeOfState, got {:?}", other),
    }
}

#[test]
fn notification_params_change_of_state_enumerated_round_trip() {
    let params = NotificationParameters::ChangeOfState {
        new_state: BACnetPropertyStates::State(3), // HIGH_LIMIT
        status_flags: 0b1000,
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::ChangeOfState {
            new_state,
            status_flags,
        } => {
            assert_eq!(new_state, BACnetPropertyStates::State(3));
            assert_eq!(status_flags, 0b1000);
        }
        other => panic!("expected ChangeOfState, got {:?}", other),
    }
}

#[test]
fn notification_params_change_of_value_real_round_trip() {
    let params = NotificationParameters::ChangeOfValue {
        new_value: ChangeOfValueChoice::ChangedValue(72.5),
        status_flags: 0b0100,
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::ChangeOfValue {
            new_value,
            status_flags,
        } => {
            assert_eq!(new_value, ChangeOfValueChoice::ChangedValue(72.5));
            assert_eq!(status_flags, 0b0100);
        }
        other => panic!("expected ChangeOfValue, got {:?}", other),
    }
}

#[test]
fn notification_params_buffer_ready_round_trip() {
    let buffer_prop = BACnetDeviceObjectPropertyReference::new_local(
        ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap(),
        131, // LOG_BUFFER
    );
    let params = NotificationParameters::BufferReady {
        buffer_property: buffer_prop.clone(),
        previous_notification: 10,
        current_notification: 20,
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::BufferReady {
            buffer_property,
            previous_notification,
            current_notification,
        } => {
            assert_eq!(buffer_property, buffer_prop);
            assert_eq!(previous_notification, 10);
            assert_eq!(current_notification, 20);
        }
        other => panic!("expected BufferReady, got {:?}", other),
    }
}

#[test]
fn notification_params_none_round_trip() {
    let params = NotificationParameters::NoneParams;
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    assert_eq!(
        decoded.event_values,
        Some(NotificationParameters::NoneParams)
    );
}

#[test]
fn notification_params_unsigned_range_round_trip() {
    let params = NotificationParameters::UnsignedRange {
        exceeding_value: 500,
        status_flags: 0b1000,
        exceeded_limit: 400,
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::UnsignedRange {
            exceeding_value,
            status_flags,
            exceeded_limit,
        } => {
            assert_eq!(exceeding_value, 500);
            assert_eq!(status_flags, 0b1000);
            assert_eq!(exceeded_limit, 400);
        }
        other => panic!("expected UnsignedRange, got {:?}", other),
    }
}

#[test]
fn event_notification_no_event_values_backward_compatible() {
    // Verify that event_values=None still round-trips correctly
    let req = make_event_req(None);
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    assert!(decoded.event_values.is_none());
    assert_eq!(decoded.process_identifier, 1);
    assert_eq!(decoded.to_state, 3);
}

#[test]
fn get_event_information_ack_round_trip() {
    let ack = GetEventInformationAck {
        list_of_event_summaries: vec![EventSummary {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            event_state: 3,
            acknowledged_transitions: 0b101,
            event_timestamps: [
                BACnetTimeStamp::SequenceNumber(42),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(100),
            ],
            notify_type: 0,
            event_enable: 0b111,
            event_priorities: [3, 3, 3],
            notification_class: 0,
        }],
        more_events: true,
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    let decoded = GetEventInformationAck::decode(&buf).unwrap();
    assert_eq!(decoded.list_of_event_summaries.len(), 1);
    assert!(decoded.more_events);
    let s = &decoded.list_of_event_summaries[0];
    assert_eq!(
        s.object_identifier,
        ack.list_of_event_summaries[0].object_identifier
    );
    assert_eq!(s.event_state, 3);
    assert_eq!(s.acknowledged_transitions, 0b101);
    assert_eq!(s.event_timestamps[0], BACnetTimeStamp::SequenceNumber(42));
    assert_eq!(s.notify_type, 0);
    assert_eq!(s.event_enable, 0b111);
    assert_eq!(s.event_priorities, [3, 3, 3]);
}

#[test]
fn notification_params_change_of_bitstring_round_trip() {
    let params = NotificationParameters::ChangeOfBitstring {
        referenced_bitstring: (2, vec![0xA0]),
        status_flags: 0b1000,
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::ChangeOfBitstring {
            referenced_bitstring,
            status_flags,
        } => {
            assert_eq!(referenced_bitstring, (2, vec![0xA0]));
            assert_eq!(status_flags, 0b1000);
        }
        other => panic!("expected ChangeOfBitstring, got {:?}", other),
    }
}

#[test]
fn notification_params_command_failure_round_trip() {
    let params = NotificationParameters::CommandFailure {
        command_value: vec![0x91, 0x01],
        status_flags: 0b1100,
        feedback_value: vec![0x91, 0x02],
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::CommandFailure {
            command_value,
            status_flags,
            feedback_value,
        } => {
            assert_eq!(command_value, vec![0x91, 0x01]);
            assert_eq!(status_flags, 0b1100);
            assert_eq!(feedback_value, vec![0x91, 0x02]);
        }
        other => panic!("expected CommandFailure, got {:?}", other),
    }
}

#[test]
fn notification_params_floating_limit_round_trip() {
    let params = NotificationParameters::FloatingLimit {
        reference_value: 50.0,
        status_flags: 0b1000,
        setpoint_value: 45.0,
        error_limit: 2.0,
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::FloatingLimit {
            reference_value,
            status_flags,
            setpoint_value,
            error_limit,
        } => {
            assert_eq!(reference_value, 50.0);
            assert_eq!(status_flags, 0b1000);
            assert_eq!(setpoint_value, 45.0);
            assert_eq!(error_limit, 2.0);
        }
        other => panic!("expected FloatingLimit, got {:?}", other),
    }
}

#[test]
fn notification_params_change_of_life_safety_round_trip() {
    let params = NotificationParameters::ChangeOfLifeSafety {
        new_state: 3,
        new_mode: 1,
        status_flags: 0b1000,
        operation_expected: 2,
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::ChangeOfLifeSafety {
            new_state,
            new_mode,
            status_flags,
            operation_expected,
        } => {
            assert_eq!(new_state, 3);
            assert_eq!(new_mode, 1);
            assert_eq!(status_flags, 0b1000);
            assert_eq!(operation_expected, 2);
        }
        other => panic!("expected ChangeOfLifeSafety, got {:?}", other),
    }
}

#[test]
fn notification_params_extended_round_trip() {
    let params = NotificationParameters::Extended {
        vendor_id: 42,
        extended_event_type: 7,
        parameters: vec![0x01, 0x02, 0x03],
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::Extended {
            vendor_id,
            extended_event_type,
            parameters,
        } => {
            assert_eq!(vendor_id, 42);
            assert_eq!(extended_event_type, 7);
            assert_eq!(parameters, vec![0x01, 0x02, 0x03]);
        }
        other => panic!("expected Extended, got {:?}", other),
    }
}

#[test]
fn notification_params_access_event_round_trip() {
    use bacnet_types::primitives::{Date, Time};

    let cred = BACnetDeviceObjectPropertyReference::new_local(
        ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 1).unwrap(),
        85, // PRESENT_VALUE
    );
    let params = NotificationParameters::AccessEvent {
        access_event: 5,
        status_flags: 0b1000,
        access_event_tag: 10,
        access_event_time: (
            Date {
                year: 124,
                month: 6,
                day: 15,
                day_of_week: 3,
            },
            Time {
                hour: 10,
                minute: 30,
                second: 0,
                hundredths: 0,
            },
        ),
        access_credential: cred.clone(),
        authentication_factor: vec![0xAB, 0xCD],
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::AccessEvent {
            access_event,
            status_flags,
            access_event_tag,
            access_event_time,
            access_credential,
            authentication_factor,
        } => {
            assert_eq!(access_event, 5);
            assert_eq!(status_flags, 0b1000);
            assert_eq!(access_event_tag, 10);
            assert_eq!(access_event_time.0.year, 124);
            assert_eq!(access_event_time.1.hour, 10);
            assert_eq!(access_credential, cred);
            assert_eq!(authentication_factor, vec![0xAB, 0xCD]);
        }
        other => panic!("expected AccessEvent, got {:?}", other),
    }
}

#[test]
fn notification_params_double_out_of_range_round_trip() {
    let params = NotificationParameters::DoubleOutOfRange {
        exceeding_value: 100.5,
        status_flags: 0b1000,
        deadband: 0.5,
        exceeded_limit: 100.0,
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::DoubleOutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        } => {
            assert_eq!(exceeding_value, 100.5);
            assert_eq!(status_flags, 0b1000);
            assert_eq!(deadband, 0.5);
            assert_eq!(exceeded_limit, 100.0);
        }
        other => panic!("expected DoubleOutOfRange, got {:?}", other),
    }
}

#[test]
fn notification_params_signed_out_of_range_round_trip() {
    let params = NotificationParameters::SignedOutOfRange {
        exceeding_value: -10,
        status_flags: 0b1000,
        deadband: 5,
        exceeded_limit: -5,
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::SignedOutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        } => {
            assert_eq!(exceeding_value, -10);
            assert_eq!(status_flags, 0b1000);
            assert_eq!(deadband, 5);
            assert_eq!(exceeded_limit, -5);
        }
        other => panic!("expected SignedOutOfRange, got {:?}", other),
    }
}

#[test]
fn notification_params_unsigned_out_of_range_round_trip() {
    let params = NotificationParameters::UnsignedOutOfRange {
        exceeding_value: 200,
        status_flags: 0b1000,
        deadband: 10,
        exceeded_limit: 190,
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::UnsignedOutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        } => {
            assert_eq!(exceeding_value, 200);
            assert_eq!(status_flags, 0b1000);
            assert_eq!(deadband, 10);
            assert_eq!(exceeded_limit, 190);
        }
        other => panic!("expected UnsignedOutOfRange, got {:?}", other),
    }
}

#[test]
fn notification_params_change_of_characterstring_round_trip() {
    let params = NotificationParameters::ChangeOfCharacterstring {
        changed_value: "hello".to_string(),
        status_flags: 0b1000,
        alarm_value: "alarm".to_string(),
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::ChangeOfCharacterstring {
            changed_value,
            status_flags,
            alarm_value,
        } => {
            assert_eq!(changed_value, "hello");
            assert_eq!(status_flags, 0b1000);
            assert_eq!(alarm_value, "alarm");
        }
        other => panic!("expected ChangeOfCharacterstring, got {:?}", other),
    }
}

#[test]
fn notification_params_change_of_status_flags_round_trip() {
    let params = NotificationParameters::ChangeOfStatusFlags {
        present_value: vec![0x91, 0x03],
        referenced_flags: 0b1010,
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::ChangeOfStatusFlags {
            present_value,
            referenced_flags,
        } => {
            assert_eq!(present_value, vec![0x91, 0x03]);
            assert_eq!(referenced_flags, 0b1010);
        }
        other => panic!("expected ChangeOfStatusFlags, got {:?}", other),
    }
}

#[test]
fn notification_params_change_of_reliability_round_trip() {
    let params = NotificationParameters::ChangeOfReliability {
        reliability: 7,
        status_flags: 0b0100,
        property_values: vec![0x01, 0x02],
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::ChangeOfReliability {
            reliability,
            status_flags,
            property_values,
        } => {
            assert_eq!(reliability, 7);
            assert_eq!(status_flags, 0b0100);
            assert_eq!(property_values, vec![0x01, 0x02]);
        }
        other => panic!("expected ChangeOfReliability, got {:?}", other),
    }
}

#[test]
fn notification_params_change_of_discrete_value_round_trip() {
    let params = NotificationParameters::ChangeOfDiscreteValue {
        new_value: vec![0x91, 0x05],
        status_flags: 0b1000,
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::ChangeOfDiscreteValue {
            new_value,
            status_flags,
        } => {
            assert_eq!(new_value, vec![0x91, 0x05]);
            assert_eq!(status_flags, 0b1000);
        }
        other => panic!("expected ChangeOfDiscreteValue, got {:?}", other),
    }
}

#[test]
fn notification_params_change_of_timer_round_trip() {
    use bacnet_types::primitives::{Date, Time};

    let params = NotificationParameters::ChangeOfTimer {
        new_state: 1,
        status_flags: 0b1000,
        update_time: (
            Date {
                year: 124,
                month: 3,
                day: 10,
                day_of_week: 1,
            },
            Time {
                hour: 8,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
        ),
        last_state_change: 0,
        initial_timeout: 300,
        expiration_time: (
            Date {
                year: 124,
                month: 3,
                day: 10,
                day_of_week: 1,
            },
            Time {
                hour: 8,
                minute: 5,
                second: 0,
                hundredths: 0,
            },
        ),
    };
    let req = make_event_req(Some(params));
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    let ev = decoded.event_values.unwrap();
    match ev {
        NotificationParameters::ChangeOfTimer {
            new_state,
            status_flags,
            update_time,
            last_state_change,
            initial_timeout,
            expiration_time,
        } => {
            assert_eq!(new_state, 1);
            assert_eq!(status_flags, 0b1000);
            assert_eq!(update_time.0.year, 124);
            assert_eq!(update_time.1.hour, 8);
            assert_eq!(last_state_change, 0);
            assert_eq!(initial_timeout, 300);
            assert_eq!(expiration_time.0.year, 124);
            assert_eq!(expiration_time.1.minute, 5);
        }
        other => panic!("expected ChangeOfTimer, got {:?}", other),
    }
}

#[test]
fn get_event_information_ack_empty_list() {
    let ack = GetEventInformationAck {
        list_of_event_summaries: vec![],
        more_events: false,
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    let decoded = GetEventInformationAck::decode(&buf).unwrap();
    assert!(decoded.list_of_event_summaries.is_empty());
    assert!(!decoded.more_events);
}
