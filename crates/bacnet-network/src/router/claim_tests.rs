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
    let ctx = IngressContext::test_local(port, 1000, source_mac, npdu);
    handle_network_message(
        table,
        &[tx0, tx1],
        &ctx,
        &super::control_policy::ControlGate::permissive(),
    )
    .await;
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
async fn i_am_refresh_rearms_rejects_and_corroborated_learning_still_warns() {
    // Hardened gate: cross-port moves need two claims; same-port refreshes
    // re-arm reject hold-down immediately in both modes.
    let table = Arc::new(Mutex::new(RouterTable::new_hardened()));
    for (index, port) in [0, 0, 1, 0, 1, 0].into_iter().enumerate() {
        if index >= 2 {
            deliver(
                &table,
                port,
                &[index as u8],
                NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
                &[0x0b, 0xb8],
            )
            .await;
        }
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
            pending_started: 4,
            corroborated_applied: 4,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn truncated_tails_change_nothing_and_send_no_ack() {
    // RB-03 correction (Clauses 6.2/6.4/6.6.3.2): complete-payload validation
    // precedes every table-mutation path. Previously a trailing octet was
    // silently ignored — prefix entries were applied and the rebroadcast
    // (I-Am) or ACK (Init) was still emitted. Now the whole message is
    // dropped: no table change, no forward, no rebroadcast, no ACK.
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    let table = Arc::new(Mutex::new(table));

    // I-Am with a 1-octet tail: 3000 must not be learned, nothing rebroadcast.
    let output = deliver(
        &table,
        0,
        &[1],
        NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
        &[0, 0, 0xff, 0xff, 0x03, 0xe8, 0x0b, 0xb8, 0],
    )
    .await;
    assert!(output.iter().all(Vec::is_empty));
    assert_eq!(table.lock().await.len(), 1);

    // Init with a truncated tail is rejected whole: nothing applied, no ACK.
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
    assert!(output.iter().all(Vec::is_empty)); // No ACK for an invalid envelope.
    assert_eq!(table.lock().await.len(), 1);

    // The same truncation in an ACK still learns nothing.
    deliver(
        &table,
        1,
        &[2],
        NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
        &init,
    )
    .await;
    let table = table.lock().await;
    assert_eq!(table.len(), 1);
    assert!(table.lookup(1000).unwrap().directly_connected);
    assert!(table.lookup(3000).is_none());
    assert!(table.lookup(4000).is_none());
    assert_eq!(table.claim_snapshot(), RoutingClaimSnapshot::default());
}

#[tokio::test]
async fn each_learning_cap_counts_its_inspected_stop_without_changing_tail_handling() {
    for message_type in [
        NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
        NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
    ] {
        // Hardened: the existing-net cross-port move needs two claims to
        // corroborate, preserving the two-step assertion below in both modes
        // for I-Am (standard would converge on the first claim).
        let mut table = RouterTable::new_hardened();
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
        // before refresh.
        let existing: &[u8] = if message_type == NetworkMessageType::I_AM_ROUTER_TO_NETWORK {
            &[0, 1]
        } else {
            &[1, 0, 1, 0, 0]
        };
        deliver(&table, 1, &[2], message_type, existing).await;
        if message_type == NetworkMessageType::I_AM_ROUTER_TO_NETWORK {
            assert_eq!(table.lock().await.lookup(1).unwrap().port_index, 0);
            deliver(&table, 1, &[2], message_type, existing).await;
        }
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
async fn init_management_keeps_cap_for_new_entries_and_replaces_existing() {
    let mut table = RouterTable::new();
    for net in 1..=256 {
        table.add_learned(net, 0, MacAddr::from_slice(&[1]));
    }
    let table = Arc::new(Mutex::new(table));
    // Two unknown networks via Port ID 2 (local port 1): the first trips the
    // cap and stops the message; the second stays unexamined.
    deliver(
        &table,
        1,
        &[2],
        NetworkMessageType::INITIALIZE_ROUTING_TABLE,
        &[2, 0x0b, 0xb8, 2, 0, 0x0b, 0xb9, 2, 0],
    )
    .await;
    {
        let table = table.lock().await;
        assert_eq!(table.len(), 256);
        assert!(table.lookup(3000).is_none());
        assert!(table.lookup(3001).is_none());
        assert_eq!(
            table.claim_snapshot(),
            RoutingClaimSnapshot {
                learned_cap_ignored: 1,
                ..Default::default()
            }
        );
    }
    // Replacement of an existing learned entry bypasses the cap and rewrites
    // the port mapping immediately (no corroboration gate, no learned_ok:
    // management writes are not learning claims).
    deliver(
        &table,
        1,
        &[2],
        NetworkMessageType::INITIALIZE_ROUTING_TABLE,
        &[1, 0, 1, 2, 0],
    )
    .await;
    let table = table.lock().await;
    let route = table.lookup(1).unwrap();
    assert_eq!(route.port_index, 1);
    assert!(!route.directly_connected);
    assert_eq!(route.next_hop_mac.as_slice(), &[2]);
    assert_eq!(
        table.claim_snapshot(),
        RoutingClaimSnapshot {
            learned_cap_ignored: 1,
            ..Default::default()
        }
    );
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
async fn init_replace_and_ack_refresh_both_rearm_reject_hold_down() {
    // RB-05: a management replace installs a fresh learned entry (clearing
    // reject records like fresh learning), so a later reject applies again;
    // the ACK same-port refresh re-arms the same way.
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1]));
    let table = Arc::new(Mutex::new(table));
    reject(&table, 0, &[1], 2).await;
    // Port ID 1 names local port 0: replaces 3000 in place, re-arming.
    deliver(
        &table,
        1,
        &[2],
        NetworkMessageType::INITIALIZE_ROUTING_TABLE,
        &[1, 0x0b, 0xb8, 1, 0],
    )
    .await;
    reject(&table, 0, &[1], 1).await;
    assert_eq!(
        table.lock().await.effective_reachability(3000),
        Some(ReachabilityStatus::Unreachable)
    );
    deliver(
        &table,
        0,
        &[2],
        NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
        &[1, 0x0b, 0xb8, 0, 0],
    )
    .await;
    reject(&table, 0, &[1], 2).await;
    let table = table.lock().await;
    assert_eq!(
        table.effective_reachability(3000),
        Some(ReachabilityStatus::Busy)
    );
    let route = table.lookup(3000).unwrap();
    assert_eq!(route.port_index, 0);
    assert_eq!(route.next_hop_mac.as_slice(), &[2]);
    assert_eq!(
        table.claim_snapshot(),
        RoutingClaimSnapshot {
            learned_ok: 1,
            reject_applied: 3,
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
        handle_network_message(
            &table,
            &send_txs,
            &IngressContext::test_local(0, 1000, &[1], npdu.clone()),
            &super::control_policy::ControlGate::permissive(),
        )
        .await;
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
            // The trailing Busy claim on a permanently Unreachable route is
            // a same-state no-op (it can never resurrect via auto-clear), not
            // a held-down change — while every reject still relays above.
            reject_dampened_same_state: 2,
            ..Default::default()
        }
    );
}
