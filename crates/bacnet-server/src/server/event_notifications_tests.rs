use super::*;
use crate::server::test_transport::{SendLog, SendMode, TestTransport, BIP_LOCAL_MAC};
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event::EventStateChange;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::enums::{EngineeringUnits, EventState, EventType};
use bytes::Bytes;

#[path = "event_notifications_commit_tests.rs"]
mod commit_tests;

#[path = "event_notifications_history_tests.rs"]
mod history_tests;

#[path = "event_message_policy_tests.rs"]
mod message_policy_tests;

#[path = "event_options_tests.rs"]
mod options_tests;

#[path = "event_notifications_priority_tests.rs"]
mod priority_tests;

/// A transport that records every broadcast NPDU it is asked to send and
/// discards unicasts. Used to capture the EventNotification a server
/// actually puts on the wire.
pub(super) fn recording_transport() -> (TestTransport, SendLog) {
    let transport = TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .unicast(SendMode::Ignore)
        .build();
    let sent = transport.sent();
    (transport, sent)
}

/// A server under DISABLE_INITIATION suppresses the periodic event
/// send: `build_and_send_event_notification` returns without sending,
/// matching the per-write path's DCC gate. Verified against a recording
/// transport that would otherwise capture the broadcast APDU.
#[tokio::test]
async fn dcc_suppresses_periodic_event_send() {
    let (transport, sent) = recording_transport();
    let network = Arc::new(NetworkLayer::new(transport));
    let comm_state = Arc::new(CommState::default());
    comm_state.set_for_test(DccState::DisableInitiation);
    let learned_routers = Arc::new(Mutex::new(LearnedRouterCache::new()));

    let mut db = clocked_test_database();
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::SQUARE_METERS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    // A recipient that WOULD be broadcast to, so the empty assertion below can
    // only be satisfied by the DCC gate. Without this the test passes because
    // no recipient was named, and it stays green even with the gate removed.
    db.add(Box::new(notification_class_0_broadcasting()))
        .unwrap();
    let db = Arc::new(tokio::sync::RwLock::new(db));
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let change = EventStateChange {
        from: EventState::NORMAL,
        to: EventState::HIGH_LIMIT,
    };
    BACnetServer::<TestTransport>::build_and_send_event_notification_with_bindings(
        &crate::server::event_delivery::EventDelivery {
            db: &db,
            network: &network,
            comm_state: &comm_state,
            learned_routers: &learned_routers,
            notification_transactions: &NotificationTransactions::new(),
            device_bindings: &Arc::new(RwLock::new(
                crate::server::device_bindings::DeviceBindingTable::new(),
            )),
            suppressions: &Default::default(),
            retry_timeout_ms: 1000,
            local_apdu_capacity: 1476,
        },
        &oid,
        (change, EventType::OUT_OF_RANGE),
    )
    .await;

    assert!(
        sent.is_empty(),
        "DCC-disabled server must not send event notifications"
    );
}

/// Decode the single broadcast EventNotification captured by a
/// [`recording_transport`] into its [`EventNotificationRequest`].
///
/// Panics with a useful message if no notification was sent (so a regression
/// that silently drops the notification is caught rather than masking as
/// "no broadcast = pass").
pub(super) fn decode_broadcast_notification(sent: &[Bytes]) -> EventNotificationRequest {
    use bacnet_encoding::apdu::decode_apdu;
    use bacnet_encoding::npdu::decode_npdu;

    assert_eq!(
        sent.len(),
        1,
        "expected exactly one broadcast EventNotification, got {}",
        sent.len()
    );
    let npdu = decode_npdu(sent[0].clone()).expect("decode NPDU");
    match decode_apdu(npdu.payload).expect("decode APDU") {
        Apdu::UnconfirmedRequest(req) => {
            assert_eq!(
                req.service_choice,
                UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION
            );
            decode_event_notification(&req.service_request).expect("decode EventNotification")
        }
        other => panic!("expected UnconfirmedRequest, got {other:?}"),
    }
}

/// The `Recipient_List` entry Clause 12.21 prescribes for a device whose list
/// is not writable: a local broadcast (network 0, zero-length MAC), all days,
/// the full daily window, process identifier 0, unconfirmed, all transitions.
///
/// Notifications reach the recording transport's broadcast wire *because this
/// entry asks them to*. An empty `Recipient_List` names no notification-clients
/// and so distributes nothing (Clause 13.2.5).
pub(super) fn local_broadcast_destination() -> bacnet_types::constructed::BACnetDestination {
    use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
    use bacnet_types::primitives::Time;
    BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        to_time: Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths: 99,
        },
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::new(),
        }),
        process_identifier: 0,
        issue_confirmed_notifications: false,
        transitions: EventTransitionBits::all(),
    }
}

/// Notification Class instance 0 — the class an object points at when its
/// `Notification_Class` property is left at its default — carrying the single
/// local-broadcast recipient from [`local_broadcast_destination`].
pub(super) fn notification_class_0_broadcasting(
) -> bacnet_objects::notification_class::NotificationClass {
    let mut nc = bacnet_objects::notification_class::NotificationClass::new(0, "NC-0").unwrap();
    nc.add_destination(local_broadcast_destination()).unwrap();
    nc
}

/// Build a server fixture: a Device, a NotificationClass (instance `nc`, whose
/// recipient list holds the Clause 12.21 local-broadcast entry) with the given
/// per-transition `priority` and `ack_required` bits, and an AnalogInput whose
/// `Notification_Class` points at it with `Notify_Type = ALARM`.
async fn fixture_with_commanded_nc(
    nc: u32,
    priority: [u8; 3],
    ack_required: EventTransitionBits,
) -> (
    Arc<RwLock<ObjectDatabase>>,
    Arc<NetworkLayer<TestTransport>>,
    Arc<CommState>,
    Arc<Mutex<LearnedRouterCache>>,
    SendLog,
    ObjectIdentifier,
) {
    let (transport, sent) = recording_transport();
    let network = Arc::new(NetworkLayer::new(transport));
    let comm_state = Arc::new(CommState::default()); // DCC enabled
    let learned_routers = Arc::new(Mutex::new(LearnedRouterCache::new()));

    let mut db = clocked_test_database();
    // NotificationClass with the configured per-transition arrays and a single
    // local-broadcast recipient, so the notification lands on the broadcast wire.
    let mut notification_class =
        bacnet_objects::notification_class::NotificationClass::new(nc, "NC").unwrap();
    notification_class.priority = priority;
    notification_class.ack_required = ack_required;
    notification_class
        .add_destination(local_broadcast_destination())
        .unwrap();
    db.add(Box::new(notification_class)).unwrap();

    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();

    // AnalogInput pointing at the NotificationClass, ALARM notify type.
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::SQUARE_METERS).unwrap();
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
    (db, network, comm_state, learned_routers, sent, oid)
}

/// A `Notification_Class` naming an object that does not exist distributes
/// nothing.
///
/// The Standard does not say what a `Notification_Class` naming a nonexistent
/// object should do — Clause 13.2.5 governs distribution given a Recipient_List,
/// and Clause 12 defines the property without a missing-object rule. So this is
/// a deliberate choice for an undefined configuration, not a mandate.
///
/// Silence is the defensible reading: a class that does not exist supplies no
/// Recipient_List, and 13.2.5 restricts distribution to that input's
/// notification-clients. The alternative was the previous
/// behavior, where a misconfigured `Notification_Class` broadcast the alarm to
/// every device on the link.
#[tokio::test]
async fn event_notification_missing_class_distributes_nothing() {
    let (transport, sent) = recording_transport();
    let network = Arc::new(NetworkLayer::new(transport));
    let comm_state = Arc::new(CommState::default());
    let learned_routers = Arc::new(Mutex::new(LearnedRouterCache::new()));

    let mut db = clocked_test_database();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    // AI points at NotificationClass 999, which does not exist.
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::SQUARE_METERS).unwrap();
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
    BACnetServer::<TestTransport>::build_and_send_event_notification_with_bindings(
        &crate::server::event_delivery::EventDelivery {
            db: &db,
            network: &network,
            comm_state: &comm_state,
            learned_routers: &learned_routers,
            notification_transactions: &NotificationTransactions::new(),
            device_bindings: &Arc::new(RwLock::new(
                crate::server::device_bindings::DeviceBindingTable::new(),
            )),
            suppressions: &Default::default(),
            retry_timeout_ms: 1000,
            local_apdu_capacity: 1476,
        },
        &oid,
        (change, EventType::OUT_OF_RANGE),
    )
    .await;

    assert!(
        sent.is_empty(),
        "a Notification_Class that does not exist names no recipients, so \
         nothing may be distributed"
    );
}

/// Notify_Type = EVENT still honors the per-transition Ack_Required (it is
/// not ALARM-specific); the legacy code derived ack_required purely from
/// `Notify_Type == ALARM`, so an EVENT notification would wrongly clear it.
#[tokio::test]
async fn event_notification_event_notify_type_honors_class_ack_required() {
    let (db, network, comm_state, learned_routers, sent, oid) = fixture_with_commanded_nc(
        5,
        [50, 150, 250],
        EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_NORMAL,
    )
    .await;
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
    BACnetServer::<TestTransport>::build_and_send_event_notification_with_bindings(
        &crate::server::event_delivery::EventDelivery {
            db: &db,
            network: &network,
            comm_state: &comm_state,
            learned_routers: &learned_routers,
            notification_transactions: &NotificationTransactions::new(),
            device_bindings: &Arc::new(RwLock::new(
                crate::server::device_bindings::DeviceBindingTable::new(),
            )),
            suppressions: &Default::default(),
            retry_timeout_ms: 1000,
            local_apdu_capacity: 1476,
        },
        &oid,
        (change, EventType::OUT_OF_RANGE),
    )
    .await;

    let notif = decode_broadcast_notification(&sent.npdus());
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
pub(super) fn db_with_high_limit_transition(
    event_enable_byte: u8,
) -> Arc<tokio::sync::RwLock<ObjectDatabase>> {
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
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

    let mut db = clocked_test_database();
    db.add(Box::new(ai)).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    // The AnalogInput's Notification_Class defaults to 0, so class 0 has to
    // exist and name a recipient for anything to be distributed.
    db.add(Box::new(notification_class_0_broadcasting()))
        .unwrap();
    Arc::new(tokio::sync::RwLock::new(db))
}

/// Drive the per-write path once and return the broadcasts it produced.
pub(super) async fn broadcasts_from_per_write_path(
    db: &Arc<tokio::sync::RwLock<ObjectDatabase>>,
    state: DccState,
) -> Vec<Bytes> {
    let (transport, sent) = recording_transport();
    let network = Arc::new(NetworkLayer::new(transport));
    let comm_state = Arc::new(CommState::default());
    comm_state.set_for_test(state);
    let learned_routers = Arc::new(Mutex::new(LearnedRouterCache::new()));
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    BACnetServer::<TestTransport>::fire_event_notifications_with_bindings(
        &crate::server::event_delivery::EventDelivery {
            db,
            network: &network,
            comm_state: &comm_state,
            learned_routers: &learned_routers,
            notification_transactions: &NotificationTransactions::new(),
            device_bindings: &Arc::new(RwLock::new(
                crate::server::device_bindings::DeviceBindingTable::new(),
            )),
            suppressions: &Default::default(),
            retry_timeout_ms: 1000,
            local_apdu_capacity: 1476,
        },
        &Arc::new(RwLock::new(crate::cov::CovSubscriptionTable::new())),
        &oid,
    )
    .await;

    sent.npdus()
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
    let sent = broadcasts_from_per_write_path(&db, DccState::Enable).await;

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
    let (transport, sent) = recording_transport();

    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
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

    let mut db = clocked_test_database();
    db.add(Box::new(ai)).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    // A recipient that WOULD be broadcast to, so both assertions below can only
    // be satisfied by the Event_Enable gate rather than by an unnamed recipient.
    db.add(Box::new(notification_class_0_broadcasting()))
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
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .expect("local write should succeed");
    assert!(
        sent.is_empty(),
        "a nonzero Time_Delay must not send on the write itself"
    );

    // Past the delay: the periodic task ticks the countdown to zero and fires.
    tokio::time::sleep(Duration::from_secs(5)).await;

    assert!(
        sent.is_empty(),
        "Event_Enable cleared: the periodic Time_Delay path must not send, got {} broadcast(s)",
        sent.len()
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
    let (transport, sent) = recording_transport();
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
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

    let mut db = clocked_test_database();
    db.add(Box::new(ai)).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(notification_class_0_broadcasting()))
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
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .expect("local write should seed delayed transition");
    tokio::time::sleep(Duration::from_secs(5)).await;

    let notif = decode_broadcast_notification(&sent.npdus());
    assert_eq!(
        notif.event_type,
        EventType::OUT_OF_RANGE,
        "the periodic path must preserve the detector's OUT_OF_RANGE type"
    );
}
