use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event::EventStateChange;
use bacnet_objects::traits::BACnetObject;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{EventState, EventType};
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
        (change, EventType::OUT_OF_RANGE),
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
        (change, EventType::OUT_OF_RANGE),
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
        (change, EventType::CHANGE_OF_RELIABILITY),
        1000,
    )
    .await;

    let notif = decode_broadcast_notification(&sent);
    assert_eq!(notif.priority, 150, "TO_FAULT priority from PRIORITY[1]");
    assert!(
        !notif.ack_required,
        "TO_FAULT ack_required from ACK_REQUIRED bit 1"
    );
    // Clause 13.2.5.3: a transition to FAULT is reported as
    // CHANGE_OF_RELIABILITY, not as the object's own algorithm. Asserted on the
    // decoded wire bytes rather than on `event_type()` in isolation, so the
    // value is checked where it actually reaches a peer.
    assert_eq!(
        notif.event_type,
        EventType::CHANGE_OF_RELIABILITY.to_raw(),
        "TO_FAULT must be reported as CHANGE_OF_RELIABILITY"
    );
}

/// The from-FAULT direction, which Clauses 13.8 and 13.9 state separately from
/// the to-FAULT case: "The Event Type CHANGE_OF_RELIABILITY shall be used for
/// reporting a transition from FAULT."
///
/// Worth its own test because the transition coordinate differs — this is a
/// TO_NORMAL transition for Priority and Ack_Required purposes, while still
/// being CHANGE_OF_RELIABILITY for Event Type. A fix that keyed the event type
/// off the transition category rather than the states would get this wrong.
#[tokio::test]
async fn event_notification_from_fault_is_change_of_reliability() {
    let (db, network, comm_state, server_tsm, sent, oid) =
        fixture_with_commanded_nc(5, [50, 150, 250], [true, false, true]).await;

    let change = EventStateChange {
        from: EventState::FAULT,
        to: EventState::NORMAL,
    };
    BACnetServer::<RecordingTransport>::build_and_send_event_notification(
        &db,
        &network,
        &comm_state,
        &server_tsm,
        &oid,
        (change, EventType::CHANGE_OF_RELIABILITY),
        1000,
    )
    .await;

    let notif = decode_broadcast_notification(&sent);
    assert_eq!(
        notif.event_type,
        EventType::CHANGE_OF_RELIABILITY.to_raw(),
        "a transition FROM FAULT is also CHANGE_OF_RELIABILITY"
    );
    // ...while the transition coordinate is still TO_NORMAL.
    assert_eq!(notif.priority, 250, "TO_NORMAL priority from PRIORITY[2]");
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
        (change, EventType::OUT_OF_RANGE),
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
        (change, EventType::OUT_OF_RANGE),
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
        (change, EventType::OUT_OF_RANGE),
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

/// Build a one-object database whose AnalogInput will transition
/// NORMAL -> HIGH_LIMIT on the next intrinsic evaluation, with `Event_Enable`
/// set from `event_enable_byte`.
///
/// `Event_Enable` is written through `write_property` rather than an internal
/// setter, so these tests cover the same path a network client takes. Bytes
/// are the Clause 20.2.10 wire encoding: MSB-first, TO_OFFNORMAL at `0x80`.
fn db_with_high_limit_transition(
    event_enable_byte: u8,
) -> Arc<tokio::sync::RwLock<ObjectDatabase>> {
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    for (p, v) in [
        (PropertyIdentifier::HIGH_LIMIT, 80.0f32),
        (PropertyIdentifier::LOW_LIMIT, 20.0),
        (PropertyIdentifier::DEADBAND, 2.0),
    ] {
        ai.write_property(p, None, PropertyValue::Real(v), None)
            .unwrap();
    }
    ai.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![0xC0], // low + high limit checking enabled
        },
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![event_enable_byte],
        },
        None,
    )
    .unwrap();
    ai.set_present_value(81.0); // above high_limit -> NORMAL -> HIGH_LIMIT

    let mut db = ObjectDatabase::new();
    db.add(Box::new(ai)).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1,
            name: "Dev".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    Arc::new(tokio::sync::RwLock::new(db))
}

/// Drive the per-write path once and return the broadcasts it produced.
async fn broadcasts_from_per_write_path(
    db: &Arc<tokio::sync::RwLock<ObjectDatabase>>,
) -> Vec<Bytes> {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let transport = RecordingTransport {
        sent_broadcast: StdArc::clone(&sent),
        local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
    };
    let network = Arc::new(NetworkLayer::new(transport));
    let comm_state = Arc::new(AtomicU8::new(0)); // DCC not blocking
    let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    BACnetServer::<RecordingTransport>::fire_event_notifications(
        &db,
        &network,
        &comm_state,
        &server_tsm,
        &oid,
        1000,
    )
    .await;

    let out = sent.lock().unwrap().clone();
    out
}

/// A cleared `Event_Enable` bit must suppress the outbound notification.
///
/// This is the gate this whole change moved. Before #136 the detector returned
/// `None` for a suppressed transition, so nothing downstream *could* send;
/// now the detector reports the transition and only this send site declines to
/// distribute it (Clause 13.2.5). Without this test, deleting or inverting that
/// check is invisible to the suite.
#[tokio::test]
async fn event_enable_cleared_suppresses_per_write_send() {
    let db = db_with_high_limit_transition(0x00); // no transition distributable
    let sent = broadcasts_from_per_write_path(&db).await;

    assert!(
        sent.is_empty(),
        "Event_Enable with TO_OFFNORMAL clear must suppress the send, got {} broadcast(s)",
        sent.len()
    );

    // The transition itself still happened — suppression is distribution-only.
    let db = db.read().await;
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw()),
        "Event_State advances even though the notification was suppressed"
    );
}

/// The other direction: with TO_OFFNORMAL set, the notification IS sent.
///
/// Paired deliberately with the suppression test — a gate stuck permanently
/// closed would satisfy that one alone. Together they pin the gate to
/// `Event_Enable` rather than to a constant.
#[tokio::test]
async fn event_enable_set_permits_per_write_send() {
    // TO_OFFNORMAL only: wire bit 0 = 0x80 (Clause 20.2.10).
    let db = db_with_high_limit_transition(0x80);
    let sent = broadcasts_from_per_write_path(&db).await;

    assert_eq!(
        sent.len(),
        1,
        "Event_Enable with TO_OFFNORMAL set must distribute the notification"
    );
    let sent = StdMutex::new(sent);
    let notif = decode_broadcast_notification(&sent);
    assert_eq!(
        notif.event_type,
        EventType::OUT_OF_RANGE.to_raw(),
        "the detector's non-FAULT OUT_OF_RANGE algorithm must reach the wire"
    );
}

/// The periodic `Time_Delay` path has its own `Event_Enable` gate, and it needs
/// its own test: the per-write tests above cannot reach it, because a nonzero
/// `Time_Delay` makes the per-write probe return `None` by design.
///
/// Drives the real spawned `intrinsic_reporting_task` on a paused clock: a
/// local write seeds a pending transition, the clock advances past the delay,
/// the task ticks and fires it — with TO_OFFNORMAL cleared, so nothing may go
/// out. Proven to fail when the `outcome.distribute` check at that site is
/// replaced with `if true`.
#[tokio::test(start_paused = true)]
async fn event_enable_cleared_suppresses_periodic_time_delay_send() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let transport = RecordingTransport {
        sent_broadcast: StdArc::clone(&sent),
        local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
    };

    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    for (p, v) in [
        (PropertyIdentifier::HIGH_LIMIT, 80.0f32),
        (PropertyIdentifier::LOW_LIMIT, 20.0),
        (PropertyIdentifier::DEADBAND, 2.0),
    ] {
        ai.write_property(p, None, PropertyValue::Real(v), None)
            .unwrap();
    }
    ai.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![0xC0],
        },
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0x00], // nothing distributable
        },
        None,
    )
    .unwrap();
    // Nonzero Time_Delay: the per-write probe only seeds; the periodic task fires.
    ai.write_property(
        PropertyIdentifier::TIME_DELAY,
        None,
        PropertyValue::Unsigned(2),
        None,
    )
    .unwrap();
    ai.set_present_value(81.0); // already above high_limit
    let oid = ai.object_identifier();

    let mut db = ObjectDatabase::new();
    db.add(Box::new(ai)).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1,
            name: "Dev".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();

    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");

    // Any local write runs the post-write trigger path, which probes the
    // detector and seeds the pending transition without sending.
    server
        .write_local(
            &oid,
            PropertyIdentifier::DEADBAND,
            None,
            PropertyValue::Real(2.0),
            None,
        )
        .await
        .expect("local write should succeed");
    assert!(
        sent.lock().unwrap().is_empty(),
        "a nonzero Time_Delay must not send on the write itself"
    );

    // Past the delay: the periodic task ticks the countdown to zero and fires.
    tokio::time::sleep(Duration::from_secs(5)).await;

    assert!(
        sent.lock().unwrap().is_empty(),
        "Event_Enable cleared: the periodic Time_Delay path must not send, got {} broadcast(s)",
        sent.lock().unwrap().len()
    );

    // The transition did fire internally — only distribution was withheld.
    let db_guard = server.database().read().await;
    assert_eq!(
        db_guard
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw()),
        "the delayed transition must still have been confirmed internally"
    );
}

#[tokio::test(start_paused = true)]
async fn periodic_time_delay_carries_detector_event_type_to_wire() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let transport = RecordingTransport {
        sent_broadcast: StdArc::clone(&sent),
        local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
    };
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    for (property, value) in [
        (PropertyIdentifier::HIGH_LIMIT, 80.0),
        (PropertyIdentifier::LOW_LIMIT, 20.0),
        (PropertyIdentifier::DEADBAND, 2.0),
    ] {
        ai.write_property(property, None, PropertyValue::Real(value), None)
            .unwrap();
    }
    ai.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![0xC0],
        },
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0x80], // TO_OFFNORMAL at wire bit 0
        },
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::TIME_DELAY,
        None,
        PropertyValue::Unsigned(2),
        None,
    )
    .unwrap();
    ai.set_present_value(81.0);
    let oid = ai.object_identifier();

    let mut db = ObjectDatabase::new();
    db.add(Box::new(ai)).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1,
            name: "Dev".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    server
        .write_local(
            &oid,
            PropertyIdentifier::DEADBAND,
            None,
            PropertyValue::Real(2.0),
            None,
        )
        .await
        .expect("local write should seed delayed transition");
    tokio::time::sleep(Duration::from_secs(5)).await;

    let notif = decode_broadcast_notification(&sent);
    assert_eq!(
        notif.event_type,
        EventType::OUT_OF_RANGE.to_raw(),
        "the periodic path must preserve the detector's OUT_OF_RANGE type"
    );
}
