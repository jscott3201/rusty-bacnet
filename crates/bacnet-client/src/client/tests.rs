use super::*;
use bacnet_encoding::apdu::{ComplexAck, SimpleAck};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_transport::sc::{LoopbackWebSocket, ScTransport, WebSocketPort};
use bacnet_transport::sc_frame::{
    decode_sc_message, encode_sc_message, ScFunction, ScMessage, Vmac,
};
use bacnet_types::enums::{ObjectType, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;
use std::net::Ipv4Addr;
use std::time::Instant;
use tokio::time::Duration;

async fn make_client() -> BACnetClient<BipTransport> {
    BACnetClient::builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap()
}

async fn sc_hub_accept(ws_hub: &LoopbackWebSocket, hub_vmac: Vmac) {
    let data = ws_hub.recv().await.unwrap();
    let req = decode_sc_message(&data).unwrap();
    assert_eq!(req.function, ScFunction::ConnectRequest);

    let mut accept_payload = Vec::with_capacity(26);
    accept_payload.extend_from_slice(&hub_vmac);
    accept_payload.extend_from_slice(&[0u8; 16]);
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());

    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(accept_payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &accept);
    ws_hub.send(&buf).await.unwrap();
}

async fn assert_sc_socket_closed_after_drop(ws_hub: &LoopbackWebSocket, context: &str) {
    timeout(Duration::from_secs(1), async {
        loop {
            match ws_hub.recv().await {
                Ok(data) => {
                    let msg = decode_sc_message(&data).unwrap();
                    assert_ne!(
                        msg.function,
                        ScFunction::HeartbeatAck,
                        "{context} must not leave SC answering heartbeats"
                    );
                }
                Err(_) => break,
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{context} did not close the SC WebSocket"));

    let heartbeat = ScMessage {
        function: ScFunction::HeartbeatRequest,
        message_id: 0x66,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &heartbeat);
    assert!(
        ws_hub.send(&buf).await.is_err(),
        "{context} must reject post-drop Heartbeat-Request on the closed socket"
    );
}

pub(super) async fn send_routed_response<T: TransportPort>(
    transport: &T,
    client_mac: &[u8],
    source_network: u16,
    source_mac: &[u8],
    apdu: Apdu,
) {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &apdu).expect("valid APDU encoding");
    let npdu = Npdu {
        source: Some(NpduAddress {
            network: source_network,
            mac_address: MacAddr::from_slice(source_mac),
        }),
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();
    transport.send_unicast(&npdu_buf, client_mac).await.unwrap();
}

#[tokio::test]
async fn client_start_stop() {
    let mut client = make_client().await;
    assert!(!client.local_mac().is_empty());
    client.stop().await.unwrap();
}

#[tokio::test]
async fn client_stop_releases_bip_socket_before_drop() {
    let mut client = make_client().await;
    let local_mac = client.local_mac().to_vec();
    let port = u16::from_be_bytes([local_mac[4], local_mac[5]]);

    client.stop().await.unwrap();

    timeout(Duration::from_secs(1), async {
        loop {
            match BACnetClient::builder()
                .interface(Ipv4Addr::LOCALHOST)
                .port(port)
                .build()
                .await
            {
                Ok(mut replacement) => {
                    replacement.stop().await.unwrap();
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("stopping BACnetClient releases the B/IP socket");
}

#[tokio::test]
async fn client_drop_aborts_dispatch_task() {
    let client = make_client().await;
    let abort_handle = client
        .dispatch_task
        .as_ref()
        .expect("client starts a dispatch task")
        .abort_handle();

    assert!(!abort_handle.is_finished());
    drop(client);

    timeout(Duration::from_secs(1), async {
        while !abort_handle.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropping BACnetClient aborts the dispatch task");
}

#[tokio::test]
async fn client_drop_releases_bip_socket() {
    let client = make_client().await;
    let local_mac = client.local_mac().to_vec();
    let port = u16::from_be_bytes([local_mac[4], local_mac[5]]);

    drop(client);

    timeout(Duration::from_secs(1), async {
        loop {
            match BACnetClient::builder()
                .interface(Ipv4Addr::LOCALHOST)
                .port(port)
                .build()
                .await
            {
                Ok(mut replacement) => {
                    replacement.stop().await.unwrap();
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("dropping BACnetClient releases the B/IP socket");
}

#[tokio::test]
async fn client_drop_releases_sc_transport_socket() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];
    let transport = ScTransport::new(ws_client, client_vmac);

    let hub_task = tokio::spawn(async move {
        sc_hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let client = BACnetClient::start(ClientConfig::default(), transport)
        .await
        .unwrap();
    let ws_hub = hub_task.await.unwrap();

    drop(client);

    assert_sc_socket_closed_after_drop(&ws_hub, "dropped BACnetClient").await;
}

#[tokio::test(start_paused = true)]
async fn device_table_purge_runs_without_inbound_apdu() {
    let mut client = make_client().await;

    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    client.device_table.lock().await.upsert(DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 2001).unwrap(),
        mac_address: MacAddr::from_slice(&[192, 168, 1, 42, 0xBA, 0xC0]),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        max_segments_accepted: None,
        vendor_id: 42,
        last_seen: Instant::now()
            .checked_sub(Duration::from_millis(1))
            .unwrap_or_else(Instant::now),
        source_network: None,
        source_address: None,
    });
    assert_eq!(client.discovered_devices().await.len(), 1);

    tokio::time::advance(Duration::from_secs(300)).await;
    tokio::task::yield_now().await;

    assert!(client.discovered_devices().await.is_empty());

    client.stop().await.unwrap();
}

#[tokio::test]
async fn client_rejects_invalid_max_apdu_length() {
    let result = BACnetClient::builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .max_apdu_length(1000)
        .build()
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn confirmed_request_simple_ack() {
    let mut client_a = make_client().await;

    let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut net_b = NetworkLayer::new(transport_b);
    let mut rx_b = net_b.start().await.unwrap();
    let b_mac = net_b.local_mac().to_vec();

    let b_handle = tokio::spawn(async move {
        let received = timeout(Duration::from_secs(2), rx_b.recv())
            .await
            .expect("B timed out")
            .expect("B channel closed");

        let decoded = apdu::decode_apdu(received.apdu.clone()).unwrap();
        if let Apdu::ConfirmedRequest(req) = decoded {
            let ack = Apdu::SimpleAck(SimpleAck {
                invoke_id: req.invoke_id,
                service_choice: req.service_choice,
            });
            let mut buf = BytesMut::new();
            encode_apdu(&mut buf, &ack).unwrap();
            net_b
                .send_apdu(&buf, &received.source_mac, false, NetworkPriority::NORMAL)
                .await
                .unwrap();
        }
        net_b.stop().await.unwrap();
    });

    let result = client_a
        .confirmed_request(
            &b_mac,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            &[0x01, 0x02],
        )
        .await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert!(response.is_empty());

    b_handle.await.unwrap();
    client_a.stop().await.unwrap();
}

#[tokio::test]
async fn confirmed_request_does_not_wrap_discovered_max_apdu_length() {
    let client_mac = vec![0x01];
    let remote_mac = vec![0x02];
    let (client_transport, remote_transport) =
        LoopbackTransport::pair(client_mac, remote_mac.clone());
    let mut remote_network = NetworkLayer::new(remote_transport);
    let mut remote_rx = remote_network.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();

    client.device_table.lock().await.upsert(DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 2002).unwrap(),
        mac_address: MacAddr::from_slice(&remote_mac),
        max_apdu_length: u32::from(u16::MAX) + 1,
        segmentation_supported: Segmentation::NONE,
        max_segments_accepted: None,
        vendor_id: 42,
        last_seen: Instant::now(),
        source_network: None,
        source_address: None,
    });

    let responder = tokio::spawn(async move {
        let received = timeout(Duration::from_secs(1), remote_rx.recv())
            .await
            .expect("remote timed out")
            .expect("remote channel closed");
        let Apdu::ConfirmedRequest(req) = apdu::decode_apdu(received.apdu).unwrap() else {
            panic!("expected ConfirmedRequest");
        };
        assert!(!req.segmented);

        let ack = Apdu::SimpleAck(SimpleAck {
            invoke_id: req.invoke_id,
            service_choice: req.service_choice,
        });
        let mut buf = BytesMut::new();
        encode_apdu(&mut buf, &ack).unwrap();
        remote_network
            .send_apdu(&buf, &received.source_mac, false, NetworkPriority::NORMAL)
            .await
            .unwrap();
        remote_network.stop().await.unwrap();
    });

    let result = client
        .confirmed_request(&remote_mac, ConfirmedServiceChoice::READ_PROPERTY, &[0x01])
        .await;

    assert!(result.unwrap().is_empty());
    responder.await.unwrap();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn confirmed_request_complex_ack() {
    let mut client_a = make_client().await;

    let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut net_b = NetworkLayer::new(transport_b);
    let mut rx_b = net_b.start().await.unwrap();
    let b_mac = net_b.local_mac().to_vec();

    let b_handle = tokio::spawn(async move {
        let received = timeout(Duration::from_secs(2), rx_b.recv())
            .await
            .unwrap()
            .unwrap();

        let decoded = apdu::decode_apdu(received.apdu.clone()).unwrap();
        if let Apdu::ConfirmedRequest(req) = decoded {
            let ack = Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: req.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: req.service_choice,
                service_ack: Bytes::from_static(&[0xDE, 0xAD, 0xBE, 0xEF]),
            });
            let mut buf = BytesMut::new();
            encode_apdu(&mut buf, &ack).unwrap();
            net_b
                .send_apdu(&buf, &received.source_mac, false, NetworkPriority::NORMAL)
                .await
                .unwrap();
        }
        net_b.stop().await.unwrap();
    });

    let result = client_a
        .confirmed_request(&b_mac, ConfirmedServiceChoice::READ_PROPERTY, &[0x01])
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), vec![0xDE, 0xAD, 0xBE, 0xEF]);

    b_handle.await.unwrap();
    client_a.stop().await.unwrap();
}

#[tokio::test]
async fn confirmed_request_timeout() {
    let mut client = make_client().await;
    let fake_mac = vec![10, 99, 99, 99, 0xBA, 0xC0];
    let result = client
        .confirmed_request(&fake_mac, ConfirmedServiceChoice::READ_PROPERTY, &[0x01])
        .await;
    assert!(result.is_err());
    assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn segmented_complex_ack_reassembly() {
    let mut client = make_client().await;

    let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut net_b = NetworkLayer::new(transport_b);
    let mut rx_b = net_b.start().await.unwrap();
    let b_mac = net_b.local_mac().to_vec();

    let b_handle = tokio::spawn(async move {
        let received = timeout(Duration::from_secs(2), rx_b.recv())
            .await
            .unwrap()
            .unwrap();

        let decoded = apdu::decode_apdu(received.apdu.clone()).unwrap();
        let invoke_id = if let Apdu::ConfirmedRequest(req) = decoded {
            req.invoke_id
        } else {
            panic!("Expected ConfirmedRequest");
        };

        let service_choice = ConfirmedServiceChoice::READ_PROPERTY;
        let segments: Vec<Bytes> = vec![
            Bytes::from_static(&[0x01, 0x02, 0x03]),
            Bytes::from_static(&[0x04, 0x05, 0x06]),
            Bytes::from_static(&[0x07, 0x08]),
        ];

        for (i, seg) in segments.iter().enumerate() {
            let is_last = i == segments.len() - 1;
            let ack = Apdu::ComplexAck(ComplexAck {
                segmented: true,
                more_follows: !is_last,
                invoke_id,
                sequence_number: Some(i as u8),
                proposed_window_size: Some(1),
                service_choice,
                service_ack: seg.clone(),
            });
            let mut buf = BytesMut::new();
            encode_apdu(&mut buf, &ack).unwrap();
            net_b
                .send_apdu(&buf, &received.source_mac, false, NetworkPriority::NORMAL)
                .await
                .unwrap();

            let seg_ack_msg = timeout(Duration::from_secs(2), rx_b.recv())
                .await
                .unwrap()
                .unwrap();
            let decoded = apdu::decode_apdu(seg_ack_msg.apdu.clone()).unwrap();
            if let Apdu::SegmentAck(sa) = decoded {
                assert_eq!(sa.invoke_id, invoke_id);
                assert_eq!(sa.sequence_number, i as u8);
            } else {
                panic!("Expected SegmentAck, got {:?}", decoded);
            }
        }

        net_b.stop().await.unwrap();
    });

    let result = client
        .confirmed_request(&b_mac, ConfirmedServiceChoice::READ_PROPERTY, &[0x01])
        .await;

    assert!(result.is_ok());
    assert_eq!(
        result.unwrap(),
        vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
    );

    b_handle.await.unwrap();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn segmented_confirmed_request_sends_segments() {
    let mut client = BACnetClient::builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .apdu_timeout_ms(5000)
        .max_apdu_length(50)
        .build()
        .await
        .unwrap();

    let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut net_b = NetworkLayer::new(transport_b);
    let mut rx_b = net_b.start().await.unwrap();
    let b_mac = net_b.local_mac().to_vec();

    let service_data: Vec<u8> = (0u8..100).collect();
    let expected_data = service_data.clone();

    let b_handle = tokio::spawn(async move {
        let mut all_service_data = Vec::new();
        let mut client_mac;
        let mut invoke_id;

        loop {
            let received = timeout(Duration::from_secs(3), rx_b.recv())
                .await
                .expect("server timed out waiting for segment")
                .expect("server channel closed");

            let decoded = apdu::decode_apdu(received.apdu.clone()).unwrap();
            if let Apdu::ConfirmedRequest(req) = decoded {
                assert!(req.segmented, "expected segmented request");
                invoke_id = req.invoke_id;
                client_mac = received.source_mac.clone();
                let seq = req.sequence_number.unwrap();
                all_service_data.extend_from_slice(&req.service_request);

                let seg_ack = Apdu::SegmentAck(SegmentAckPdu {
                    negative_ack: false,
                    sent_by_server: true,
                    invoke_id,
                    sequence_number: seq,
                    actual_window_size: 1,
                });
                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &seg_ack).unwrap();
                net_b
                    .send_apdu(&buf, &received.source_mac, false, NetworkPriority::NORMAL)
                    .await
                    .unwrap();

                if !req.more_follows {
                    break;
                }
            } else {
                panic!("Expected ConfirmedRequest, got {:?}", decoded);
            }
        }

        let ack = Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        });
        let mut buf = BytesMut::new();
        encode_apdu(&mut buf, &ack).unwrap();
        net_b
            .send_apdu(&buf, &client_mac, false, NetworkPriority::NORMAL)
            .await
            .unwrap();

        net_b.stop().await.unwrap();
        all_service_data
    });

    let result = client
        .confirmed_request(
            &b_mac,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            &service_data,
        )
        .await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());

    let received_data = b_handle.await.unwrap();
    assert_eq!(received_data, expected_data);

    client.stop().await.unwrap();
}

#[tokio::test]
async fn segmented_request_with_complex_ack_response() {
    let mut client = BACnetClient::builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .apdu_timeout_ms(5000)
        .max_apdu_length(50)
        .build()
        .await
        .unwrap();

    let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut net_b = NetworkLayer::new(transport_b);
    let mut rx_b = net_b.start().await.unwrap();
    let b_mac = net_b.local_mac().to_vec();

    let service_data: Vec<u8> = (0u8..60).collect();

    let b_handle = tokio::spawn(async move {
        let mut client_mac;
        let mut invoke_id;

        loop {
            let received = timeout(Duration::from_secs(3), rx_b.recv())
                .await
                .unwrap()
                .unwrap();

            let decoded = apdu::decode_apdu(received.apdu.clone()).unwrap();
            if let Apdu::ConfirmedRequest(req) = decoded {
                invoke_id = req.invoke_id;
                client_mac = received.source_mac.clone();
                let seq = req.sequence_number.unwrap();

                let seg_ack = Apdu::SegmentAck(SegmentAckPdu {
                    negative_ack: false,
                    sent_by_server: true,
                    invoke_id,
                    sequence_number: seq,
                    actual_window_size: 1,
                });
                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &seg_ack).unwrap();
                net_b
                    .send_apdu(&buf, &received.source_mac, false, NetworkPriority::NORMAL)
                    .await
                    .unwrap();

                if !req.more_follows {
                    break;
                }
            }
        }

        let ack = Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_ack: Bytes::from_static(&[0xCA, 0xFE]),
        });
        let mut buf = BytesMut::new();
        encode_apdu(&mut buf, &ack).unwrap();
        net_b
            .send_apdu(&buf, &client_mac, false, NetworkPriority::NORMAL)
            .await
            .unwrap();

        net_b.stop().await.unwrap();
    });

    let result = client
        .confirmed_request(&b_mac, ConfirmedServiceChoice::READ_PROPERTY, &service_data)
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), vec![0xCA, 0xFE]);

    b_handle.await.unwrap();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn routed_segmented_request_uses_routed_tsm_key() {
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
        .max_apdu_length(50)
        .build()
        .await
        .unwrap();

    let service_data: Vec<u8> = (0u8..100).collect();
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
        assert_eq!(&received.source_mac[..], &client_mac[..]);

        let npdu = decode_npdu(received.npdu).unwrap();
        let destination = npdu.destination.expect("routed NPDU destination");
        assert_eq!(destination.network, remote_network);
        assert_eq!(&destination.mac_address[..], &remote_mac[..]);

        let decoded = apdu::decode_apdu(npdu.payload).unwrap();
        let Apdu::ConfirmedRequest(req) = decoded else {
            panic!("Expected ConfirmedRequest, got {:?}", decoded);
        };
        assert!(req.segmented, "expected routed request to be segmented");
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

#[tokio::test]
async fn segment_overflow_guard() {
    let mut client = BACnetClient::builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .apdu_timeout_ms(2000)
        .max_apdu_length(50)
        .build()
        .await
        .unwrap();

    let huge_payload = vec![0u8; 257 * 44];
    let fake_mac = vec![10, 99, 99, 99, 0xBA, 0xC0];

    let result = client
        .confirmed_request(
            &fake_mac,
            ConfirmedServiceChoice::READ_PROPERTY,
            &huge_payload,
        )
        .await;

    assert!(
        result.is_err(),
        "expected error for oversized payload, got success"
    );

    client.stop().await.unwrap();
}
