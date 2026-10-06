use super::cov_clock::cov_multiple_datetime;
use super::*;
use crate::server::test_transport::{SendLog, SendMode, TestTransport, BIP_LOCAL_MAC};
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::cov_multiple::{COVNotificationItem, COVNotificationValue};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{Date, Time};
use bytes::Bytes;
use std::sync::Arc as StdArc;

/// Records unicasts and ignores broadcasts, from a B/IP-shaped local MAC.
pub(super) fn recording_transport() -> (TestTransport, SendLog) {
    let transport = TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .broadcast(SendMode::Ignore)
        .build();
    let sent = transport.sent();
    (transport, sent)
}

fn sample_cov_multiple_notification() -> COVNotificationMultipleRequest {
    COVNotificationMultipleRequest {
        subscriber_process_identifier: 7,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap(),
        time_remaining: 0,
        timestamp: Some((
            Date {
                year: 126,
                month: 4,
                day: 13,
                day_of_week: 1,
            },
            Time {
                hour: 10,
                minute: 11,
                second: 12,
                hundredths: 13,
            },
        )),
        list_of_cov_notifications: vec![COVNotificationItem {
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            list_of_values: vec![COVNotificationValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x44, 0x42, 0x91, 0x00, 0x00],
                time_of_change: Some(Time {
                    hour: 10,
                    minute: 11,
                    second: 12,
                    hundredths: 13,
                }),
            }],
        }],
    }
}

#[test]
fn unconfirmed_cov_multiple_apdu_uses_multiple_service_choice() {
    let notification = sample_cov_multiple_notification();
    let buf =
        BACnetServer::<BipTransport>::encode_unconfirmed_cov_multiple_apdu(&notification).unwrap();

    match decode_apdu(buf.freeze()).unwrap() {
        Apdu::UnconfirmedRequest(req) => {
            assert_eq!(
                req.service_choice,
                UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE
            );
            let decoded = COVNotificationMultipleRequest::decode(&req.service_request).unwrap();
            assert_eq!(decoded, notification);
        }
        other => panic!("expected unconfirmed COVNotificationMultiple, got {other:?}"),
    }
}

#[test]
fn confirmed_cov_multiple_apdu_uses_multiple_service_choice() {
    let notification = sample_cov_multiple_notification();
    let buf =
        BACnetServer::<BipTransport>::encode_confirmed_cov_multiple_apdu(&notification, 9, 1476)
            .unwrap();

    match decode_apdu(buf.freeze()).unwrap() {
        Apdu::ConfirmedRequest(req) => {
            assert_eq!(req.invoke_id, 9);
            assert_eq!(
                req.service_choice,
                ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE
            );
            let decoded = COVNotificationMultipleRequest::decode(&req.service_request).unwrap();
            assert_eq!(decoded, notification);
        }
        other => panic!("expected confirmed COVNotificationMultiple, got {other:?}"),
    }
}

#[test]
fn cov_multiple_timestamp_uses_device_local_bacnet_date_and_time() {
    let frame = ClockFrame {
        local_date: Date {
            year: 124,
            month: 2,
            day: 29,
            day_of_week: 4,
        },
        local_time: Time {
            hour: 12,
            minute: 34,
            second: 56,
            hundredths: 78,
        },
        utc_offset: 300,
        daylight_savings_status: true,
    };
    assert_eq!(
        cov_multiple_datetime(frame),
        (frame.local_date, frame.local_time)
    );
}

#[tokio::test]
async fn routed_cov_send_preserves_npdu_destination() {
    let (transport, sent) = recording_transport();
    let network = NetworkLayer::new(transport);
    let router_mac = MacAddr::from_slice(&[192, 168, 1, 1, 0xBA, 0xC0]);
    let remote = NpduAddress {
        network: 100,
        mac_address: MacAddr::from_slice(&[0x0A, 0x14, 0x1E]),
    };
    let sub = CovSubscription {
        subscriber_mac: router_mac.clone(),
        subscriber_network: Some(remote.clone()),
        subscriber_process_identifier: 7,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        issue_confirmed_notifications: true,
        expires_at: None,
        last_notified_observation: None,
        monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    };
    let apdu = [0x10, 0x02, 0xAA, 0xBB];

    BACnetServer::<TestTransport>::send_cov_apdu(&network, &apdu, &sub, true)
        .await
        .unwrap();

    let sent = sent.lock();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].mac, router_mac);
    let npdu = decode_npdu(sent[0].npdu.clone()).unwrap();
    assert_eq!(npdu.destination, Some(remote));
    assert!(npdu.expecting_reply);
    assert_eq!(npdu.payload, Bytes::copy_from_slice(&apdu));
}

#[tokio::test]
async fn routed_segmented_complex_ack_preserves_npdu_destination() {
    let (transport, sent) = recording_transport();
    let network = Arc::new(NetworkLayer::new(transport));
    let seg_ack_senders = Arc::new(segmented_send::SegmentedSendRegistry::default());
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let router_mac = MacAddr::from_slice(&[192, 168, 1, 1, 0xBA, 0xC0]);
    let remote = NpduAddress {
        network: 100,
        mac_address: MacAddr::from_slice(&[0x0A, 0x14, 0x1E]),
    };
    let service_ack_data = vec![0x55; 128];

    let handle = {
        let network = Arc::clone(&network);
        let seg_ack_senders = Arc::clone(&seg_ack_senders);
        let seg_send_permits = Arc::clone(&seg_send_permits);
        let remote = remote.clone();
        let router_mac = router_mac.clone();
        tokio::spawn(async move {
            BACnetServer::<TestTransport>::send_segmented_complex_ack(
                SegmentedSendResources {
                    network: &network,
                    seg_ack_senders: &seg_ack_senders,
                    seg_send_permits: &seg_send_permits,
                },
                ResponseTarget {
                    source_mac: router_mac.as_slice(),
                    source_network: Some(&remote),
                    route: &bacnet_network::response_route::ResponseRoute::unverified(),
                },
                ComplexAckParams {
                    invoke_id: 0x44,
                    service_choice: ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                    client_max_apdu: 50,
                    client_max_segments: None,
                },
                &service_ack_data,
                None,
            )
            .await;
        })
    };

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if !sent.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("segmented response did not send first segment");

    handle.abort();
    let _ = handle.await;

    let sent = sent.lock();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].mac, router_mac);
    let npdu = decode_npdu(sent[0].npdu.clone()).unwrap();
    assert_eq!(npdu.destination, Some(remote));
    assert!(npdu.expecting_reply);
    match decode_apdu(npdu.payload).unwrap() {
        Apdu::ComplexAck(ack) => {
            assert!(ack.segmented);
            assert_eq!(ack.invoke_id, 0x44);
            assert_eq!(
                ack.service_choice,
                ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE
            );
        }
        other => panic!("expected segmented ComplexAck, got {other:?}"),
    }
}

/// Paused: COV lifetimes are measured on tokio's clock (#1556), so the
/// notification reports exactly the 300 s the subscription was given.
#[tokio::test(start_paused = true)]
async fn cov_property_multiple_subscription_uses_multiple_notification_on_change() {
    let (transport, sent) = recording_transport();
    let network = Arc::new(NetworkLayer::new(transport));

    let ao_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();

    let mut db = ObjectDatabase::new();
    let mut device = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "COV-Multiple-Test".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    device.set_object_list(vec![device_oid, ao_oid]);
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(AnalogOutputObject::new(1, "AO-1", 62).unwrap()))
        .unwrap();

    let db = Arc::new(RwLock::new(db));
    let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
    {
        let mut table = cov_table.write().await;
        // A Multiple context always has a finite lifetime.
        table
            .admit_for_test(
                CovSubscription {
                    subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC1]),
                    subscriber_network: None,
                    subscriber_process_identifier: 7,
                    monitored_object_identifier: ao_oid,
                    issue_confirmed_notifications: false,
                    expires_at: Some(runtime_clock::now() + Duration::from_secs(300)),
                    last_notified_observation: None,
                    monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
                    monitored_property_array_index: None,
                    cov_increment: None,
                    notification_kind: CovNotificationKind::Multiple,
                    timestamped: false,
                },
                10,
            )
            .unwrap();
    }

    BACnetServer::<TestTransport>::fire_cov_notifications(
        &crate::server::cov_notify_context::CovNotifyContext {
            db: &db,
            network: &network,
            cov_table: &cov_table,
            cov_in_flight: &Arc::new(Semaphore::new(255)),
            notification_transactions: &NotificationTransactions::new(),
            comm_state: &Arc::new(CommState::default()),
            config: &Arc::default(),
        },
        &ao_oid,
    )
    .await;

    let sent = sent.lock();
    assert_eq!(sent.len(), 1);
    let npdu = decode_npdu(sent[0].npdu.clone()).unwrap();
    match decode_apdu(npdu.payload).unwrap() {
        Apdu::UnconfirmedRequest(req) => {
            assert_eq!(
                req.service_choice,
                UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE
            );
            let notification =
                COVNotificationMultipleRequest::decode(&req.service_request).unwrap();
            assert_eq!(notification.time_remaining, 300);
            assert_eq!(notification.timestamp, None);
            assert_eq!(
                notification.list_of_cov_notifications[0].list_of_values[0].time_of_change,
                None
            );
        }
        other => panic!("expected unconfirmed COVNotificationMultiple, got {other:?}"),
    }
}

struct FixedClock(ClockFrame);

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(self.0)
    }
}

fn fixed_clock_frame() -> ClockFrame {
    ClockFrame {
        local_date: Date {
            year: 124,
            month: 2,
            day: 29,
            day_of_week: 4,
        },
        local_time: Time {
            hour: 12,
            minute: 34,
            second: 56,
            hundredths: 78,
        },
        utc_offset: 300,
        daylight_savings_status: true,
    }
}

async fn capture_timestamped_cov_multiple(
    clock_frame: Option<ClockFrame>,
    include_untimestamped: bool,
) -> Vec<COVNotificationMultipleRequest> {
    let (transport, sent) = recording_transport();
    let network = Arc::new(NetworkLayer::new(transport));

    let ao_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
    let mut db = ObjectDatabase::new();
    if let Some(frame) = clock_frame {
        db.set_clock_reader(Some(StdArc::new(FixedClock(frame))));
    }
    let mut device = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "Timestamped-COV-Multiple-Test".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    device.set_object_list(vec![device_oid, ao_oid]);
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(AnalogOutputObject::new(1, "AO-1", 62).unwrap()))
        .unwrap();

    let db = Arc::new(RwLock::new(db));
    let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
    {
        let mut table = cov_table.write().await;
        table
            .admit_for_test(
                CovSubscription {
                    subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC1]),
                    subscriber_network: None,
                    subscriber_process_identifier: 7,
                    monitored_object_identifier: ao_oid,
                    issue_confirmed_notifications: false,
                    expires_at: Some(runtime_clock::now() + Duration::from_secs(300)),
                    last_notified_observation: None,
                    monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
                    monitored_property_array_index: None,
                    cov_increment: None,
                    notification_kind: CovNotificationKind::Multiple,
                    timestamped: true,
                },
                10,
            )
            .unwrap();
        if include_untimestamped {
            table
                .admit_for_test(
                    CovSubscription {
                        subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC1]),
                        subscriber_network: None,
                        subscriber_process_identifier: 7,
                        monitored_object_identifier: ao_oid,
                        issue_confirmed_notifications: false,
                        expires_at: Some(runtime_clock::now() + Duration::from_secs(300)),
                        last_notified_observation: None,
                        monitored_property: Some(PropertyIdentifier::STATUS_FLAGS),
                        monitored_property_array_index: None,
                        cov_increment: None,
                        notification_kind: CovNotificationKind::Multiple,
                        timestamped: false,
                    },
                    10,
                )
                .unwrap();
        }
    }

    BACnetServer::<TestTransport>::fire_cov_notifications(
        &crate::server::cov_notify_context::CovNotifyContext {
            db: &db,
            network: &network,
            cov_table: &cov_table,
            cov_in_flight: &Arc::new(Semaphore::new(255)),
            notification_transactions: &NotificationTransactions::new(),
            comm_state: &Arc::new(CommState::default()),
            config: &Arc::default(),
        },
        &ao_oid,
    )
    .await;

    let sent = sent.lock();
    sent.iter()
        .map(|frame| {
            let npdu = decode_npdu(frame.npdu.clone()).unwrap();
            let Apdu::UnconfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
                panic!("expected unconfirmed COVNotificationMultiple");
            };
            COVNotificationMultipleRequest::decode(&request.service_request).unwrap()
        })
        .collect()
}

/// Paused, so the time remaining is exact (#1556).
#[tokio::test(start_paused = true)]
async fn timestamped_cov_multiple_uses_one_frame_and_exact_per_value_optionality() {
    let frame = fixed_clock_frame();
    let notifications = capture_timestamped_cov_multiple(Some(frame), true).await;
    assert_eq!(notifications.len(), 1);
    let notification = &notifications[0];
    assert_eq!(notification.time_remaining, 300);
    assert_eq!(
        notification.timestamp,
        Some((frame.local_date, frame.local_time))
    );
    let values = &notification.list_of_cov_notifications[0].list_of_values;
    let timestamped = values
        .iter()
        .find(|value| value.property_identifier == PropertyIdentifier::PRESENT_VALUE)
        .unwrap();
    let untimestamped = values
        .iter()
        .find(|value| value.property_identifier == PropertyIdentifier::STATUS_FLAGS)
        .unwrap();
    assert_eq!(
        timestamped.time_of_change,
        Some(frame.local_time),
        "request and per-value timestamps originate from the same frame"
    );
    assert_eq!(untimestamped.time_of_change, None);
}

#[tokio::test]
async fn clockless_legacy_timestamped_cov_multiple_fails_closed() {
    let notifications = capture_timestamped_cov_multiple(None, false).await;
    assert!(notifications.is_empty());
}

#[tokio::test(start_paused = true)]
async fn confirmed_cov_single_and_multiple_retries_retain_their_leases() {
    let (transport, sent) = recording_transport();
    let network = Arc::new(NetworkLayer::new(transport));
    let transactions = NotificationTransactions::new();
    let ao_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
    let single_mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC1]);
    let multiple_mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC2]);

    let mut db = clocked_test_database();
    let mut device = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "Confirmed-COV-Retry-Test".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    device.set_object_list(vec![device_oid, ao_oid]);
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(AnalogOutputObject::new(1, "AO-1", 62).unwrap()))
        .unwrap();

    let db = Arc::new(RwLock::new(db));
    let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
    {
        let mut table = cov_table.write().await;
        for (subscriber_mac, process_id, notification_kind) in [
            (single_mac.clone(), 7, CovNotificationKind::Single),
            (multiple_mac.clone(), 8, CovNotificationKind::Multiple),
        ] {
            table
                .admit_for_test(
                    CovSubscription {
                        subscriber_mac,
                        subscriber_network: None,
                        subscriber_process_identifier: process_id,
                        monitored_object_identifier: ao_oid,
                        issue_confirmed_notifications: true,
                        // A Multiple context always has a finite lifetime.
                        expires_at: (notification_kind == CovNotificationKind::Multiple)
                            .then(|| runtime_clock::now() + Duration::from_secs(3600)),
                        last_notified_observation: None,
                        monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
                        monitored_property_array_index: None,
                        cov_increment: None,
                        notification_kind,
                        timestamped: false,
                    },
                    0,
                )
                .unwrap();
        }
    }
    let config = Arc::new(ServerConfig {
        cov_retry_timeout_ms: 100,
        max_apdu_length: 1474,
        ..ServerConfig::default()
    });

    BACnetServer::<TestTransport>::fire_cov_notifications(
        &crate::server::cov_notify_context::CovNotifyContext {
            db: &db,
            network: &network,
            cov_table: &cov_table,
            cov_in_flight: &Arc::new(Semaphore::new(255)),
            notification_transactions: &transactions,
            comm_state: &Arc::new(CommState::default()),
            config: &config,
        },
        &ao_oid,
    )
    .await;
    for _ in 0..32 {
        if sent.len() >= 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(sent.len(), 2);

    let initial: Vec<(ConfirmedServiceChoice, u8)> = sent
        .lock()
        .iter()
        .map(|frame| {
            let npdu = decode_npdu(frame.npdu.clone()).unwrap();
            let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
                panic!("expected confirmed COV notification");
            };
            assert_eq!(
                request.max_apdu_length, 1024,
                "raw1474 floors in both COV headers"
            );
            (request.service_choice, request.invoke_id)
        })
        .collect();
    assert_eq!(initial.len(), 2);
    assert_ne!(initial[0].1, initial[1].1);
    assert_eq!(transactions.active_count(), 2);

    tokio::time::advance(Duration::from_millis(101)).await;
    for _ in 0..32 {
        if sent.len() >= 4 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(sent.len(), 4);
    let retries: Vec<(ConfirmedServiceChoice, u8)> = sent
        .lock()
        .iter()
        .skip(2)
        .map(|frame| {
            let npdu = decode_npdu(frame.npdu.clone()).unwrap();
            let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
                panic!("expected confirmed COV retry");
            };
            assert_eq!(
                request.max_apdu_length, 1024,
                "raw1474 floors in both COV headers"
            );
            (request.service_choice, request.invoke_id)
        })
        .collect();
    for (service, invoke_id) in &initial {
        assert!(retries.contains(&(*service, *invoke_id)));
    }
    assert_eq!(transactions.active_count(), 2);

    for (service, invoke_id) in initial {
        let source = if service == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION {
            &single_mac
        } else {
            &multiple_mac
        };
        assert!(transactions.admit_terminal(
            source,
            None,
            None,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice: service,
            }),
        ));
    }
    assert_eq!(transactions.active_count(), 0);
}
