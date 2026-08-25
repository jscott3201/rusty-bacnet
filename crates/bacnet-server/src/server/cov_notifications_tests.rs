use super::cov_clock::cov_multiple_datetime;
use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{Date, Time};
use bytes::Bytes;
use std::sync::{Arc as StdArc, Mutex as StdMutex};
use tokio::sync::mpsc;

#[derive(Clone, Default)]
struct RecordingTransport {
    sent_unicast: StdArc<StdMutex<Vec<(Bytes, MacAddr)>>>,
    local_mac: Vec<u8>,
}

impl RecordingTransport {
    fn new(sent_unicast: StdArc<StdMutex<Vec<(Bytes, MacAddr)>>>) -> Self {
        Self {
            sent_unicast,
            local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
        }
    }
}

impl TransportPort for RecordingTransport {
    async fn start(
        &mut self,
    ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
        let (_tx, rx) = mpsc::channel(1);
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.sent_unicast
            .lock()
            .unwrap()
            .push((Bytes::copy_from_slice(npdu), MacAddr::from_slice(mac)));
        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
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
    let (date, time) = cov_multiple_datetime(Duration::ZERO, 0);
    assert_eq!(
        date,
        Date {
            year: 70,
            month: 1,
            day: 1,
            day_of_week: 4,
        }
    );
    assert_eq!(
        time,
        Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        }
    );

    // 2024-02-29T12:34:56.780Z exercises leap-day and hundredths handling.
    let (date, time) = cov_multiple_datetime(Duration::new(1_709_210_096, 780_000_000), 0);
    assert_eq!(
        date,
        Date {
            year: 124,
            month: 2,
            day: 29,
            day_of_week: 4,
        }
    );
    assert_eq!(
        time,
        Time {
            hour: 12,
            minute: 34,
            second: 56,
            hundredths: 78,
        }
    );

    // BACnet UTC_Offset is positive west of UTC and is subtracted. At +60,
    // 1970-01-02T00:00Z is still 1970-01-01 locally.
    let (date, time) = cov_multiple_datetime(Duration::new(86_400, 0), 60);
    assert_eq!(
        date,
        Date {
            year: 70,
            month: 1,
            day: 1,
            day_of_week: 4,
        }
    );
    assert_eq!(time.hour, 23);

    // A negative (east-of-UTC) offset advances the local civil day.
    let (date, time) = cov_multiple_datetime(Duration::new(86_399, 0), -60);
    assert_eq!(
        date,
        Date {
            year: 70,
            month: 1,
            day: 2,
            day_of_week: 5,
        }
    );
    assert_eq!(time.hour, 0);
}

#[tokio::test]
async fn routed_cov_send_preserves_npdu_destination() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = NetworkLayer::new(RecordingTransport::new(StdArc::clone(&sent)));
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
        last_notified_value: None,
        monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    };
    let apdu = [0x10, 0x02, 0xAA, 0xBB];

    BACnetServer::<RecordingTransport>::send_cov_apdu(&network, &apdu, &sub, true)
        .await
        .unwrap();

    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1, router_mac);
    let npdu = decode_npdu(sent[0].0.clone()).unwrap();
    assert_eq!(npdu.destination, Some(remote));
    assert!(npdu.expecting_reply);
    assert_eq!(npdu.payload, Bytes::copy_from_slice(&apdu));
}

#[tokio::test]
async fn routed_segmented_complex_ack_preserves_npdu_destination() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
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
            BACnetServer::<RecordingTransport>::send_segmented_complex_ack(
                &network,
                &seg_ack_senders,
                &seg_send_permits,
                router_mac.as_slice(),
                Some(&remote),
                0x44,
                ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                &service_ack_data,
                50,
                None,
            )
            .await;
        })
    };

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if !sent.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("segmented response did not send first segment");

    handle.abort();
    let _ = handle.await;

    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1, router_mac);
    let npdu = decode_npdu(sent[0].0.clone()).unwrap();
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

#[tokio::test]
async fn cov_property_multiple_subscription_uses_multiple_notification_on_change() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));

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
        table.subscribe(CovSubscription {
            subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC1]),
            subscriber_network: None,
            subscriber_process_identifier: 7,
            monitored_object_identifier: ao_oid,
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_value: None,
            monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Multiple,
            timestamped: false,
        });
    }

    BACnetServer::<RecordingTransport>::fire_cov_notifications(
        &db,
        &network,
        &cov_table,
        &Arc::new(Semaphore::new(255)),
        &NotificationTransactions::new(),
        &Arc::new(AtomicU8::new(0)),
        &ServerConfig::default(),
        &ao_oid,
    )
    .await;

    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    let npdu = decode_npdu(sent[0].0.clone()).unwrap();
    match decode_apdu(npdu.payload).unwrap() {
        Apdu::UnconfirmedRequest(req) => {
            assert_eq!(
                req.service_choice,
                UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE
            );
            let notification =
                COVNotificationMultipleRequest::decode(&req.service_request).unwrap();
            assert_eq!(notification.time_remaining, 0);
            assert_eq!(notification.timestamp, None);
            assert_eq!(
                notification.list_of_cov_notifications[0].list_of_values[0].time_of_change,
                None
            );
        }
        other => panic!("expected unconfirmed COVNotificationMultiple, got {other:?}"),
    }
}

#[tokio::test]
async fn timestamped_cov_multiple_reports_datetime_and_remaining_lifetime() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));

    let ao_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
    let mut db = ObjectDatabase::new();
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
        table.subscribe(CovSubscription {
            subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC1]),
            subscriber_network: None,
            subscriber_process_identifier: 7,
            monitored_object_identifier: ao_oid,
            issue_confirmed_notifications: false,
            expires_at: Some(Instant::now() + Duration::from_secs(300)),
            last_notified_value: None,
            monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Multiple,
            timestamped: true,
        });
    }

    BACnetServer::<RecordingTransport>::fire_cov_notifications(
        &db,
        &network,
        &cov_table,
        &Arc::new(Semaphore::new(255)),
        &NotificationTransactions::new(),
        &Arc::new(AtomicU8::new(0)),
        &ServerConfig::default(),
        &ao_oid,
    )
    .await;

    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    let npdu = decode_npdu(sent[0].0.clone()).unwrap();
    let Apdu::UnconfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
        panic!("expected unconfirmed COVNotificationMultiple");
    };
    let notification = COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
    assert!((298..=300).contains(&notification.time_remaining));
    let (_, request_time) = notification
        .timestamp
        .expect("timestamped values require request BACnetDateTime");
    assert_eq!(
        notification.list_of_cov_notifications[0].list_of_values[0].time_of_change,
        Some(request_time)
    );
}

#[tokio::test(start_paused = true)]
async fn confirmed_cov_single_and_multiple_retries_retain_their_leases() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(StdArc::clone(
        &sent,
    ))));
    let transactions = NotificationTransactions::new();
    let ao_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
    let single_mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC1]);
    let multiple_mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC2]);

    let mut db = ObjectDatabase::new();
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
            table.subscribe(CovSubscription {
                subscriber_mac,
                subscriber_network: None,
                subscriber_process_identifier: process_id,
                monitored_object_identifier: ao_oid,
                issue_confirmed_notifications: true,
                expires_at: None,
                last_notified_value: None,
                monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
                monitored_property_array_index: None,
                cov_increment: None,
                notification_kind,
                timestamped: false,
            });
        }
    }
    let config = ServerConfig {
        cov_retry_timeout_ms: 100,
        ..ServerConfig::default()
    };

    BACnetServer::<RecordingTransport>::fire_cov_notifications(
        &db,
        &network,
        &cov_table,
        &Arc::new(Semaphore::new(255)),
        &transactions,
        &Arc::new(AtomicU8::new(0)),
        &config,
        &ao_oid,
    )
    .await;
    for _ in 0..32 {
        if sent.lock().unwrap().len() >= 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(sent.lock().unwrap().len(), 2);

    let initial: Vec<(ConfirmedServiceChoice, u8)> = sent
        .lock()
        .unwrap()
        .iter()
        .map(|(frame, _)| {
            let npdu = decode_npdu(frame.clone()).unwrap();
            let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
                panic!("expected confirmed COV notification");
            };
            (request.service_choice, request.invoke_id)
        })
        .collect();
    assert_eq!(initial.len(), 2);
    assert_ne!(initial[0].1, initial[1].1);
    assert_eq!(transactions.active_count(), 2);

    tokio::time::advance(Duration::from_millis(101)).await;
    for _ in 0..32 {
        if sent.lock().unwrap().len() >= 4 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(sent.lock().unwrap().len(), 4);
    let retries: Vec<(ConfirmedServiceChoice, u8)> = sent
        .lock()
        .unwrap()
        .iter()
        .skip(2)
        .map(|(frame, _)| {
            let npdu = decode_npdu(frame.clone()).unwrap();
            let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
                panic!("expected confirmed COV retry");
            };
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
            &Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice: service,
            }),
        ));
    }
    assert_eq!(transactions.active_count(), 0);
}
