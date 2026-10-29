//! I-Have delivery and router discovery through the client (#960), on
//! in-process loopback links:
//!
//! - a server on the client's own link answers a Who-Has with an I-Have;
//! - a `BACnetRouter` joins network 1, the client's, to network 2, where
//!   the server is station 5. The client finds the router and the network
//!   it reaches, reads the server through it, and hears the router's
//!   reject for a network it doesn't know.

use std::time::Duration;

use bacnet_client::client::{BACnetClient, DiscoveredRouter};
use bacnet_network::layer::{NetworkReport, ReceivedNetworkReport};
use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_npdu::{RejectMessageReason, RejectMessageToNetwork};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_services::who_has::{IHaveRequest, WhoHasObject};
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;
use tokio::sync::broadcast::Receiver;

const INSTANCE: u32 = 4321;
const NAME: &str = "Supply Air Temp";
const WAIT: Duration = Duration::from_secs(2);
const ROUTER_MAC: u8 = 0x01;
const REMOTE: u16 = 2;
const STATION: u8 = 0x05;

async fn server(transport: LoopbackTransport) -> BACnetServer<LoopbackTransport> {
    let identity = bacnet_objects::DeviceIdentity::new(INSTANCE, 0).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: identity.clone().with_name("Air handler"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(3, NAME, EngineeringUnits::DEGREES_FAHRENHEIT).unwrap(),
    ))
    .unwrap();
    BACnetServer::generic_builder()
        .device(identity)
        .database(db)
        .transport(transport)
        .build()
        .await
        .unwrap()
}

async fn client(transport: LoopbackTransport) -> BACnetClient<LoopbackTransport> {
    BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap()
}

fn supply_air() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap()
}

fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, INSTANCE).unwrap()
}

#[tokio::test(start_paused = true)]
async fn who_has_collects_the_i_have_of_a_local_server() {
    let (client_end, server_end) = LoopbackTransport::pair(vec![0x0A], vec![STATION]);
    let mut server = server(server_end).await;
    let mut client = client(client_end).await;

    for object in [
        WhoHasObject::Name(NAME.into()),
        WhoHasObject::Identifier(supply_air()),
    ] {
        let replies = client.who_has_replies(object, None, WAIT).await.unwrap();
        assert_eq!(replies.len(), 1, "{replies:?}");
        let reply = &replies[0];
        assert_eq!(reply.i_have.device_identifier, device());
        assert_eq!(reply.i_have.object_identifier, supply_air());
        assert_eq!(reply.i_have.object_name, NAME);
        assert_eq!(reply.source_mac.as_slice(), [STATION]);
        assert_eq!(reply.source_network, None);
    }

    // No such name, and a range that leaves the device out: nothing, once
    // the wait is over.
    let unknown = client
        .who_has_replies(WhoHasObject::Name("Return Air Temp".into()), None, WAIT)
        .await
        .unwrap();
    assert_eq!(unknown, []);
    let elsewhere = DeviceInstanceRange::new(1, 10).unwrap();
    let out_of_range = client
        .who_has_replies(WhoHasObject::Name(NAME.into()), Some(elsewhere), WAIT)
        .await
        .unwrap();
    assert_eq!(out_of_range, []);

    // An I-Have for another question, heard during the wait, isn't kept.
    let (unrelated, sent) = tokio::join!(
        client.who_has_replies(WhoHasObject::Name("Return Air Temp".into()), None, WAIT),
        async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            client.who_has(WhoHasObject::Name(NAME.into()), None).await
        }
    );
    sent.unwrap();
    assert_eq!(unrelated.unwrap(), []);

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// A peer that answers one Who-Has with the same I-Have twice is listed
/// once.
#[tokio::test(start_paused = true)]
async fn the_same_answer_twice_is_listed_once() {
    let (client_end, mut peer) = LoopbackTransport::pair(vec![0x0A], vec![STATION]);
    let mut frames = peer.start().await.unwrap();
    let mut client = client(client_end).await;
    let answer = tokio::spawn(async move {
        frames.recv().await.expect("the Who-Has");
        let mut npdu = BytesMut::from(&[1u8, 0, 0x10, 0x01][..]);
        IHaveRequest {
            device_identifier: device(),
            object_identifier: supply_air(),
            object_name: NAME.into(),
        }
        .encode(&mut npdu)
        .unwrap();
        for _ in 0..2 {
            peer.send_unicast(&npdu, &[0x0A]).await.unwrap();
        }
        peer
    });
    let replies = client
        .who_has_replies(WhoHasObject::Name(NAME.into()), None, WAIT)
        .await
        .unwrap();
    assert_eq!(replies.len(), 1, "{replies:?}");
    drop(answer.await.unwrap());
    client.stop().await.unwrap();
}

/// An I-Have the client didn't ask for still reaches `i_have_events`.
#[tokio::test(start_paused = true)]
async fn every_i_have_reaches_the_event_subscribers() {
    let (client_end, server_end) = LoopbackTransport::pair(vec![0x0A], vec![STATION]);
    let mut server = server(server_end).await;
    let mut client = client(client_end).await;
    let mut events = client.i_have_events();
    client
        .who_has(WhoHasObject::Name(NAME.into()), None)
        .await
        .unwrap();
    let event = tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("an I-Have arrives")
        .unwrap();
    assert_eq!(event.i_have.object_identifier, supply_air());
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// Client on network 1, the router at MAC 1 there, and the server as
/// station 5 on network 2.
async fn routed_site() -> (
    BACnetRouter,
    BACnetServer<LoopbackTransport>,
    BACnetClient<LoopbackTransport>,
) {
    let (client_end, router_near) = LoopbackTransport::pair(vec![0x0A], vec![ROUTER_MAC]);
    let (router_far, station) = LoopbackTransport::pair(vec![0x02], vec![STATION]);
    let ports = vec![
        RouterPort {
            transport: AnyTransport::Loopback(router_near),
            network_number: 1,
        },
        RouterPort {
            transport: AnyTransport::Loopback(router_far),
            network_number: REMOTE,
        },
    ];
    let StartedRouter { router, .. } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    let server = server(station).await;
    let client = client(client_end).await;
    // Let the router's start-up announcements go by, so a test hears only
    // what answers it.
    tokio::time::sleep(Duration::from_millis(100)).await;
    (router, server, client)
}

async fn next_report(reports: &mut Receiver<ReceivedNetworkReport>) -> ReceivedNetworkReport {
    tokio::time::timeout(WAIT, reports.recv())
        .await
        .expect("a network report arrives")
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn the_router_and_its_remote_network_are_found_and_reached() {
    let (mut router, mut server, mut client) = routed_site().await;
    let expected = DiscoveredRouter {
        mac: [ROUTER_MAC].as_slice().into(),
        networks: vec![REMOTE],
    };
    for network in [None, Some(REMOTE)] {
        let routers = client.discover_routers(network, WAIT).await.unwrap();
        assert_eq!(
            routers,
            std::slice::from_ref(&expected),
            "asking about {network:?}"
        );
    }
    // A network nobody reaches: the router searches further and stays
    // quiet, so nothing is listed.
    assert_eq!(client.discover_routers(Some(77), WAIT).await.unwrap(), []);
    // Nor is the router kept when, during that wait, it lists only others.
    let (scoped, asked) = tokio::join!(client.discover_routers(Some(77), WAIT), async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        client.who_is_router_to_network(None, None).await
    });
    asked.unwrap();
    assert_eq!(scoped.unwrap(), []);
    assert!(client.discover_routers(Some(0), WAIT).await.is_err());
    assert!(client.discover_routers(Some(u16::MAX), WAIT).await.is_err());

    // The device behind the router is read through it.
    let ack = client
        .read_property_routed(
            &[ROUTER_MAC],
            REMOTE,
            &[STATION],
            supply_air(),
            PropertyIdentifier::OBJECT_NAME,
            None,
        )
        .await
        .unwrap();
    let value = bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0)
        .unwrap()
        .0;
    assert_eq!(value, PropertyValue::CharacterString(NAME.into()));

    client.stop().await.unwrap();
    server.stop().await.unwrap();
    router.stop().await.unwrap();
}

/// The Who-Has is a global broadcast, so the router passes it on to network
/// 2 and the server's I-Have comes back through it, naming its network.
#[tokio::test(start_paused = true)]
async fn who_has_finds_an_object_behind_the_router() {
    let (mut router, mut server, mut client) = routed_site().await;
    let replies = client
        .who_has_replies(WhoHasObject::Name(NAME.into()), None, WAIT)
        .await
        .unwrap();
    assert_eq!(replies.len(), 1, "{replies:?}");
    assert_eq!(replies[0].i_have.device_identifier, device());
    assert_eq!(replies[0].source_mac.as_slice(), [ROUTER_MAC]);
    assert_eq!(
        replies[0].source_network.as_ref().map(|s| s.network),
        Some(REMOTE)
    );
    client.stop().await.unwrap();
    server.stop().await.unwrap();
    router.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_reject_for_an_unknown_network_is_surfaced() {
    let (mut router, mut server, mut client) = routed_site().await;
    let mut reports = client.network_reports();
    client.who_is_network(99, None).await.unwrap();
    let received = next_report(&mut reports).await;
    assert_eq!(received.source_mac.as_slice(), [ROUTER_MAC]);
    assert_eq!(
        received.report,
        NetworkReport::RejectMessageToNetwork(RejectMessageToNetwork {
            reason: RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            network: 99,
        })
    );
    client.stop().await.unwrap();
    server.stop().await.unwrap();
    router.stop().await.unwrap();
}

/// The router knows network 1's number, configured, and says so when asked.
#[tokio::test(start_paused = true)]
async fn the_router_answers_what_is_network_number() {
    let (mut router, mut server, mut client) = routed_site().await;
    let mut reports = client.network_reports();
    client.what_is_network_number(None).await.unwrap();
    let received = loop {
        let received = next_report(&mut reports).await;
        if matches!(received.report, NetworkReport::NetworkNumberIs { .. }) {
            break received;
        }
    };
    assert_eq!(received.source_mac.as_slice(), [ROUTER_MAC]);
    assert_eq!(
        received.report,
        NetworkReport::NetworkNumberIs {
            network: 1,
            configured: true
        }
    );
    client.stop().await.unwrap();
    server.stop().await.unwrap();
    router.stop().await.unwrap();
}
