use super::*;

#[test]
fn acknowledge_alarm_round_trip() {
    let req = AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 1,
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        event_state_acknowledged: 3, // high-limit
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(0),
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = AcknowledgeAlarmRequest::decode(&buf).unwrap();
    assert_eq!(decoded.acknowledging_process_identifier, 1);
    assert_eq!(decoded.event_object_identifier, req.event_object_identifier);
    assert_eq!(decoded.event_state_acknowledged, 3);
    assert_eq!(decoded.timestamp, BACnetTimeStamp::SequenceNumber(42));
    assert_eq!(decoded.acknowledgment_source, "operator");
}

#[test]
fn get_event_info_empty_request() {
    let req = GetEventInformationRequest {
        last_received_object_identifier: None,
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);
    let decoded = GetEventInformationRequest::decode(&buf).unwrap();
    assert!(decoded.last_received_object_identifier.is_none());
}

#[test]
fn get_event_info_with_last_received() {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 5).unwrap();
    let req = GetEventInformationRequest {
        last_received_object_identifier: Some(oid),
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);
    let decoded = GetEventInformationRequest::decode(&buf).unwrap();
    assert_eq!(decoded.last_received_object_identifier, Some(oid));
}

#[test]
fn event_notification_round_trip() {
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();

    let req = EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: device_oid,
        event_object_identifier: ai_oid,
        timestamp: BACnetTimeStamp::SequenceNumber(7),
        notification_class: 5,
        priority: 100,
        event_type: 5, // OUT_OF_RANGE
        message_text: None,
        notify_type: 0, // ALARM
        ack_required: true,
        from_state: 0, // NORMAL
        to_state: 3,   // HIGH_LIMIT
        event_values: None,
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();

    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    assert_eq!(decoded.process_identifier, 1);
    assert_eq!(decoded.initiating_device_identifier, device_oid);
    assert_eq!(decoded.event_object_identifier, ai_oid);
    assert_eq!(decoded.timestamp, BACnetTimeStamp::SequenceNumber(7));
    assert_eq!(decoded.notification_class, 5);
    assert_eq!(decoded.priority, 100);
    assert_eq!(decoded.event_type, 5);
    assert_eq!(decoded.notify_type, 0);
    assert!(decoded.ack_required);
    assert_eq!(decoded.from_state, 0);
    assert_eq!(decoded.to_state, 3);
    assert!(decoded.event_values.is_none());
}

#[test]
fn event_notification_datetime_timestamp_round_trip() {
    use bacnet_types::primitives::{Date, Time};

    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();

    let ts = BACnetTimeStamp::DateTime {
        date: Date {
            year: 126,
            month: 2,
            day: 28,
            day_of_week: 6,
        },
        time: Time {
            hour: 14,
            minute: 30,
            second: 0,
            hundredths: 0,
        },
    };

    let req = EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: device_oid,
        event_object_identifier: ai_oid,
        timestamp: ts.clone(),
        notification_class: 5,
        priority: 100,
        event_type: 5,
        message_text: None,
        notify_type: 0,
        ack_required: true,
        from_state: 0,
        to_state: 3,
        event_values: None,
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();

    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    assert_eq!(decoded.timestamp, ts);
}

#[test]
fn event_notification_time_timestamp_round_trip() {
    use bacnet_types::primitives::Time;

    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();

    let ts = BACnetTimeStamp::Time(Time {
        hour: 10,
        minute: 15,
        second: 30,
        hundredths: 50,
    });

    let req = EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: device_oid,
        event_object_identifier: ai_oid,
        timestamp: ts.clone(),
        notification_class: 5,
        priority: 100,
        event_type: 5,
        message_text: None,
        notify_type: 0,
        ack_required: true,
        from_state: 0,
        to_state: 3,
        event_values: None,
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();

    let decoded = EventNotificationRequest::decode(&buf).unwrap();
    assert_eq!(decoded.timestamp, ts);
}

// -----------------------------------------------------------------------
// Malformed-input decode error tests
// -----------------------------------------------------------------------

#[test]
fn test_decode_acknowledge_alarm_empty_input() {
    assert!(AcknowledgeAlarmRequest::decode(&[]).is_err());
}

#[test]
fn test_decode_acknowledge_alarm_truncated_1_byte() {
    let req = AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 1,
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        event_state_acknowledged: 3,
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(0),
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    assert!(AcknowledgeAlarmRequest::decode(&buf[..1]).is_err());
}

#[test]
fn test_decode_acknowledge_alarm_truncated_3_bytes() {
    let req = AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 1,
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        event_state_acknowledged: 3,
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(0),
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    assert!(AcknowledgeAlarmRequest::decode(&buf[..3]).is_err());
}

#[test]
fn test_decode_acknowledge_alarm_truncated_half() {
    let req = AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 1,
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        event_state_acknowledged: 3,
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(0),
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let half = buf.len() / 2;
    assert!(AcknowledgeAlarmRequest::decode(&buf[..half]).is_err());
}

#[test]
fn test_decode_acknowledge_alarm_invalid_tag() {
    assert!(AcknowledgeAlarmRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
}

#[test]
fn test_decode_event_notification_empty_input() {
    assert!(EventNotificationRequest::decode(&[]).is_err());
}

#[test]
fn test_decode_event_notification_truncated_1_byte() {
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
    let req = EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: device_oid,
        event_object_identifier: ai_oid,
        timestamp: BACnetTimeStamp::SequenceNumber(7),
        notification_class: 5,
        priority: 100,
        event_type: 5,
        message_text: None,
        notify_type: 0,
        ack_required: true,
        from_state: 0,
        to_state: 3,
        event_values: None,
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    assert!(EventNotificationRequest::decode(&buf[..1]).is_err());
}

#[test]
fn test_decode_event_notification_truncated_3_bytes() {
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
    let req = EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: device_oid,
        event_object_identifier: ai_oid,
        timestamp: BACnetTimeStamp::SequenceNumber(7),
        notification_class: 5,
        priority: 100,
        event_type: 5,
        message_text: None,
        notify_type: 0,
        ack_required: true,
        from_state: 0,
        to_state: 3,
        event_values: None,
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    assert!(EventNotificationRequest::decode(&buf[..3]).is_err());
}

#[test]
fn test_decode_event_notification_truncated_half() {
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
    let req = EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: device_oid,
        event_object_identifier: ai_oid,
        timestamp: BACnetTimeStamp::SequenceNumber(7),
        notification_class: 5,
        priority: 100,
        event_type: 5,
        message_text: None,
        notify_type: 0,
        ack_required: true,
        from_state: 0,
        to_state: 3,
        event_values: None,
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let half = buf.len() / 2;
    assert!(EventNotificationRequest::decode(&buf[..half]).is_err());
}

#[test]
fn test_decode_event_notification_invalid_tag() {
    assert!(EventNotificationRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
}

#[test]
fn test_decode_get_event_info_invalid_tag() {
    // Non-matching context tag — decoder treats as no last_received (lenient)
    let result = GetEventInformationRequest::decode(&[0xFF, 0xFF]).unwrap();
    assert!(result.last_received_object_identifier.is_none());
}

#[test]
fn test_decode_get_event_info_truncated() {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 5).unwrap();
    let req = GetEventInformationRequest {
        last_received_object_identifier: Some(oid),
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);
    assert!(GetEventInformationRequest::decode(&buf[..1]).is_err());
}

// -----------------------------------------------------------------------
// NotificationParameters round-trip tests
// -----------------------------------------------------------------------
