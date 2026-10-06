//! Bounds on what one received notification makes the forwarders send
//! (#1258, #1259, #1299): a recipient naming this device's own network by
//! number is local to the loop rules and is sent to with no DNET, the
//! destinations per notification are capped across all forwarders, and a
//! retransmitted confirmed notification is acknowledged without being
//! forwarded again.

use super::confirmed_request_tracker::ConfirmedRequestTracker;
use super::event_forwarding::Reception;
use super::event_forwarding_origin_tests::{confirmed_services, reply_apdu};
use super::event_forwarding_tests::{
    copies, database, destination, encoded, forwarding_transport, notification, unconfirmed,
    Forwarding, To, LOCAL_DEVICE, PEER_A, PEER_B,
};
use super::event_recipient_routing_tests::address_recipient;
use super::test_transport::{SendLog, TestTransport};
use super::*;
use bacnet_objects::notification_forwarder::NotificationForwarderObject;
use bacnet_types::constructed::BACnetRecipient;

/// The number of the network this device is attached to.
const THIS_NETWORK: u16 = 7;
const REMOTE_MAC: [u8; 2] = [0x0E, 0x0F];

const BROADCAST: Reception = Reception {
    group: true,
    global: false,
};

/// A forwarder sending to the broadcast on this network and to a node on
/// it, both named by number, and to a node on another network, processes 1
/// to 3.
fn numbered_recipients() -> NotificationForwarderObject {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    for (process, recipient) in [
        (1, address_recipient(THIS_NETWORK, &[])),
        (2, address_recipient(THIS_NETWORK, &PEER_A)),
        (3, address_recipient(5, &REMOTE_MAC)),
    ] {
        nf.add_destination(destination(recipient, process, false))
            .unwrap();
    }
    nf
}

#[tokio::test]
async fn a_recipient_naming_this_network_by_number_is_local_to_the_loop_rules() {
    let forwarding = Forwarding::new(database(vec![numbered_recipients()]));
    forwarding.set_local_network(THIS_NETWORK);
    // Addressed to this device alone: the node on this network gets its
    // copy as a local unicast, with no DNET, but nothing is broadcast back
    // onto this network.
    assert_eq!(
        forwarding
            .receive(&notification(9), Reception::UNICAST)
            .await,
        [
            unconfirmed(To::Local(PEER_A.to_vec()), 2),
            unconfirmed(To::Remote(5, REMOTE_MAC.to_vec()), 3),
        ]
    );
    // By broadcast on this network: every node here has it already.
    assert_eq!(
        forwarding.receive(&notification(9), BROADCAST).await,
        [unconfirmed(To::Remote(5, REMOTE_MAC.to_vec()), 3)]
    );
    assert_eq!(forwarding.counters(), EventNotificationCounters::default());
}

#[tokio::test]
async fn a_network_number_is_remote_while_this_network_has_none() {
    // Nothing published the local network's number: a numbered recipient
    // is taken as remote and sent its routed copy.
    let forwarding = Forwarding::new(database(vec![numbered_recipients()]));
    assert_eq!(
        forwarding.receive(&notification(9), BROADCAST).await,
        [
            unconfirmed(To::RemoteBroadcast(THIS_NETWORK), 1),
            unconfirmed(To::Remote(THIS_NETWORK, PEER_A.to_vec()), 2),
            unconfirmed(To::Remote(5, REMOTE_MAC.to_vec()), 3),
        ]
    );
}

/// A node on this network, one per `index`.
pub(super) fn peer(index: usize) -> [u8; 6] {
    [10, 0, 1, index as u8, 0xBA, 0xC0]
}

#[tokio::test]
async fn destinations_past_the_cap_across_forwarders_are_dropped_and_counted() {
    // Forwarders 1 to 3 take process 5 with 30 destinations each, and
    // forwarder 1 also hands the notification on to this device as process
    // 6, which forwarder 4 takes with 30 more.
    let this_device =
        BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, LOCAL_DEVICE).unwrap());
    let forwarders = (0..4u32)
        .map(|n| {
            let mut nf = NotificationForwarderObject::new(n + 1, format!("NF-{n}")).unwrap();
            nf.set_process_identifier_filter(Some(if n == 3 { 6 } else { 5 }));
            for index in 0..30 {
                let index = n as usize * 30 + index;
                nf.add_destination(destination(
                    address_recipient(0, &peer(index)),
                    index as u32,
                    false,
                ))
                .unwrap();
            }
            if n == 0 {
                nf.add_destination(destination(this_device.clone(), 6, false))
                    .unwrap();
            }
            nf
        })
        .collect();
    let forwarding = Forwarding::new(database(forwarders));
    // The first destinations, in forwarder then list order, up to the cap;
    // the hand-off to this device is not one of them.
    let expected: Vec<_> = (0..MAX_FORWARDED_DESTINATIONS)
        .map(|index| unconfirmed(To::Local(peer(index).to_vec()), index as u32))
        .collect();
    assert_eq!(
        forwarding
            .receive(&notification(5), Reception::UNICAST)
            .await,
        expected
    );
    assert_eq!(
        forwarding.counters(),
        EventNotificationCounters {
            forwarding_cap_dropped: (120 - MAX_FORWARDED_DESTINATIONS) as u64,
            ..Default::default()
        }
    );
}

/// Dispatch one ConfirmedEventNotification from `PEER_B` and return the
/// answer it drew.
async fn dispatch(
    services: &RequestServices<TestTransport>,
    invoke_id: u8,
    request: &EventNotificationRequest,
) -> Apdu {
    let (reply, answered) = oneshot::channel();
    BACnetServer::handle_confirmed_request(
        services,
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
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
            service_request: encoded(request),
        },
        Some(reply),
    )
    .await;
    reply_apdu(answered.await.unwrap())
}

fn acknowledges(apdu: &Apdu, invoke_id: u8) -> bool {
    matches!(apdu, Apdu::SimpleAck(ack) if ack.invoke_id == invoke_id
        && ack.service_choice == ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION)
}

/// One forwarder sending each notification to `PEER_A` as process 40, the
/// send log, and the services that dispatch to it.
fn forwarding_to_peer_a() -> (RequestServices<TestTransport>, SendLog) {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    nf.add_destination(destination(address_recipient(0, &PEER_A), 40, false))
        .unwrap();
    let transport = forwarding_transport();
    let sent: SendLog = transport.sent();
    (confirmed_services(database(vec![nf]), transport), sent)
}

/// Paused: the record's window is measured on tokio's clock (#1556), so a
/// runner stall between the sends can't close it.
#[tokio::test(start_paused = true)]
async fn a_retransmitted_confirmed_notification_is_acknowledged_but_not_forwarded_again() {
    let (services, sent) = forwarding_to_peer_a();
    let request = notification(5);
    let copy = [unconfirmed(To::Local(PEER_A.to_vec()), 40)];

    assert!(acknowledges(&dispatch(&services, 7, &request).await, 7));
    assert_eq!(copies(&sent, &request), copy);
    // The sender missed the acknowledgment and sends the request again,
    // after the first one has finished: answered again, not forwarded.
    assert!(acknowledges(&dispatch(&services, 7, &request).await, 7));
    assert!(copies(&sent, &request).is_empty());

    // Another invoke ID, or other octets under the same one, is a new
    // notification.
    assert!(acknowledges(&dispatch(&services, 8, &request).await, 8));
    assert_eq!(copies(&sent, &request), copy);
    let mut later = notification(5);
    later.timestamp = bacnet_types::primitives::BACnetTimeStamp::SequenceNumber(10);
    assert!(acknowledges(&dispatch(&services, 7, &later).await, 7));
    assert_eq!(copies(&sent, &later), copy);
}

/// A received notification is remembered for exactly its window from the
/// first receipt, on tokio's clock (#1556): a retransmission 1 ms before the
/// window ends is answered and not forwarded, and one at its end is
/// forwarded again. The repeat in between didn't renew the entry.
#[tokio::test(start_paused = true)]
async fn a_retransmission_is_recognized_until_exactly_the_window_ends() {
    let (services, sent) = forwarding_to_peer_a();
    let request = notification(5);
    let copy = [unconfirmed(To::Local(PEER_A.to_vec()), 40)];
    let window = super::event_forwarding_repeats::RECORD_WINDOW;

    assert!(acknowledges(&dispatch(&services, 7, &request).await, 7));
    assert_eq!(copies(&sent, &request), copy);

    tokio::time::advance(window - Duration::from_millis(1)).await;
    assert!(acknowledges(&dispatch(&services, 7, &request).await, 7));
    assert!(
        copies(&sent, &request).is_empty(),
        "1 ms before the window ends"
    );

    tokio::time::advance(Duration::from_millis(1)).await;
    assert!(acknowledges(&dispatch(&services, 7, &request).await, 7));
    assert_eq!(copies(&sent, &request), copy, "at the window's end");
}
