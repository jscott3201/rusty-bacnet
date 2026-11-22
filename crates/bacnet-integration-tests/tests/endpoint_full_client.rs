//! `bacnet_ops::Client` over an endpoint session's client (#2617): the
//! high-level client runs unchanged on a node that also serves a device,
//! through the node's one transport.
#![allow(dead_code)]

#[path = "../../bacnet-endpoint/tests/support/wire.rs"]
mod wire;

use std::time::Duration;

use bacnet_endpoint::session::{EndpointSession, SessionConfig, SessionRole};
use bacnet_endpoint::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_ops::{Client, Discover};
use bacnet_server::server::BACnetServer;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use wire::Wire;

const NODE: u8 = 1;
const PEER: u8 = 2;
const OTHER: u8 = 3;
const NODE_DEVICE: u32 = 1001;
const PEER_DEVICE: u32 = 2002;

/// A device with AI-1 at `value`.
fn device(instance: u32, value: f32) -> (DeviceIdentity, bacnet_objects::database::ObjectDatabase) {
    let identity = DeviceIdentity::new(instance, 42).unwrap();
    let mut input =
        AnalogInputObject::new(1, "Zone Temp", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    input.set_present_value(value);
    let database = build_database_with_extra(&identity, vec![Box::new(input)]).unwrap();
    (identity, database)
}

#[tokio::test(start_paused = true)]
async fn the_high_level_client_discovers_and_reads_over_a_nodes_client() {
    let wire = Wire::new();
    let (identity, database) = device(PEER_DEVICE, 21.5);
    let mut peer = BACnetServer::generic_builder()
        .device(identity)
        .database(database)
        .transport(wire.port(PEER).build())
        .build()
        .await
        .unwrap();
    let (identity, database) = device(NODE_DEVICE, 20.0);
    let port = wire.port(NODE);
    let link = port.handle();
    let mut session =
        EndpointSession::new(port.build(), SessionRole::Both, SessionConfig::default())
            .unwrap()
            .device(identity)
            .with_database(database);
    session.start().await.unwrap();

    let client = Client::over(session.client().unwrap());
    // Discovery in each scope the link can answer: this network, every
    // network, and the one station.
    let wait = Duration::from_millis(500);
    for request in [Discover::local(), Discover::global()] {
        let found = client.discover(request.wait(wait)).await.unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].instance, PEER_DEVICE);
        // Past the peer's window for taking a Who-Is as a repeat.
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
    let found = client.discover(Discover::local().wait(wait)).await.unwrap();
    let address = found[0].address.clone();
    tokio::time::sleep(Duration::from_secs(30)).await;
    let directed = client
        .discover(Discover::directed(address.clone().into()).wait(wait))
        .await
        .unwrap();
    assert_eq!(directed.len(), 1);

    // A device handle by address and by instance, and a read through each.
    let by_address = client.device(address).await.unwrap();
    assert_eq!(by_address.read("ai:1").await.unwrap().as_f64(), Some(21.5));
    let by_instance = client.device(PEER_DEVICE).await.unwrap();
    assert_eq!(
        by_instance.read("ai:1 pv").await.unwrap().as_f64(),
        Some(21.5)
    );

    // The node still serves its own device to others, on the same link.
    let other = bacnet_client::client::BACnetClient::generic_builder()
        .transport(wire.port(OTHER).build())
        .apdu_timeout_ms(1_000)
        .build()
        .await
        .unwrap();
    let name = other
        .read_property(
            &Wire::mac(NODE),
            ObjectIdentifier::new(ObjectType::DEVICE, NODE_DEVICE).unwrap(),
            PropertyIdentifier::OBJECT_NAME,
            None,
        )
        .await;
    assert!(name.is_ok(), "{:?}", name.err());
    assert_eq!(link.starts(), 1, "one transport for both roles");

    // Every handle on the full client goes before the node stops its link.
    drop((by_address, by_instance, client, other));
    session.stop().await.unwrap();
    assert_eq!(link.stops(), 1);
    peer.stop().await.unwrap();
}
