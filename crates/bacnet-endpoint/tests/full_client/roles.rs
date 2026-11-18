//! The full client beside the node's other roles: one pool of invoke IDs,
//! each answer to the role that asked, and who answers what.
use super::*;
use bacnet_client::client::COVNotificationDelivery;
use bacnet_encoding::constructed::encode_event_notification;
use bacnet_endpoint_core::coordinator::{CanonicalPeer, LeaseHolder};
use bacnet_services::alarm_event::EventNotificationRequest;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_types::constructed::PropertyReference;
use bacnet_types::enums::{
    ConfirmedServiceChoice, EventState, EventType, NotifyType, PropertyIdentifier, RejectReason,
};
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};
use std::sync::Arc;

/// The next notification `receiver` hears, failing after [`WAIT`] rather
/// than hanging when none comes.
async fn next<T: Clone>(receiver: &mut tokio::sync::broadcast::Receiver<T>) -> T {
    tokio::time::timeout(WAIT, receiver.recv())
        .await
        .expect("no notification within the wait")
        .unwrap()
}

/// The invoke IDs of the node's Confirmed-Requests for `service`, in order.
fn requests(log: &SendLog, service: ConfirmedServiceChoice) -> Vec<u8> {
    apdus(log)
        .into_iter()
        .filter_map(|(_, apdu)| match apdu {
            Apdu::ConfirmedRequest(request) if request.service_choice == service => {
                Some(request.invoke_id)
            }
            _ => None,
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_client_request_and_a_server_notification_share_one_invoke_id_pool() {
    let lab = lab(SessionRole::Both).await;
    let client = Arc::new(lab.session.full_client().unwrap());
    let (node, peer) = (Wire::mac(NODE), Wire::mac(PEER));
    // Another device subscribes to the node's AI-1 with confirmed
    // notifications, which the node's server role sends and that device
    // acknowledges.
    let other = other_client(&lab.wire).await;
    let mut heard = other.cov_notifications();
    other
        .subscribe_cov(&node, 7, ai1(), true, Some(600))
        .await
        .unwrap();
    next(&mut heard).await;
    eventually(WAIT, "the first notification acknowledged", async || {
        lab.session.active_leases() == 0
    })
    .await;

    for round in 0..20u8 {
        let sent_before = (
            requests(&lab.node_sent, ConfirmedServiceChoice::READ_PROPERTY).len(),
            requests(
                &lab.node_sent,
                ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            )
            .len(),
        );
        // Nothing reaches the node for now, so both requests stay open.
        lab.wire.hold(&node);
        let reader = Arc::clone(&client);
        let peer_mac = peer.clone();
        let read = tokio::spawn(async move {
            reader
                .read_property(&peer_mac, ai1(), PropertyIdentifier::PRESENT_VALUE, None)
                .await
        });
        lab.session
            .server()
            .unwrap()
            .set_present_value_local(&ai1(), PropertyValue::Real(30.0 + f32::from(round)))
            .await
            .unwrap();
        eventually(WAIT, "both answers on their way", async || {
            lab.wire.held(&node) == 2
        })
        .await;
        assert_eq!(lab.session.active_leases(), 2);
        let read_id =
            requests(&lab.node_sent, ConfirmedServiceChoice::READ_PROPERTY)[sent_before.0];
        let notification_id = requests(
            &lab.node_sent,
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        )[sent_before.1];
        assert_ne!(read_id, notification_id, "one pool, so never the same ID");

        // Each answer goes to the role that asked.
        lab.wire.release(&node);
        assert!(read.await.unwrap().is_ok());
        assert_eq!(
            next(&mut heard).await.delivery(),
            COVNotificationDelivery::Confirmed
        );
        eventually(WAIT, "the notification acknowledged", async || {
            lab.session.active_leases() == 0
        })
        .await;
    }
    // No notification was sent twice: each acknowledgment reached the
    // server role, not the client.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(
        requests(
            &lab.node_sent,
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION
        )
        .len(),
        21
    );
    assert_eq!(lab.session.policy_counters().await.unclaimed_terminal, 0);

    drop((client, other));
    stop(lab).await;
}

#[tokio::test(start_paused = true)]
async fn the_narrow_requester_and_the_full_client_work_at_once() {
    let lab = lab(SessionRole::Both).await;
    let client = Arc::new(lab.session.full_client().unwrap());
    let narrow = lab.session.cloned_client_handle().unwrap();
    let (node, peer) = (Wire::mac(NODE), Wire::mac(PEER));
    let coordinator = lab.session.coordinator();

    for round in 0..20usize {
        lab.wire.hold(&node);
        let (reader, to) = (Arc::clone(&client), peer.clone());
        let full = tokio::spawn(async move {
            reader
                .read_property(&to, ai1(), PropertyIdentifier::PRESENT_VALUE, None)
                .await
        });
        let (reader, to) = (narrow.clone(), peer.clone());
        let small = tokio::spawn(async move {
            reader
                .read_property(&to, ai1(), PropertyIdentifier::OBJECT_NAME, None)
                .await
        });
        eventually(WAIT, "both answers on their way", async || {
            lab.wire.held(&node) == 2
        })
        .await;
        // Two leases on the one coordinator, told apart by who holds them.
        let ids = requests(&lab.node_sent, ConfirmedServiceChoice::READ_PROPERTY);
        let ids = &ids[2 * round..];
        assert_eq!(ids.len(), 2);
        let mut holders: Vec<_> = ids
            .iter()
            .map(|id| coordinator.holder_of(&CanonicalPeer::direct(&peer), *id))
            .collect();
        holders.sort_by_key(|holder| *holder == Some(LeaseHolder::Client));
        assert_eq!(
            holders,
            [
                Some(LeaseHolder::EndpointRequester),
                Some(LeaseHolder::Client)
            ]
        );

        lab.wire.release(&node);
        let value = full.await.unwrap().unwrap();
        let name = small.await.unwrap().unwrap();
        assert_eq!(value.property_identifier, PropertyIdentifier::PRESENT_VALUE);
        assert_eq!(name.property_identifier, PropertyIdentifier::OBJECT_NAME);
    }
    assert_eq!(lab.session.active_leases(), 0);
    assert_eq!(lab.session.policy_counters().await.unclaimed_terminal, 0);

    drop(client);
    stop(lab).await;
}

#[tokio::test(start_paused = true)]
async fn a_peer_request_with_the_clients_outstanding_invoke_id_reaches_the_server() {
    // The client's lease is a (peer, invoke ID) pair in one direction; a
    // Confirmed-Request from that peer under the same number is the peer's
    // own transaction, so the server answers it and the client's lease is
    // left alone for the answer that is still to come.
    let lab = lab(SessionRole::Both).await;
    let client = Arc::new(lab.session.full_client().unwrap());
    let (node, peer) = (Wire::mac(NODE), Wire::mac(PEER));
    let coordinator = lab.session.coordinator();

    // The client's read waits at the peer.
    lab.wire.hold(&peer);
    let (reader, to) = (Arc::clone(&client), peer.clone());
    let read = tokio::spawn(async move {
        reader
            .read_property(&to, ai1(), PropertyIdentifier::PRESENT_VALUE, None)
            .await
    });
    eventually(WAIT, "the client's read held at the peer", async || {
        lab.wire.held(&peer) == 1
    })
    .await;
    let ids = requests(&lab.node_sent, ConfirmedServiceChoice::READ_PROPERTY);
    let [id] = ids[..] else {
        panic!("one read from the node: {ids:?}");
    };
    let held = || coordinator.holder_of(&CanonicalPeer::direct(&peer), id);
    assert_eq!(held(), Some(LeaseHolder::Client));

    // The peer asks the node under the same invoke ID.
    let device = oid(ObjectType::DEVICE, NODE_DEVICE);
    let request = confirmed(
        id,
        ConfirmedServiceChoice::READ_PROPERTY,
        &read_name(device),
    );
    lab.wire
        .deliver(&node, wire::frame(&peer, npdu_of(&request, true), false));
    let answered = || {
        apdus(&lab.node_sent).into_iter().any(|(to, apdu)| {
            to == peer
                && matches!(apdu, Apdu::ComplexAck(ack)
                    if ack.invoke_id == id
                        && ack.service_choice == ConfirmedServiceChoice::READ_PROPERTY)
        })
    };
    eventually(WAIT, "the server's answer under that ID", async || {
        answered()
    })
    .await;
    assert_eq!(
        held(),
        Some(LeaseHolder::Client),
        "the client's lease stays"
    );
    assert!(!read.is_finished());

    // The peer's answer to the client then completes the client's read.
    lab.wire.release(&peer);
    let value = read.await.unwrap().unwrap();
    assert_eq!(value.object_identifier, ai1());
    assert_eq!(value.property_identifier, PropertyIdentifier::PRESENT_VALUE);
    assert_eq!(held(), None);
    assert_eq!(lab.session.active_leases(), 0);
    assert_eq!(lab.session.policy_counters().await.unclaimed_terminal, 0);

    drop(client);
    stop(lab).await;
}

#[tokio::test(start_paused = true)]
async fn a_confirmed_cov_notification_reaches_the_full_clients_subscription() {
    let lab = lab(SessionRole::Both).await;
    let client = lab.session.full_client().unwrap();
    let peer = Wire::mac(PEER);
    let mut notifications = client.cov_notifications();

    client
        .subscribe_cov(&peer, 9, ai1(), true, Some(600))
        .await
        .unwrap();
    let first = next(&mut notifications).await;
    assert_eq!(first.delivery(), COVNotificationDelivery::Confirmed);
    assert_eq!(first.source_mac(), &peer);
    assert!(first.object().is_some());
    lab.peer
        .set_present_value_local(&ai1(), PropertyValue::Real(30.0))
        .await
        .unwrap();
    let second = next(&mut notifications).await;
    assert_eq!(second.subscriber_process_identifier(), 9);

    // The client acknowledged each one, once, and the node's server role
    // answered neither: the peer had no cause to send one again.
    tokio::time::sleep(Duration::from_secs(10)).await;
    let answers: Vec<Apdu> = apdus(&lab.node_sent)
        .into_iter()
        .map(|(_, apdu)| apdu)
        .filter(|apdu| !matches!(apdu, Apdu::ConfirmedRequest(_)))
        .collect();
    assert_eq!(answers.len(), 2, "{answers:?}");
    for answer in answers {
        let Apdu::SimpleAck(ack) = answer else {
            panic!("not a Simple-ACK: {answer:?}");
        };
        assert_eq!(
            ack.service_choice,
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION
        );
    }
    assert_eq!(
        requests(
            &lab.peer_sent,
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION
        )
        .len(),
        2
    );

    drop(client);
    stop(lab).await;
}

/// A SubscribeCOVPropertyMultiple to the peer's AI-1 and AV-1, their
/// Present_Values, with confirmed notifications, timestamped when
/// `timestamped`.
fn present_values(process: u32, timestamped: bool) -> SubscribeCOVPropertyMultipleRequest {
    let reference = COVReference {
        monitored_property: PropertyReference {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        },
        cov_increment: None,
        timestamped,
    };
    SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: process,
        issue_confirmed_notifications: true,
        lifetime: Some(600),
        max_notification_delay: Some(1),
        list_of_cov_subscription_specifications: [ai1(), oid(ObjectType::ANALOG_VALUE, 1)]
            .into_iter()
            .map(|object| COVSubscriptionSpecification {
                monitored_object_identifier: object,
                list_of_cov_references: vec![reference.clone()],
            })
            .collect(),
    }
}

#[tokio::test(start_paused = true)]
async fn a_confirmed_cov_multiple_notification_reaches_the_full_client() {
    // Beside the node's server role, the classifier hands the full client a
    // ConfirmedCOVNotificationMultiple as it does a ConfirmedCOVNotification
    // (#2654): the client acknowledges it, and the server role sees none.
    let lab = lab(SessionRole::Both).await;
    let client = lab.session.full_client().unwrap();
    let peer = Wire::mac(PEER);
    let mut notifications = client.cov_notifications();

    client
        .subscribe_cov_property_multiple(&peer, &present_values(10, true))
        .await
        .unwrap();
    let first = next(&mut notifications).await;
    let received = first.multiple().expect("a COV-multiple notification");
    assert_eq!(received.delivery, COVNotificationDelivery::Confirmed);
    assert_eq!(received.source_mac, peer);
    assert_eq!(received.notification.subscriber_process_identifier, 10);
    assert!(received.notification.timestamp.is_some());
    lab.peer
        .set_present_value_local(&ai1(), PropertyValue::Real(30.0))
        .await
        .unwrap();
    let second = next(&mut notifications).await;
    assert_eq!(second.subscriber_process_identifier(), 10);
    assert!(second.multiple().is_some_and(|received| {
        received
            .notification
            .list_of_cov_notifications
            .iter()
            .any(|object| object.monitored_object_identifier == ai1())
    }));

    // Each was acknowledged once, by the client, and the peer had no cause
    // to send one again.
    tokio::time::sleep(Duration::from_secs(10)).await;
    let multiple = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE;
    let answers: Vec<Apdu> = apdus(&lab.node_sent)
        .into_iter()
        .map(|(_, apdu)| apdu)
        .filter(|apdu| !matches!(apdu, Apdu::ConfirmedRequest(_)))
        .collect();
    let sent = requests(&lab.peer_sent, multiple);
    let distinct: std::collections::BTreeSet<_> = sent.iter().collect();
    assert!(sent.len() >= 2, "{sent:?}");
    assert_eq!(
        distinct.len(),
        sent.len(),
        "a notification went twice: {sent:?}"
    );
    assert_eq!(answers.len(), sent.len(), "{answers:?}");
    for answer in answers {
        let Apdu::SimpleAck(ack) = answer else {
            panic!("not a Simple-ACK: {answer:?}");
        };
        assert_eq!(ack.service_choice, multiple);
    }

    drop(client);
    stop(lab).await;
}

/// A ConfirmedEventNotification's parameters.
fn event_notification() -> Vec<u8> {
    let request = EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: oid(ObjectType::DEVICE, 3003),
        event_object_identifier: oid(ObjectType::ANALOG_INPUT, 3),
        timestamp: BACnetTimeStamp::SequenceNumber(5),
        notification_class: 0,
        priority: 100,
        event_type: EventType::OUT_OF_RANGE,
        message_text: Some("too hot".into()),
        notify_type: NotifyType::ALARM,
        ack_required: false,
        from_state: EventState::NORMAL,
        to_state: EventState::HIGH_LIMIT,
        event_values: None,
    };
    let mut parameters = BytesMut::new();
    encode_event_notification(&request, &mut parameters).unwrap();
    parameters.to_vec()
}

/// Three confirmed requests from node [`OTHER`] to the node under test: a
/// service its server role doesn't execute, an event notification, and a
/// read of its Device's name. Returns what the node sent back, in order.
async fn ask_the_node(lab: &Lab) -> Vec<Apdu> {
    let (node, other) = (Wire::mac(NODE), Wire::mac(OTHER));
    let requests = [
        confirmed(1, ConfirmedServiceChoice::ATOMIC_READ_FILE, &[]),
        confirmed(
            2,
            ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
            &event_notification(),
        ),
        confirmed(
            3,
            ConfirmedServiceChoice::READ_PROPERTY,
            &read_name(oid(ObjectType::DEVICE, NODE_DEVICE)),
        ),
    ];
    for request in &requests {
        lab.wire
            .deliver(&node, wire::frame(&other, npdu_of(request, true), false));
        // Each is answered, or left unanswered, before the next arrives.
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    apdus(&lab.node_sent)
        .into_iter()
        .filter(|(to, _)| *to == other)
        .map(|(_, apdu)| apdu)
        .collect()
}

#[tokio::test(start_paused = true)]
async fn beside_a_server_role_the_node_answers_as_it_does_with_no_full_client() {
    // What the node sends with no full client.
    let alone = lab(SessionRole::Both).await;
    let expected = ask_the_node(&alone).await;
    // The full server rejects the file read, whose parameters are missing,
    // acknowledges the event notification and reads its Device.
    let [
        Apdu::Reject(file),
        Apdu::SimpleAck(event),
        Apdu::ComplexAck(read),
    ] = &expected[..]
    else {
        panic!("the server's answers: {expected:?}");
    };
    assert_eq!(file.invoke_id, 1);
    assert_ne!(file.reject_reason, RejectReason::UNRECOGNIZED_SERVICE);
    assert_eq!(event.invoke_id, 2);
    assert_eq!(read.invoke_id, 3);
    stop(alone).await;

    // With one, the octets are the same: the client adds no reject and no
    // acknowledgment of its own.
    let lab = lab(SessionRole::Both).await;
    let client = lab.session.full_client().unwrap();
    let mut events = client.event_notifications();
    assert_eq!(ask_the_node(&lab).await, expected);
    // The client still heard the event notification the server role had to
    // answer.
    let event = events.try_recv().expect("the event notification");
    assert_eq!(event.source_mac, Wire::mac(OTHER));
    assert!(events.try_recv().is_err());

    drop(client);
    stop(lab).await;
}

#[tokio::test(start_paused = true)]
async fn with_no_server_role_the_full_client_answers_as_a_standalone_client_does() {
    let lab = lab(SessionRole::ClientOnly).await;
    // With no full client a client-only session answers nothing.
    assert_eq!(ask_the_node(&lab).await, []);

    let client = lab.session.full_client().unwrap();
    let mut events = client.event_notifications();
    let answers = ask_the_node(&lab).await;
    let [
        Apdu::Reject(file),
        Apdu::SimpleAck(event),
        Apdu::Reject(read),
    ] = &answers[..]
    else {
        panic!("a standalone client's answers: {answers:?}");
    };
    assert_eq!(
        (file.invoke_id, file.reject_reason),
        (1, RejectReason::UNRECOGNIZED_SERVICE)
    );
    assert_eq!(event.invoke_id, 2);
    assert_eq!(
        (read.invoke_id, read.reject_reason),
        (3, RejectReason::UNRECOGNIZED_SERVICE)
    );
    assert!(events.try_recv().is_ok());

    // An Unconfirmed-Request still goes to the session itself, with a copy
    // for the client. A client-only session has no server to answer a
    // Who-Is, so the session's own count of requests that found no server
    // role is what shows it was handed one.
    let before = (
        lab.session.policy_counters().await.no_server_role,
        lab.session.client_seat_counters().apdus,
    );
    let who_is = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
        service_choice: bacnet_types::enums::UnconfirmedServiceChoice::WHO_IS,
        service_request: Bytes::new(),
    });
    lab.wire
        .broadcast(&Wire::mac(OTHER), npdu_of(&who_is, false));
    eventually(
        WAIT,
        "the Who-Is at the session and at the seat",
        async || {
            let now = (
                lab.session.policy_counters().await.no_server_role,
                lab.session.client_seat_counters().apdus,
            );
            now == (before.0 + 1, before.1 + 1)
        },
    )
    .await;

    drop(client);
    stop(lab).await;
}
