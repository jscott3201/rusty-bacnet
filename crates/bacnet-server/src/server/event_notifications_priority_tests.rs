use super::*;

/// Per-transition `Priority` from the NotificationClass is projected into
/// the broadcast EventNotification (TO_OFFNORMAL -> PRIORITY[0] = 50),
/// not the legacy hardcoded 100.
#[tokio::test]
async fn event_notification_projects_offnormal_priority_from_class() {
    let (db, network, comm_state, learned_routers, sent, oid) = fixture_with_commanded_nc(
        5,
        [50, 150, 250],
        EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_NORMAL,
    )
    .await;

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
            device: crate::server::event_delivery::fixture_device(&db).await,
        },
        &oid,
        (change, EventType::OUT_OF_RANGE),
    )
    .await;

    let notif = decode_broadcast_notification(&sent.npdus());
    assert_eq!(notif.priority, 50, "TO_OFFNORMAL priority from PRIORITY[0]");
    assert!(
        notif.ack_required,
        "TO_OFFNORMAL ack_required from ACK_REQUIRED bit 0"
    );
}

/// TO_FAULT projects PRIORITY[1] and ACK_REQUIRED bit 1.
#[tokio::test]
async fn event_notification_projects_fault_priority_from_class() {
    let (db, network, comm_state, learned_routers, sent, oid) = fixture_with_commanded_nc(
        5,
        [50, 150, 250],
        EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_NORMAL,
    )
    .await;

    let change = EventStateChange {
        from: EventState::NORMAL,
        to: EventState::FAULT,
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
            device: crate::server::event_delivery::fixture_device(&db).await,
        },
        &oid,
        (change, EventType::CHANGE_OF_RELIABILITY),
    )
    .await;

    let notif = decode_broadcast_notification(&sent.npdus());
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
        EventType::CHANGE_OF_RELIABILITY,
        "TO_FAULT must be reported as CHANGE_OF_RELIABILITY"
    );
}

/// The from-FAULT direction, which Clauses 13.8 and 13.9 state separately from
/// the to-FAULT case: departure from FAULT also requires
/// CHANGE_OF_RELIABILITY as the Event Type.
///
/// Worth its own test because the transition coordinate differs — this is a
/// TO_NORMAL transition for Priority and Ack_Required purposes, while still
/// being CHANGE_OF_RELIABILITY for Event Type. A fix that keyed the event type
/// off the transition category rather than the states would get this wrong.
#[tokio::test]
async fn event_notification_from_fault_is_change_of_reliability() {
    let (db, network, comm_state, learned_routers, sent, oid) = fixture_with_commanded_nc(
        5,
        [50, 150, 250],
        EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_NORMAL,
    )
    .await;

    let change = EventStateChange {
        from: EventState::FAULT,
        to: EventState::NORMAL,
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
            device: crate::server::event_delivery::fixture_device(&db).await,
        },
        &oid,
        (change, EventType::CHANGE_OF_RELIABILITY),
    )
    .await;

    let notif = decode_broadcast_notification(&sent.npdus());
    assert_eq!(
        notif.event_type,
        EventType::CHANGE_OF_RELIABILITY,
        "a transition FROM FAULT is also CHANGE_OF_RELIABILITY"
    );
    // ...while the transition coordinate is still TO_NORMAL.
    assert_eq!(notif.priority, 250, "TO_NORMAL priority from PRIORITY[2]");
}

/// TO_NORMAL projects PRIORITY[2] (250), not the legacy hardcoded 200.
#[tokio::test]
async fn event_notification_projects_normal_priority_from_class() {
    let (db, network, comm_state, learned_routers, sent, oid) = fixture_with_commanded_nc(
        5,
        [50, 150, 250],
        EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_NORMAL,
    )
    .await;

    let change = EventStateChange {
        from: EventState::HIGH_LIMIT,
        to: EventState::NORMAL,
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
            device: crate::server::event_delivery::fixture_device(&db).await,
        },
        &oid,
        (change, EventType::OUT_OF_RANGE),
    )
    .await;

    let notif = decode_broadcast_notification(&sent.npdus());
    assert_eq!(notif.priority, 250, "TO_NORMAL priority from PRIORITY[2]");
    assert!(
        notif.ack_required,
        "TO_NORMAL ack_required from ACK_REQUIRED bit 2"
    );
}
