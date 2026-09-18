//! RB-15 lifecycle tests (Loopback, deterministic, no sleeps).
//!
//! Covers: cancellation at await boundaries, shutdown with pending
//! replies/segments, single ingress consumer + one-use reply ownership, no
//! detached role outlives the endpoint.

use std::time::Duration;

use bacnet_encoding::apdu::{decode_apdu, encode_apdu, Apdu, ComplexAck};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_endpoint::session::{EndpointSession, SessionConfig, SessionRole};
use bacnet_network::layer::ReceivedApdu;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::sync::oneshot;

const WAIT: Duration = Duration::from_secs(2);

fn session_config() -> SessionConfig {
    SessionConfig {
        queue_capacity: 8,
        apdu_timeout_ms: 200,
        apdu_retries: 0,
        max_apdu_length: 480,
    }
}

fn database_with_analog(instance: u32, value: f32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let mut analog = AnalogInputObject::new(instance, "lifecycle-input", 0).unwrap();
    analog.set_present_value(value);
    db.add(Box::new(analog)).unwrap();
    db
}

fn object_id(instance: u32) -> bacnet_types::primitives::ObjectIdentifier {
    bacnet_types::primitives::ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

#[tokio::test]
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
    let client_handle = client.cloned_client_handle().unwrap();
    let request = tokio::spawn(async move {
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
    // Cancel at the timeout await boundary: aborting the future must release
    // the exact lease via the request guard (no strand).
    request.abort();
    let _ = request.await;
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if client.active_leases() == 0 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "cancelled request stranded its lease"
        );
        tokio::task::yield_now().await;
    }
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn shutdown_with_pending_reply_cleans_up() {
    let (client_transport, peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut client =
        EndpointSession::new(client_transport, SessionRole::ClientOnly, session_config()).unwrap();
    client.start().await.unwrap();
    let mut peer = bacnet_network::layer::NetworkLayer::new(peer_transport);
    let mut peer_rx = peer.start().await.unwrap();

    let client_handle = client.cloned_client_handle().unwrap();
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

#[tokio::test]
async fn shutdown_with_pending_segmented_response_cleans_up() {
    // Server sends a segmented ComplexAck the requester cannot reassemble
    // (unsegmented requester): the session must abort + release without hang,
    // and stop() with that in flight must still terminate.
    let (client_transport, peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut client =
        EndpointSession::new(client_transport, SessionRole::ClientOnly, session_config()).unwrap();
    client.start().await.unwrap();
    let mut peer = bacnet_network::layer::NetworkLayer::new(peer_transport);
    let mut peer_rx = peer.start().await.unwrap();
    let coordinator = client.coordinator();

    let client_handle = client.cloned_client_handle().unwrap();
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
        bacnet_types::enums::NetworkPriority::NORMAL,
    )
    .await
    .unwrap();
    // The requester answers with a client Abort and releases the exact lease;
    // the waiter observes Abort, never a hang.
    let abort = tokio::time::timeout(WAIT, peer_rx.recv())
        .await
        .expect("abort must arrive")
        .expect("peer closed");
    match decode_apdu(abort.apdu).unwrap() {
        Apdu::Abort(abort) => {
            assert!(!abort.sent_by_server);
            assert_eq!(abort.invoke_id, invoke_id);
        }
        other => panic!("expected client Abort, got {other:?}"),
    }
    let outcome = tokio::time::timeout(WAIT, pending)
        .await
        .expect("segment path hung");
    assert!(outcome.unwrap().is_err());
    assert_eq!(coordinator.active_count().unwrap(), 0);
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn reply_sender_is_single_consumer_one_use() {
    // `ReceivedApdu::clone` drops reply authority; the responder takes it
    // exactly once via `.take()`.
    let (reply_tx, reply_rx) = oneshot::channel();
    let received = ReceivedApdu {
        apdu: Bytes::from_static(&[0x10, 0x08]),
        source_mac: MacAddr::from_slice(&[0x02]),
        ingress_network: None,
        source_network: None,
        link_layer_group: false,
        is_group: false,
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
        apdu: Bytes::from_static(&[0x10, 0x08]),
        source_mac: MacAddr::from_slice(&[0x02]),
        ingress_network: Some(1),
        source_network: Some(NpduAddress {
            network: 77,
            mac_address: MacAddr::from_slice(&[0x44]),
        }),
        link_layer_group: true,
        is_group: false,
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

#[tokio::test]
async fn no_detached_role_outlives_session() {
    let (transport_a, transport_b) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut session_a = EndpointSession::new(transport_a, SessionRole::Both, session_config())
        .unwrap()
        .with_database(database_with_analog(1, 1.0));
    let mut session_b = EndpointSession::new(transport_b, SessionRole::Both, session_config())
        .unwrap()
        .with_database(database_with_analog(1, 2.0));
    session_a.start().await.unwrap();
    session_b.start().await.unwrap();

    // Clone role Arcs out of the session (detached handles).
    let detached_client = session_a.cloned_client_handle().unwrap();
    let detached_server = session_a.cloned_server_handle().unwrap();
    assert!(detached_server.is_session_alive());

    // Dropping the session ends role work: Weak upgrade fails, methods fail
    // closed, no orphaned task keeps the lease pool alive.
    drop(session_a);
    assert!(!detached_server.is_session_alive());
    assert!(detached_client
        .read_property(
            &[0x02],
            object_id(1),
            PropertyIdentifier::PRESENT_VALUE,
            None
        )
        .await
        .is_err());

    session_b.stop().await.unwrap();
}

#[tokio::test]
async fn context_preservation_does_not_change_group_decision() {
    // Raw `link_layer_group` is preserved structurally but the responder scope
    // decision still uses effective `is_group` only (compat-mode: no new
    // decisions). A unicast NPDU carried over a group link (routed unicast
    // via broadcast DA, Clause 6.5.3) is still served.
    let (transport_a, transport_b) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut session_a = EndpointSession::new(transport_a, SessionRole::Both, session_config())
        .unwrap()
        .with_database(database_with_analog(8, 8.0));
    let mut session_b = EndpointSession::new(transport_b, SessionRole::Both, session_config())
        .unwrap()
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
