//! The client must not knowingly segment a confirmed request to a peer whose
//! authoritative Segmentation_Supported says it cannot receive segments.
//!
//! Clause 12.11: NO_SEGMENTATION and SEGMENTED_TRANSMIT peers accept exactly
//! one segment — only unsegmented requests. Clause 18: sending anyway yields
//! the peer's SEGMENTATION_NOT_SUPPORTED abort, so the client refuses locally
//! before allocating a transaction or transmitting (issue #371).
//!
//! Capability provenance matters: only I-Am-learned or explicitly supplied
//! capability is authoritative. Legacy `add_device` rows store placeholder
//! defaults and must not cause a local refusal (#372 follow-up constraint).
//!
//! New file: `tests.rs` is over the 700-LOC cap.

use std::time::Instant;

use bacnet_encoding::apdu::{self, Apdu, SegmentAck as SegmentAckPdu, SimpleAck};
use bacnet_encoding::npdu::{decode_npdu, Npdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, Segmentation};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use bytes::BytesMut;
use tokio::time::{timeout, Duration};

use crate::discovery::{DiscoveredDevice, RoutedDeviceConfig};

use super::BACnetClient;

const OVERSIZED: usize = 200;
const PEER_MAX_APDU: u32 = 128;

fn local_peer(
    instance: u32,
    mac: &[u8],
    max_apdu_length: u32,
    segmentation_supported: Segmentation,
) -> DiscoveredDevice {
    DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        mac_address: MacAddr::from_slice(mac),
        max_apdu_length,
        segmentation_supported,
        max_segments_accepted: None,
        vendor_id: 42,
        last_seen: Instant::now(),
        source_network: None,
        source_address: None,
    }
}

fn oversized_data(len: usize) -> Vec<u8> {
    (0..len as u16).map(|i| i as u8).collect()
}

/// Assert the error is the typed local segmentation refusal naming the
/// peer's advertised capability.
fn assert_segmentation_unsupported(err: Error, capability: &str) {
    let text = err.to_string();
    assert!(
        matches!(err, Error::Segmentation(_)),
        "expected Error::Segmentation, got: {text}"
    );
    assert!(
        text.contains(capability),
        "refusal must name the advertised capability {capability}, got: {text}"
    );
}

/// A refusal must leave the wire silent: no frame reaches the peer.
async fn assert_no_frame_reaches(
    peer_rx: &mut tokio::sync::mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
) {
    // A closed channel (the request task dropped its transport end) is not a
    // transmission; only a delivered NPDU fails this assertion.
    match timeout(Duration::from_millis(50), peer_rx.recv()).await {
        Err(_) | Ok(None) => {}
        Ok(Some(received)) => panic!(
            "no frame may be transmitted for a locally refused request; got npdu {:x?}",
            received.npdu
        ),
    }
}

async fn send_local_reply(peer_transport: &LoopbackTransport, client_mac: &[u8], apdu: Apdu) {
    let mut apdu_buf = BytesMut::new();
    bacnet_encoding::apdu::encode_apdu(&mut apdu_buf, &apdu).expect("valid APDU encoding");
    let npdu = Npdu {
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    bacnet_encoding::npdu::encode_npdu(&mut npdu_buf, &npdu).unwrap();
    peer_transport
        .send_unicast(&npdu_buf, client_mac)
        .await
        .unwrap();
}

struct ScriptedExchange {
    all_service_data: Vec<u8>,
    every_apdu_segmented: bool,
}

/// Drive one segmented send to completion against a scripted peer that
/// acknowledges each segment (window 1) and finally SimpleAcks.
async fn script_segmented_exchange(
    peer_rx: &mut tokio::sync::mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
    peer_transport: &LoopbackTransport,
    client_mac: &[u8],
    service_choice: ConfirmedServiceChoice,
) -> ScriptedExchange {
    let mut all_service_data = Vec::new();
    let mut every_apdu_segmented = true;
    let invoke_id = loop {
        let received = timeout(Duration::from_secs(2), peer_rx.recv())
            .await
            .expect("timed out waiting for a segment")
            .expect("peer channel closed");
        let npdu = decode_npdu(received.npdu).unwrap();
        let decoded = apdu::decode_apdu(npdu.payload).unwrap();
        let Apdu::ConfirmedRequest(req) = decoded else {
            panic!("Expected ConfirmedRequest, got {decoded:?}");
        };
        if !req.segmented {
            every_apdu_segmented = false;
        }
        let seq = req.sequence_number.unwrap();
        all_service_data.extend_from_slice(&req.service_request);

        let seg_ack = Apdu::SegmentAck(SegmentAckPdu {
            negative_ack: false,
            sent_by_server: true,
            invoke_id: req.invoke_id,
            sequence_number: seq,
            actual_window_size: 1,
        });
        send_local_reply(peer_transport, client_mac, seg_ack).await;

        if !req.more_follows {
            break req.invoke_id;
        }
    };

    send_local_reply(
        peer_transport,
        client_mac,
        Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice,
        }),
    )
    .await;

    ScriptedExchange {
        all_service_data,
        every_apdu_segmented,
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Authoritative refusals (red discriminators against #371's base)
// ──────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn authoritative_local_no_segmentation_refuses_segmented_request() {
    let client_mac = vec![0x01];
    let peer_mac = vec![0x02];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(1000)
        .build()
        .await
        .unwrap();

    // Explicit upsert is authoritative configuration (Clause 12.11 values).
    client.device_table.lock().await.upsert(local_peer(
        1234,
        &peer_mac,
        PEER_MAX_APDU,
        Segmentation::NONE,
    ));

    let service_data = oversized_data(OVERSIZED);
    let peer_mac_for_request = peer_mac.clone();
    let request_task = tokio::spawn(async move {
        client
            .confirmed_request(
                &peer_mac_for_request,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await
    });

    let err = timeout(Duration::from_secs(2), request_task)
        .await
        .expect("refusal must be immediate, not a timeout")
        .unwrap()
        .unwrap_err();
    assert_segmentation_unsupported(err, "NONE");
    assert_no_frame_reaches(&mut peer_rx).await;
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn authoritative_local_transmit_only_refuses_segmented_request() {
    let client_mac = vec![0x01];
    let peer_mac = vec![0x02];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(1000)
        .build()
        .await
        .unwrap();

    client.device_table.lock().await.upsert(local_peer(
        1234,
        &peer_mac,
        PEER_MAX_APDU,
        Segmentation::TRANSMIT,
    ));

    let service_data = oversized_data(OVERSIZED);
    let peer_mac_for_request = peer_mac.clone();
    let request_task = tokio::spawn(async move {
        client
            .confirmed_request(
                &peer_mac_for_request,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await
    });

    let err = timeout(Duration::from_secs(2), request_task)
        .await
        .expect("refusal must be immediate, not a timeout")
        .unwrap()
        .unwrap_err();
    assert_segmentation_unsupported(err, "TRANSMIT");
    assert_no_frame_reaches(&mut peer_rx).await;
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn authoritative_routed_no_segmentation_refuses_segmented_request() {
    let client_mac = vec![0x01];
    let router_mac = vec![0x02];
    let remote_network = 100;
    let remote_mac = vec![0x03];
    let (client_transport, mut router_transport) =
        LoopbackTransport::pair(client_mac.clone(), router_mac.clone());
    let mut router_rx = router_transport.start().await.unwrap();

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(1000)
        .build()
        .await
        .unwrap();

    // add_routed_device capability is explicitly supplied -> authoritative.
    client
        .add_routed_device(RoutedDeviceConfig {
            instance: 3003,
            router_mac: router_mac.clone(),
            remote_network,
            remote_mac: remote_mac.clone(),
            max_apdu_length: PEER_MAX_APDU,
            segmentation_supported: Segmentation::NONE,
            max_segments_accepted: None,
        })
        .await
        .unwrap();

    let service_data = oversized_data(OVERSIZED);
    let router_mac_for_request = router_mac.clone();
    let remote_mac_for_request = remote_mac.clone();
    let request_task = tokio::spawn(async move {
        client
            .confirmed_request_routed(
                &router_mac_for_request,
                remote_network,
                &remote_mac_for_request,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await
    });

    let err = timeout(Duration::from_secs(2), request_task)
        .await
        .expect("refusal must be immediate, not a timeout")
        .unwrap()
        .unwrap_err();
    assert_segmentation_unsupported(err, "NONE");
    assert_no_frame_reaches(&mut router_rx).await;
    router_transport.stop().await.unwrap();
}

// ──────────────────────────────────────────────────────────────────────────
// Receive-capable peers still segment; fitting requests stay unsegmented
// ──────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn authoritative_receive_peer_still_segments_successfully() {
    let client_mac = vec![0x01];
    let peer_mac = vec![0x02];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    client.device_table.lock().await.upsert(local_peer(
        1234,
        &peer_mac,
        PEER_MAX_APDU,
        Segmentation::RECEIVE,
    ));

    let expected = oversized_data(OVERSIZED);
    let service_data = expected.clone();
    let peer_mac_for_request = peer_mac.clone();
    let request_task = tokio::spawn(async move {
        let result = client
            .confirmed_request(
                &peer_mac_for_request,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    let exchange = script_segmented_exchange(
        &mut peer_rx,
        &peer_transport,
        &client_mac,
        ConfirmedServiceChoice::WRITE_PROPERTY,
    )
    .await;
    assert_eq!(exchange.all_service_data, expected);
    assert!(
        exchange.every_apdu_segmented,
        "an oversized request to a RECEIVE peer segments"
    );

    let result = request_task.await.unwrap();
    assert!(result.unwrap().is_empty());
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn fitting_request_to_no_segmentation_peer_still_sends() {
    let client_mac = vec![0x01];
    let peer_mac = vec![0x02];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    client.device_table.lock().await.upsert(local_peer(
        1234,
        &peer_mac,
        PEER_MAX_APDU,
        Segmentation::NONE,
    ));

    let service_data = oversized_data(10);
    let peer_mac_for_request = peer_mac.clone();
    let request_task = tokio::spawn(async move {
        let result = client
            .confirmed_request(
                &peer_mac_for_request,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    // The request fits inside the peer's Max APDU, so it goes out whole even
    // though the peer cannot receive segments.
    let received = timeout(Duration::from_secs(2), peer_rx.recv())
        .await
        .expect("timed out waiting for the request")
        .expect("peer channel closed");
    let npdu = decode_npdu(received.npdu).unwrap();
    let decoded = apdu::decode_apdu(npdu.payload).unwrap();
    let Apdu::ConfirmedRequest(req) = decoded else {
        panic!("Expected ConfirmedRequest, got {decoded:?}");
    };
    assert!(!req.segmented, "a fitting request must not be segmented");

    send_local_reply(
        &peer_transport,
        &client_mac,
        Apdu::SimpleAck(SimpleAck {
            invoke_id: req.invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .await;

    let result = request_task.await.unwrap();
    assert!(result.unwrap().is_empty());
    peer_transport.stop().await.unwrap();
}

// ──────────────────────────────────────────────────────────────────────────
// Provenance: legacy placeholder stays unknown; refresh becomes authoritative
// ──────────────────────────────────────────────────────────────────────────

/// Legacy `add_device` stores placeholder NONE/1476 that are NOT I-Am
/// evidence: an oversized request must still enter the segmented path rather
/// than being locally refused on placeholder values (#371 constraint from
/// the Round 3 review).
#[tokio::test]
async fn legacy_add_device_placeholder_does_not_refuse_segmented_request() {
    let client_mac = vec![0x01];
    let peer_mac = vec![0x02];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    client.add_device(1234, &peer_mac).await.unwrap();

    // 1600 octets exceed the placeholder Max APDU of 1476, so sizing already
    // requires segmentation; the placeholder NONE must not refuse it.
    let expected = oversized_data(1600);
    let service_data = expected.clone();
    let peer_mac_for_request = peer_mac.clone();
    let request_task = tokio::spawn(async move {
        let result = client
            .confirmed_request(
                &peer_mac_for_request,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    let exchange = script_segmented_exchange(
        &mut peer_rx,
        &peer_transport,
        &client_mac,
        ConfirmedServiceChoice::WRITE_PROPERTY,
    )
    .await;
    assert_eq!(exchange.all_service_data, expected);
    assert!(
        exchange.every_apdu_segmented,
        "the request must actually enter the segmented path, proving no placeholder refusal"
    );

    let result = request_task.await.unwrap();
    assert!(result.unwrap().is_empty());
    peer_transport.stop().await.unwrap();
}

/// A legacy placeholder row refreshed through an explicit authoritative
/// upsert becomes authoritative: the same oversized request that previously
/// segmented is now refused locally (provenance transition + explicit-upsert
/// semantics in one request-path test).
#[tokio::test]
async fn explicit_upsert_refresh_makes_placeholder_authoritative() {
    let client_mac = vec![0x01];
    let peer_mac = vec![0x02];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(1000)
        .build()
        .await
        .unwrap();

    // Placeholder row via legacy add_device: oversized requests segment.
    client.add_device(1234, &peer_mac).await.unwrap();

    // Authoritative replacement of the same instance: now NONE at a small
    // Max APDU is real configuration, not a placeholder.
    client.device_table.lock().await.upsert(local_peer(
        1234,
        &peer_mac,
        PEER_MAX_APDU,
        Segmentation::NONE,
    ));

    let service_data = oversized_data(OVERSIZED);
    let peer_mac_for_request = peer_mac.clone();
    let request_task = tokio::spawn(async move {
        client
            .confirmed_request(
                &peer_mac_for_request,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await
    });

    let err = timeout(Duration::from_secs(2), request_task)
        .await
        .expect("refusal must be immediate, not a timeout")
        .unwrap()
        .unwrap_err();
    assert_segmentation_unsupported(err, "NONE");
    assert_no_frame_reaches(&mut peer_rx).await;
    peer_transport.stop().await.unwrap();
}

/// An authoritative row carrying an unknown raw Segmentation value must not
/// be classified as a known receive prohibition: the oversized request still
/// enters the segmented path (Clause 12.11 enumerates only the four named
/// capabilities; nothing here invents receive capability either way).
#[tokio::test]
async fn unknown_raw_capability_is_not_treated_as_no_segmentation() {
    let client_mac = vec![0x01];
    let peer_mac = vec![0x02];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    client.device_table.lock().await.upsert(local_peer(
        1234,
        &peer_mac,
        PEER_MAX_APDU,
        Segmentation::from_raw(200),
    ));

    let expected = oversized_data(OVERSIZED);
    let service_data = expected.clone();
    let peer_mac_for_request = peer_mac.clone();
    let request_task = tokio::spawn(async move {
        let result = client
            .confirmed_request(
                &peer_mac_for_request,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    let exchange = script_segmented_exchange(
        &mut peer_rx,
        &peer_transport,
        &client_mac,
        ConfirmedServiceChoice::WRITE_PROPERTY,
    )
    .await;
    assert_eq!(exchange.all_service_data, expected);
    assert!(exchange.every_apdu_segmented);

    let result = request_task.await.unwrap();
    assert!(result.unwrap().is_empty());
    peer_transport.stop().await.unwrap();
}
