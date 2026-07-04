use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_types::enums::ObjectType;
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
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        list_of_cov_notifications: vec![COVNotificationItem {
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            list_of_values: vec![COVNotificationValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x44, 0x42, 0x91, 0x00, 0x00],
                time_of_change: Some(vec![0x19, 0x2A]),
            }],
        }],
    }
}

#[test]
fn unconfirmed_cov_multiple_apdu_uses_multiple_service_choice() {
    let notification = sample_cov_multiple_notification();
    let buf = BACnetServer::<BipTransport>::encode_unconfirmed_cov_multiple_apdu(&notification);

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
        BACnetServer::<BipTransport>::encode_confirmed_cov_multiple_apdu(&notification, 9, 1476);

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
        &Arc::new(Mutex::new(ServerTsm::new())),
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
        }
        other => panic!("expected unconfirmed COVNotificationMultiple, got {other:?}"),
    }
}
