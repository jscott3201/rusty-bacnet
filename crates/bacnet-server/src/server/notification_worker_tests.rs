use super::*;
use bacnet_objects::analog::AnalogOutputObject;

async fn fire_event(server: &BACnetServer<HeldTransport>) {
    use crate::server::event_recipient_routing_tests::{address_recipient, destination_for};
    use bacnet_objects::analog::AnalogInputObject;
    use bacnet_objects::event::EventStateChange;
    use bacnet_objects::notification_class::NotificationClass;
    use bacnet_types::enums::{EventState, EventType};

    let mut db = crate::server::clocked_test_database();
    db.add(Box::new(DeviceObject::new(Default::default()).unwrap()))
        .unwrap();
    let mut nc = NotificationClass::new(0, "NC-0").unwrap();
    nc.add_destination(destination_for(address_recipient(0, &[1]), true));
    db.add(Box::new(nc)).unwrap();
    db.add(Box::new(AnalogInputObject::new(1, "AI-1", 0).unwrap()))
        .unwrap();
    BACnetServer::<HeldTransport>::build_and_send_event_notification_with_bindings(
        &Arc::new(RwLock::new(db)),
        &server.network,
        &server.comm_state,
        &server.server_tsm,
        &server.notification_transactions,
        &server.device_bindings,
        &ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        (
            EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            EventType::OUT_OF_RANGE,
        ),
        3000,
    )
    .await;
}

async fn fire_cov(server: &BACnetServer<HeldTransport>, kind: CovNotificationKind) {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    server
        .db
        .write()
        .await
        .add(Box::new(AnalogOutputObject::new(1, "AO-1", 62).unwrap()))
        .unwrap();
    server.cov_table.write().await.subscribe(CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[1]),
        subscriber_network: None,
        subscriber_process_identifier: 7,
        monitored_object_identifier: oid,
        issue_confirmed_notifications: true,
        expires_at: None,
        last_notified_value: None,
        monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: kind,
        timestamped: false,
    });
    BACnetServer::<HeldTransport>::fire_cov_notifications(
        &server.db,
        &server.network,
        &server.cov_table,
        &server.cov_in_flight,
        &server.notification_transactions,
        &server.comm_state,
        &server.config,
        &oid,
    )
    .await;
}

async fn stop_cov(kind: CovNotificationKind, service: ConfirmedServiceChoice) {
    let (mut server, _ingress, mut started) = fixture().await;
    fire_cov(&server, kind).await;
    let mut released = tokio::time::timeout(Duration::from_secs(2), started.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(server.network.transport().frames.lock().unwrap().last(),
        Some(Apdu::ConfirmedRequest(request)) if request.service_choice == service)
    );
    assert_eq!(server.notification_transactions.active_count(), 1);
    assert_eq!(server.cov_in_flight.available_permits(), 254);
    assert_eq!(
        server
            .cov_table
            .read()
            .await
            .in_flight_tracker()
            .active_peer_count(),
        1
    );
    server.stop().await.unwrap();
    assert_eq!(
        released.try_recv(),
        Ok(()),
        "stop returned with a live notification send"
    );
    assert_eq!(server.notification_transactions.active_count(), 0);
    assert!(server.notification_transactions.workers_empty());
    assert_eq!(server.cov_in_flight.available_permits(), 255);
    assert_eq!(
        server
            .cov_table
            .read()
            .await
            .in_flight_tracker()
            .active_peer_count(),
        0
    );
}

#[tokio::test]
async fn notification_worker_stop_single_cov() {
    stop_cov(
        CovNotificationKind::Single,
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
    )
    .await;
}

#[tokio::test]
async fn notification_worker_stop_multiple_cov() {
    stop_cov(
        CovNotificationKind::Multiple,
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
    )
    .await;
}

#[tokio::test]
async fn notification_worker_stop_event() {
    let (mut server, _ingress, mut started) = fixture().await;
    fire_event(&server).await;
    let mut released = tokio::time::timeout(Duration::from_secs(2), started.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(server.network.transport().frames.lock().unwrap().last(),
        Some(Apdu::ConfirmedRequest(request)) if request.service_choice == ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION)
    );
    assert_eq!(server.notification_transactions.active_count(), 1);
    server.stop().await.unwrap();
    assert_eq!(released.try_recv(), Ok(()));
    assert!(server.notification_transactions.workers_empty());
    assert_eq!(server.notification_transactions.active_count(), 0);
}

#[tokio::test]
async fn notification_worker_cancelled_stop_retains_joins() {
    use std::future::Future;
    use std::task::Poll;
    let (mut server, _ingress, mut started) = fixture().await;
    fire_cov(&server, CovNotificationKind::Single).await;
    let mut released = started.recv().await.unwrap();
    {
        let mut stop = std::pin::pin!(server.stop());
        std::future::poll_fn(|cx| {
            assert!(stop.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
    }
    assert!(server.notification_transactions.is_closed());
    assert!(!server.notification_transactions.workers_empty());
    server.stop().await.unwrap();
    assert_eq!(released.try_recv(), Ok(()));
    assert!(server.notification_transactions.workers_empty());
    assert_eq!(server.cov_in_flight.available_permits(), 255);
}

#[tokio::test]
async fn notification_worker_reaps_all_families_success_panic_idle_active() {
    use bacnet_encoding::apdu::SimpleAck;
    for family in 0..3 {
        for panic in [false, true] {
            for active in [false, true] {
                let (mut server, ingress, mut started) = fixture().await;
                match family {
                    0 => fire_cov(&server, CovNotificationKind::Single).await,
                    1 => fire_cov(&server, CovNotificationKind::Multiple).await,
                    _ => fire_event(&server).await,
                }
                let released = started.recv().await.unwrap();
                let request = match server
                    .network
                    .transport()
                    .frames
                    .lock()
                    .unwrap()
                    .last()
                    .unwrap()
                {
                    Apdu::ConfirmedRequest(request) => request.clone(),
                    other => panic!("unexpected notification: {other:?}"),
                };
                if active {
                    inject(&ingress, confirmed(false)).await;
                    let _held_request =
                        tokio::time::timeout(Duration::from_secs(2), started.recv())
                            .await
                            .unwrap()
                            .unwrap();
                }
                server
                    .network
                    .transport()
                    .panic_next
                    .store(panic, Ordering::Release);
                server.network.transport().release.notify_one();
                released.await.unwrap();
                if !panic {
                    inject(
                        &ingress,
                        Apdu::SimpleAck(SimpleAck {
                            invoke_id: request.invoke_id,
                            service_choice: request.service_choice,
                        }),
                    )
                    .await;
                }
                // Keep ordinary request ingress active while dispatch also reaps.
                tokio::time::timeout(Duration::from_secs(2), async {
                    while !server.notification_transactions.workers_empty() {
                        if active {
                            inject(&ingress, confirmed(false)).await;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .expect("notification completion was not reaped");
                assert_eq!(server.notification_transactions.active_count(), 0);
                assert_eq!(server.cov_in_flight.available_permits(), 255);
                assert_eq!(
                    server
                        .cov_table
                        .read()
                        .await
                        .in_flight_tracker()
                        .active_peer_count(),
                    0
                );
                assert!(!server.dispatch_task.as_ref().unwrap().is_finished());
                server.stop().await.unwrap();
            }
        }
    }
}
