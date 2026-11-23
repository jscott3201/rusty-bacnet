//! No confirmed request the server starts goes to a station on another
//! network whose MAC there is a broadcast or group form (#2732): the router
//! that delivers it would hand it to every station there. Each sender is
//! judged from the shared test transport's send log: a confirmed recipient
//! at each form gets nothing and counts once, an unconfirmed one at the same
//! address is still sent to, and the other recipients of the same
//! notification are served. A server on a router's application port
//! behaves the same.

use super::device_bindings::{DeviceBinding, DeviceBindingTable};
use super::event_forwarding::Reception;
use super::event_forwarding_tests::{
    Copy, Forwarding, To, database, destination, notification, unconfirmed,
};
use super::event_recipient_route::broadcast_station_tests::FORMS;
use super::event_recipient_routing_tests::{
    LITERAL_BROADCAST_MAC, address_recipient, distribute_counted_on,
};
use super::test_transport::{BIP_LOCAL_MAC, SendLog, TestTransport};
use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::notification_forwarder::NotificationForwarderObject;
use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};

#[path = "remote_broadcast_cov_tests.rs"]
mod cov;
#[path = "one_station_recipient_tests.rs"]
mod one_station;
#[path = "remote_broadcast_router_app_tests.rs"]
mod router_app;

/// The remote network, and a station on it.
const NETWORK: u16 = 5;
const STATION: [u8; 1] = [0x14];
/// The router to it, on this B/IP link.
const ROUTER: [u8; 6] = [10, 0, 0, 9, 0xBA, 0xC0];

fn counted(confirmed_broadcast_recipient: u64) -> EventNotificationCounters {
    EventNotificationCounters {
        confirmed_broadcast_recipient,
        ..Default::default()
    }
}

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

/// One event notification as it left the link: its process identifier,
/// whether it was confirmed, its DNET and DADR, and the link MAC it went to
/// (`None` for the link's broadcast).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Sent {
    process: u32,
    confirmed: bool,
    dnet: u16,
    dadr: Vec<u8>,
    via: Option<Vec<u8>>,
}

fn read(npdu: Bytes, via: Option<Vec<u8>>) -> Sent {
    let npdu = decode_npdu(npdu).unwrap();
    let (confirmed, request) = match decode_apdu(npdu.payload).unwrap() {
        Apdu::UnconfirmedRequest(request) => (false, request.service_request),
        Apdu::ConfirmedRequest(request) => (true, request.service_request),
        other => panic!("expected an event notification, got {other:?}"),
    };
    let destination = npdu.destination.expect("a routed notification");
    Sent {
        process: decode_event_notification(&request)
            .unwrap()
            .process_identifier,
        confirmed,
        dnet: destination.network,
        dadr: destination.mac_address.to_vec(),
        via,
    }
}

/// Fire one transition at a Notification Class holding `destinations`,
/// over a B/IP-shaped link that knows no network number, and return what
/// went out, in process order, with the counters it moved.
async fn distribute(
    destinations: Vec<BACnetDestination>,
    bindings: DeviceBindingTable,
) -> (Vec<Sent>, EventNotificationCounters) {
    let mut class = NotificationClass::new(0, "NC-0").unwrap();
    for destination in destinations {
        class.add_destination(destination).unwrap();
    }
    let mut db = ObjectDatabase::new();
    db.add(Box::new(class)).unwrap();
    let link = TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .broadcast_mac(LITERAL_BROADCAST_MAC)
        .build();
    let (broadcasts, unicasts, counters) =
        distribute_counted_on(link, db, Arc::new(RwLock::new(bindings)), DccState::Enable).await;
    let mut sent: Vec<Sent> = broadcasts
        .into_iter()
        .map(|npdu| read(npdu, None))
        .chain(
            unicasts
                .into_iter()
                .map(|(mac, npdu)| read(npdu, Some(mac))),
        )
        .collect();
    sent.sort();
    (sent, counters)
}

/// An Address recipient on network 5: with no router learned, a routed
/// notification goes out as this link's broadcast, its DNET and DADR naming
/// the station.
#[tokio::test(start_paused = true)]
async fn an_address_recipient_at_a_remote_broadcast_form_gets_unconfirmed_notifications_only() {
    for form in FORMS {
        let (sent, counters) = distribute(
            vec![
                destination(address_recipient(NETWORK, form), 1, true),
                destination(address_recipient(NETWORK, form), 2, false),
                destination(address_recipient(NETWORK, &STATION), 3, true),
            ],
            DeviceBindingTable::new(),
        )
        .await;
        let routed = |process, confirmed, dadr: &[u8]| Sent {
            process,
            confirmed,
            dnet: NETWORK,
            dadr: dadr.to_vec(),
            via: None,
        };
        assert_eq!(
            sent,
            [routed(2, false, form), routed(3, true, &STATION)],
            "{form:02X?}"
        );
        assert_eq!(counters, counted(1), "{form:02X?}");
    }
}

/// A Device recipient whose binding, configured or observed from an I-Am's
/// SNET and SADR, names a broadcast form: the binding is kept, and the
/// device gets its unconfirmed notifications through the router.
#[tokio::test(start_paused = true)]
async fn a_device_bound_at_a_remote_broadcast_form_gets_unconfirmed_notifications_only() {
    let mut bindings = DeviceBindingTable::new();
    let mut destinations = Vec::new();
    for (instance, form) in (100..).zip(FORMS) {
        bindings
            .insert_configured(
                DeviceBinding::routed(device(instance), NETWORK, form, ROUTER).unwrap(),
                |_| false,
            )
            .unwrap();
        let recipient = BACnetRecipient::Device(device(instance));
        destinations.push(destination(recipient.clone(), instance, true));
        destinations.push(destination(recipient, instance + 100, false));
    }
    let observed = device(300);
    let source = NpduAddress {
        network: NETWORK,
        mac_address: MacAddr::from_slice(&[0xFF]),
    };
    bindings.observe_i_am_at(
        observed,
        &ROUTER,
        Some(&source),
        runtime_clock::now(),
        |_| false,
    );
    destinations.push(destination(BACnetRecipient::Device(observed), 300, true));
    destinations.push(destination(BACnetRecipient::Device(observed), 301, false));
    bindings
        .insert_configured(
            DeviceBinding::routed(device(99), NETWORK, STATION, ROUTER).unwrap(),
            |_| false,
        )
        .unwrap();
    destinations.push(destination(BACnetRecipient::Device(device(99)), 99, true));

    let (sent, counters) = distribute(destinations, bindings).await;
    let through_router = |process, confirmed, dadr: &[u8]| Sent {
        process,
        confirmed,
        dnet: NETWORK,
        dadr: dadr.to_vec(),
        via: Some(ROUTER.to_vec()),
    };
    let mut expected = vec![through_router(99, true, &STATION)];
    expected.extend(
        (200..)
            .zip(FORMS)
            .map(|(p, form)| through_router(p, false, form)),
    );
    expected.push(through_router(301, false, &[0xFF]));
    assert_eq!(sent, expected);
    assert_eq!(counters, counted(FORMS.len() as u64 + 1));
}

/// A Notification Forwarder's copies take the same check: the confirmed
/// copy to a broadcast form is skipped and counted, its unconfirmed copy
/// and the confirmed one to a station go.
#[tokio::test]
async fn a_forwarded_confirmed_copy_to_a_remote_broadcast_form_is_skipped() {
    for form in FORMS {
        let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
        for (process, mac, confirmed) in
            [(1, form, true), (2, form, false), (3, &STATION[..], true)]
        {
            nf.add_destination(destination(
                address_recipient(NETWORK, mac),
                process,
                confirmed,
            ))
            .unwrap();
        }
        let forwarding = Forwarding::new(database(vec![nf]));
        let mut sent = forwarding
            .receive(&notification(5), Reception::UNICAST)
            .await;
        sent.sort_by_key(|copy| copy.process_identifier);
        assert_eq!(
            sent,
            [
                unconfirmed(To::Remote(NETWORK, form.to_vec()), 2),
                Copy {
                    to: To::Remote(NETWORK, STATION.to_vec()),
                    confirmed: true,
                    process_identifier: 3,
                },
            ],
            "{form:02X?}"
        );
        assert_eq!(forwarding.counters(), counted(1), "{form:02X?}");
    }
}
