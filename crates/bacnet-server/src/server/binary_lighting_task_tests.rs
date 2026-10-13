use super::cov_notifications_tests::recording_transport;
use super::*;
use crate::server::test_transport::{SendLog, TestTransport};
use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::lighting::BinaryLightingOutputObject;
use bacnet_objects::traits::BACnetObject;

async fn start_server(
    egress_seconds: u64,
) -> (BACnetServer<TestTransport>, ObjectIdentifier, SendLog) {
    let (transport, sent) = recording_transport();
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
    server: &BACnetServer<TestTransport>,
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
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

async fn read(
    server: &BACnetServer<TestTransport>,
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
    server
        .cov_table
        .write()
        .await
        .subscribe(CovSubscription {
            subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC1]),
            subscriber_network: None,
            subscriber_process_identifier: 7,
            monitored_object_identifier: oid,
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_observation: None,
            monitored_property: None,
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        })
        .unwrap();

    write_command(&server, oid, 3, 8).await;
    assert_eq!(sent.len(), 1, "accepted-write coarse COV");
    sent.clear();

    tokio::time::advance(Duration::from_secs(2)).await;
    settle().await;
    assert_eq!(
        sent.len(),
        1,
        "expiry must release the database write lock before the generic COV path rereads state"
    );

    tokio::time::advance(Duration::from_secs(10)).await;
    settle().await;
    assert_eq!(sent.len(), 1, "one COV per actual expiry");
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
                table
                    .admit_for_test(
                        CovSubscription {
                            subscriber_mac: MacAddr::from_slice(&[
                                127,
                                0,
                                0,
                                1,
                                0xBA,
                                process as u8,
                            ]),
                            subscriber_network: None,
                            subscriber_process_identifier: process,
                            monitored_object_identifier: oid,
                            issue_confirmed_notifications: false,
                            // A Multiple context always has a finite lifetime.
                            expires_at: (kind == CovNotificationKind::Multiple)
                                .then(|| runtime_clock::now() + Duration::from_secs(3600)),
                            last_notified_observation: None,
                            monitored_property: property,
                            monitored_property_array_index: None,
                            cov_increment: None,
                            notification_kind: kind,
                            timestamped: false,
                        },
                        0,
                    )
                    .unwrap();
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

        BACnetServer::<TestTransport>::fire_cov_notifications_from_snapshot(
            &crate::server::cov_notify_context::CovNotifyContext {
                db: &server.db,
                network: server.test_network(),
                cov_table: &server.cov_table,
                cov_in_flight: &server.cov_in_flight,
                notification_transactions: &server.notification_transactions,
                comm_state: &server.comm_state,
                config: &server.config,
            },
            &oid,
            snapshot.as_ref(),
        )
        .await;

        let apdus = sent
            .lock()
            .iter()
            .map(|frame| decode_apdu(decode_npdu(frame.npdu.clone()).unwrap().payload).unwrap())
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

/// The COV-multiple notifications sent since `from`, decoded.
fn multiple_notifications(sent: &SendLog, from: usize) -> Vec<COVNotificationMultipleRequest> {
    sent.lock()
        .iter()
        .skip(from)
        .filter_map(|frame| {
            let apdu = decode_apdu(decode_npdu(frame.npdu.clone()).unwrap().payload).unwrap();
            match apdu {
                Apdu::UnconfirmedRequest(request)
                    if request.service_choice
                        == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE =>
                {
                    Some(COVNotificationMultipleRequest::decode(&request.service_request).unwrap())
                }
                _ => None,
            }
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_snapshot_report_times_a_timestamped_field_only_with_its_own_value() {
    use super::cov_wire_test_support::{at, time, SharedClock};
    let (mut server, oid, sent) = start_server(2).await;
    if let Some(task) = server.binary_lighting_operation_task.take() {
        task.abort();
        let _ = task.await;
    }
    let clock = SharedClock(Arc::new(std::sync::Mutex::new(at(1))));
    server
        .database()
        .write()
        .await
        .set_clock_reader(Some(Arc::new(clock.clone())));
    // One context: an untimestamped PV reference and a timestamped
    // Status_Flags reference, whose field the PV reports carry along.
    let flags = {
        let mut table = server.cov_table.write().await;
        let mut admit = |property, timestamped| {
            table
                .admit_for_test(
                    CovSubscription {
                        subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 33]),
                        subscriber_network: None,
                        subscriber_process_identifier: 33,
                        monitored_object_identifier: oid,
                        issue_confirmed_notifications: false,
                        expires_at: Some(runtime_clock::now() + Duration::from_secs(3600)),
                        last_notified_observation: None,
                        monitored_property: Some(property),
                        monitored_property_array_index: None,
                        cov_increment: None,
                        notification_kind: CovNotificationKind::Multiple,
                        timestamped,
                    },
                    0,
                )
                .unwrap()
        };
        admit(PropertyIdentifier::PRESENT_VALUE, false);
        admit(PropertyIdentifier::STATUS_FLAGS, true)
    };
    // Convey whatever the timestamped reference holds, as a report would.
    let convey = |table: &CovSubscriptionTable| {
        let store = table.timed().clone();
        let (incarnation, drained) = store.lock().drain(flags.key(), flags.generation());
        let mut claim = crate::cov::timed::TimedClaim::new(store.clone());
        claim.add(flags.key().clone(), incarnation, drained);
        claim.commit();
    };
    {
        let db = server.db.read().await;
        let table = server.cov_table.read().await;
        table
            .initial_timed_capture(std::slice::from_ref(&flags), at(1))
            .run(&db);
        convey(&table);
    }
    let fire = |snapshot: Box<dyn BACnetObject>| {
        let server = &server;
        async move {
            BACnetServer::<TestTransport>::fire_cov_notifications_from_snapshot(
                &crate::server::cov_notify_context::CovNotifyContext {
                    db: &server.db,
                    network: server.test_network(),
                    cov_table: &server.cov_table,
                    cov_in_flight: &server.cov_in_flight,
                    notification_transactions: &server.notification_transactions,
                    comm_state: &server.comm_state,
                    config: &server.config,
                },
                &oid,
                snapshot.as_ref(),
            )
            .await;
        }
    };
    let flag_times = |notification: &COVNotificationMultipleRequest| {
        notification.list_of_cov_notifications[0]
            .list_of_values
            .iter()
            .filter(|value| value.property_identifier == PropertyIdentifier::STATUS_FLAGS)
            .map(|value| value.time_of_change)
            .collect::<Vec<_>>()
    };
    let snapshot_with_pv = |value| {
        let server = &server;
        async move {
            let mut db = server.db.write().await;
            let object = db.get_mut(&oid).unwrap();
            object
                .write_property(
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    PropertyValue::Enumerated(value),
                    Some(8),
                )
                .unwrap();
            object.cov_snapshot_internal().unwrap()
        }
    };

    // The snapshot's Status_Flags is the value captured at 1: it keeps 1.
    let snapshot = snapshot_with_pv(0).await;
    *clock.0.lock().unwrap() = at(5);
    let before = sent.lock().len();
    fire(snapshot).await;
    let reports = multiple_notifications(&sent, before);
    assert_eq!(reports.len(), 1);
    assert_eq!(flag_times(&reports[0]), vec![Some(time(1))]);
    assert_eq!(reports[0].timestamp, Some((at(1).local_date, time(1))));

    // A snapshot older than a captured Status_Flags change carries a stale
    // value: no time belongs to it here, so it is left out (#987).
    let snapshot = snapshot_with_pv(1).await;
    *clock.0.lock().unwrap() = at(6);
    {
        let mut db = server.db.write().await;
        db.get_mut(&oid)
            .unwrap()
            .write_property(
                PropertyIdentifier::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(true),
                None,
            )
            .unwrap();
        let table = server.cov_table.read().await;
        table.timed_capture(oid).run(&db);
        convey(&table);
    }
    *clock.0.lock().unwrap() = at(7);
    let before = sent.lock().len();
    fire(snapshot).await;
    let reports = multiple_notifications(&sent, before);
    assert_eq!(reports.len(), 1);
    assert!(flag_times(&reports[0]).is_empty(), "{:?}", reports[0]);
    assert_eq!(reports[0].timestamp, None);
    server.stop().await.unwrap();
}

/// An object whose deadline stays due however often it is advanced, as an
/// Averaging object's would if nothing claimed its due sample (#1144).
struct StuckDeadline;

impl BACnetObject for StuckDeadline {
    fn object_identifier(&self) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 77).unwrap()
    }
    fn object_name(&self) -> &str {
        "STUCK-1"
    }
    fn read_property(&self, _: PropertyIdentifier, _: Option<u32>) -> Result<PropertyValue, Error> {
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
        })
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }
    fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
        std::borrow::Cow::Borrowed(&[])
    }
    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        Some(Duration::ZERO)
    }
}

#[tokio::test(start_paused = true)]
async fn a_deadline_left_due_does_not_spin_the_monotonic_task() {
    let (transport, _) = recording_transport();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(StuckDeadline)).unwrap();
    let mut server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .unwrap();
    // Paused time only moves while every task is idle: a task waking at once
    // for the same deadline would hold this sleep forever.
    let before = tokio::time::Instant::now();
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(before.elapsed(), Duration::from_secs(5));
    server.stop().await.unwrap();
}

mod observation_order;
