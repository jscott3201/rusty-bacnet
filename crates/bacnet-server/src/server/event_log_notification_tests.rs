//! The device's own event notifications in its Event Log objects, on a
//! running server (#1275, Clause 12.27).
//!
//! AV-1 alarms above 80 (deadband 1) and reports each transition through
//! Notification Class 0 to `PEER`, unconfirmed, as process [`PROCESS`]. The
//! logs are read over the wire with ReadRange. The clock is paused: time
//! passes only where a test moves it.
use super::cov_wire_test_support::*;
use super::event_forwarding_tests::destination;
use super::event_recipient_routing_tests::address_recipient;
use super::*;
use bacnet_encoding::constructed::{
    decode_event_log_record, decode_event_notification, encode_event_notification,
};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::notification_forwarder::NotificationForwarderObject;
use bacnet_services::alarm_event::{AcknowledgeAlarmRequest, NotificationParameters};
use bacnet_services::read_range::{ReadRangeAck, ReadRangeRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::bitstring::{EventTransitionBits, LogStatus};
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventLogRecord, BACnetEventParameter,
    BACnetPropertyStates, EventLogDatum,
};
use bacnet_types::enums::{EventState, EventType};
use bacnet_types::primitives::{BACnetTimeStamp, StatusFlags};

/// The process identifier Notification Class 0 names for `PEER`.
const PROCESS: u32 = 7;

fn el(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::EVENT_LOG, instance).unwrap()
}

/// Notification Class 0 sending every transition to `PEER`.
fn to_peer() -> NotificationClass {
    let mut class = NotificationClass::new(0, "NC-0").unwrap();
    class
        .add_destination(destination(address_recipient(0, &PEER), PROCESS, false))
        .unwrap();
    class
}

/// AV-1 alarms above 80 and reports through `class`.
fn alarm(db: &mut ObjectDatabase, class: NotificationClass) {
    db.add(Box::new(class)).unwrap();
    let object = db.get_mut(&av1()).unwrap();
    for (property, value) in [
        (PropertyIdentifier::HIGH_LIMIT, 80.0f32),
        (PropertyIdentifier::LOW_LIMIT, 0.0),
        (PropertyIdentifier::DEADBAND, 1.0),
    ] {
        object
            .write_property(property, None, PropertyValue::Real(value), None)
            .unwrap();
    }
    for (property, unused_bits, bits) in [
        (PropertyIdentifier::LIMIT_ENABLE, 6, 0xC0),
        (PropertyIdentifier::EVENT_ENABLE, 5, 0xE0),
    ] {
        object
            .write_property(
                property,
                None,
                PropertyValue::BitString {
                    unused_bits,
                    data: vec![bits],
                },
                None,
            )
            .unwrap();
    }
}

/// Event Logs `logs`, each with room for `buffer_size` records.
fn logs(db: &mut ObjectDatabase, logs: &[u32], buffer_size: u32) {
    for &instance in logs {
        db.add(Box::new(
            EventLogObject::new(instance, format!("EL-{instance}"), buffer_size).unwrap(),
        ))
        .unwrap();
    }
}

/// AV-1's alarm reporting to `PEER`, and Event Logs `instances`.
fn alarm_and_logs(db: &mut ObjectDatabase, instances: &[u32], buffer_size: u32) {
    alarm(db, to_peer());
    logs(db, instances, buffer_size);
}

/// Take the next event notification sent, in sending order.
async fn sent_notification(h: &Harness) -> EventNotificationRequest {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let next = {
                let mut frames = h.frames.lock().unwrap();
                let at = frames.iter().position(|apdu| {
                    matches!(apdu, Apdu::UnconfirmedRequest(request)
                        if request.service_choice
                            == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION)
                });
                at.map(|at| frames.remove(at))
            };
            match next {
                Some(Apdu::UnconfirmedRequest(request)) => {
                    return decode_event_notification(&request.service_request).unwrap();
                }
                _ => h.settle().await,
            }
        }
    })
    .await
    .expect("an event notification")
}

/// Whether any event notification is waiting in the send log.
fn notification_pending(h: &Harness) -> bool {
    h.frames.lock().unwrap().iter().any(|apdu| {
        matches!(apdu, Apdu::UnconfirmedRequest(request)
            if request.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION)
    })
}

/// Every record of `log`, read over the wire with one ReadRange.
async fn read_log(h: &mut Harness, log: ObjectIdentifier) -> Vec<BACnetEventLogRecord> {
    let mut body = BytesMut::new();
    ReadRangeRequest {
        object_identifier: log,
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::READ_RANGE, body).await;
    let invoke_id = h.invoke_id;
    let service_ack = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let answer = {
                let mut frames = h.frames.lock().unwrap();
                let at = frames.iter().position(|apdu| match apdu {
                    Apdu::ComplexAck(ack) => ack.invoke_id == invoke_id,
                    Apdu::Error(error) => error.invoke_id == invoke_id,
                    _ => false,
                });
                at.map(|at| frames.remove(at))
            };
            match answer {
                Some(Apdu::ComplexAck(ack)) => return ack.service_ack,
                Some(other) => panic!("ReadRange of {log} answered {other:?}"),
                None => h.settle().await,
            }
        }
    })
    .await
    .expect("a ReadRange answer");
    let ack = ReadRangeAck::decode(&service_ack).unwrap();
    let mut records = Vec::new();
    let mut offset = 0;
    while offset < ack.item_data.len() {
        let (record, next) = decode_event_log_record(&ack.item_data, offset).unwrap();
        records.push(record);
        offset = next;
    }
    assert_eq!(records.len(), ack.item_count as usize);
    records
}

/// WriteProperty of a BOOLEAN over the wire, which must succeed.
async fn write_boolean(
    h: &mut Harness,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    value: bool,
) {
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, &PropertyValue::Boolean(value)).unwrap();
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: None,
        property_value: encoded.to_vec(),
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
    response(h).await.unwrap();
}

/// The record a log holds for `sent`, logged at `second`: the notification
/// as recipients get it, with the process identifier left at zero.
fn logged(second: u8, sent: &EventNotificationRequest) -> BACnetEventLogRecord {
    let mut notification = sent.clone();
    notification.process_identifier = 0;
    BACnetEventLogRecord {
        date: at(second).local_date,
        time: time(second),
        log_datum: EventLogDatum::Notification(notification),
    }
}

fn log_status(second: u8, status: LogStatus) -> BACnetEventLogRecord {
    BACnetEventLogRecord {
        date: at(second).local_date,
        time: time(second),
        log_datum: EventLogDatum::LogStatus(status),
    }
}

#[tokio::test(start_paused = true)]
async fn a_local_alarm_and_its_return_to_normal_are_logged_and_read_back() {
    let mut h =
        Harness::start_with(ServerConfig::default(), |db| alarm_and_logs(db, &[1], 8)).await;
    h.set_clock(40);
    h.write_local(90.0).await;
    let alarm = sent_notification(&h).await;
    h.set_clock(41);
    h.write_local(10.0).await;
    let normal = sent_notification(&h).await;

    assert_eq!(
        (alarm.event_object_identifier, alarm.to_state),
        (av1(), EventState::HIGH_LIMIT)
    );
    assert_eq!(normal.to_state, EventState::NORMAL);
    assert_eq!(alarm.process_identifier, PROCESS);
    assert_eq!(
        read_log(&mut h, el(1)).await,
        [logged(40, &alarm), logged(41, &normal)]
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn an_acknowledgment_notification_is_logged() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        let mut class = to_peer();
        class.ack_required = EventTransitionBits::all();
        alarm(db, class);
        logs(db, &[1], 8);
    })
    .await;
    h.set_clock(40);
    h.write_local(90.0).await;
    let alarm = sent_notification(&h).await;
    h.set_clock(45);
    let mut body = BytesMut::new();
    AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 71,
        event_object_identifier: av1(),
        event_state_acknowledged: EventState::HIGH_LIMIT,
        timestamp: alarm.timestamp.clone(),
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(77),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::ACKNOWLEDGE_ALARM, body)
        .await;
    response(&h).await.unwrap();
    let acknowledgment = sent_notification(&h).await;

    assert!(alarm.ack_required);
    assert_eq!(acknowledgment.notify_type, NotifyType::ACK_NOTIFICATION);
    assert_eq!(
        read_log(&mut h, el(1)).await,
        [logged(40, &alarm), logged(45, &acknowledgment)]
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_disabled_log_takes_no_notification() {
    let mut h =
        Harness::start_with(ServerConfig::default(), |db| alarm_and_logs(db, &[1, 2], 8)).await;
    h.set_clock(30);
    write_boolean(&mut h, el(2), PropertyIdentifier::LOG_ENABLE, false).await;
    h.set_clock(40);
    h.write_local(90.0).await;
    let alarm = sent_notification(&h).await;

    assert_eq!(read_log(&mut h, el(1)).await, [logged(40, &alarm)]);
    assert_eq!(
        read_log(&mut h, el(2)).await,
        [log_status(30, LogStatus::LOG_DISABLED)],
        "only the disable itself is recorded"
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn stop_when_full_stops_logging_before_the_buffer_fills() {
    let mut h =
        Harness::start_with(ServerConfig::default(), |db| alarm_and_logs(db, &[1], 3)).await;
    write_boolean(&mut h, el(1), PropertyIdentifier::STOP_WHEN_FULL, true).await;
    let mut sent = Vec::new();
    for (second, value) in [(40, 90.0), (41, 10.0), (42, 90.0), (43, 10.0)] {
        h.set_clock(second);
        h.write_local(value).await;
        sent.push(sent_notification(&h).await);
    }

    // The third would leave no room, so the log stops and records that
    // instead; the fourth finds it disabled. Every transition still went out.
    assert_eq!(
        read_log(&mut h, el(1)).await,
        [
            logged(40, &sent[0]),
            logged(41, &sent[1]),
            log_status(42, LogStatus::LOG_DISABLED),
        ]
    );
    assert_eq!(
        h.server
            .database()
            .read()
            .await
            .get(&el(1))
            .unwrap()
            .read_property(PropertyIdentifier::LOG_ENABLE, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_notification_no_recipient_takes_is_still_logged() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        alarm(db, NotificationClass::new(0, "NC-0").unwrap());
        logs(db, &[1], 8);
    })
    .await;
    h.set_clock(40);
    h.write_local(90.0).await;
    h.settle().await;
    assert!(!notification_pending(&h), "Recipient_List is empty");

    let records = read_log(&mut h, el(1)).await;
    let [record] = &records[..] else {
        panic!("one record expected: {records:?}");
    };
    let EventLogDatum::Notification(notification) = &record.log_datum else {
        panic!("a notification record expected: {record:?}");
    };
    assert_eq!(
        (record.time, notification.process_identifier),
        (time(40), 0)
    );
    assert_eq!(
        (notification.event_object_identifier, notification.to_state),
        (av1(), EventState::HIGH_LIMIT)
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_transition_whose_notification_class_is_missing_is_not_logged() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        alarm_and_logs(db, &[1], 8);
        db.get_mut(&av1())
            .unwrap()
            .write_property(
                PropertyIdentifier::NOTIFICATION_CLASS,
                None,
                PropertyValue::Unsigned(9),
                None,
            )
            .unwrap();
    })
    .await;
    h.set_clock(40);
    h.write_local(90.0).await;
    h.settle().await;
    assert!(!notification_pending(&h), "the lookup fails closed");
    // With Notification Class 0 back, the return to normal goes out.
    h.server
        .database()
        .write()
        .await
        .get_mut(&av1())
        .unwrap()
        .write_property(
            PropertyIdentifier::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(0),
            None,
        )
        .unwrap();
    h.set_clock(41);
    h.write_local(10.0).await;
    let normal = sent_notification(&h).await;

    assert_eq!(normal.to_state, EventState::NORMAL);
    assert_eq!(read_log(&mut h, el(1)).await, [logged(41, &normal)]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_transition_event_enable_suppresses_is_not_logged() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        alarm_and_logs(db, &[1], 8);
        // TO_FAULT and TO_NORMAL only.
        db.get_mut(&av1())
            .unwrap()
            .write_property(
                PropertyIdentifier::EVENT_ENABLE,
                None,
                PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![0x60],
                },
                None,
            )
            .unwrap();
    })
    .await;
    h.set_clock(40);
    h.write_local(90.0).await;
    h.set_clock(41);
    h.write_local(10.0).await;
    let normal = sent_notification(&h).await;

    assert_eq!(normal.to_state, EventState::NORMAL);
    assert_eq!(read_log(&mut h, el(1)).await, [logged(41, &normal)]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_transition_while_dcc_disables_initiation_is_not_logged() {
    let mut h =
        Harness::start_with(ServerConfig::default(), |db| alarm_and_logs(db, &[1], 8)).await;
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    h.set_clock(40);
    h.write_local(90.0).await;
    h.settle().await;
    assert!(!notification_pending(&h));
    h.server.comm_state.set_for_test(DccState::Enable);
    h.set_clock(41);
    h.write_local(10.0).await;
    let normal = sent_notification(&h).await;

    assert_eq!(read_log(&mut h, el(1)).await, [logged(41, &normal)]);
    h.server.stop().await.unwrap();
}

/// A remote device's alarm, as `PEER` sends it to this device.
fn received_alarm() -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 3,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 50).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(9),
        notification_class: 4,
        priority: 100,
        event_type: EventType::OUT_OF_RANGE,
        message_text: None,
        notify_type: NotifyType::ALARM,
        ack_required: false,
        from_state: EventState::NORMAL,
        to_state: EventState::HIGH_LIMIT,
        event_values: Some(NotificationParameters::OutOfRange {
            exceeding_value: 81.0,
            status_flags: StatusFlags::IN_ALARM,
            deadband: 1.0,
            exceeded_limit: 80.0,
        }),
    }
}

/// Deliver an UnconfirmedEventNotification from `PEER`, addressed to this
/// device alone.
async fn receive(h: &Harness, notification: &EventNotificationRequest) {
    let mut service_request = BytesMut::new();
    encode_event_notification(notification, &mut service_request).unwrap();
    let mut payload = BytesMut::new();
    encode_apdu(
        &mut payload,
        &Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION,
            service_request: service_request.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload: payload.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    h.tx.send(ReceivedNpdu {
        direct_response: None,
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(&PEER),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    })
    .await
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_log_without_the_opt_in_takes_no_received_notification() {
    // NF-1 forwards what this device receives to PEER as process 9, so its
    // copy shows the received notification has been handled. EL-1 hasn't
    // opted in to received notifications (#1346), so it holds only its own.
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        alarm_and_logs(db, &[1], 8);
        let mut forwarder = NotificationForwarderObject::new(1, "NF-1").unwrap();
        forwarder
            .add_destination(destination(address_recipient(0, &PEER), 9, false))
            .unwrap();
        db.add(Box::new(forwarder)).unwrap();
    })
    .await;
    h.set_clock(40);
    h.write_local(90.0).await;
    let local = sent_notification(&h).await;
    h.set_clock(41);
    receive(&h, &received_alarm()).await;
    let forwarded = sent_notification(&h).await;
    let mut expected = received_alarm();
    expected.process_identifier = 9;
    assert_eq!(forwarded, expected);

    assert_eq!(read_log(&mut h, el(1)).await, [logged(40, &local)]);
    h.server.stop().await.unwrap();
}

/// Run `passes` one-second ticks of the server's periodic event tasks.
async fn run_passes(h: &Harness, passes: usize) {
    for _ in 0..passes {
        tokio::time::advance(Duration::from_secs(1)).await;
        h.settle().await;
    }
}

/// Event Enrollment `instance`, reporting each count of `log`'s records
/// from 1 to 16: from NORMAL to OFFNORMAL at 1, then OFFNORMAL again at each
/// other count. Were its reports logged where it watches, each would raise
/// the count and prompt the next.
fn count_watcher(instance: u32, log: ObjectIdentifier) -> EventEnrollmentObject {
    let mut enrollment = EventEnrollmentObject::new(
        instance,
        format!("EE-{instance}"),
        EventType::CHANGE_OF_STATE,
    )
    .unwrap();
    enrollment
        .set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            log,
            PropertyIdentifier::TOTAL_RECORD_COUNT.to_raw(),
        )))
        .unwrap();
    enrollment.set_event_parameters(BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: (1..=16).map(BACnetPropertyStates::UnsignedValue).collect(),
    });
    enrollment
}

fn ee(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, instance).unwrap()
}

/// A server running Event Enrollment every second, with AV-1's alarm, EL-1
/// and EL-2, and `watchers` as `(enrollment, watched log)` pairs. Once the
/// enrollments have seen the logs empty, it raises the alarm at second 40
/// and returns the alarm's notification.
async fn alarm_watched_logs(watchers: &[(u32, u32)]) -> (Harness, EventNotificationRequest) {
    let config = ServerConfig {
        event_enrollment_interval_secs: 1,
        ..ServerConfig::default()
    };
    let h = Harness::start_with(config, |db| {
        alarm_and_logs(db, &[1, 2], 32);
        for &(instance, log) in watchers {
            db.add(Box::new(count_watcher(instance, el(log)))).unwrap();
        }
    })
    .await;
    run_passes(&h, 2).await;
    h.set_clock(40);
    h.write_local(90.0).await;
    let alarm = sent_notification(&h).await;
    (h, alarm)
}

#[tokio::test(start_paused = true)]
async fn a_report_on_an_event_log_is_logged_nowhere() {
    let (mut h, alarm) = alarm_watched_logs(&[(1, 1)]).await;
    let report = sent_notification(&h).await;
    run_passes(&h, 5).await;

    assert_eq!(
        (report.event_object_identifier, report.to_state),
        (ee(1), EventState::OFFNORMAL)
    );
    assert_eq!(
        report.event_values,
        Some(NotificationParameters::ChangeOfState {
            new_state: BACnetPropertyStates::UnsignedValue(1),
            status_flags: StatusFlags::empty(),
        })
    );
    assert!(!notification_pending(&h), "EE-1 reported only once");
    for log in [el(1), el(2)] {
        assert_eq!(read_log(&mut h, log).await, [logged(40, &alarm)]);
    }
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn enrollments_watching_each_others_logs_do_not_feed_each_other() {
    // Were EE-1's report logged in EL-2, EE-2 would report the new count,
    // and its report logged in EL-1 would prompt EE-1 again.
    let (mut h, alarm) = alarm_watched_logs(&[(1, 1), (2, 2)]).await;
    let mut reporters = vec![
        sent_notification(&h).await.event_object_identifier,
        sent_notification(&h).await.event_object_identifier,
    ];
    run_passes(&h, 5).await;

    reporters.sort_by_key(|reporter| reporter.instance_number());
    assert_eq!(reporters, [ee(1), ee(2)]);
    assert!(!notification_pending(&h), "each reported only once");
    for log in [el(1), el(2)] {
        assert_eq!(read_log(&mut h, log).await, [logged(40, &alarm)]);
    }
    h.server.stop().await.unwrap();
}

#[path = "received_event_log_wire_tests.rs"]
mod received;
