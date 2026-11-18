//! RB-15 lease + queue ownership tests (Loopback, deterministic).
//!
//! Covers: lease exhaustion → release → reuse, lost consumer, a full role
//! queue, ingress policy ownership.

use std::time::Duration;

#[path = "support/eventually.rs"]
mod eventually;
use eventually::eventually;

use bacnet_encoding::apdu::{Apdu, ConfirmedRequest, decode_apdu, encode_apdu};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_endpoint::session::{EndpointSession, SessionConfig, SessionRole};
use bacnet_endpoint_core::coordinator::{CanonicalPeer, LeaseMetadata, TerminalPolicy};
use bacnet_endpoint_core::endpoint_ingress::EndpointIngress;
use bacnet_network::layer::NetworkLayer;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::{
    ConfirmedServiceChoice, EngineeringUnits, ObjectType, PropertyIdentifier,
};
use bytes::BytesMut;

const WAIT: Duration = Duration::from_secs(2);

fn session_config() -> SessionConfig {
    SessionConfig {
        queue_capacity: 4,
        apdu_timeout_ms: 200,
        apdu_retries: 0,
        max_apdu_length: 480,
        ..SessionConfig::default()
    }
}

fn database_with_analog(instance: u32, value: f32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let mut analog =
        AnalogInputObject::new(instance, "lease-input", EngineeringUnits::SQUARE_METERS).unwrap();
    analog.set_present_value(value);
    db.add(Box::new(analog)).unwrap();
    db
}

fn object_id(instance: u32) -> bacnet_types::primitives::ObjectIdentifier {
    bacnet_types::primitives::ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

#[tokio::test(start_paused = true)]
async fn lease_exhaustion_release_reuse() {
    let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut session =
        EndpointSession::new(transport, SessionRole::ClientOnly, session_config()).unwrap();
    session.start().await.unwrap();
    let coordinator = session.coordinator();

    // Exhaust the device-wide pool of 256 invoke IDs.
    let mut tokens = Vec::new();
    for _ in 0..256 {
        let token = coordinator
            .reserve(LeaseMetadata::requester(
                CanonicalPeer::direct(&[0x02]),
                ConfirmedServiceChoice::READ_PROPERTY,
                TerminalPolicy::ComplexAck,
            ))
            .expect("reserve must succeed below 256");
        tokens.push(token);
    }
    assert!(
        coordinator
            .reserve(LeaseMetadata::requester(
                CanonicalPeer::direct(&[0x02]),
                ConfirmedServiceChoice::READ_PROPERTY,
                TerminalPolicy::ComplexAck,
            ))
            .is_err()
    );
    assert_eq!(coordinator.active_count().unwrap(), 256);

    // Release one lease → reuse succeeds with the same numeric ID eventually.
    let released = tokens.pop().unwrap();
    let invoke_id = released.invoke_id();
    coordinator.release(released).unwrap();
    assert_eq!(coordinator.active_count().unwrap(), 255);
    let reused = coordinator
        .reserve(LeaseMetadata::requester(
            CanonicalPeer::direct(&[0x02]),
            ConfirmedServiceChoice::READ_PROPERTY,
            TerminalPolicy::ComplexAck,
        ))
        .expect("reuse after release must succeed");
    assert_eq!(reused.invoke_id(), invoke_id);
    tokens.push(reused);

    // Release everything; the pool is fully reusable.
    for token in tokens {
        coordinator.release(token).unwrap();
    }
    assert_eq!(coordinator.active_count().unwrap(), 0);
    coordinator
        .reserve(LeaseMetadata::requester(
            CanonicalPeer::direct(&[0x02]),
            ConfirmedServiceChoice::READ_PROPERTY,
            TerminalPolicy::ComplexAck,
        ))
        .expect("pool must be reusable after full release");

    session.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn lost_consumer_routes_to_policy_without_hanging() {
    // Client-only session has no server role: an inbound ConfirmedRequest
    // from the peer is policy-owned (`no_server_role`), not served.
    let (client_transport, peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut client =
        EndpointSession::new(client_transport, SessionRole::ClientOnly, session_config()).unwrap();
    client.start().await.unwrap();
    let mut peer = NetworkLayer::new(peer_transport);
    let _peer_rx = peer.start().await.unwrap();

    // Craft a direct ReadProperty request from the peer side.
    let mut service = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: object_id(9),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut service);
    let request = Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: 7,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: service.freeze(),
    });
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &request).unwrap();
    peer.send_apdu(
        &encoded,
        &[0x01],
        true,
        bacnet_npdu::NetworkPriority::Normal,
    )
    .await
    .unwrap();

    // The dispatch task routes the request to policy without a server role.
    eventually(WAIT, "lost inbound was not policy-owned", async || {
        client.policy_counters().await.no_server_role >= 1
    })
    .await;

    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_full_ingress_queue_drops_and_counts_instead_of_blocking() {
    // Ingress with capacity 1 that nothing reads: the first request takes
    // the one slot, the next two find it full and are dropped and counted
    // (#2635), never handed to policy, and the sender never waits.
    let (endpoint_transport, peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut endpoint = EndpointIngress::new(endpoint_transport, 1);
    let mut ingress = endpoint.start().await.unwrap();
    let mut peer = NetworkLayer::new(peer_transport);
    let _peer_rx = peer.start().await.unwrap();

    let mut service = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: object_id(3),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut service);
    for invoke_id in [10u8, 11, 12] {
        let request = Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 480,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: service.clone().freeze(),
        });
        let mut encoded = BytesMut::new();
        encode_apdu(&mut encoded, &request).unwrap();
        peer.send_apdu(
            &encoded,
            &[0x01],
            true,
            bacnet_npdu::NetworkPriority::Normal,
        )
        .await
        .unwrap();
    }
    eventually(WAIT, "the two that found no room counted", async || {
        endpoint.full_drop_counters().confirmed_full_drops == 2
    })
    .await;
    let first = ingress.inbound_requests.recv().await.unwrap();
    match decode_apdu(first.apdu.clone()).unwrap() {
        Apdu::ConfirmedRequest(request) => assert_eq!(request.invoke_id, 10),
        other => panic!("expected request, got {other:?}"),
    }
    assert!(ingress.inbound_requests.try_recv().is_err());
    assert!(ingress.policy_outcomes.try_recv().is_err());
    endpoint.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn malformed_apdu_is_policy_owned() {
    let (client_transport, peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut client = EndpointSession::new(client_transport, SessionRole::Both, session_config())
        .unwrap()
        .device(bacnet_endpoint::DeviceIdentity::new(901, 42).unwrap())
        .with_database(database_with_analog(5, 5.0));
    client.start().await.unwrap();
    let mut peer = NetworkLayer::new(peer_transport);
    let _peer_rx = peer.start().await.unwrap();

    // Und decodable APDU (empty payload decodes as malformed at classifier).
    peer.send_apdu(&[], &[0x01], false, bacnet_npdu::NetworkPriority::Normal)
        .await
        .unwrap();

    eventually(WAIT, "malformed APDU was not policy-owned", async || {
        client.policy_counters().await.ingress_policy >= 1
    })
    .await;

    // Encoded property value helper is linked (structural preservation of the
    // service payload path); keep the import live without affecting policy.
    let mut probe = BytesMut::new();
    encode_property_value(
        &mut probe,
        &bacnet_types::primitives::PropertyValue::Real(1.0),
    )
    .unwrap();
    assert!(!probe.is_empty());

    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}
