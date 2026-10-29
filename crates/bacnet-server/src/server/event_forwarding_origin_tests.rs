//! The two other ways into the Notification Forwarders (#1225): a confirmed
//! notification, answered at once whatever forwarding then does, and a
//! notification this device's own object addresses to its own Device.

use super::confirmed_request_tracker::ConfirmedRequestTracker;
use super::event_forwarding_tests::{
    Copy, LOCAL_DEVICE, PEER_A, PEER_B, To, copies, database, destination, encoded,
    forwarding_transport, notification, unconfirmed,
};
use super::event_recipient_routing_tests::{address_recipient, distribute_counted};
use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::notification_forwarder::NotificationForwarderObject;
use bacnet_types::constructed::BACnetRecipient;

fn this_device() -> BACnetRecipient {
    BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, LOCAL_DEVICE).unwrap())
}

pub(super) fn confirmed_services(
    db: ObjectDatabase,
    transport: crate::server::test_transport::TestTransport,
) -> RequestServices<crate::server::test_transport::TestTransport> {
    RequestServices {
        db: Arc::new(RwLock::new(db)),
        ..RequestServices::for_test(
            Arc::new(NetworkLayer::new(transport)),
            ServerConfig::default(),
        )
    }
}

/// Dispatch one ConfirmedEventNotification (invoke ID 7) from `PEER_B` and
/// hand back the reply channel's NPDU.
async fn dispatch(
    services: RequestServices<crate::server::test_transport::TestTransport>,
    request: Bytes,
    reply: oneshot::Sender<Bytes>,
) {
    BACnetServer::handle_confirmed_request(
        &services,
        &Arc::new(ConfirmedRequestTracker::default()),
        &Arc::new(super::request_tasks::RequestTasks::default()).spawner(),
        &PEER_B,
        None,
        ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 7,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
            service_request: request,
        },
        Some(reply),
    )
    .await;
}

pub(super) fn reply_apdu(npdu: Bytes) -> Apdu {
    decode_apdu(decode_npdu(npdu).unwrap().payload).unwrap()
}

pub(super) fn is_simple_ack(apdu: &Apdu) -> bool {
    matches!(
        apdu,
        Apdu::SimpleAck(SimpleAck {
            invoke_id: 7,
            service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        })
    )
}

#[tokio::test]
async fn confirmed_notification_is_acknowledged_while_its_copy_is_still_sending() {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    nf.add_destination(destination(address_recipient(0, &PEER_A), 40, false))
        .unwrap();
    let transport = forwarding_transport();
    let handle = transport.handle();
    let sent = transport.sent();
    handle.block_next_send();
    let services = confirmed_services(database(vec![nf]), transport);
    let (reply, answered) = oneshot::channel();
    let request = notification(5);
    let task = tokio::spawn(dispatch(services, encoded(&request), reply));

    // The sender has its acknowledgment while the forwarded copy is held in
    // the transport.
    let ack = tokio::time::timeout(Duration::from_secs(5), answered)
        .await
        .expect("acknowledged before forwarding finished")
        .unwrap();
    assert!(is_simple_ack(&reply_apdu(ack)));
    handle.wait_blocked().await;
    assert!(!task.is_finished(), "the copy is still being sent");
    handle.release_sends(1);
    task.await.unwrap();
    assert_eq!(
        copies(&sent, &request),
        [unconfirmed(To::Local(PEER_A.to_vec()), 40)]
    );
}

#[tokio::test]
async fn confirmed_notification_is_acknowledged_whatever_forwarding_finds() {
    // No forwarder at all: acknowledged, and counted as not forwarded.
    let (reply, answered) = oneshot::channel();
    let transport = forwarding_transport();
    let sent = transport.sent();
    let services = confirmed_services(database(Vec::new()), transport);
    let suppressions = Arc::clone(&services.event_suppressions);
    dispatch(services, encoded(&notification(5)), reply).await;
    assert!(is_simple_ack(&reply_apdu(answered.await.unwrap())));
    assert!(sent.is_empty());
    assert_eq!(
        suppressions.snapshot(),
        EventNotificationCounters {
            received_not_forwarded: 1,
            ..Default::default()
        }
    );

    // A forwarder whose only destination cannot be routed: the skip is
    // counted, and the sender's answer does not change. The server has no
    // binding for the Device, so it asks for it first (#1368), and counts
    // the skip once its device stays silent.
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    nf.add_destination(destination(
        BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 77).unwrap()),
        40,
        false,
    ))
    .unwrap();
    let transport = forwarding_transport();
    let sent = transport.sent();
    let services = confirmed_services(database(vec![nf]), transport);
    let suppressions = Arc::clone(&services.event_suppressions);
    let bindings = Arc::clone(&services.device_bindings);
    let (reply, answered) = oneshot::channel();
    dispatch(services, encoded(&notification(5)), reply).await;
    assert!(is_simple_ack(&reply_apdu(answered.await.unwrap())));
    let frames = sent.frames();
    assert!(frames.len() == 1 && super::event_recipient_routing_tests::is_who_is(&frames[0]));
    bindings.write().await.probes.run_out_for_test();
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert_eq!(sent.len(), 1);
    assert_eq!(suppressions.snapshot().device_recipient_unbound, 1);

    // A request the forwarders cannot read is rejected with the reason
    // naming its fault, as the client does (#1446): cut short, a missing
    // parameter.
    let (reply, answered) = oneshot::channel();
    dispatch(
        confirmed_services(database(Vec::new()), forwarding_transport()),
        encoded(&notification(5)).slice(..10),
        reply,
    )
    .await;
    assert!(matches!(
        reply_apdu(answered.await.unwrap()),
        Apdu::Reject(RejectPdu {
            invoke_id: 7,
            reject_reason: RejectReason::MISSING_REQUIRED_PARAMETER,
        })
    ));
}

/// Read back the copies a local transition sent, as `copies` reads them.
fn local_copies(broadcasts: Vec<Bytes>, unicasts: Vec<(Vec<u8>, Bytes)>) -> Vec<Copy> {
    let read = |npdu: Bytes| {
        let npdu = decode_npdu(npdu).unwrap();
        let Apdu::UnconfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
            panic!("expected an unconfirmed notification");
        };
        let notification = decode_event_notification(&request.service_request).unwrap();
        (npdu.destination, notification.process_identifier)
    };
    let mut sent: Vec<Copy> = unicasts
        .into_iter()
        .map(|(mac, npdu)| {
            let (destination, process) = read(npdu);
            assert!(destination.is_none());
            unconfirmed(To::Local(mac), process)
        })
        .collect();
    for npdu in broadcasts {
        let (destination, process) = read(npdu);
        let to = match destination {
            None => To::LocalBroadcast,
            Some(dest) if dest.network == 0xFFFF => To::Global,
            Some(dest) => To::RemoteBroadcast(dest.network),
        };
        sent.push(unconfirmed(to, process));
    }
    sent
}

#[tokio::test]
async fn local_notification_reaches_the_forwarders_its_class_names() {
    let mut class = NotificationClass::new(0, "NC-0").unwrap();
    class
        .add_destination(destination(this_device(), 5, false))
        .unwrap();
    class
        .add_destination(destination(address_recipient(0, &PEER_B), 6, false))
        .unwrap();
    // Takes only this device's notifications with process 5, and may send
    // them by local broadcast, but never by global broadcast.
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    nf.set_process_identifier_filter(Some(5));
    nf.set_local_forwarding_only(true);
    for (process, recipient) in [
        (9, address_recipient(0, &PEER_A)),
        (10, address_recipient(0xFFFF, &[])),
        (11, address_recipient(0, &[])),
    ] {
        nf.add_destination(destination(recipient, process, false))
            .unwrap();
    }
    // Takes process 6 only, which the class sends to a peer, not to this
    // device.
    let mut other = NotificationForwarderObject::new(2, "NF-6").unwrap();
    other.set_process_identifier_filter(Some(6));
    other
        .add_destination(destination(address_recipient(0, &PEER_A), 12, false))
        .unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(class)).unwrap();
    db.add(Box::new(nf)).unwrap();
    db.add(Box::new(other)).unwrap();

    let (broadcasts, unicasts, counters) = distribute_counted(
        db,
        Arc::new(RwLock::new(
            super::device_bindings::DeviceBindingTable::new(),
        )),
        DccState::Enable,
    )
    .await;
    assert_eq!(
        local_copies(broadcasts, unicasts),
        [
            unconfirmed(To::Local(PEER_B.to_vec()), 6),
            unconfirmed(To::Local(PEER_A.to_vec()), 9),
            unconfirmed(To::LocalBroadcast, 11),
        ]
    );
    // This device's own Device recipient is delivered, not an unbound skip.
    assert_eq!(counters, EventNotificationCounters::default());
}
