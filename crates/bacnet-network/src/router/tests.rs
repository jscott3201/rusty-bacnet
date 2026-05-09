use super::*;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_transport::bip::BipTransport;
use std::net::Ipv4Addr;
use tokio::time::Duration;

#[tokio::test]
async fn router_forwards_between_networks() {
    let transport_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

    let mut device_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut device_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

    let _rx_b = device_b.start().await.unwrap();
    let _rx_a = device_a.start().await.unwrap();

    let port_a = RouterPort {
        transport: transport_a,
        network_number: 1000,
    };
    let port_b = RouterPort {
        transport: transport_b,
        network_number: 2000,
    };

    let (mut router, _local_rx) = BACnetRouter::start(vec![port_a, port_b]).await.unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let apdu = vec![0x10, 0x08];
    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: bacnet_types::enums::NetworkPriority::NORMAL,
        destination: Some(NpduAddress {
            network: 2000,
            mac_address: MacAddr::from_slice(device_b.local_mac()),
        }),
        source: None,
        hop_count: 255,
        payload: Bytes::copy_from_slice(&apdu),
        ..Npdu::default()
    };

    let mut buf = BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();

    let table = router.table().lock().await;
    assert_eq!(table.len(), 2);
    assert!(table.lookup(1000).unwrap().directly_connected);
    assert!(table.lookup(2000).unwrap().directly_connected);
    drop(table);

    router.stop().await;
}

#[tokio::test]
async fn router_table_populated_on_start() {
    let transport_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let transport_c = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

    let ports = vec![
        RouterPort {
            transport: transport_a,
            network_number: 100,
        },
        RouterPort {
            transport: transport_b,
            network_number: 200,
        },
        RouterPort {
            transport: transport_c,
            network_number: 300,
        },
    ];

    let (mut router, _local_rx) = BACnetRouter::start(ports).await.unwrap();

    let table = router.table().lock().await;
    assert_eq!(table.len(), 3);
    assert_eq!(table.lookup(100).unwrap().port_index, 0);
    assert_eq!(table.lookup(200).unwrap().port_index, 1);
    assert_eq!(table.lookup(300).unwrap().port_index, 2);
    drop(table);

    router.stop().await;
}

#[tokio::test]
async fn local_message_delivered_to_application() {
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut sender = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _sender_rx = sender.start().await.unwrap();

    let router_port = RouterPort {
        transport,
        network_number: 1000,
    };

    let (mut router, _local_rx) = BACnetRouter::start(vec![router_port]).await.unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    router.stop().await;
}

#[test]
fn forward_unicast_drops_hop_count_zero() {
    let (tx_a, mut rx_a) = mpsc::channel::<SendRequest>(256);
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
        hop_count: 0, // Should cause the message to be dropped
        payload: Bytes::from_static(&[0x10, 0x08]),
        ..Npdu::default()
    };

    forward_unicast(&send_txs, &route, 1000, &[0x0A], npdu, 0);

    assert!(rx_a.try_recv().is_err());
    assert!(rx_b.try_recv().is_err());
}

#[test]
fn forward_broadcast_drops_hop_count_zero() {
    let (tx_a, mut rx_a) = mpsc::channel::<SendRequest>(256);
    let (tx_b, mut rx_b) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx_a, tx_b];

    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: bacnet_types::enums::NetworkPriority::NORMAL,
        destination: Some(NpduAddress {
            network: 0xFFFF,
            mac_address: MacAddr::new(),
        }),
        source: None,
        hop_count: 0, // Should cause broadcast to be dropped
        payload: Bytes::from_static(&[0x10, 0x08]),
        ..Npdu::default()
    };

    forward_broadcast(&send_txs, 0, 1000, &[0x0A], &npdu);

    assert!(rx_a.try_recv().is_err());
    assert!(rx_b.try_recv().is_err());
}

#[test]
fn forward_unicast_decrements_hop_count() {
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

    forward_unicast(&send_txs, &route, 1000, &[0x0A], npdu, 0);

    let sent = rx_b.try_recv().unwrap();
    match sent {
        SendRequest::Unicast { npdu: data, .. } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.destination.is_none());
            assert!(decoded.source.is_some());
        }
        SendRequest::Broadcast { npdu: data } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.destination.is_none());
        }
    }
}

#[test]
fn send_reject_generates_reject_message() {
    let (tx, mut rx) = mpsc::channel::<SendRequest>(256);

    let source_mac = vec![0x0A, 0x00, 0x01, 0x01];
    let unknown_network: u16 = 9999;

    send_reject(
        &tx,
        &source_mac,
        unknown_network,
        RejectMessageReason::NOT_DIRECTLY_CONNECTED,
    );

    let sent = rx.try_recv().unwrap();
    match sent {
        SendRequest::Unicast { npdu: data, mac } => {
            assert_eq!(mac.as_slice(), &source_mac[..]);
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.is_network_message);
            assert_eq!(
                decoded.message_type,
                Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw())
            );
            assert_eq!(decoded.payload.len(), 3);
            assert_eq!(
                decoded.payload[0],
                RejectMessageReason::NOT_DIRECTLY_CONNECTED.to_raw()
            );
            let rejected_net = u16::from_be_bytes([decoded.payload[1], decoded.payload[2]]);
            assert_eq!(rejected_net, 9999);
        }
        _ => panic!("Expected Unicast send for reject message"),
    }
}

#[tokio::test]
async fn single_port_router_no_i_am_router_announcement() {
    let (send_tx, mut send_rx) = mpsc::channel::<SendRequest>(256);

    let port_networks: Vec<u16> = vec![1000];
    let send_txs = [send_tx];

    for (port_idx, tx) in send_txs.iter().enumerate() {
        let other_networks: Vec<u16> = port_networks
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx != port_idx)
            .map(|(_, net)| *net)
            .collect();

        if other_networks.is_empty() {
            continue;
        }

        let mut payload = BytesMut::with_capacity(other_networks.len() * 2);
        for net in &other_networks {
            payload.put_u16(*net);
        }

        let payload_len = payload.len();
        let response = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(8 + payload_len);
        encode_npdu(&mut buf, &response).unwrap();

        let _ = tx.try_send(SendRequest::Broadcast { npdu: buf.freeze() });
    }

    assert!(send_rx.try_recv().is_err());
}

#[tokio::test]
async fn two_port_router_sends_i_am_router_announcement() {
    let (tx_a, mut rx_a) = mpsc::channel::<SendRequest>(256);
    let (tx_b, mut rx_b) = mpsc::channel::<SendRequest>(256);

    let port_networks: Vec<u16> = vec![1000, 2000];
    let send_txs = [tx_a, tx_b];

    for (port_idx, tx) in send_txs.iter().enumerate() {
        let other_networks: Vec<u16> = port_networks
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx != port_idx)
            .map(|(_, net)| *net)
            .collect();

        if other_networks.is_empty() {
            continue;
        }

        let mut payload = BytesMut::with_capacity(other_networks.len() * 2);
        for net in &other_networks {
            payload.put_u16(*net);
        }

        let payload_len = payload.len();
        let response = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(8 + payload_len);
        encode_npdu(&mut buf, &response).unwrap();

        let _ = tx.try_send(SendRequest::Broadcast { npdu: buf.freeze() });
    }

    let sent_a = rx_a.try_recv().unwrap();
    match sent_a {
        SendRequest::Broadcast { npdu: data } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.is_network_message);
            assert_eq!(
                decoded.message_type,
                Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw())
            );
            assert_eq!(decoded.payload.len(), 2);
            let net = u16::from_be_bytes([decoded.payload[0], decoded.payload[1]]);
            assert_eq!(net, 2000);
        }
        _ => panic!("Expected Broadcast for I-Am-Router announcement on port A"),
    }

    let sent_b = rx_b.try_recv().unwrap();
    match sent_b {
        SendRequest::Broadcast { npdu: data } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.is_network_message);
            assert_eq!(
                decoded.message_type,
                Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw())
            );
            assert_eq!(decoded.payload.len(), 2);
            let net = u16::from_be_bytes([decoded.payload[0], decoded.payload[1]]);
            assert_eq!(net, 1000);
        }
        _ => panic!("Expected Broadcast for I-Am-Router announcement on port B"),
    }
}

#[tokio::test]
async fn three_port_router_announces_multiple_networks() {
    let (tx_a, mut rx_a) = mpsc::channel::<SendRequest>(256);
    let (tx_b, mut rx_b) = mpsc::channel::<SendRequest>(256);
    let (tx_c, mut rx_c) = mpsc::channel::<SendRequest>(256);

    let port_networks: Vec<u16> = vec![100, 200, 300];
    let send_txs = [tx_a, tx_b, tx_c];

    for (port_idx, tx) in send_txs.iter().enumerate() {
        let other_networks: Vec<u16> = port_networks
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx != port_idx)
            .map(|(_, net)| *net)
            .collect();

        if other_networks.is_empty() {
            continue;
        }

        let mut payload = BytesMut::with_capacity(other_networks.len() * 2);
        for net in &other_networks {
            payload.put_u16(*net);
        }

        let payload_len = payload.len();
        let response = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(8 + payload_len);
        encode_npdu(&mut buf, &response).unwrap();

        let _ = tx.try_send(SendRequest::Broadcast { npdu: buf.freeze() });
    }

    let sent_a = rx_a.try_recv().unwrap();
    match sent_a {
        SendRequest::Broadcast { npdu: data } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.is_network_message);
            assert_eq!(decoded.payload.len(), 4); // two u16 values
            let net1 = u16::from_be_bytes([decoded.payload[0], decoded.payload[1]]);
            let net2 = u16::from_be_bytes([decoded.payload[2], decoded.payload[3]]);
            assert_eq!(net1, 200);
            assert_eq!(net2, 300);
        }
        _ => panic!("Expected Broadcast on port A"),
    }

    let sent_b = rx_b.try_recv().unwrap();
    match sent_b {
        SendRequest::Broadcast { npdu: data } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert_eq!(decoded.payload.len(), 4);
            let net1 = u16::from_be_bytes([decoded.payload[0], decoded.payload[1]]);
            let net2 = u16::from_be_bytes([decoded.payload[2], decoded.payload[3]]);
            assert_eq!(net1, 100);
            assert_eq!(net2, 300);
        }
        _ => panic!("Expected Broadcast on port B"),
    }

    let sent_c = rx_c.try_recv().unwrap();
    match sent_c {
        SendRequest::Broadcast { npdu: data } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert_eq!(decoded.payload.len(), 4);
            let net1 = u16::from_be_bytes([decoded.payload[0], decoded.payload[1]]);
            let net2 = u16::from_be_bytes([decoded.payload[2], decoded.payload[3]]);
            assert_eq!(net1, 100);
            assert_eq!(net2, 200);
        }
        _ => panic!("Expected Broadcast on port C"),
    }
}

#[test]
fn forward_unicast_with_hop_count_one_still_forwards() {
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
        hop_count: 1,
        payload: Bytes::from_static(&[0x10, 0x08]),
        ..Npdu::default()
    };

    forward_unicast(&send_txs, &route, 1000, &[0x0A], npdu, 0);

    let sent = rx_b.try_recv().unwrap();
    match sent {
        SendRequest::Unicast { npdu: data, .. } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.destination.is_none());
            assert!(decoded.source.is_some());
        }
        SendRequest::Broadcast { npdu: data } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.destination.is_none());
        }
    }
}

#[tokio::test]
async fn received_reject_removes_learned_route() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_learned(3000, 0, MacAddr::from_slice(&[10, 0, 1, 1]));
    assert!(table.lookup(3000).is_some());

    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(3);
    payload.put_u8(RejectMessageReason::OTHER.to_raw());
    payload.put_u16(3000);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    let tbl = table.lock().await;
    assert!(tbl.lookup(3000).is_none());
    assert!(tbl.lookup(1000).is_some());
}

#[tokio::test]
async fn received_reject_does_not_remove_direct_route() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);

    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];
    let mut payload = BytesMut::with_capacity(3);
    payload.put_u8(RejectMessageReason::OTHER.to_raw());
    payload.put_u16(1000);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    let tbl = table.lock().await;
    assert!(tbl.lookup(1000).is_some());
}

#[tokio::test]
async fn who_is_router_with_specific_network() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_direct(3000, 2);

    let table = Arc::new(Mutex::new(table));

    let (tx, mut rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut req_payload = BytesMut::with_capacity(2);
    req_payload.put_u16(2000);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw()),
        payload: req_payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    let sent = rx.try_recv().unwrap();
    match sent {
        SendRequest::Broadcast { npdu: data } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.is_network_message);
            assert_eq!(
                decoded.message_type,
                Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw())
            );
            assert_eq!(decoded.payload.len(), 2);
            let net = u16::from_be_bytes([decoded.payload[0], decoded.payload[1]]);
            assert_eq!(net, 2000);
        }
        _ => panic!("Expected Broadcast response for I-Am-Router"),
    }
}

#[tokio::test]
async fn who_is_router_with_unknown_network_no_response() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);

    let table = Arc::new(Mutex::new(table));

    let (tx, mut rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut req_payload = BytesMut::with_capacity(2);
    req_payload.put_u16(9999);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw()),
        payload: req_payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn initialize_routing_table_ack() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);

    let table = Arc::new(Mutex::new(table));

    let (tx, mut rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE.to_raw()),
        payload: Bytes::new(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    let sent = rx.try_recv().unwrap();
    match sent {
        SendRequest::Unicast { npdu: data, mac } => {
            assert_eq!(mac.as_slice(), &[0x0A]);
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.is_network_message);
            assert_eq!(
                decoded.message_type,
                Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK.to_raw())
            );
            assert_eq!(decoded.payload.len(), 9);
            assert_eq!(decoded.payload[0], 2);
        }
        _ => panic!("Expected Unicast response for Init-Routing-Table"),
    }
}

#[tokio::test]
async fn router_busy_does_not_crash() {
    let table = RouterTable::new();
    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(4);
    payload.put_u16(1000);
    payload.put_u16(2000);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::ROUTER_BUSY_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;
}

#[tokio::test]
async fn router_available_does_not_crash() {
    let table = RouterTable::new();
    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(4);
    payload.put_u16(1000);
    payload.put_u16(2000);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;
}

#[tokio::test]
async fn i_could_be_router_stores_potential_route() {
    let table = RouterTable::new();
    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(3);
    payload.put_u16(5000);
    payload.put_u8(50);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A, 0x0B], &npdu).await;

    let tbl = table.lock().await;
    let entry = tbl.lookup(5000).unwrap();
    assert!(!entry.directly_connected);
    assert_eq!(entry.port_index, 0);
    assert_eq!(entry.next_hop_mac.as_slice(), &[0x0A, 0x0B]);
}

#[tokio::test]
async fn i_could_be_router_does_not_overwrite_existing_route() {
    let mut table = RouterTable::new();
    table.add_direct(5000, 1);
    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(3);
    payload.put_u16(5000);
    payload.put_u8(50);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    let tbl = table.lock().await;
    let entry = tbl.lookup(5000).unwrap();
    assert!(entry.directly_connected);
    assert_eq!(entry.port_index, 1);
}

#[tokio::test]
async fn establish_connection_does_not_crash() {
    let table = RouterTable::new();
    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(3);
    payload.put_u16(6000);
    payload.put_u8(30);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::ESTABLISH_CONNECTION_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;
}

#[tokio::test]
async fn disconnect_removes_learned_route() {
    let mut table = RouterTable::new();
    table.add_learned(7000, 0, MacAddr::from_slice(&[10, 0, 1, 1]));
    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(2);
    payload.put_u16(7000);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    let tbl = table.lock().await;
    assert!(tbl.lookup(7000).is_none());
}

#[tokio::test]
async fn disconnect_does_not_remove_direct_route() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(2);
    payload.put_u16(1000);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    let tbl = table.lock().await;
    assert!(tbl.lookup(1000).is_some());
    assert!(tbl.lookup(1000).unwrap().directly_connected);
}
