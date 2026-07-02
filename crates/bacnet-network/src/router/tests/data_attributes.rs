use super::*;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_transport::sc::{LoopbackWebSocket, ScTransport, WebSocketPort};
use bacnet_transport::sc_frame::{
    decode_sc_message, encode_sc_message, ScFunction, ScMessage, ScOption, Vmac,
};
use tokio::time::{timeout, Duration};

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

#[test]
fn forward_unicast_preserves_data_attributes() {
    let (tx_a, _rx_a) = mpsc::channel::<SendRequest>(256);
    let (tx_b, mut rx_b) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx_a, tx_b];

    let route = crate::router_table::RouteEntry {
        port_index: 1,
        directly_connected: true,
        next_hop_mac: MacAddr::new(),
        last_seen: None,
        reachability: crate::router_table::ReachabilityStatus::Reachable,
        busy_until: None,
        flap_count: 0,
        last_port_change: None,
    };

    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: bacnet_types::enums::NetworkPriority::NORMAL,
        destination: Some(NpduAddress {
            network: 2000,
            mac_address: MacAddr::from_slice(&[0x01, 0x02]),
        }),
        source: None,
        hop_count: 10,
        payload: Bytes::from_static(&[0x10, 0x08]),
        ..Npdu::default()
    };
    let data_attributes = vec![
        DataAttribute {
            option_type: 1,
            must_understand: true,
            data: Vec::new(),
        },
        DataAttribute {
            option_type: 31,
            must_understand: false,
            data: vec![0x12, 0x34, 0x56],
        },
    ];

    forward_unicast(&send_txs, &route, 1000, &[0x0A], npdu, 0, &data_attributes);

    match rx_b.try_recv().unwrap() {
        SendRequest::Unicast {
            npdu: data,
            data_attributes: sent_attributes,
            ..
        } => {
            let decoded = decode_npdu(data).unwrap();
            assert!(decoded.destination.is_none());
            assert!(decoded.source.is_some());
            assert_eq!(sent_attributes, data_attributes);
        }
        SendRequest::Broadcast { .. } => panic!("expected forwarded unicast"),
    }
}

#[test]
fn forward_broadcast_preserves_data_attributes_on_each_output_port() {
    let (tx_a, mut rx_a) = mpsc::channel::<SendRequest>(256);
    let (tx_b, mut rx_b) = mpsc::channel::<SendRequest>(256);
    let (tx_c, mut rx_c) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx_a, tx_b, tx_c];

    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: bacnet_types::enums::NetworkPriority::NORMAL,
        destination: Some(NpduAddress {
            network: 0xFFFF,
            mac_address: MacAddr::new(),
        }),
        source: None,
        hop_count: 10,
        payload: Bytes::from_static(&[0x10, 0x08]),
        ..Npdu::default()
    };
    let data_attributes = vec![DataAttribute {
        option_type: 2,
        must_understand: true,
        data: vec![0xAA],
    }];

    forward_broadcast(&send_txs, 1, 1000, &[0x0A], &npdu, &data_attributes);

    for sent in [rx_a.try_recv().unwrap(), rx_c.try_recv().unwrap()] {
        match sent {
            SendRequest::Broadcast {
                npdu: data,
                data_attributes: sent_attributes,
            } => {
                let decoded = decode_npdu(data).unwrap();
                assert_eq!(decoded.destination.as_ref().unwrap().network, 0xFFFF);
                assert!(decoded.source.is_some());
                assert_eq!(sent_attributes, data_attributes);
            }
            SendRequest::Unicast { .. } => panic!("expected forwarded broadcast"),
        }
    }
    assert!(rx_b.try_recv().is_err());
}

#[tokio::test]
async fn sc_to_sc_forwarding_preserves_data_attributes_as_data_options() {
    let (ws_client_a, ws_hub_a) = LoopbackWebSocket::pair();
    let (ws_client_b, ws_hub_b) = LoopbackWebSocket::pair();
    let hub_a_vmac: Vmac = [0xA0; 6];
    let hub_b_vmac: Vmac = [0xB0; 6];
    let dest_vmac: Vmac = [0x22; 6];

    let accept_a = tokio::spawn(async move {
        sc_hub_accept(&ws_hub_a, hub_a_vmac).await;
        ws_hub_a
    });
    let accept_b = tokio::spawn(async move {
        sc_hub_accept(&ws_hub_b, hub_b_vmac).await;
        ws_hub_b
    });

    let port_a = RouterPort {
        transport: ScTransport::new(ws_client_a, [0x01; 6]),
        network_number: 1000,
    };
    let port_b = RouterPort {
        transport: ScTransport::new(ws_client_b, [0x02; 6]),
        network_number: 2000,
    };
    let (mut router, _local_rx) = BACnetRouter::start(vec![port_a, port_b]).await.unwrap();
    let ws_hub_a = accept_a.await.unwrap();
    let ws_hub_b = accept_b.await.unwrap();

    let apdu = Bytes::from_static(&[0x10, 0x08]);
    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: bacnet_types::enums::NetworkPriority::NORMAL,
        destination: Some(NpduAddress {
            network: 2000,
            mac_address: MacAddr::from_slice(&dest_vmac),
        }),
        source: None,
        hop_count: 255,
        payload: apdu.clone(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();

    let inbound = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2233,
        originating_vmac: Some(hub_a_vmac),
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: vec![
            ScOption {
                option_type: 1,
                must_understand: true,
                data: Vec::new(),
            },
            ScOption {
                option_type: 31,
                must_understand: false,
                data: vec![0x12, 0x34, 0x56],
            },
        ],
        payload: npdu_buf.freeze(),
    };
    let mut sc_buf = BytesMut::new();
    encode_sc_message(&mut sc_buf, &inbound);
    ws_hub_a.send(&sc_buf).await.unwrap();

    let forwarded = async {
        for _ in 0..5 {
            let data = ws_hub_b.recv().await.unwrap();
            let msg = decode_sc_message(&data).unwrap();
            if msg.function != ScFunction::EncapsulatedNpdu {
                continue;
            }
            let decoded = decode_npdu(msg.payload.clone()).unwrap();
            if !decoded.is_network_message && decoded.payload == apdu {
                return msg;
            }
        }
        panic!("did not receive forwarded APDU");
    };
    let msg = timeout(Duration::from_secs(1), forwarded)
        .await
        .expect("timed out waiting for forwarded SC NPDU");

    assert_eq!(msg.destination_vmac, Some(dest_vmac));
    assert_eq!(msg.data_options.len(), 2);
    assert_eq!(msg.data_options[0].option_type, 1);
    assert!(msg.data_options[0].must_understand);
    assert!(msg.data_options[0].data.is_empty());
    assert_eq!(msg.data_options[1].option_type, 31);
    assert!(!msg.data_options[1].must_understand);
    assert_eq!(msg.data_options[1].data, vec![0x12, 0x34, 0x56]);

    router.stop().await;
}
