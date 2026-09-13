use super::*;
use crate::router_table::RoutingClaimSnapshot;

// Invoke the real handler with in-memory port queues; no sleeps, sockets or
// background aging. Exact hold-down boundaries are covered by injected table time.
async fn deliver(
    table: &Arc<Mutex<RouterTable>>,
    port: usize,
    source_mac: &[u8],
    message_type: NetworkMessageType,
    payload: &[u8],
) -> [Vec<SendRequest>; 2] {
    let (tx0, mut rx0) = mpsc::channel(16);
    let (tx1, mut rx1) = mpsc::channel(16);
    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(message_type.to_raw()),
        payload: Bytes::copy_from_slice(payload),
        ..Default::default()
    };
    handle_network_message(table, &[tx0, tx1], port, 1000, source_mac, &npdu).await;
    let mut output = [Vec::new(), Vec::new()];
    while let Ok(request) = rx0.try_recv() {
        output[0].push(request);
    }
    while let Ok(request) = rx1.try_recv() {
        output[1].push(request);
    }
    output
}

async fn reject(table: &Arc<Mutex<RouterTable>>, port: usize, source_mac: &[u8], reason: u8) {
    let output = deliver(
        table,
        port,
        source_mac,
        NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
        &[reason, 0x0b, 0xb8],
    )
    .await;
    assert!(output.iter().all(Vec::is_empty));
}

#[tokio::test]
async fn first_rejects_keep_legacy_effects_and_repeated_claims_count_once() {
    for reason in [0, 1, 2] {
        let mut table = RouterTable::new();
        table.add_learned(3000, 0, MacAddr::from_slice(&[1]));
        let table = Arc::new(Mutex::new(table));
        for _ in 0..12 {
            reject(&table, 0, &[1], reason).await;
        }
        let table = table.lock().await;
        match reason {
            1 => assert_eq!(
                table.effective_reachability(3000),
                Some(ReachabilityStatus::Unreachable)
            ),
            2 => assert_eq!(
                table.effective_reachability(3000),
                Some(ReachabilityStatus::Busy)
            ),
            _ => assert!(table.lookup(3000).is_none()),
        }
        assert_eq!(
            table.claim_snapshot(),
            RoutingClaimSnapshot {
                reject_applied: 1,
                reject_dampened: 11,
                reject_dampened_same_state: 11,
                ..Default::default()
            }
        );
    }
}

#[tokio::test]
async fn spoofing_mac_does_not_bypass_hold_down_but_another_ingress_is_independent() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1]));
    let table = Arc::new(Mutex::new(table));
    reject(&table, 0, &[1], 2).await;
    for mac in 1..=10 {
        reject(&table, 0, &[mac], 1).await;
    }
    assert_eq!(
        table.lock().await.effective_reachability(3000),
        Some(ReachabilityStatus::Busy)
    );
    reject(&table, 1, &[1], 1).await;
    let table = table.lock().await;
    assert_eq!(
        table.effective_reachability(3000),
        Some(ReachabilityStatus::Unreachable)
    );
    assert_eq!(
        table.claim_snapshot(),
        RoutingClaimSnapshot {
            reject_applied: 2,
            reject_dampened: 10,
            reject_dampened_hold_down: 10,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn i_am_refresh_rearms_rejects_and_last_wins_learning_still_warns() {
    let table = Arc::new(Mutex::new(RouterTable::new()));
    for (index, port) in [0, 0, 1, 0, 1, 0].into_iter().enumerate() {
        let output = deliver(
            &table,
            port,
            &[index as u8],
            NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
            &[0x0b, 0xb8],
        )
        .await;
        assert!(output[port].is_empty());
        assert_eq!(output[1 - port].len(), 1); // Existing unconditional rebroadcast.
        {
            let table = table.lock().await;
            let route = table.lookup(3000).unwrap();
            assert_eq!(route.port_index, port);
            assert_eq!(route.next_hop_mac.as_slice(), &[index as u8]);
            assert_eq!(route.reachability, ReachabilityStatus::Reachable);
        }
        reject(&table, 0, &[1], 1).await;
        assert_eq!(
            table.lock().await.effective_reachability(3000),
            Some(ReachabilityStatus::Unreachable)
        );
    }
    let table = table.lock().await;
    assert_eq!(table.lookup(3000).unwrap().flap_count, 4);
    assert_eq!(
        table.claim_snapshot(),
        RoutingClaimSnapshot {
            learned_ok: 6,
            reject_applied: 6,
            flap_warned: 2,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn learning_outcomes_exclude_direct_reserved_and_truncated_entries() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    let table = Arc::new(Mutex::new(table));
    deliver(
        &table,
        0,
        &[1],
        NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
        &[0, 0, 0xff, 0xff, 0x03, 0xe8, 0x0b, 0xb8, 0],
    )
    .await;
    // Init preserves existing routes; only 4000 is new, the trailing entry is truncated.
    let init = [
        6, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 0x03, 0xe8, 0, 0, 0x0b, 0xb8, 0, 0, 0x0f, 0xa0, 0, 0, 0x13,
    ];
    let output = deliver(
        &table,
        1,
        &[2],
        NetworkMessageType::INITIALIZE_ROUTING_TABLE,
        &init,
    )
    .await;
    assert_eq!(output[1].len(), 1); // ACK still generated.
    assert_eq!(table.lock().await.lookup(3000).unwrap().port_index, 0);
    // ACK refreshes existing learned routes, but still cannot overwrite direct.
    deliver(
        &table,
        1,
        &[2],
        NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
        &init,
    )
    .await;
    let table = table.lock().await;
    assert_eq!(table.len(), 3);
    assert!(table.lookup(1000).unwrap().directly_connected);
    assert_eq!(table.lookup(3000).unwrap().port_index, 1);
    assert_eq!(
        table.claim_snapshot(),
        RoutingClaimSnapshot {
            learned_ok: 4,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn each_learning_cap_counts_its_inspected_stop_without_changing_tail_handling() {
    for message_type in [
        NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
        NetworkMessageType::INITIALIZE_ROUTING_TABLE,
        NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
    ] {
        let mut table = RouterTable::new();
        for net in 1..=256 {
            table.add_learned(net, 0, MacAddr::from_slice(&[1]));
        }
        let table = Arc::new(Mutex::new(table));
        // The unknown 3000 triggers the cap; the existing 1 behind it stays unexamined.
        let payload: &[u8] = if message_type == NetworkMessageType::I_AM_ROUTER_TO_NETWORK {
            &[0x0b, 0xb8, 0, 1]
        } else {
            &[2, 0x0b, 0xb8, 0, 0, 0, 1, 0, 0]
        };
        deliver(&table, 1, &[2], message_type, payload).await;
        {
            let table = table.lock().await;
            assert_eq!(table.len(), 256);
            assert!(table.lookup(3000).is_none());
            assert_eq!(table.lookup(1).unwrap().port_index, 0);
            assert_eq!(
                table.claim_snapshot(),
                RoutingClaimSnapshot {
                    learned_cap_ignored: 1,
                    ..Default::default()
                }
            );
        }
        // Existing per-message cap behavior is preserved, including ACK's cap
        // before refresh and Init's refusal to overwrite an existing route.
        let existing: &[u8] = if message_type == NetworkMessageType::I_AM_ROUTER_TO_NETWORK {
            &[0, 1]
        } else {
            &[1, 0, 1, 0, 0]
        };
        deliver(&table, 1, &[2], message_type, existing).await;
        let table = table.lock().await;
        let refreshed = message_type == NetworkMessageType::I_AM_ROUTER_TO_NETWORK;
        assert_eq!(table.lookup(1).unwrap().port_index, usize::from(refreshed));
        assert_eq!(table.claim_snapshot().learned_ok, u64::from(refreshed));
        assert_eq!(
            table.claim_snapshot().learned_cap_ignored,
            if message_type == NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK {
                2
            } else {
                1
            }
        );
    }
}

#[tokio::test]
async fn direct_route_immunity_and_malformed_rejects_under_spam() {
    let mut table = RouterTable::new();
    table.add_direct(3000, 0);
    let table = Arc::new(Mutex::new(table));
    for _ in 0..10 {
        for reason in [0, 1, 2] {
            reject(&table, 1, &[1], reason).await;
        }
    }
    for payload in [&[][..], &[1][..], &[1, 0x0b][..]] {
        deliver(
            &table,
            0,
            &[1],
            NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
            payload,
        )
        .await;
    }
    let table = table.lock().await;
    assert_eq!(
        table.effective_reachability(3000),
        Some(ReachabilityStatus::Reachable)
    );
    assert!(table.lookup(3000).unwrap().directly_connected);
    assert_eq!(
        table.claim_snapshot(),
        RoutingClaimSnapshot {
            reject_dampened: 30,
            reject_dampened_same_state: 30,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn init_ack_refresh_rearms_but_init_existing_entry_does_not() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1]));
    let table = Arc::new(Mutex::new(table));
    let payload = [1, 0x0b, 0xb8, 0, 0];
    reject(&table, 0, &[1], 2).await;
    deliver(
        &table,
        1,
        &[2],
        NetworkMessageType::INITIALIZE_ROUTING_TABLE,
        &payload,
    )
    .await;
    reject(&table, 0, &[1], 1).await;
    assert_eq!(
        table.lock().await.effective_reachability(3000),
        Some(ReachabilityStatus::Busy)
    );
    deliver(
        &table,
        1,
        &[2],
        NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
        &payload,
    )
    .await;
    reject(&table, 0, &[1], 1).await;
    let table = table.lock().await;
    assert_eq!(
        table.effective_reachability(3000),
        Some(ReachabilityStatus::Unreachable)
    );
    assert_eq!(table.lookup(3000).unwrap().port_index, 1);
    assert_eq!(
        table.claim_snapshot(),
        RoutingClaimSnapshot {
            learned_ok: 1,
            reject_applied: 2,
            reject_dampened: 1,
            reject_dampened_hold_down: 1,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn dampened_table_transition_still_relays_every_reject() {
    use bacnet_encoding::npdu::NpduAddress;

    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1]));
    table.add_direct(4000, 1);
    let table = Arc::new(Mutex::new(table));
    let (tx0, mut rx0) = mpsc::channel(16);
    let (tx1, mut rx1) = mpsc::channel(16);
    let send_txs = [tx0, tx1];
    let source = NpduAddress {
        network: 4000,
        mac_address: MacAddr::from_slice(&[7]),
    };
    for reason in [1, 1, 2] {
        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw()),
            source: Some(source.clone()),
            payload: Bytes::from(vec![reason, 0x0b, 0xb8]),
            ..Default::default()
        };
        handle_network_message(&table, &send_txs, 0, 1000, &[1], &npdu).await;
        let SendRequest::Unicast {
            npdu: data, mac, ..
        } = rx1.try_recv().unwrap()
        else {
            panic!("expected original reject relay");
        };
        assert_eq!(mac, source.mac_address);
        let relayed = decode_npdu(data).unwrap();
        assert_eq!(relayed.destination, Some(source.clone()));
        assert_eq!(relayed.payload, npdu.payload);
        assert_eq!(relayed.message_type, npdu.message_type);
        assert!(relayed.source.is_none());
        assert_eq!(relayed.hop_count, 255);
        assert!(rx0.try_recv().is_err());
        assert!(rx1.try_recv().is_err());
    }
    assert_eq!(
        table.lock().await.claim_snapshot(),
        RoutingClaimSnapshot {
            reject_applied: 1,
            reject_dampened: 2,
            reject_dampened_same_state: 1,
            reject_dampened_hold_down: 1,
            ..Default::default()
        }
    );
}
