use super::cov_notifications_tests::RecordingTransport;
use super::*;
use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::lighting::BinaryLightingOutputObject;
use bacnet_objects::traits::BACnetObject;
use std::sync::{Arc as StdArc, Mutex as StdMutex};

async fn start_server(
    egress_seconds: u64,
) -> (
    BACnetServer<RecordingTransport>,
    ObjectIdentifier,
    StdArc<StdMutex<Vec<(Bytes, MacAddr)>>>,
) {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let transport = RecordingTransport::new(StdArc::clone(&sent));
    let mut object = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    object
        .write_property(
            PropertyIdentifier::BLINK_WARN_ENABLE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    object
        .write_property(
            PropertyIdentifier::EGRESS_TIME,
            None,
            PropertyValue::Unsigned(egress_seconds),
            None,
        )
        .unwrap();
    object
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(8),
        )
        .unwrap();
    let oid = object.object_identifier();

    let device = DeviceObject::new(DeviceConfig {
        instance: 100,
        name: "Binary-lighting-task-device".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(object)).unwrap();
    let config = ServerConfig {
        enable_event_enrollment: false,
        ..ServerConfig::default()
    };
    let server = BACnetServer::start(config, db, transport).await.unwrap();
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    (server, oid, sent)
}

async fn write_command(
    server: &BACnetServer<RecordingTransport>,
    oid: ObjectIdentifier,
    value: u32,
    priority: u8,
) {
    server
        .write_local(
            &oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(value),
            Some(priority),
        )
        .await
        .unwrap();
}

async fn read(
    server: &BACnetServer<RecordingTransport>,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> PropertyValue {
    server
        .database()
        .read()
        .await
        .get(&oid)
        .unwrap()
        .read_property(property, index)
        .unwrap()
}

async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn operation_task_has_no_early_completion_and_expires_at_exact_monotonic_time() {
    let (mut server, oid, _) = start_server(2).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    settle().await;
    tokio::time::advance(Duration::from_millis(1)).await;
    write_command(&server, oid, 3, 8).await;

    tokio::time::advance(Duration::from_secs(1)).await;
    settle().await;
    tokio::time::advance(Duration::from_millis(999)).await;
    settle().await;
    assert_eq!(
        read(&server, oid, PropertyIdentifier::EGRESS_ACTIVE, None).await,
        PropertyValue::Boolean(true)
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(8)).await,
        PropertyValue::Enumerated(1)
    );

    tokio::time::advance(Duration::from_millis(1)).await;
    settle().await;
    assert_eq!(
        read(&server, oid, PropertyIdentifier::EGRESS_ACTIVE, None).await,
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(8)).await,
        PropertyValue::Enumerated(0)
    );
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn local_stop_cancels_timer_and_prevents_later_expiry() {
    let (mut server, oid, _) = start_server(2).await;
    write_command(&server, oid, 3, 8).await;
    write_command(&server, oid, 5, 8).await;

    tokio::time::advance(Duration::from_secs(10)).await;
    settle().await;
    assert_eq!(
        read(&server, oid, PropertyIdentifier::EGRESS_ACTIVE, None).await,
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(8)).await,
        PropertyValue::Enumerated(1)
    );
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn stop_aborts_and_awaits_operation_task_with_no_work_after_stop() {
    let (mut server, oid, _) = start_server(2).await;
    write_command(&server, oid, 3, 8).await;
    server.stop().await.unwrap();
    assert!(server.binary_lighting_operation_task.is_none());

    tokio::time::advance(Duration::from_secs(10)).await;
    settle().await;
    assert_eq!(
        read(&server, oid, PropertyIdentifier::EGRESS_ACTIVE, None).await,
        PropertyValue::Boolean(true)
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(8)).await,
        PropertyValue::Enumerated(1)
    );
}

#[tokio::test(start_paused = true)]
async fn actual_delayed_elapsed_completes_without_missed_tick_bursting() {
    let (mut server, oid, _) = start_server(3).await;
    write_command(&server, oid, 3, 8).await;

    tokio::time::advance(Duration::from_secs(3)).await;
    settle().await;
    assert_eq!(
        read(&server, oid, PropertyIdentifier::EGRESS_ACTIVE, None).await,
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(8)).await,
        PropertyValue::Enumerated(0)
    );
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn expiry_fires_one_generic_cov_after_database_lock_release() {
    let (mut server, oid, sent) = start_server(2).await;
    server.cov_table.write().await.subscribe(CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC1]),
        subscriber_network: None,
        subscriber_process_identifier: 7,
        monitored_object_identifier: oid,
        issue_confirmed_notifications: false,
        expires_at: None,
        last_notified_value: None,
        monitored_property: None,
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    });

    write_command(&server, oid, 3, 8).await;
    assert_eq!(sent.lock().unwrap().len(), 1, "accepted-write coarse COV");
    sent.lock().unwrap().clear();

    tokio::time::advance(Duration::from_secs(2)).await;
    settle().await;
    assert_eq!(
        sent.lock().unwrap().len(),
        1,
        "expiry must release the database write lock before the generic COV path rereads state"
    );

    tokio::time::advance(Duration::from_secs(10)).await;
    settle().await;
    assert_eq!(sent.lock().unwrap().len(), 1, "one COV per actual expiry");
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn terminal_cov_snapshot_survives_a_later_command_before_delivery() {
    for operation in [3, 4] {
        let (mut server, oid, sent) = start_server(2).await;
        if let Some(task) = server.binary_lighting_operation_task.take() {
            task.abort();
            let _ = task.await;
        }
        {
            let mut table = server.cov_table.write().await;
            for (process, property, kind) in [
                (31, None, CovNotificationKind::Single),
                (
                    32,
                    Some(PropertyIdentifier::EGRESS_ACTIVE),
                    CovNotificationKind::Multiple,
                ),
            ] {
                table.subscribe(CovSubscription {
                    subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, process as u8]),
                    subscriber_network: None,
                    subscriber_process_identifier: process,
                    monitored_object_identifier: oid,
                    issue_confirmed_notifications: false,
                    expires_at: None,
                    last_notified_value: None,
                    monitored_property: property,
                    monitored_property_array_index: None,
                    cov_increment: None,
                    notification_kind: kind,
                    timestamped: false,
                });
            }
        }

        let snapshot = {
            let mut db = server.db.write().await;
            let object = db.get_mut(&oid).unwrap();
            object
                .write_property(
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    PropertyValue::Enumerated(operation),
                    Some(8),
                )
                .unwrap();
            let deadline = object.next_monotonic_deadline_internal().unwrap();
            assert!(object.advance_monotonic_time_internal(deadline));
            object.cov_snapshot_internal().unwrap()
        };
        {
            let mut db = server.db.write().await;
            db.get_mut(&oid)
                .unwrap()
                .write_property(
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    PropertyValue::Enumerated(1),
                    Some(4),
                )
                .unwrap();
        }

        BACnetServer::<RecordingTransport>::fire_cov_notifications_from_snapshot(
            &server.db,
            &server.network,
            &server.cov_table,
            &server.cov_in_flight,
            &server.notification_transactions,
            &server.comm_state,
            &server.config,
            &oid,
            snapshot.as_ref(),
        )
        .await;

        let apdus = sent
            .lock()
            .unwrap()
            .iter()
            .map(|(frame, _)| decode_apdu(decode_npdu(frame.clone()).unwrap().payload).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(apdus.len(), 2);
        for apdu in apdus {
            let Apdu::UnconfirmedRequest(request) = apdu else {
                panic!("expected unconfirmed COV");
            };
            if request.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION {
                let notification =
                    COVNotificationRequest::decode(&request.service_request).unwrap();
                let value = &notification
                    .list_of_values
                    .iter()
                    .find(|value| value.property_identifier == PropertyIdentifier::PRESENT_VALUE)
                    .unwrap()
                    .value;
                assert_eq!(
                    bacnet_encoding::primitives::decode_application_value(value, 0)
                        .unwrap()
                        .0,
                    PropertyValue::Enumerated(0)
                );
            } else {
                let notification =
                    COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
                let value = &notification.list_of_cov_notifications[0].list_of_values[0].value;
                assert_eq!(
                    bacnet_encoding::primitives::decode_application_value(value, 0)
                        .unwrap()
                        .0,
                    PropertyValue::Boolean(false)
                );
            }
        }
        server.stop().await.unwrap();
    }
}
