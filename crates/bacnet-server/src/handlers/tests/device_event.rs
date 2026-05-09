use super::*;

#[test]
fn device_communication_control_handler() {
    let comm_state = AtomicU8::new(0);

    let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
        time_duration: Some(60),
        enable_disable: EnableDisable::DISABLE_INITIATION,
        password: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let (state, duration) = handle_device_communication_control(&buf, &comm_state, &None).unwrap();
    assert_eq!(state, EnableDisable::DISABLE_INITIATION);
    assert_eq!(duration, Some(60));
    assert_eq!(comm_state.load(Ordering::Acquire), 2);
}

#[test]
fn device_communication_control_enable() {
    let comm_state = AtomicU8::new(1); // start disabled

    let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
        time_duration: None,
        enable_disable: EnableDisable::ENABLE,
        password: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let (state, duration) = handle_device_communication_control(&buf, &comm_state, &None).unwrap();
    assert_eq!(state, EnableDisable::ENABLE);
    assert_eq!(duration, None);
    assert_eq!(comm_state.load(Ordering::Acquire), 0);
}

#[test]
fn device_communication_control_disable_initiation() {
    let comm_state = AtomicU8::new(0);

    let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
        time_duration: None,
        enable_disable: EnableDisable::DISABLE_INITIATION,
        password: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let (state, duration) = handle_device_communication_control(&buf, &comm_state, &None).unwrap();
    assert_eq!(state, EnableDisable::DISABLE_INITIATION);
    assert_eq!(duration, None);
    assert_eq!(comm_state.load(Ordering::Acquire), 2);
}

#[test]
fn reinitialize_device_handler() {
    let request = bacnet_services::device_mgmt::ReinitializeDeviceRequest {
        reinitialized_state: bacnet_types::enums::ReinitializedState::WARMSTART,
        password: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    handle_reinitialize_device(&buf, &None).unwrap();
}

#[test]
fn get_event_information_empty() {
    let db = make_db_with_ai();
    let request = bacnet_services::alarm_event::GetEventInformationRequest {
        last_received_object_identifier: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_get_event_information(&db, &buf, &mut ack_buf).unwrap();
    let ack_bytes = ack_buf.to_vec();
    assert!(!ack_bytes.is_empty());
}

#[test]
fn get_event_information_reports_non_normal_objects() {
    use bacnet_objects::event::LimitEnable;

    let mut db = ObjectDatabase::new();
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    // Configure intrinsic reporting
    ai.write_property(
        PropertyIdentifier::HIGH_LIMIT,
        None,
        PropertyValue::Real(80.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::LOW_LIMIT,
        None,
        PropertyValue::Real(20.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::DEADBAND,
        None,
        PropertyValue::Real(2.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![LimitEnable::BOTH.to_bits()],
        },
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(5),
        None,
    )
    .unwrap();
    // Push value above high limit and evaluate
    ai.set_present_value(85.0);
    ai.evaluate_intrinsic_reporting(); // → HIGH_LIMIT
    db.add(Box::new(ai)).unwrap();

    let request = bacnet_services::alarm_event::GetEventInformationRequest {
        last_received_object_identifier: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_get_event_information(&db, &buf, &mut ack_buf).unwrap();
    let ack_bytes = ack_buf.to_vec();
    // The ACK should contain one event summary for AI-1
    assert!(ack_bytes.len() > 5); // non-trivial response
}

#[test]
fn get_event_information_reads_event_enable_and_notify_type() {
    use bacnet_objects::event::LimitEnable;
    use bacnet_objects::notification_class::NotificationClass;

    let mut db = ObjectDatabase::new();
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.write_property(
        PropertyIdentifier::HIGH_LIMIT,
        None,
        PropertyValue::Real(80.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::LOW_LIMIT,
        None,
        PropertyValue::Real(20.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::DEADBAND,
        None,
        PropertyValue::Real(2.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![LimitEnable::BOTH.to_bits()],
        },
        None,
    )
    .unwrap();
    // Set EVENT_ENABLE to 0x05 (TO_OFFNORMAL + TO_NORMAL, not TO_FAULT)
    ai.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0x05 << 5],
        },
        None,
    )
    .unwrap();
    // Set NOTIFY_TYPE to 1 (EVENT, not ALARM)
    ai.write_property(
        PropertyIdentifier::NOTIFY_TYPE,
        None,
        PropertyValue::Enumerated(1),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(5),
        None,
    )
    .unwrap();
    // Push above high limit and evaluate to trigger alarm
    ai.set_present_value(85.0);
    ai.evaluate_intrinsic_reporting();
    db.add(Box::new(ai)).unwrap();

    // Add NotificationClass object with custom priorities
    let mut nc = NotificationClass::new(5, "NC-5").unwrap();
    nc.priority = [100, 150, 200];
    db.add(Box::new(nc)).unwrap();

    let request = bacnet_services::alarm_event::GetEventInformationRequest {
        last_received_object_identifier: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_get_event_information(&db, &buf, &mut ack_buf).unwrap();
    let ack_bytes = ack_buf.to_vec();
    assert!(
        ack_bytes.len() > 10,
        "ACK should contain event summary data"
    );

    // Verify the object reads EVENT_ENABLE=0x05 and NOTIFY_TYPE=1 (EVENT)
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let ai_ref = db.get(&ai_oid).unwrap();
    let ev_enable = ai_ref
        .read_property(PropertyIdentifier::EVENT_ENABLE, None)
        .unwrap();
    match ev_enable {
        PropertyValue::BitString { data, .. } => {
            assert_eq!(data[0] >> 5, 0x05, "EVENT_ENABLE should be 0x05");
        }
        other => panic!("expected BitString, got {:?}", other),
    }
    let notify = ai_ref
        .read_property(PropertyIdentifier::NOTIFY_TYPE, None)
        .unwrap();
    assert_eq!(
        notify,
        PropertyValue::Enumerated(1),
        "NOTIFY_TYPE should be EVENT(1)"
    );
}
