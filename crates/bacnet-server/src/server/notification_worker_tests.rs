use super::*;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_types::enums::EngineeringUnits;

async fn reap_after_ingress_closure(hold_request: bool) {
    let (mut server, ingress, mut started) = fixture().await;
    let mut request_released = if hold_request {
        inject(&ingress, confirmed(false)).await;
        Some(started.recv().await.unwrap())
    } else {
        None
    };
    // Close the real NPDU input, which closes the network layer's APDU sender.
    // Keep the server and its outbound producers alive.
    drop(ingress);
    held_sends(&server).pass_cov.store(true, Ordering::Release);
    for batch in 0..2 {
        tokio::time::pause();
        if batch == 0 {
            fire_cov(&server, CovNotificationKind::Single).await;
        } else {
            // The first report failed: wait out its hold-off (#896).
            tokio::time::advance(
                Duration::from_millis(server.config.cov_retry_timeout_ms)
                    * (u32::from(DEFAULT_APDU_RETRIES) + 1),
            )
            .await;
            server
                .write_local(
                    &ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    PropertyValue::Real(42.0),
                    Some(16),
                    crate::LocalCommandSource::ServerDevice,
                )
                .await
                .unwrap();
        }
        // No ACK can arrive after ingress closure. Drive the unchanged retry
        // sequence to ordinary exhaustion, observing every real transport send.
        for _ in 0..=DEFAULT_APDU_RETRIES {
            let released = tokio::time::timeout(Duration::from_secs(2), started.recv())
                .await
                .unwrap()
                .expect("outbound COV admission stopped with ingress");
            released.await.unwrap();
            assert!(matches!(held_sends(&server).frames.lock().unwrap().last(),
                Some(Apdu::ConfirmedRequest(request))
                    if request.service_choice == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION));
            tokio::time::advance(Duration::from_millis(server.config.cov_retry_timeout_ms)).await;
        }
        tokio::time::resume();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !server.notification_transactions.workers_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("completed notification retained after ingress closure");
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
        assert!(!server.notification_transactions.is_closed());
        assert!(!server.dispatch_task.as_ref().unwrap().is_finished());
        if let Some(released) = request_released.as_mut() {
            assert!(matches!(
                released.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ));
            assert!(
                !server.request_tasks.is_empty(),
                "request must remain blocked during notification reaping"
            );
        }
    }
    if let Some(released) = request_released {
        held_sends(&server).release.notify_one();
        released.await.unwrap();
        wait_reaped(&server).await;
    }
    server.stop().await.unwrap();
    assert!(server.notification_transactions.workers_empty());
}

#[tokio::test]
async fn notification_worker_reaps_after_ingress_closure() {
    reap_after_ingress_closure(false).await;
}

#[tokio::test]
async fn notification_worker_reaps_after_ingress_closure_with_blocked_request() {
    reap_after_ingress_closure(true).await;
}

async fn fire_event(server: &BACnetServer<TestTransport>) {
    use crate::server::event_recipient_routing_tests::{address_recipient, destination_for};
    use bacnet_objects::analog::AnalogInputObject;
    use bacnet_objects::event::EventStateChange;
    use bacnet_objects::notification_class::NotificationClass;
    use bacnet_types::enums::{EventState, EventType};

    let mut db = crate::server::clocked_test_database();
    db.add(Box::new(DeviceObject::new(Default::default()).unwrap()))
        .unwrap();
    let mut nc = NotificationClass::new(0, "NC-0").unwrap();
    nc.add_destination(destination_for(address_recipient(0, &[1]), true))
        .unwrap();
    db.add(Box::new(nc)).unwrap();
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::SQUARE_METERS).unwrap(),
    ))
    .unwrap();
    BACnetServer::<TestTransport>::build_and_send_event_notification_with_bindings(
        &EventDelivery {
            db: &Arc::new(RwLock::new(db)),
            network: server.test_network(),
            comm_state: &server.comm_state,
            learned_routers: &server.learned_routers,
            notification_transactions: &server.notification_transactions,
            device_bindings: &server.device_bindings,
            suppressions: &Default::default(),
            retry_timeout_ms: 3000,
            local_apdu_capacity: 1476,
        },
        &ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        (
            EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            EventType::OUT_OF_RANGE,
        ),
    )
    .await;
}

async fn fire_cov(server: &BACnetServer<TestTransport>, kind: CovNotificationKind) {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    server
        .db
        .write()
        .await
        .add(Box::new(
            AnalogOutputObject::new(1, "AO-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
    server
        .cov_table
        .write()
        .await
        .admit_for_test(
            CovSubscription {
                subscriber_mac: MacAddr::from_slice(&[1]),
                subscriber_network: None,
                subscriber_process_identifier: 7,
                monitored_object_identifier: oid,
                issue_confirmed_notifications: true,
                // A Multiple context always has a finite lifetime.
                expires_at: (kind == CovNotificationKind::Multiple)
                    .then(|| std::time::Instant::now() + std::time::Duration::from_secs(3600)),
                last_notified_observation: None,
                monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
                monitored_property_array_index: None,
                cov_increment: None,
                notification_kind: kind,
                timestamped: false,
            },
            0,
        )
        .unwrap();
    BACnetServer::<TestTransport>::fire_cov_notifications(
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
    assert!(matches!(held_sends(&server).frames.lock().unwrap().last(),
        Some(Apdu::ConfirmedRequest(request)) if request.service_choice == service));
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
    // Stop ends the outstanding report without a hold-off, and the baseline
    // still waits for an Ack (#896).
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    let mut table = server.cov_table.write().await;
    let entry = table.subscriptions_for(&oid)[0].clone();
    assert!(table.confirmed_idle(&entry), "{kind:?}");
    assert_eq!(entry.last_notified_observation, None, "{kind:?}");
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
    assert!(matches!(held_sends(&server).frames.lock().unwrap().last(),
        Some(Apdu::ConfirmedRequest(request)) if request.service_choice == ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION));
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
                let request = match held_sends(&server).frames.lock().unwrap().last().unwrap() {
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
                held_sends(&server)
                    .panic_next
                    .store(panic, Ordering::Release);
                held_sends(&server).release.notify_one();
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
