//! How the forwarding cap counts (#1259): only copies that would be sent
//! take room, a local notification that names this device's Device object
//! under several process identifiers has one cap, and the cap warning is
//! throttled. A copy that waits for its Device destination's I-Am keeps to
//! the cap and the loop rules once it goes (#1368).

use super::event_forwarding::Reception;
use super::event_forwarding_bounds_tests::peer;
use super::event_forwarding_tests::{
    Forwarding, LOCAL_DEVICE, To, copies, database, destination, notification, unconfirmed,
};
use super::event_recipient_routing_tests::{address_recipient, distribute_counted};
use super::*;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::notification_forwarder::NotificationForwarderObject;
use bacnet_types::constructed::BACnetRecipient;

/// A node on network 5, one per `index`.
fn remote(index: usize) -> [u8; 2] {
    [0x0E, index as u8]
}

/// Forwarder 1 names 30 nodes on this network, which the loop rules refuse
/// for a notification received by broadcast; forwarders 2 to 4 name 70
/// nodes on network 5 (indices 30 to 99).
fn capped_forwarders() -> Vec<NotificationForwarderObject> {
    (0..4u32)
        .map(|n| {
            let mut nf = NotificationForwarderObject::new(n + 1, format!("NF-{n}")).unwrap();
            let count = if n == 3 { 10 } else { 30 };
            for offset in 0..count {
                let index = n as usize * 30 + offset;
                let recipient = if n == 0 {
                    address_recipient(0, &peer(index))
                } else {
                    address_recipient(5, &remote(index))
                };
                nf.add_destination(destination(recipient, index as u32, false))
                    .unwrap();
            }
            nf
        })
        .collect()
}

#[tokio::test]
async fn the_cap_counts_only_copies_the_loop_rules_let_through() {
    let forwarding = Forwarding::new(database(capped_forwarders()));
    let broadcast = Reception {
        group: true,
        global: false,
    };
    // The refused nodes take no room: the first 64 of the 70 remote nodes go.
    let expected: Vec<_> = (30..30 + MAX_FORWARDED_DESTINATIONS)
        .map(|index| unconfirmed(To::Remote(5, remote(index).to_vec()), index as u32))
        .collect();
    assert_eq!(
        forwarding.receive(&notification(5), broadcast).await,
        expected
    );
    assert_eq!(
        forwarding.counters(),
        EventNotificationCounters {
            forwarding_cap_dropped: (70 - MAX_FORWARDED_DESTINATIONS) as u64,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn a_local_notification_under_several_process_identifiers_has_one_cap() {
    let this_device =
        BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, LOCAL_DEVICE).unwrap());
    // Notification Class 0 names this device under processes 5 and 6.
    let mut class = NotificationClass::new(0, "NC-0").unwrap();
    for process in [5, 6] {
        class
            .add_destination(destination(this_device.clone(), process, false))
            .unwrap();
    }
    let node = |index: usize| destination(address_recipient(0, &peer(index)), index as u32, false);
    // Forwarder 1 takes process 5 with nodes 0 to 31. Forwarders 2 and 3
    // take process 6: forwarder 2 with node 0 again and nodes 32 to 62,
    // forwarder 3 with nodes 63 to 72. That is 73 distinct destinations.
    let mut db = ObjectDatabase::new();
    db.add(Box::new(class)).unwrap();
    for (instance, process, nodes) in [
        (1, 5, (0..32).collect::<Vec<_>>()),
        (2, 6, std::iter::once(0).chain(32..63).collect()),
        (3, 6, (63..73).collect()),
    ] {
        let mut nf = NotificationForwarderObject::new(instance, format!("NF-{instance}")).unwrap();
        nf.set_process_identifier_filter(Some(process));
        for index in nodes {
            nf.add_destination(node(index)).unwrap();
        }
        db.add(Box::new(nf)).unwrap();
    }

    let (broadcasts, unicasts, counters) = distribute_counted(
        db,
        Arc::new(RwLock::new(
            super::device_bindings::DeviceBindingTable::new(),
        )),
        DccState::Enable,
    )
    .await;
    assert!(broadcasts.is_empty());
    // Node 0 gets one copy, and the 73 destinations share one cap of 64.
    let reached: Vec<Vec<u8>> = unicasts.into_iter().map(|(mac, _)| mac).collect();
    let expected: Vec<Vec<u8>> = (0..MAX_FORWARDED_DESTINATIONS)
        .map(|index| peer(index).to_vec())
        .collect();
    assert_eq!(reached, expected);
    assert_eq!(
        counters,
        EventNotificationCounters {
            forwarding_cap_dropped: (73 - MAX_FORWARDED_DESTINATIONS) as u64,
            ..Default::default()
        }
    );
}

/// Device 9, which the forwarding server has no binding for.
fn device_9() -> BACnetRecipient {
    BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap())
}

/// Whether `frame` is the Who-Is a look for Device 9 sends.
fn who_is(frame: &crate::server::test_transport::SentFrame) -> bool {
    super::event_recipient_routing_tests::is_who_is(frame)
}

#[tokio::test(start_paused = true)]
async fn a_copy_that_waited_for_its_device_keeps_to_the_loop_rules() {
    // Forwarder 1 sends to Device 9, which answers from this network, so its
    // copy goes to one node here: one a received broadcast already reached.
    for (group, sent) in [(true, false), (false, true)] {
        let mut nf = NotificationForwarderObject::new(1, "NF-1").unwrap();
        nf.add_destination(destination(device_9(), 40, false))
            .unwrap();
        let forwarding = Forwarding::new(database(vec![nf]));
        let reception = Reception {
            group,
            global: false,
        };
        forwarding.deliver(&notification(5), reception).await;
        assert!(forwarding.sent.frames().iter().all(who_is));
        forwarding.sent.clear();
        forwarding.hear_i_am(9, &peer(9)).await;
        let expected = if sent {
            vec![unconfirmed(To::Local(peer(9).to_vec()), 40)]
        } else {
            Vec::new()
        };
        assert_eq!(copies(&forwarding.sent, &notification(5)), expected);
        // A loop rule's refusal is configured behaviour: nothing counts it.
        assert_eq!(forwarding.counters(), EventNotificationCounters::default());
    }
}

#[tokio::test(start_paused = true)]
async fn a_copy_that_waited_for_its_device_draws_on_the_cap_as_it_goes() {
    // Forwarder 1 names Device 9 first, then nodes 0 to 20 on network 5;
    // forwarders 2 and 3 name nodes 21 to 63 there. The 64 nodes take the
    // whole cap while Device 9 is looked for, so its copy has no room left.
    let forwarders = [(1, 0..21), (2, 21..42), (3, 42..64)]
        .into_iter()
        .map(|(instance, nodes)| {
            let mut nf =
                NotificationForwarderObject::new(instance, format!("NF-{instance}")).unwrap();
            if instance == 1 {
                nf.add_destination(destination(device_9(), 999, false))
                    .unwrap();
            }
            for index in nodes {
                nf.add_destination(destination(
                    address_recipient(5, &remote(index)),
                    index as u32,
                    false,
                ))
                .unwrap();
            }
            nf
        })
        .collect();
    let forwarding = Forwarding::new(database(forwarders));
    let unicast = Reception {
        group: false,
        global: false,
    };
    forwarding.deliver(&notification(5), unicast).await;
    let (who_is, sent): (Vec<_>, Vec<_>) = forwarding.sent.take().into_iter().partition(who_is);
    assert_eq!((who_is.len(), sent.len()), (1, MAX_FORWARDED_DESTINATIONS));
    forwarding.hear_i_am(9, &peer(9)).await;
    assert!(
        forwarding.sent.is_empty(),
        "Device 9's copy is past the cap"
    );
    assert_eq!(
        forwarding.counters(),
        EventNotificationCounters {
            forwarding_cap_dropped: 1,
            ..Default::default()
        }
    );
}

/// A run of capped notifications warns once per interval through the
/// server's own throttle (#1950); the rest log at debug and are counted for
/// the next warning.
#[tokio::test(start_paused = true)]
async fn the_cap_warning_goes_out_at_most_once_an_interval() {
    use bacnet_transport::warn_throttle::WARNING_INTERVAL;
    let forwarding = Forwarding::new(database(capped_forwarders()));
    for _ in 0..3 {
        let broadcast = Reception {
            group: true,
            global: false,
        };
        forwarding.receive(&notification(5), broadcast).await;
    }
    let throttle = forwarding.cap_warning();
    assert_eq!(
        throttle.note(),
        None,
        "the first capped notification warned"
    );
    tokio::time::advance(WARNING_INTERVAL).await;
    assert_eq!(
        throttle.note(),
        Some(4),
        "two held back, the check above, and this one"
    );
}
