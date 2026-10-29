//! More forwarding rules (#1225): one copy per destination across a chain,
//! received notifications no forwarder takes, copies too large to send, and
//! the loop rules for bound Device recipients and confirmed copies.

use super::event_forwarding::Reception;
use super::event_forwarding_tests::{
    Copy, Forwarding, LOCAL_DEVICE, PEER_A, PEER_B, To, database, destination, notification,
    unconfirmed,
};
use super::event_recipient_routing_tests::address_recipient;
use super::*;
use bacnet_objects::notification_forwarder::NotificationForwarderObject;
use bacnet_types::constructed::BACnetRecipient;

const REMOTE_MAC: [u8; 2] = [0x0C, 0x0D];

const BROADCAST: Reception = Reception {
    group: true,
    global: false,
};

fn confirmed(to: To, process_identifier: u32) -> Copy {
    Copy {
        to,
        confirmed: true,
        process_identifier,
    }
}

#[tokio::test]
async fn a_destination_two_chained_forwarders_name_gets_one_copy() {
    let local =
        BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, LOCAL_DEVICE).unwrap());
    // A sends to R as process 9 and hands the notification on to this device
    // as process 2; B takes process 2 and also sends to R as process 9.
    let mut a = NotificationForwarderObject::new(1, "A").unwrap();
    a.set_process_identifier_filter(Some(1));
    a.add_destination(destination(address_recipient(0, &PEER_A), 9, false))
        .unwrap();
    a.add_destination(destination(local, 2, false)).unwrap();
    let mut b = NotificationForwarderObject::new(2, "B").unwrap();
    b.set_process_identifier_filter(Some(2));
    b.add_destination(destination(address_recipient(0, &PEER_A), 9, false))
        .unwrap();
    b.add_destination(destination(address_recipient(0, &PEER_B), 9, false))
        .unwrap();
    let forwarding = Forwarding::new(database(vec![a, b]));
    assert_eq!(
        forwarding
            .receive(&notification(1), Reception::UNICAST)
            .await,
        [
            unconfirmed(To::Local(PEER_A.to_vec()), 9),
            unconfirmed(To::Local(PEER_B.to_vec()), 9),
        ]
    );
}

#[tokio::test]
async fn a_received_notification_no_forwarder_takes_is_counted() {
    let mut only_seven = NotificationForwarderObject::new(1, "NF").unwrap();
    only_seven.set_process_identifier_filter(Some(7));
    only_seven
        .add_destination(destination(address_recipient(0, &PEER_A), 40, false))
        .unwrap();
    let forwarding = Forwarding::new(database(vec![only_seven]));
    assert!(
        forwarding
            .receive(&notification(5), Reception::UNICAST)
            .await
            .is_empty()
    );
    assert_eq!(forwarding.counters().received_not_forwarded, 1);
    // Taken notifications, and global broadcasts forwarders ignore, are not
    // counted.
    assert_eq!(
        forwarding
            .receive(&notification(7), Reception::UNICAST)
            .await
            .len(),
        1
    );
    let global = Reception {
        group: true,
        global: true,
    };
    assert!(
        forwarding
            .receive(&notification(5), global)
            .await
            .is_empty()
    );
    assert_eq!(
        forwarding.counters(),
        EventNotificationCounters {
            received_not_forwarded: 1,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn a_copy_longer_than_the_local_apdu_capacity_is_dropped_and_counted() {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    nf.add_destination(destination(address_recipient(0, &PEER_A), 40, true))
        .unwrap();
    nf.add_destination(destination(address_recipient(0, &PEER_B), 41, false))
        .unwrap();
    let forwarding = Forwarding::new(database(vec![nf]));
    // A notification that arrived segmented: its request is three octets
    // short of the 1476-octet capacity, so the unconfirmed copy (two header
    // octets) fits and the confirmed one (four) does not.
    let with_text = |length: usize| {
        let mut request = notification(5);
        request.message_text = Some("x".repeat(length));
        request
    };
    let base = super::event_forwarding_tests::encoded(&with_text(300)).len();
    let long = with_text(300 + 1473 - base);
    assert_eq!(super::event_forwarding_tests::encoded(&long).len(), 1473);
    assert_eq!(
        forwarding.receive(&long, Reception::UNICAST).await,
        [unconfirmed(To::Local(PEER_B.to_vec()), 41)]
    );
    assert_eq!(
        forwarding.counters(),
        EventNotificationCounters {
            apdu_too_large: 1,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn a_bound_device_on_the_receiving_network_gets_no_copy_of_a_broadcast() {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    nf.add_destination(destination(
        BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 60).unwrap()),
        40,
        false,
    ))
    .unwrap();
    let forwarding = Forwarding::new(database(vec![nf]));
    forwarding.bind_device(60, &PEER_A).await;
    assert_eq!(
        forwarding
            .receive(&notification(5), Reception::UNICAST)
            .await,
        [unconfirmed(To::Local(PEER_A.to_vec()), 40)]
    );
    assert!(
        forwarding
            .receive(&notification(5), BROADCAST)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn a_broadcast_notification_goes_confirmed_only_off_the_receiving_network() {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    nf.add_destination(destination(address_recipient(0, &PEER_A), 40, true))
        .unwrap();
    nf.add_destination(destination(address_recipient(5, &REMOTE_MAC), 41, true))
        .unwrap();
    let forwarding = Forwarding::new(database(vec![nf]));
    assert_eq!(
        forwarding.receive(&notification(5), BROADCAST).await,
        [confirmed(To::Remote(5, REMOTE_MAC.to_vec()), 41)]
    );
    assert_eq!(forwarding.counters(), EventNotificationCounters::default());
}
