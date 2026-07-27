use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event::EventStateChange;
use bacnet_objects::traits::BACnetObject;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::EventState;
use bytes::Bytes;
use std::sync::{Arc as StdArc, Mutex as StdMutex};
use tokio::sync::mpsc;

/// A transport that records every broadcast NPDU it is asked to send and
/// discards unicasts. Used to capture the EventNotification a server
/// actually puts on the wire.
#[derive(Clone, Default)]
struct RecordingTransport {
    sent_broadcast: StdArc<StdMutex<Vec<Bytes>>>,
    local_mac: Vec<u8>,
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
    async fn send_unicast(&self, _npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.sent_broadcast
            .lock()
            .unwrap()
            .push(Bytes::copy_from_slice(npdu));
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
}

/// A DCC-disabled server (comm_state >= 1) suppresses the periodic event
/// send: `build_and_send_event_notification` returns without sending,
/// matching the per-write path's DCC gate. Verified against a recording
/// transport that would otherwise capture the broadcast APDU.
#[tokio::test]
async fn dcc_suppresses_periodic_event_send() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let transport = RecordingTransport {
        sent_broadcast: StdArc::clone(&sent),
        local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
    };
    let network = Arc::new(NetworkLayer::new(transport));
    let comm_state = Arc::new(AtomicU8::new(1)); // DCC disabled
    let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));

    let mut db = ObjectDatabase::new();
    db.add(Box::new(AnalogInputObject::new(1, "AI-1", 0).unwrap()))
        .unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1,
            name: "Dev".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let db = Arc::new(tokio::sync::RwLock::new(db));
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let change = EventStateChange {
        from: EventState::NORMAL,
        to: EventState::HIGH_LIMIT,
    };
    BACnetServer::<RecordingTransport>::build_and_send_event_notification(
        &db,
        &network,
        &comm_state,
        &server_tsm,
        &oid,
        change,
        1000,
    )
    .await;

    assert!(
        sent.lock().unwrap().is_empty(),
        "DCC-disabled server must not send event notifications"
    );
}

/// Decode the single broadcast EventNotification captured by a
/// [`RecordingTransport`] into its [`EventNotificationRequest`].
///
/// Panics with a useful message if no notification was sent (so a regression
/// that silently drops the notification is caught rather than masking as
/// "no broadcast = pass").
fn decode_broadcast_notification(sent: &StdMutex<Vec<Bytes>>) -> EventNotificationRequest {
    use bacnet_encoding::apdu::decode_apdu;
    use bacnet_encoding::npdu::decode_npdu;

    let guard = sent.lock().unwrap();
    assert_eq!(
        guard.len(),
        1,
        "expected exactly one broadcast EventNotification, got {}",
        guard.len()
    );
    let npdu = decode_npdu(guard[0].clone()).expect("decode NPDU");
    match decode_apdu(npdu.payload).expect("decode APDU") {
        Apdu::UnconfirmedRequest(req) => {
            assert_eq!(
                req.service_choice,
                UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION
            );
            EventNotificationRequest::decode(&req.service_request)
                .expect("decode EventNotification")
        }
        other => panic!("expected UnconfirmedRequest, got {other:?}"),
    }
}

/// Build a server fixture: a Device, a NotificationClass (instance `nc`,
/// empty recipient list so notifications broadcast) with the given
/// per-transition `priority` / `ack_required`, and an AnalogInput whose
/// `Notification_Class` points at it with `Notify_Type = ALARM`.
async fn fixture_with_commanded_nc(
    nc: u32,
    priority: [u8; 3],
    ack_required: [bool; 3],
) -> (
    Arc<RwLock<ObjectDatabase>>,
    Arc<NetworkLayer<RecordingTransport>>,
    Arc<AtomicU8>,
    Arc<Mutex<ServerTsm>>,
    Arc<StdMutex<Vec<Bytes>>>,
    ObjectIdentifier,
) {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let transport = RecordingTransport {
        sent_broadcast: StdArc::clone(&sent),
        local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
    };
    let network = Arc::new(NetworkLayer::new(transport));
    let comm_state = Arc::new(AtomicU8::new(0)); // DCC enabled
    let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));

    let mut db = ObjectDatabase::new();
    // NotificationClass with the configured per-transition arrays and an
    // empty recipient list, so the notification is broadcast (not per-recipient).
    let mut notification_class =
        bacnet_objects::notification_class::NotificationClass::new(nc, "NC").unwrap();
    notification_class.priority = priority;
    notification_class.ack_required = ack_required;
    db.add(Box::new(notification_class)).unwrap();

    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1,
            name: "Dev".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();

    // AnalogInput pointing at the NotificationClass, ALARM notify type.
    let mut ai = AnalogInputObject::new(1, "AI-1", 0).unwrap();
    ai.write_property(
        PropertyIdentifier::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(nc as u64),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::NOTIFY_TYPE,
        None,
        PropertyValue::Enumerated(NotifyType::ALARM.to_raw()),
        None,
    )
    .unwrap();
    db.add(Box::new(ai)).unwrap();

    let db = Arc::new(RwLock::new(db));
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    (db, network, comm_state, server_tsm, sent, oid)
}

/// Per-transition `Priority` from the NotificationClass is projected into
/// the broadcast EventNotification (TO_OFFNORMAL -> PRIORITY[0] = 50),
/// not the legacy hardcoded 100.
#[tokio::test]
async fn event_notification_projects_offnormal_priority_from_class() {
    let (db, network, comm_state, server_tsm, sent, oid) =
        fixture_with_commanded_nc(5, [50, 150, 250], [true, false, true]).await;

    let change = EventStateChange {
        from: EventState::NORMAL,
        to: EventState::HIGH_LIMIT,
    };
    BACnetServer::<RecordingTransport>::build_and_send_event_notification(
        &db,
        &network,
        &comm_state,
        &server_tsm,
        &oid,
        change,
        1000,
    )
    .await;

    let notif = decode_broadcast_notification(&sent);
    assert_eq!(notif.priority, 50, "TO_OFFNORMAL priority from PRIORITY[0]");
    assert!(
        notif.ack_required,
        "TO_OFFNORMAL ack_required from ACK_REQUIRED bit 0"
    );
}

/// TO_FAULT projects PRIORITY[1] and ACK_REQUIRED bit 1.
#[tokio::test]
async fn event_notification_projects_fault_priority_from_class() {
    let (db, network, comm_state, server_tsm, sent, oid) =
        fixture_with_commanded_nc(5, [50, 150, 250], [true, false, true]).await;

    let change = EventStateChange {
        from: EventState::NORMAL,
        to: EventState::FAULT,
    };
    BACnetServer::<RecordingTransport>::build_and_send_event_notification(
        &db,
        &network,
        &comm_state,
        &server_tsm,
        &oid,
        change,
        1000,
    )
    .await;

    let notif = decode_broadcast_notification(&sent);
    assert_eq!(notif.priority, 150, "TO_FAULT priority from PRIORITY[1]");
    assert!(
        !notif.ack_required,
        "TO_FAULT ack_required from ACK_REQUIRED bit 1"
    );
}

/// TO_NORMAL projects PRIORITY[2] (250), not the legacy hardcoded 200.
#[tokio::test]
async fn event_notification_projects_normal_priority_from_class() {
    let (db, network, comm_state, server_tsm, sent, oid) =
        fixture_with_commanded_nc(5, [50, 150, 250], [true, false, true]).await;

    let change = EventStateChange {
        from: EventState::HIGH_LIMIT,
        to: EventState::NORMAL,
    };
    BACnetServer::<RecordingTransport>::build_and_send_event_notification(
        &db,
        &network,
        &comm_state,
        &server_tsm,
        &oid,
        change,
        1000,
    )
    .await;

    let notif = decode_broadcast_notification(&sent);
    assert_eq!(notif.priority, 250, "TO_NORMAL priority from PRIORITY[2]");
    assert!(
        notif.ack_required,
        "TO_NORMAL ack_required from ACK_REQUIRED bit 2"
    );
}

/// Missing NotificationClass falls back to Priority 255 and no ack, and
/// the notification is still delivered (not silently dropped).
#[tokio::test]
async fn event_notification_missing_class_falls_back_to_defaults() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let transport = RecordingTransport {
        sent_broadcast: StdArc::clone(&sent),
        local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
    };
    let network = Arc::new(NetworkLayer::new(transport));
    let comm_state = Arc::new(AtomicU8::new(0));
    let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));

    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1,
            name: "Dev".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    // AI points at NotificationClass 999, which does not exist.
    let mut ai = AnalogInputObject::new(1, "AI-1", 0).unwrap();
    ai.write_property(
        PropertyIdentifier::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(999),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::NOTIFY_TYPE,
        None,
        PropertyValue::Enumerated(NotifyType::ALARM.to_raw()),
        None,
    )
    .unwrap();
    db.add(Box::new(ai)).unwrap();
    let db = Arc::new(RwLock::new(db));
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let change = EventStateChange {
        from: EventState::NORMAL,
        to: EventState::HIGH_LIMIT,
    };
    BACnetServer::<RecordingTransport>::build_and_send_event_notification(
        &db,
        &network,
        &comm_state,
        &server_tsm,
        &oid,
        change,
        1000,
    )
    .await;

    let notif = decode_broadcast_notification(&sent);
    assert_eq!(
        notif.priority, 255,
        "missing class falls back to lowest priority"
    );
    assert!(
        !notif.ack_required,
        "missing class falls back to no acknowledgement"
    );
}

/// Notify_Type = EVENT still honors the per-transition Ack_Required (it is
/// not ALARM-specific); the legacy code derived ack_required purely from
/// `Notify_Type == ALARM`, so an EVENT notification would wrongly clear it.
#[tokio::test]
async fn event_notification_event_notify_type_honors_class_ack_required() {
    let (db, network, comm_state, server_tsm, sent, oid) =
        fixture_with_commanded_nc(5, [50, 150, 250], [true, false, true]).await;
    // Reconfigure the AI to Notify_Type = EVENT.
    {
        let mut guard = db.write().await;
        let ai = guard.get_mut(&oid).expect("AI present");
        ai.write_property(
            PropertyIdentifier::NOTIFY_TYPE,
            None,
            PropertyValue::Enumerated(NotifyType::EVENT.to_raw()),
            None,
        )
        .unwrap();
    }

    let change = EventStateChange {
        from: EventState::NORMAL,
        to: EventState::HIGH_LIMIT,
    };
    BACnetServer::<RecordingTransport>::build_and_send_event_notification(
        &db,
        &network,
        &comm_state,
        &server_tsm,
        &oid,
        change,
        1000,
    )
    .await;

    let notif = decode_broadcast_notification(&sent);
    // ack_required is encoded for both ALARM and EVENT notify types; the
    // per-transition ACK_REQUIRED bit 0 (TO_OFFNORMAL) is true here.
    assert!(
        notif.ack_required,
        "EVENT notify type honors per-transition ACK_REQUIRED"
    );
    assert_eq!(notif.priority, 50);
}
