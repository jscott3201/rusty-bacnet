//! End-to-end Event Enrollment notification lifecycle regressions.

use super::*;
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
    ChangeOfValueCriteria, FaultParameters,
};
use bacnet_types::enums::{EventState, EventType, Reliability};
use bacnet_types::primitives::BACnetTimeStamp;
use std::sync::{Arc as StdArc, Mutex as StdMutex};

#[path = "event_enrollment_notification_test_support.rs"]
mod support;
use support::*;

#[tokio::test(start_paused = true)]
async fn every_evaluated_normal_algorithm_uses_committed_history_once_on_wire() {
    let mut db = ObjectDatabase::new();

    let oor_target = ObservedObject::new(1, PropertyValue::Real(85.0));
    let oor_target_oid = oor_target.object_identifier();
    db.add(Box::new(oor_target)).unwrap();
    db.add(Box::new(enrollment(
        1,
        EventType::OUT_OF_RANGE,
        Some(oor_target_oid),
        out_of_range_parameters(0),
    )))
    .unwrap();

    let floating_target = ObservedObject::new(2, PropertyValue::Real(65.0));
    let floating_target_oid = floating_target.object_identifier();
    db.add(Box::new(floating_target)).unwrap();
    let setpoint = ObservedObject::new(3, PropertyValue::Real(50.0));
    let setpoint_oid = setpoint.object_identifier();
    db.add(Box::new(setpoint)).unwrap();
    db.add(Box::new(enrollment(
        2,
        EventType::FLOATING_LIMIT,
        Some(floating_target_oid),
        BACnetEventParameter::FloatingLimit {
            time_delay: 0,
            setpoint_reference: BACnetDeviceObjectPropertyReference::new_local(
                setpoint_oid,
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            ),
            low_diff_limit: 10.0,
            high_diff_limit: 10.0,
            deadband: 2.0,
        },
    )))
    .unwrap();

    let state_target = ObservedObject::new(4, PropertyValue::Enumerated(1));
    let state_target_oid = state_target.object_identifier();
    db.add(Box::new(state_target)).unwrap();
    db.add(Box::new(enrollment(
        3,
        EventType::CHANGE_OF_STATE,
        Some(state_target_oid),
        BACnetEventParameter::ChangeOfState {
            time_delay: 0,
            list_of_values: vec![BACnetPropertyStates::BinaryValue(1)],
        },
    )))
    .unwrap();

    let bits_target = ObservedObject::new(
        5,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xE0],
        },
    );
    let bits_target_oid = bits_target.object_identifier();
    db.add(Box::new(bits_target)).unwrap();
    db.add(Box::new(enrollment(
        4,
        EventType::CHANGE_OF_BITSTRING,
        Some(bits_target_oid),
        BACnetEventParameter::ChangeOfBitstring {
            time_delay: 0,
            bitmask: (5, vec![0xE0]),
            list_of_values: vec![(5, vec![0xE0])],
        },
    )))
    .unwrap();

    let cov_target = ObservedObject::new(6, PropertyValue::Real(3.0));
    let cov_target_oid = cov_target.object_identifier();
    db.add(Box::new(cov_target)).unwrap();
    db.add(Box::new(enrollment(
        5,
        EventType::CHANGE_OF_VALUE,
        Some(cov_target_oid),
        BACnetEventParameter::ChangeOfValue {
            time_delay: 0,
            criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
        },
    )))
    .unwrap();

    let event_log_oid = ObjectIdentifier::new(ObjectType::EVENT_LOG, 1).unwrap();
    db.add(Box::new(EventLogObject::new(1, "Event Log", 16).unwrap()))
        .unwrap();

    let (mut server, sent) = start_server(db, true).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let first = drain_notifications(&sent);
    assert_eq!(first.len(), 4, "four immediate algorithms must deliver");
    for (index, (notification, expected_type)) in first
        .iter()
        .zip([
            EventType::OUT_OF_RANGE,
            EventType::FLOATING_LIMIT,
            EventType::CHANGE_OF_STATE,
            EventType::CHANGE_OF_BITSTRING,
        ])
        .enumerate()
    {
        assert_eq!(notification.event_type, expected_type.to_raw());
        assert_eq!(
            notification.timestamp,
            BACnetTimeStamp::SequenceNumber(index as u16)
        );
        assert!(notification.ack_required);
        assert_eq!(notification.message_text, None);
        assert_eq!(notification.event_values, None);
        let db = server.database().read().await;
        assert_eq!(
            notification.timestamp,
            history_timestamp(
                &db,
                notification.event_object_identifier,
                EventState::from_raw(notification.to_state),
            ),
            "wire time must be the committed transition coordinate"
        );
    }

    server
        .database()
        .write()
        .await
        .get_mut(&cov_target_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(8.0),
            None,
        )
        .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let cov = drain_notifications(&sent);
    assert_eq!(cov.len(), 1, "CHANGE_OF_VALUE threshold crossing delivers");
    assert_eq!(cov[0].event_type, EventType::CHANGE_OF_VALUE.to_raw());
    assert_eq!(cov[0].from_state, EventState::NORMAL.to_raw());
    assert_eq!(cov[0].to_state, EventState::NORMAL.to_raw());
    assert_eq!(cov[0].timestamp, BACnetTimeStamp::SequenceNumber(4));

    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        drain_notifications(&sent).is_empty(),
        "the next no-transition pass must not duplicate a token or send"
    );
    let db = server.database().write().await;
    assert_eq!(db.reserve_event_sequence_number().number(), 5);
    assert_eq!(
        db.get(&event_log_oid)
            .unwrap()
            .read_property(PropertyIdentifier::RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(0),
        "this slice deliberately has no Event Log side effect"
    );
    drop(db);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn event_enrollment_ack_policy_is_the_commit_time_snapshot() {
    let mut db = ObjectDatabase::new();
    let target = ObservedObject::new(7, PropertyValue::Real(85.0));
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();
    db.add(Box::new(enrollment(
        7,
        EventType::OUT_OF_RANGE,
        Some(target_oid),
        out_of_range_parameters(0),
    )))
    .unwrap();
    add_server_context(&mut db, true);
    let enrollment_oid = ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 7).unwrap();

    let db = Arc::new(RwLock::new(db));
    let transition = {
        let mut guard = db.write().await;
        let mut evaluation =
            crate::event_enrollment::evaluate_event_enrollments_for_delivery(&mut guard, 1);
        assert_eq!(evaluation.deliveries.len(), 1);
        let committed = evaluation.deliveries.pop().unwrap();
        crate::server::event_notifications::resolve_committed_event_enrollment_transition(
            &guard, committed,
        )
        .unwrap()
        .2
    };

    {
        let mut replacement = NotificationClass::new(0, "NC-replaced").unwrap();
        replacement.ack_required = [false; 3];
        replacement.add_destination(
            crate::server::event_notifications_tests::local_broadcast_destination(),
        );
        db.write().await.add(Box::new(replacement)).unwrap();
    }

    let sent = StdArc::new(StdMutex::new(Vec::new()));
    BACnetServer::<RecordingTransport>::build_and_send_event_notification(
        &db,
        &Arc::new(NetworkLayer::new(RecordingTransport {
            sent: StdArc::clone(&sent),
            lock_probe: StdArc::default(),
        })),
        &Arc::new(AtomicU8::new(0)),
        &Arc::new(Mutex::new(ServerTsm::new())),
        &NotificationTransactions::new(),
        &enrollment_oid,
        transition,
        1000,
    )
    .await;

    let notifications = drain_notifications(&sent);
    assert_eq!(notifications.len(), 1);
    assert!(
        notifications[0].ack_required,
        "send-time Notification Class edits must not replace committed ACK policy"
    );
    assert_eq!(
        db.write().await.reserve_event_sequence_number().number(),
        1,
        "projection and send must not reserve another timestamp"
    );
}

#[tokio::test(start_paused = true)]
async fn reliability_producers_and_fault_cycle_deliver_change_of_reliability() {
    let mut db = ObjectDatabase::new();

    let configuration_target = ObservedObject::new(10, PropertyValue::Real(50.0))
        .with_reliability_value(PropertyValue::Real(0.0));
    let configuration_target_oid = configuration_target.object_identifier();
    db.add(Box::new(configuration_target)).unwrap();
    db.add(Box::new(enrollment(
        10,
        EventType::NONE,
        Some(configuration_target_oid),
        out_of_range_parameters(0),
    )))
    .unwrap();

    let monitored = ObservedObject::new(11, PropertyValue::Real(50.0))
        .with_reliability(Reliability::OVER_RANGE);
    let monitored_oid = monitored.object_identifier();
    db.add(Box::new(monitored)).unwrap();
    db.add(Box::new(enrollment(
        11,
        EventType::NONE,
        Some(monitored_oid),
        out_of_range_parameters(0),
    )))
    .unwrap();

    let status_source = ObservedObject::new(12, PropertyValue::Real(50.0));
    let status_source_oid = status_source.object_identifier();
    db.add(Box::new(status_source)).unwrap();
    let status_member = ObservedObject::new(13, PropertyValue::Null).with_status_flags(0x40);
    let status_member_oid = status_member.object_identifier();
    db.add(Box::new(status_member)).unwrap();
    let mut status_enrollment = enrollment(
        12,
        EventType::NONE,
        Some(status_source_oid),
        out_of_range_parameters(0),
    );
    status_enrollment.set_fault_parameters(Some(FaultParameters::FaultStatusFlags {
        reference: BACnetDeviceObjectPropertyReference::new_local(
            status_member_oid,
            PropertyIdentifier::STATUS_FLAGS.to_raw(),
        ),
    }));
    db.add(Box::new(status_enrollment)).unwrap();

    let fault_target = ObservedObject::new(14, PropertyValue::Real(-1.0));
    let fault_target_oid = fault_target.object_identifier();
    db.add(Box::new(fault_target)).unwrap();
    let mut fault_enrollment = enrollment(
        13,
        EventType::NONE,
        Some(fault_target_oid),
        BACnetEventParameter::OutOfRange {
            time_delay: 0,
            low_limit: -100.0,
            high_limit: 100.0,
            deadband: 2.0,
        },
    );
    fault_enrollment.set_fault_parameters(Some(FaultParameters::FaultOutOfRange {
        min_normal: 0.0,
        max_normal: 10.0,
    }));
    db.add(Box::new(fault_enrollment)).unwrap();

    let (mut server, sent) = start_server(db, true).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let entries = drain_notifications(&sent);
    {
        let db = server.database().read().await;
        assert_committed_reliability_notifications(
            &db,
            &entries,
            &[10, 11, 12, 13],
            EventState::NORMAL,
            EventState::FAULT,
            0,
        );
    }

    {
        let mut db = server.database().write().await;
        db.get_mut(&monitored_oid)
            .unwrap()
            .write_property(
                PropertyIdentifier::RELIABILITY,
                None,
                PropertyValue::Enumerated(Reliability::UNDER_RANGE.to_raw()),
                None,
            )
            .unwrap();
        db.get_mut(&fault_target_oid)
            .unwrap()
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(11.0),
                None,
            )
            .unwrap();
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let reentry = drain_notifications(&sent);
    {
        let db = server.database().read().await;
        assert_committed_reliability_notifications(
            &db,
            &reentry,
            &[13],
            EventState::FAULT,
            EventState::FAULT,
            4,
        );
    }

    {
        let mut db = server.database().write().await;
        db.get_mut(&configuration_target_oid)
            .unwrap()
            .write_property(
                PropertyIdentifier::RELIABILITY,
                None,
                PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
                None,
            )
            .unwrap();
        db.get_mut(&monitored_oid)
            .unwrap()
            .write_property(
                PropertyIdentifier::RELIABILITY,
                None,
                PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
                None,
            )
            .unwrap();
        db.get_mut(&status_member_oid)
            .unwrap()
            .write_property(
                PropertyIdentifier::STATUS_FLAGS,
                None,
                PropertyValue::BitString {
                    unused_bits: 4,
                    data: vec![0],
                },
                None,
            )
            .unwrap();
        db.get_mut(&fault_target_oid)
            .unwrap()
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(5.0),
                None,
            )
            .unwrap();
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let recovery = drain_notifications(&sent);
    {
        let db = server.database().read().await;
        assert_committed_reliability_notifications(
            &db,
            &recovery,
            &[10, 11, 12, 13],
            EventState::FAULT,
            EventState::NORMAL,
            5,
        );
    }

    tokio::time::sleep(Duration::from_secs(1)).await;
    let repeated = drain_notifications(&sent);
    assert!(repeated.is_empty(), "unexpected repeat: {repeated:?}");
    assert_eq!(
        server
            .database()
            .write()
            .await
            .reserve_event_sequence_number()
            .number(),
        9,
        "delivery must not allocate a second timestamp or repeat on the next pass"
    );
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn mixed_normal_and_reliability_commits_preserve_enrollment_order() {
    let mut db = ObjectDatabase::new();
    for instance in [20, 22] {
        let target = ObservedObject::new(instance, PropertyValue::Real(85.0));
        let target_oid = target.object_identifier();
        db.add(Box::new(target)).unwrap();
        db.add(Box::new(enrollment(
            instance,
            EventType::OUT_OF_RANGE,
            Some(target_oid),
            out_of_range_parameters(0),
        )))
        .unwrap();
        if instance == 20 {
            db.add(Box::new(enrollment(
                21,
                EventType::OUT_OF_RANGE,
                None,
                out_of_range_parameters(0),
            )))
            .unwrap();
        }
    }

    let (mut server, sent) = start_server(db, true).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let object_order: Vec<_> = drain_notifications(&sent)
        .into_iter()
        .map(|notification| notification.event_object_identifier.instance_number())
        .collect();
    assert_eq!(object_order, vec![20, 21, 22]);
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn local_suppression_paths_commit_only_the_applicable_transitions() {
    let mut db = ObjectDatabase::new();

    let suppressed_target = ObservedObject::new(30, PropertyValue::Real(85.0));
    let suppressed_target_oid = suppressed_target.object_identifier();
    db.add(Box::new(suppressed_target)).unwrap();
    let mut event_enable_suppressed = enrollment(
        30,
        EventType::OUT_OF_RANGE,
        Some(suppressed_target_oid),
        out_of_range_parameters(0),
    );
    event_enable_suppressed.set_event_enable(0);
    let event_enable_oid = event_enable_suppressed.object_identifier();
    db.add(Box::new(event_enable_suppressed)).unwrap();

    let disabled_target = ObservedObject::new(31, PropertyValue::Real(85.0));
    let disabled_target_oid = disabled_target.object_identifier();
    db.add(Box::new(disabled_target)).unwrap();
    let mut detection_disabled = enrollment(
        31,
        EventType::OUT_OF_RANGE,
        Some(disabled_target_oid),
        out_of_range_parameters(0),
    );
    detection_disabled
        .write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    let detection_disabled_oid = detection_disabled.object_identifier();
    db.add(Box::new(detection_disabled)).unwrap();

    let unavailable_oid = ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 32).unwrap();
    db.add(Box::new(enrollment(
        32,
        EventType::OUT_OF_RANGE,
        Some(ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 999).unwrap()),
        out_of_range_parameters(0),
    )))
    .unwrap();

    let no_recipient_target = ObservedObject::new(33, PropertyValue::Real(85.0));
    let no_recipient_target_oid = no_recipient_target.object_identifier();
    db.add(Box::new(no_recipient_target)).unwrap();
    let mut no_recipient = enrollment(
        33,
        EventType::OUT_OF_RANGE,
        Some(no_recipient_target_oid),
        out_of_range_parameters(0),
    );
    no_recipient.set_notification_class(1);
    let no_recipient_oid = no_recipient.object_identifier();
    db.add(Box::new(no_recipient)).unwrap();
    db.add(Box::new(NotificationClass::new(1, "NC-empty").unwrap()))
        .unwrap();

    let event_log_oid = ObjectIdentifier::new(ObjectType::EVENT_LOG, 2).unwrap();
    db.add(Box::new(EventLogObject::new(2, "Event Log", 16).unwrap()))
        .unwrap();

    let (mut server, sent) = start_server(db, true).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(drain_notifications(&sent).is_empty());

    let db = server.database().write().await;
    assert_eq!(event_state(&db, event_enable_oid), EventState::HIGH_LIMIT);
    assert_eq!(event_state(&db, no_recipient_oid), EventState::HIGH_LIMIT);
    assert_eq!(event_state(&db, detection_disabled_oid), EventState::NORMAL);
    assert_eq!(event_state(&db, unavailable_oid), EventState::NORMAL);
    assert_eq!(db.reserve_event_sequence_number().number(), 2);
    assert_eq!(
        db.get(&event_log_oid)
            .unwrap()
            .read_property(PropertyIdentifier::RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
    drop(db);

    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(drain_notifications(&sent).is_empty());
    assert_eq!(
        server
            .database()
            .write()
            .await
            .reserve_event_sequence_number()
            .number(),
        2,
        "suppressed, disabled, unavailable, and no-recipient paths do not repeat commits"
    );
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn dcc_suppresses_event_enrollment_io_without_suppressing_commit() {
    for dcc_state in [1, 2] {
        let mut db = ObjectDatabase::new();
        let target = ObservedObject::new(40 + dcc_state as u32, PropertyValue::Real(85.0));
        let target_oid = target.object_identifier();
        db.add(Box::new(target)).unwrap();
        let enrollment = enrollment(
            40 + dcc_state as u32,
            EventType::OUT_OF_RANGE,
            Some(target_oid),
            out_of_range_parameters(1),
        );
        let enrollment_oid = enrollment.object_identifier();
        db.add(Box::new(enrollment)).unwrap();

        let (mut server, sent) = start_server(db, true).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        server.comm_state.store(dcc_state, Ordering::Release);
        tokio::time::sleep(Duration::from_secs(1)).await;

        assert!(drain_notifications(&sent).is_empty());
        let db = server.database().write().await;
        assert_eq!(event_state(&db, enrollment_oid), EventState::HIGH_LIMIT);
        assert_eq!(db.reserve_event_sequence_number().number(), 1);
        drop(db);
        server.stop().await.unwrap();
    }
}
