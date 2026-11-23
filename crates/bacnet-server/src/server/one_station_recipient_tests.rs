//! A Device binding marked one station (#2738) reaches a station at a form
//! the mark lifts, and at the one octet `0x00`, which the mark never
//! refuses, with confirmed event notifications and a Notification
//! Forwarder's confirmed copies, read from the shared test transport's send
//! log; the forms the mark doesn't lift stay unconfirmed only and count, as
//! an unmarked binding's do.

use super::super::event_recipient_route::broadcast_station_tests::{
    LIFTED, MARKED_FORMS, MARKED_STATIONS,
};
use super::*;

fn marked(instance: u32, mac: &[u8]) -> DeviceBinding {
    DeviceBinding::routed(device(instance), NETWORK, mac, ROUTER)
        .unwrap()
        .one_station()
}

/// Each lifted form takes the confirmed notification and the unconfirmed
/// one; each form still refused takes only the unconfirmed one, and its
/// confirmed one is counted.
#[tokio::test(start_paused = true)]
async fn a_device_bound_marked_one_station_gets_confirmed_notifications_where_the_mark_lifts() {
    let mut bindings = DeviceBindingTable::new();
    let mut destinations = Vec::new();
    let macs: Vec<&[u8]> = MARKED_STATIONS.into_iter().chain(MARKED_FORMS).collect();
    for (instance, mac) in (100..).zip(&macs) {
        bindings
            .insert_configured(marked(instance, mac), |_| false)
            .unwrap();
        let recipient = BACnetRecipient::Device(device(instance));
        destinations.push(destination(recipient.clone(), instance, true));
        destinations.push(destination(recipient, instance + 100, false));
    }
    let (sent, counters) = distribute(destinations, bindings).await;
    let through_router = |process, confirmed, dadr: &[u8]| Sent {
        process,
        confirmed,
        dnet: NETWORK,
        dadr: dadr.to_vec(),
        via: Some(ROUTER.to_vec()),
    };
    let mut expected: Vec<Sent> = (100..)
        .zip(MARKED_STATIONS)
        .map(|(process, mac)| through_router(process, true, mac))
        .chain(
            (200..)
                .zip(&macs)
                .map(|(process, mac)| through_router(process, false, mac)),
        )
        .collect();
    expected.sort();
    assert_eq!(sent, expected);
    assert_eq!(counters, counted(MARKED_FORMS.len() as u64));
}

/// The same binding unmarked: every one of those forms is refused a
/// confirmed notification, the lifted ones included.
#[tokio::test(start_paused = true)]
async fn the_same_binding_unmarked_gets_unconfirmed_notifications_only() {
    let mut bindings = DeviceBindingTable::new();
    let mut destinations = Vec::new();
    for (instance, mac) in (100..).zip(LIFTED) {
        bindings
            .insert_configured(
                DeviceBinding::routed(device(instance), NETWORK, mac, ROUTER).unwrap(),
                |_| false,
            )
            .unwrap();
        destinations.push(destination(
            BACnetRecipient::Device(device(instance)),
            instance,
            true,
        ));
    }
    let (sent, counters) = distribute(destinations, bindings).await;
    assert_eq!(sent, []);
    assert_eq!(counters, counted(LIFTED.len() as u64));
}

/// A Notification Forwarder's confirmed copy to a Device bound marked one
/// station at a lifted form goes; at a form still refused it is skipped and
/// counted, and the unconfirmed copy goes either way.
#[tokio::test]
async fn a_forwarded_confirmed_copy_to_a_device_bound_marked_one_station() {
    for (mac, delivered) in MARKED_STATIONS
        .into_iter()
        .map(|mac| (mac, true))
        .chain(MARKED_FORMS.into_iter().map(|mac| (mac, false)))
    {
        let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
        let recipient = BACnetRecipient::Device(device(40));
        nf.add_destination(destination(recipient.clone(), 1, true))
            .unwrap();
        nf.add_destination(destination(recipient, 2, false))
            .unwrap();
        let forwarding = Forwarding::new(database(vec![nf]));
        forwarding.bind(marked(40, mac)).await;
        let mut sent = forwarding
            .receive(&notification(5), Reception::UNICAST)
            .await;
        sent.sort_by_key(|copy| copy.process_identifier);
        let to = To::Remote(NETWORK, mac.to_vec());
        let mut expected = vec![unconfirmed(to.clone(), 2)];
        if delivered {
            expected.insert(
                0,
                Copy {
                    to,
                    confirmed: true,
                    process_identifier: 1,
                },
            );
        }
        assert_eq!(sent, expected, "{mac:02X?}");
        assert_eq!(
            forwarding.counters(),
            counted(u64::from(!delivered)),
            "{mac:02X?}"
        );
    }
}
