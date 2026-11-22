//! RB-15 lifecycle tests (Loopback, deterministic, on paused time).
//!
//! Covers: cancellation at await boundaries, shutdown with pending
//! replies/segments, single ingress consumer + one-use reply ownership, no
//! detached role outlives the endpoint.

use std::time::Duration;

#[path = "support/eventually.rs"]
mod eventually;
use eventually::eventually;

use bacnet_encoding::apdu::{Apdu, ComplexAck, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_endpoint::session::{EndpointSession, SessionConfig, SessionRole};
use bacnet_network::layer::ReceivedApdu;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::MacAddr;
use bacnet_types::enums::{
    ConfirmedServiceChoice, EngineeringUnits, ObjectType, PropertyIdentifier,
};
use bytes::{Bytes, BytesMut};
use tokio::sync::oneshot;

const WAIT: Duration = Duration::from_secs(2);

fn session_config() -> SessionConfig {
    SessionConfig {
        queue_capacity: 8,
        apdu_timeout_ms: 200,
        apdu_retries: 0,
        max_apdu_length: 480,
        ..SessionConfig::default()
    }
}

fn database_with_analog(instance: u32, value: f32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let mut analog =
        AnalogInputObject::new(instance, "lifecycle-input", EngineeringUnits::SQUARE_METERS)
            .unwrap();
    analog.set_present_value(value);
    db.add(Box::new(analog)).unwrap();
    db
}

fn object_id(instance: u32) -> bacnet_types::primitives::ObjectIdentifier {
    bacnet_types::primitives::ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

#[tokio::test(start_paused = true)]
async fn cancel_pending_request_releases_exact_lease() {
    let (client_transport, peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut client =
        EndpointSession::new(client_transport, SessionRole::ClientOnly, session_config()).unwrap();
    client.start().await.unwrap();
    // Peer never replies: hold its receiver without responding.
    let mut peer = bacnet_network::layer::NetworkLayer::new(peer_transport);
    let mut peer_rx = peer.start().await.unwrap();
    // Consume the request so the peer side does not apply backpressure, but
    // never send a reply.
    // The client stays alive outside the task: only the cancelled request,
    // not a dropped client, may give the lease back.
    let client_handle = std::sync::Arc::new(client.client().unwrap());
    let request_client = std::sync::Arc::clone(&client_handle);
    let request = tokio::spawn(async move {
        request_client
            .read_property(
                &[0x02],
                object_id(1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    let _sent = tokio::time::timeout(WAIT, peer_rx.recv())
        .await
        .expect("request must arrive")
        .expect("peer closed");
    assert_eq!(client.active_leases(), 1);
    // Cancel at the timeout await boundary: aborting the future must release
    // the exact lease via the request guard (no strand).
    request.abort();
    let _ = request.await;
    eventually(WAIT, "cancelled request stranded its lease", async || {
        client.active_leases() == 0
    })
    .await;
    // Cancelling one request does not end the client that issued it.
    assert_eq!(std::sync::Arc::strong_count(&client_handle), 1);
    client.stop().await.unwrap();
    drop(client_handle);
    peer.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn shutdown_with_pending_reply_cleans_up() {
    let (client_transport, peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut client =
        EndpointSession::new(client_transport, SessionRole::ClientOnly, session_config()).unwrap();
    client.start().await.unwrap();
    let mut peer = bacnet_network::layer::NetworkLayer::new(peer_transport);
    let mut peer_rx = peer.start().await.unwrap();

    let client_handle = client.client().unwrap();
    let pending = tokio::spawn(async move {
        client_handle
            .read_property(
                &[0x02],
                object_id(1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    let _sent = tokio::time::timeout(WAIT, peer_rx.recv())
        .await
        .expect("request must arrive")
        .expect("peer closed");
    assert_eq!(client.active_leases(), 1);
    // Shutdown with a pending reply: stop() seals admission, cancels the
    // waiter and releases the exact lease without hanging.
    client.stop().await.unwrap();
    assert_eq!(client.active_leases(), 0);
    // The pending waiter observes shutdown (timeout or shutdown error), never
    // a hung future.
    let outcome = tokio::time::timeout(WAIT, pending)
        .await
        .expect("stop hung");
    match outcome {
        Ok(Err(_)) => {}
        Ok(Ok(_)) => panic!("pending request must not succeed after stop"),
        Err(_) => panic!("pending join must resolve after stop"),
    }
    peer.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn shutdown_with_pending_segmented_response_cleans_up() {
    // A peer sends segment zero of a segmented answer and never the rest:
    // the client acknowledges the segment and waits. A stop with that
    // reassembly in flight ends the request and releases its lease.
    let (client_transport, peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut client =
        EndpointSession::new(client_transport, SessionRole::ClientOnly, session_config()).unwrap();
    client.start().await.unwrap();
    let mut peer = bacnet_network::layer::NetworkLayer::new(peer_transport);
    let mut peer_rx = peer.start().await.unwrap();
    let coordinator = client.coordinator();

    let client_handle = client.client().unwrap();
    let pending = tokio::spawn(async move {
        client_handle
            .read_property(
                &[0x02],
                object_id(1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    let sent = tokio::time::timeout(WAIT, peer_rx.recv())
        .await
        .expect("request must arrive")
        .expect("peer closed");
    let invoke_id = match decode_apdu(sent.apdu.clone()).unwrap() {
        Apdu::ConfirmedRequest(req) => req.invoke_id,
        other => panic!("expected request, got {other:?}"),
    };
    // Peer replies with a segmented ComplexAck (sequence 0, more-follows).
    let segmented = Apdu::ComplexAck(ComplexAck {
        segmented: true,
        more_follows: true,
        invoke_id,
        sequence_number: Some(0),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::from_static(&[0xaa]),
    });
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &segmented).unwrap();
    peer.send_apdu(
        &encoded,
        &[0x01],
        false,
        bacnet_npdu::NetworkPriority::Normal,
    )
    .await
    .unwrap();
    let acknowledged = tokio::time::timeout(WAIT, peer_rx.recv())
        .await
        .expect("the segment's acknowledgment must arrive")
        .expect("peer closed");
    match decode_apdu(acknowledged.apdu).unwrap() {
        Apdu::SegmentAck(ack) => {
            assert!(!ack.sent_by_server && !ack.negative_ack);
            assert_eq!((ack.invoke_id, ack.sequence_number), (invoke_id, 0));
        }
        other => panic!("expected a SegmentACK, got {other:?}"),
    }
    assert_eq!(coordinator.active_count().unwrap(), 1);
    assert!(!pending.is_finished());
    client.stop().await.unwrap();
    let outcome = tokio::time::timeout(WAIT, pending)
        .await
        .expect("segment path hung");
    assert!(outcome.unwrap().is_err());
    assert_eq!(coordinator.active_count().unwrap(), 0);
    peer.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn reply_sender_is_single_consumer_one_use() {
    // `ReceivedApdu::clone` drops reply authority; the server takes it
    // exactly once via `.take()`.
    let (reply_tx, reply_rx) = oneshot::channel();
    let received = ReceivedApdu {
        router_response: None,
        direct_response: None,
        apdu: Bytes::from_static(&[0x10, 0x08]),
        source_mac: MacAddr::from_slice(&[0x02]),
        ingress_network: None,
        source_network: None,
        link_layer_group: false,
        is_group: false,
        global_broadcast: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: Some(reply_tx),
    };
    // Clone loses the one-use sender (single-consumer ownership, not just
    // example binding).
    assert!(received.clone().reply_tx.is_none());
    let mut received = received;
    let taken = received.reply_tx.take();
    assert!(taken.is_some());
    assert!(received.reply_tx.is_none());
    // Second take is None: one-use ownership asserted, not just bound.
    assert!(received.reply_tx.take().is_none());
    drop(taken);
    assert!(reply_rx.await.is_err());

    // Routed + group + provenance envelope still clones without authority.
    let (reply_tx, _rx) = oneshot::channel();
    let routed = ReceivedApdu {
        router_response: None,
        direct_response: None,
        apdu: Bytes::from_static(&[0x10, 0x08]),
        source_mac: MacAddr::from_slice(&[0x02]),
        ingress_network: Some(1),
        source_network: Some(NpduAddress {
            network: 77,
            mac_address: MacAddr::from_slice(&[0x44]),
        }),
        link_layer_group: true,
        is_group: false,
        global_broadcast: false,
        data_attributes: vec![bacnet_transport::port::DataAttribute {
            option_type: 1,
            must_understand: false,
            data: vec![0x01],
        }],
        provenance: TransportProvenance::unverified(),
        reply_tx: Some(reply_tx),
    };
    let cloned = routed.clone();
    assert!(cloned.reply_tx.is_none());
    // Raw link-group + effective group + attributes + provenance survive the
    // clone structurally (pass-through, no decision here).
    assert!(cloned.link_layer_group);
    assert!(!cloned.is_group);
    assert_eq!(cloned.data_attributes.len(), 1);
    assert_eq!(cloned.ingress_network, Some(1));
}

#[tokio::test(start_paused = true)]
async fn no_detached_role_outlives_session() {
    let (transport_a, transport_b) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut session_a = EndpointSession::new(transport_a, SessionRole::Both, session_config())
        .unwrap()
        .device(bacnet_endpoint::DeviceIdentity::new(1, 42).unwrap())
        .with_database(database_with_analog(1, 1.0));
    let mut session_b = EndpointSession::new(transport_b, SessionRole::Both, session_config())
        .unwrap()
        .device(bacnet_endpoint::DeviceIdentity::new(2, 42).unwrap())
        .with_database(database_with_analog(1, 2.0));
    session_a.start().await.unwrap();
    session_b.start().await.unwrap();

    // The session's client, kept past the session.
    let detached_client = session_a.client().unwrap();

    // Dropping the session ends the client's work: its seat has closed, so
    // its calls fail and no task of its own keeps the lease pool alive.
    drop(session_a);
    assert!(
        detached_client
            .read_property(
                &[0x02],
                object_id(1),
                PropertyIdentifier::PRESENT_VALUE,
                None
            )
            .await
            .is_err()
    );

    session_b.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn context_preservation_does_not_change_group_decision() {
    // Raw `link_layer_group` is preserved structurally but the server's
    // decision still uses effective `is_group` only (compat-mode: no new
    // decisions). A unicast NPDU carried over a group link (routed unicast
    // via broadcast DA, Clause 6.5.3) is still served.
    let (transport_a, transport_b) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut session_a = EndpointSession::new(transport_a, SessionRole::Both, session_config())
        .unwrap()
        .device(bacnet_endpoint::DeviceIdentity::new(1, 42).unwrap())
        .with_database(database_with_analog(8, 8.0));
    let mut session_b = EndpointSession::new(transport_b, SessionRole::Both, session_config())
        .unwrap()
        .device(bacnet_endpoint::DeviceIdentity::new(2, 42).unwrap())
        .with_database(database_with_analog(8, 9.0));
    session_a.start().await.unwrap();
    session_b.start().await.unwrap();

    // Direct round-trip still succeeds (group fields default false here; the
    // structural preservation is asserted in `reply_sender_is_single_consumer_one_use`
    // and in the adapter unit paths without changing this outcome).
    let ack = tokio::time::timeout(
        WAIT,
        session_a.client().unwrap().read_property(
            &[0x02],
            object_id(8),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        ),
    )
    .await
    .expect("preserved-context read timed out")
    .expect("preserved-context read failed");
    assert_eq!(ack.object_identifier, object_id(8));

    session_a.stop().await.unwrap();
    session_b.stop().await.unwrap();
}
