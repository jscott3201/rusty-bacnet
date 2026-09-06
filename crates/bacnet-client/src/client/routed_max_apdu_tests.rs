//! The routed peer's advertised Max APDU Length Accepted bounds the request.
//!
//! Clause 5.2.1.2 term (c) binds the remote peer's limit with no exemption
//! for a destination reached through a router, and the client records that
//! limit from the I-Am's 'Max APDU Length Accepted' parameter, keyed by the
//! SNET/SADR of the NPDU that carried it — so a routed request must honor it
//! exactly as a local request does (issue #362).
//!
//! Split from `tests.rs`, which is at the 700-LOC cap.

use std::time::Instant;

use bacnet_encoding::apdu::{self, Apdu, SegmentAck as SegmentAckPdu, SimpleAck};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use tokio::time::{timeout, Duration};

use crate::discovery::{DiscoveredDevice, RoutedDeviceConfig};

use super::tests::send_routed_response;
use super::BACnetClient;

fn routed_peer(
    max_apdu_length: u32,
    router_mac: &[u8],
    network: u16,
    mac: &[u8],
) -> DiscoveredDevice {
    DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 3003).unwrap(),
        mac_address: MacAddr::from_slice(router_mac),
        max_apdu_length,
        segmentation_supported: Segmentation::BOTH,
        max_segments_accepted: None,
        vendor_id: 42,
        last_seen: Instant::now(),
        source_network: Some(network),
        source_address: Some(MacAddr::from_slice(mac)),
    }
}

/// A routed peer advertising 128 on a 1476-octet transport forces
/// segmentation at 128 rather than the request going out whole.
#[tokio::test]
async fn routed_confirmed_request_honors_discovered_peer_max_apdu() {
    let client_mac = vec![0x01];
    let router_mac = vec![0x02];
    let remote_network = 100;
    let remote_mac = vec![0x03];
    let (client_transport, mut router_transport) =
        LoopbackTransport::pair(client_mac.clone(), router_mac.clone());
    let mut router_rx = router_transport.start().await.unwrap();

    // Local config and transport both allow 1476, so only the routed peer's
    // discovered limit of 128 can force segmentation of a 204-octet APDU.
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .max_apdu_length(1476)
        .build()
        .await
        .unwrap();

    client.device_table.lock().await.upsert(routed_peer(
        128,
        &router_mac,
        remote_network,
        &remote_mac,
    ));

    let service_data: Vec<u8> = (0..200u16).map(|i| i as u8).collect();
    let expected_data = service_data.clone();
    let router_mac_for_request = router_mac.clone();
    let remote_mac_for_request = remote_mac.clone();

    let request_task = tokio::spawn(async move {
        let result = client
            .confirmed_request_routed(
                &router_mac_for_request,
                remote_network,
                &remote_mac_for_request,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    let mut all_service_data = Vec::new();
    let invoke_id = loop {
        let received = timeout(Duration::from_secs(2), router_rx.recv())
            .await
            .expect("router timed out waiting for routed segment")
            .expect("router channel closed");

        let npdu = decode_npdu(received.npdu).unwrap();
        assert!(
            npdu.payload.len() <= 128,
            "APDU of {} octets exceeds the peer's advertised limit of 128",
            npdu.payload.len()
        );

        let decoded = apdu::decode_apdu(npdu.payload).unwrap();
        let Apdu::ConfirmedRequest(req) = decoded else {
            panic!("Expected ConfirmedRequest, got {:?}", decoded);
        };
        assert!(
            req.segmented,
            "request must segment at the routed peer's limit of 128, not go out whole"
        );
        let seq = req.sequence_number.unwrap();
        all_service_data.extend_from_slice(&req.service_request);

        let seg_ack = Apdu::SegmentAck(SegmentAckPdu {
            negative_ack: false,
            sent_by_server: true,
            invoke_id: req.invoke_id,
            sequence_number: seq,
            actual_window_size: 1,
        });
        send_routed_response(
            &router_transport,
            &client_mac,
            remote_network,
            &remote_mac,
            seg_ack,
        )
        .await;

        if !req.more_follows {
            break req.invoke_id;
        }
    };

    assert_eq!(all_service_data, expected_data);

    let ack = Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
    });
    send_routed_response(
        &router_transport,
        &client_mac,
        remote_network,
        &remote_mac,
        ack,
    )
    .await;

    let result = request_task.await.unwrap();
    assert!(result.unwrap().is_empty());
    router_transport.stop().await.unwrap();
}

/// A routed peer advertising less than MinimumMessageSize is rejected with the
/// peer named as the binding term, and nothing goes on the wire.
#[tokio::test]
async fn routed_confirmed_request_rejects_subminimum_peer_limit() {
    let client_mac = vec![0x01];
    let router_mac = vec![0x02];
    let remote_network = 100;
    let remote_mac = vec![0x03];
    let (client_transport, mut router_transport) =
        LoopbackTransport::pair(client_mac, router_mac.clone());
    let mut router_rx = router_transport.start().await.unwrap();

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    client.device_table.lock().await.upsert(routed_peer(
        3,
        &router_mac,
        remote_network,
        &remote_mac,
    ));

    let err = client
        .confirmed_request_routed(
            &router_mac,
            remote_network,
            &remote_mac,
            ConfirmedServiceChoice::READ_PROPERTY,
            &[0x01],
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("peer's advertised Max APDU Length Accepted of 3"),
        "the routed peer is the binding term, got: {err}"
    );

    assert!(
        timeout(Duration::from_millis(50), router_rx.recv())
            .await
            .is_err(),
        "no frame may reach the router for a rejected request"
    );

    client.stop().await.unwrap();
    router_transport.stop().await.unwrap();
}

/// A routed peer seeded through the public manual-registration API (no I-Am
/// exchange) has its advertised Max APDU Length Accepted consumed exactly as
/// if it had been discovered (#372): a 204-octet request segments at 128
/// instead of falling back to local config.
#[tokio::test]
async fn manual_routed_registration_honors_seeded_peer_limit() {
    let client_mac = vec![0x01];
    let router_mac = vec![0x02];
    let remote_network = 100;
    let remote_mac = vec![0x03];
    let (client_transport, mut router_transport) =
        LoopbackTransport::pair(client_mac.clone(), router_mac.clone());
    let mut router_rx = router_transport.start().await.unwrap();

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .max_apdu_length(1476)
        .build()
        .await
        .unwrap();

    client
        .add_routed_device(RoutedDeviceConfig {
            instance: 3003,
            router_mac: router_mac.clone(),
            remote_network,
            remote_mac: remote_mac.clone(),
            max_apdu_length: 128,
            segmentation_supported: Segmentation::BOTH,
            max_segments_accepted: None,
        })
        .await
        .unwrap();

    // The registration must preserve every supplied field.
    let row = client.device_table.lock().await.get(3003).cloned().unwrap();
    assert_eq!(row.mac_address.as_slice(), &router_mac);
    assert_eq!(row.source_network, Some(remote_network));
    assert_eq!(
        row.source_address.as_ref().map(|a| a.to_vec()),
        Some(remote_mac.clone())
    );
    assert_eq!(row.max_apdu_length, 128);
    assert_eq!(row.segmentation_supported, Segmentation::BOTH);
    assert_eq!(row.max_segments_accepted, None);

    // resolve_device yields the router next-hop plus the remote identity.
    let (next_hop, routing) = client.resolve_device(3003).await.unwrap();
    assert_eq!(next_hop, router_mac);
    assert_eq!(routing, Some((remote_network, remote_mac.clone())));

    let service_data: Vec<u8> = (0..200u16).map(|i| i as u8).collect();
    let expected_data = service_data.clone();
    let router_mac_for_request = router_mac.clone();
    let remote_mac_for_request = remote_mac.clone();

    let request_task = tokio::spawn(async move {
        let result = client
            .confirmed_request_routed(
                &router_mac_for_request,
                remote_network,
                &remote_mac_for_request,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    let mut all_service_data = Vec::new();
    let invoke_id = loop {
        let received = timeout(Duration::from_secs(2), router_rx.recv())
            .await
            .expect("router timed out waiting for routed segment")
            .expect("router channel closed");

        let npdu = decode_npdu(received.npdu).unwrap();
        assert!(
            npdu.payload.len() <= 128,
            "APDU of {} octets exceeds the seeded peer's limit of 128",
            npdu.payload.len()
        );

        let decoded = apdu::decode_apdu(npdu.payload).unwrap();
        let Apdu::ConfirmedRequest(req) = decoded else {
            panic!("Expected ConfirmedRequest, got {:?}", decoded);
        };
        assert!(
            req.segmented,
            "seeded limit of 128 must force segmentation, not local-config fallback"
        );
        let seq = req.sequence_number.unwrap();
        all_service_data.extend_from_slice(&req.service_request);

        let seg_ack = Apdu::SegmentAck(SegmentAckPdu {
            negative_ack: false,
            sent_by_server: true,
            invoke_id: req.invoke_id,
            sequence_number: seq,
            actual_window_size: 1,
        });
        send_routed_response(
            &router_transport,
            &client_mac,
            remote_network,
            &remote_mac,
            seg_ack,
        )
        .await;

        if !req.more_follows {
            break req.invoke_id;
        }
    };

    assert_eq!(all_service_data, expected_data);

    let ack = Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
    });
    send_routed_response(
        &router_transport,
        &client_mac,
        remote_network,
        &remote_mac,
        ack,
    )
    .await;

    let result = request_task.await.unwrap();
    assert!(result.unwrap().is_empty());
    router_transport.stop().await.unwrap();
}

/// Malformed routing metadata is rejected before any table mutation: empty
/// next-hop or peer addresses can never identify a route or a peer (#372).
#[tokio::test]
async fn manual_routed_registration_rejects_empty_addresses() {
    let (client_transport, _peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();

    for (router_mac, remote_mac) in [(Vec::new(), vec![0x03]), (vec![0x02], Vec::new())] {
        let result = client
            .add_routed_device(RoutedDeviceConfig {
                instance: 3003,
                router_mac,
                remote_network: 100,
                remote_mac,
                max_apdu_length: 128,
                segmentation_supported: Segmentation::BOTH,
                max_segments_accepted: None,
            })
            .await;
        assert!(result.is_err(), "empty address must be rejected");
    }

    assert!(
        client.device_table.lock().await.get(3003).is_none(),
        "a refused registration must not create a row"
    );
}

/// Legacy `add_device` keeps creating unambiguously local rows: the local
/// lookup finds them and the routed lookup by SNET/SADR does not (#372).
#[tokio::test]
async fn legacy_add_device_creates_local_row_only() {
    let (client_transport, _peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();

    let mac = vec![192, 168, 1, 100, 0xBA, 0xC0];
    client.add_device(1234, &mac).await.unwrap();

    let dt = client.device_table.lock().await;
    let local = dt.get_by_mac(&mac).expect("local lookup must find the row");
    assert_eq!(local.object_identifier.instance_number(), 1234);
    assert!(
        dt.get_by_network_address(100, &mac).is_none(),
        "the same bytes as SNET/SADR must not satisfy a routed lookup"
    );
}
