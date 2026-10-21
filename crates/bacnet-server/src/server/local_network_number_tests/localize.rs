//! `RecipientRoute::localize` case by case (#1299), `into_confirmed`'s group
//! check (#1493), and two outcomes on a running server: a confirmed
//! notification sent to a node on this network is completed by that node's
//! own SimpleACK, and a Device bound to the link broadcast MAC on this network
//! is skipped as unroutable.

use super::*;
use crate::server::device_bindings::BindingFreshness;
use crate::server::event_delivery::EventDelivery;
use crate::server::event_recipient_route::{ConfirmedRouteRefusal, RecipientRoute};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::event::EventStateChange;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EngineeringUnits, EventState, EventType, NotifyType};

fn is_link_broadcast(mac: &[u8]) -> bool {
    mac == LITERAL_BROADCAST_MAC
}

/// A multicast address: a group destination, not the link's broadcast.
const MULTICAST: [u8; 6] = [224, 0, 0, 1, 0xBA, 0xC0];

/// The link's group rule: its broadcast, and [`MULTICAST`]. Distinct from
/// [`is_link_broadcast`], so a test tells which one each case consults.
fn is_group(mac: &[u8]) -> bool {
    is_link_broadcast(mac) || mac == MULTICAST
}

fn mac(octets: &[u8]) -> MacAddr {
    MacAddr::from_slice(octets)
}

fn remote_unicast(network: u16, octets: &[u8]) -> RecipientRoute {
    RecipientRoute::RemoteUnicast {
        network,
        mac: mac(octets),
    }
}

/// A configured Device binding to `octets` on `network`, through `ROUTER`.
fn bound_routed(network: u16, octets: &[u8]) -> RecipientRoute {
    RecipientRoute::BoundRoutedUnicast {
        network,
        mac: mac(octets),
        router: mac(&ROUTER),
        freshness: BindingFreshness::Configured,
    }
}

#[test]
fn localize_leaves_every_route_alone_while_the_number_is_unknown() {
    for route in [
        || RecipientRoute::RemoteBroadcast(THIS_NETWORK),
        || remote_unicast(THIS_NETWORK, &PEER_A),
        || remote_unicast(THIS_NETWORK, LITERAL_BROADCAST_MAC),
        || bound_routed(THIS_NETWORK, &PEER_C),
        || bound_routed(THIS_NETWORK, LITERAL_BROADCAST_MAC),
        || bound_routed(THIS_NETWORK, &MULTICAST),
    ] {
        assert_eq!(route().localize(None, is_link_broadcast, is_group), route());
    }
}

#[test]
fn localize_takes_only_routes_naming_this_network_as_local() {
    let local = Some(THIS_NETWORK);
    // Routes that name no network, the global broadcast or another network
    // stay as they are, a contradictory global address included.
    let contradictory = RecipientRoute::resolve_address(
        &bacnet_types::constructed::BACnetAddress {
            network_number: 0xFFFF,
            mac_address: mac(&PEER_A),
        },
        is_link_broadcast,
    );
    assert_eq!(contradictory, RecipientRoute::ContradictoryGlobal);
    for route in [
        || RecipientRoute::GlobalBroadcast,
        || RecipientRoute::ContradictoryGlobal,
        || RecipientRoute::LocalBroadcast,
        || RecipientRoute::LocalUnicast(mac(&PEER_A)),
        || RecipientRoute::RemoteBroadcast(REMOTE_NETWORK),
        || remote_unicast(REMOTE_NETWORK, &REMOTE_MAC),
        || bound_routed(REMOTE_NETWORK, &PEER_C),
    ] {
        assert_eq!(
            route().localize(local, is_link_broadcast, is_group),
            route()
        );
    }
    // Routes naming this network become local ones.
    for (route, expected) in [
        (
            RecipientRoute::RemoteBroadcast(THIS_NETWORK),
            RecipientRoute::LocalBroadcast,
        ),
        (
            remote_unicast(THIS_NETWORK, &PEER_A),
            RecipientRoute::LocalUnicast(mac(&PEER_A)),
        ),
        (
            remote_unicast(THIS_NETWORK, LITERAL_BROADCAST_MAC),
            RecipientRoute::LocalBroadcast,
        ),
        (
            bound_routed(THIS_NETWORK, &PEER_C),
            RecipientRoute::BoundLocalUnicast {
                mac: mac(&PEER_C),
                freshness: BindingFreshness::Configured,
            },
        ),
        // A Device bound to the broadcast MAC here names no single device,
        // and neither does one bound to any other group address (#1493).
        (
            bound_routed(THIS_NETWORK, LITERAL_BROADCAST_MAC),
            RecipientRoute::InvalidDevice,
        ),
        (
            bound_routed(THIS_NETWORK, &MULTICAST),
            RecipientRoute::InvalidDevice,
        ),
        // An address at a group address other than the link's broadcast is
        // routed as a unicast to it: an unconfirmed notification may go
        // there, and `into_confirmed` refuses a confirmed one.
        (
            remote_unicast(THIS_NETWORK, &MULTICAST),
            RecipientRoute::LocalUnicast(mac(&MULTICAST)),
        ),
    ] {
        assert_eq!(route.localize(local, is_link_broadcast, is_group), expected);
    }
}

/// #1493: a confirmed request's route is refused when its local next hop is
/// a group address, the recipient's own MAC or a binding's router. A remote
/// recipient with no router named has no local next hop to check, since its
/// DADR is on another network, and a broadcast names no device.
#[test]
fn into_confirmed_refuses_a_group_next_hop() {
    let refusal = |route: RecipientRoute| route.into_confirmed(is_group).err();
    let group = Some(ConfirmedRouteRefusal::GroupNextHop);
    let bound_local = |octets: &[u8]| RecipientRoute::BoundLocalUnicast {
        mac: mac(octets),
        freshness: BindingFreshness::Configured,
    };
    let routed_via = |router: &[u8]| RecipientRoute::BoundRoutedUnicast {
        network: REMOTE_NETWORK,
        mac: mac(&PEER_C),
        router: mac(router),
        freshness: BindingFreshness::Configured,
    };
    for (case, route, expected) in [
        (
            "local station",
            RecipientRoute::LocalUnicast(mac(&PEER_C)),
            None,
        ),
        (
            "local multicast",
            RecipientRoute::LocalUnicast(mac(&MULTICAST)),
            group,
        ),
        (
            "local broadcast MAC",
            RecipientRoute::LocalUnicast(mac(LITERAL_BROADCAST_MAC)),
            group,
        ),
        ("bound local station", bound_local(&PEER_C), None),
        ("bound local multicast", bound_local(&MULTICAST), group),
        ("routed via a station", routed_via(&ROUTER), None),
        (
            "routed via a multicast router",
            routed_via(&MULTICAST),
            group,
        ),
        (
            "remote with no router",
            remote_unicast(REMOTE_NETWORK, &MULTICAST),
            None,
        ),
        (
            "local broadcast",
            RecipientRoute::LocalBroadcast,
            Some(ConfirmedRouteRefusal::NotOneDevice),
        ),
    ] {
        assert_eq!(refusal(route), expected, "{case}");
    }
}

fn input_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

/// An alarm-reporting Analog Input under Notification Class 0.
fn alarm_input() -> AnalogInputObject {
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::SQUARE_METERS).unwrap();
    ai.write_property(
        PropertyIdentifier::NOTIFY_TYPE,
        None,
        PropertyValue::Enumerated(NotifyType::ALARM.to_raw()),
        None,
    )
    .unwrap();
    ai
}

#[tokio::test(start_paused = true)]
async fn a_confirmed_notification_to_this_network_is_completed_by_the_direct_ack() {
    let mut db = database(Vec::new());
    let mut class = NotificationClass::new(0, "NC-0").unwrap();
    class
        .add_destination(destination(
            address_recipient(THIS_NETWORK, &PEER_B),
            4,
            true,
        ))
        .unwrap();
    db.add(Box::new(class)).unwrap();
    db.add(Box::new(alarm_input())).unwrap();
    let started = Started::new(db, None).await;
    assert_eq!(started.announce(THIS_NETWORK, 0).await, THIS_NETWORK);
    let server = &started.server;
    BACnetServer::<TestTransport>::build_and_send_event_notification_with_bindings(
        &EventDelivery {
            db: &server.db,
            network: server.test_network(),
            comm_state: &server.comm_state,
            learned_routers: &server.learned_routers,
            notification_transactions: &server.notification_transactions,
            device_bindings: &server.device_bindings,
            suppressions: &server.event_suppressions,
            retry_timeout_ms: 1000,
            local_apdu_capacity: 1476,
        },
        &input_oid(),
        (
            EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            EventType::OUT_OF_RANGE,
        ),
    )
    .await;

    // The request goes to the recipient's MAC with no DNET.
    bounded(started.sent.wait_for_len(1)).await;
    let request = started.sent.take();
    let [frame] = &request[..] else {
        panic!("expected one confirmed request, got {request:?}");
    };
    assert!(!frame.broadcast);
    assert_eq!(frame.mac[..], PEER_B);
    assert_eq!(frame.decode_npdu().destination, None);
    let Apdu::ConfirmedRequest(sent) = frame.apdu() else {
        panic!("expected a confirmed request");
    };
    assert_eq!(server.notification_transactions.active_count(), 1);

    // The recipient answers from its own MAC with no SNET, which is the peer
    // the transaction waits on, so the answer completes it before any retry
    // timer runs (the paused clock only moves when every task is idle).
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id: sent.invoke_id,
            service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload: apdu.freeze(),
            ..Default::default()
        },
    )
    .unwrap();
    started.feed(&npdu, false).await;
    for _ in 0..1_000 {
        if server.notification_transactions.active_count() == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(server.notification_transactions.active_count(), 0);
    assert!(started.sent.is_empty(), "the request is not sent again");
    assert_eq!(
        server.event_suppressions.snapshot(),
        EventNotificationCounters::default()
    );
    started.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_device_bound_at_the_broadcast_mac_on_this_network_is_unroutable() {
    let started = Started::new(database(Vec::new()), None).await;
    assert_eq!(started.announce(THIS_NETWORK, 0).await, THIS_NETWORK);
    let bound = ObjectIdentifier::new(ObjectType::DEVICE, 78).unwrap();
    let mut bindings = DeviceBindingTable::new();
    bindings
        .insert_configured(
            DeviceBinding::routed(bound, THIS_NETWORK, LITERAL_BROADCAST_MAC, ROUTER).unwrap(),
            |_| false,
        )
        .unwrap();
    let mut class = NotificationClass::new(0, "NC-0").unwrap();
    for (process, recipient) in [
        (1, BACnetRecipient::Device(bound)),
        (2, address_recipient(REMOTE_NETWORK, &REMOTE_MAC)),
    ] {
        class
            .add_destination(destination(recipient, process, false))
            .unwrap();
    }
    let mut db = ObjectDatabase::new();
    db.add(Box::new(class)).unwrap();

    let (broadcasts, unicasts, counters) = distribute_counted_through(
        started.server.test_network(),
        &started.sent,
        db,
        Arc::new(RwLock::new(bindings)),
        DccState::Enable,
    )
    .await;
    // Skipped and counted; the other destination is still served.
    assert_eq!(
        routes(broadcasts, unicasts),
        [(2, To::Remote(REMOTE_NETWORK, REMOTE_MAC.to_vec()), false)]
    );
    assert_eq!(
        counters,
        EventNotificationCounters {
            recipient_unroutable: 1,
            ..Default::default()
        }
    );
    started.stop().await;
}
