//! RB-03 discovery tests: Who-Is origin, Init envelopes, directed routing.
//!
//! Clause map (ASHRAE 135-2020, supplied PDF): 6.5.4 (retain routed
//! SNET/SADR, else add ingress net + SA; forward data attributes), 6.6.3.2
//! (Who-Is discovery relay; scoped queries never promote to global scans),
//! 6.4.7/6.6.3.8 (Initialize-Routing-Table query vs. update envelopes).
//!
//! Shares [`super::envelope_harness`] with the control-envelope module.

use super::envelope_harness::*;
use super::*;
use bacnet_encoding::npdu::NpduAddress;

// --- Who-Is discovery origin (Clause 6.6.3.2) ---

#[tokio::test]
async fn who_is_local_origin_adds_ingress_source_and_attributes_on_forward() {
    let mut h = Harness::two_port();
    let mut npdu = control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x0b, 0xb8]);
    let mut ctx = h.ctx(0, &[7], npdu.clone());
    ctx.data_attributes = attributes();
    npdu = ctx.npdu.clone();
    h.handle(ctx).await;

    assert!(h.drain(0).is_empty());
    let forwarded = broadcast_data(h.drain(1));
    assert_eq!(forwarded.len(), 1);
    let relayed = decode_npdu(forwarded[0].clone()).unwrap();
    // Locally originated: ingress network + immediate source MAC added.
    assert_eq!(
        relayed.source,
        Some(NpduAddress {
            network: 1000,
            mac_address: MacAddr::from_slice(&[7]),
        })
    );
    assert!(relayed.destination.is_none());
    assert_eq!(relayed.payload, npdu.payload);
    assert_eq!(
        relayed.message_type,
        Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw())
    );
    // Next hop stays separate: the relay is a link broadcast, not addressed.
    assert_eq!(h.table.lock().await.len(), 2);
}

#[tokio::test]
async fn who_is_routed_origin_retains_source() {
    let mut h = Harness::two_port();
    let mut npdu = control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x0b, 0xb8]);
    npdu.source = Some(NpduAddress {
        network: 900,
        mac_address: MacAddr::from_slice(&[9]),
    });
    let mut ctx = h.ctx(0, &[7], npdu);
    ctx.data_attributes = attributes();
    h.handle(ctx).await;

    let forwarded = broadcast_data(h.drain(1));
    assert_eq!(forwarded.len(), 1);
    let relayed = decode_npdu(forwarded[0].clone()).unwrap();
    // Already routed: SNET/SADR retained, ingress MAC not merged in.
    assert_eq!(
        relayed.source,
        Some(NpduAddress {
            network: 900,
            mac_address: MacAddr::from_slice(&[9]),
        })
    );
}

#[tokio::test]
async fn who_is_scoped_known_network_answered_locally_not_forwarded() {
    let mut h = Harness::two_port();
    let mut npdu = control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x07, 0xd0]);
    let mut ctx = h.ctx(0, &[7], npdu.clone());
    ctx.data_attributes = attributes();
    npdu = ctx.npdu.clone();
    h.handle(ctx).await;

    // I-Am reply broadcast on the ingress port only, carrying attributes.
    assert!(h.drain(1).is_empty());
    let mut replies = h.drain(0);
    assert_eq!(replies.len(), 1);
    let reply = replies.pop().unwrap();
    let (data, attrs) = match reply {
        SendRequest::Broadcast {
            npdu,
            data_attributes,
        } => (npdu, data_attributes),
        SendRequest::Unicast { .. } => panic!("expected broadcast I-Am reply"),
    };
    let decoded = decode_npdu(data).unwrap();
    assert_eq!(
        decoded.message_type,
        Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw())
    );
    assert_eq!(decoded.payload.as_ref(), &[0x07, 0xd0]);
    assert_eq!(attrs, attributes());
    assert_eq!(npdu.payload.as_ref(), &[0x07, 0xd0]);
}

#[tokio::test]
async fn who_is_unscoped_lists_reachable_not_ingress() {
    let mut h = Harness::two_port();
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[]),
    ))
    .await;

    assert!(h.drain(1).is_empty());
    let replies = broadcast_data(h.drain(0));
    assert_eq!(replies.len(), 1);
    let decoded = decode_npdu(replies[0].clone()).unwrap();
    // 2000 reachable via another port; the ingress 1000 is excluded.
    assert_eq!(decoded.payload.as_ref(), &[0x07, 0xd0]);
}

#[tokio::test]
async fn who_is_truncated_tail_never_promotes_to_global_scan() {
    let mut h = Harness::two_port();
    // A 1-octet tail was previously answered as an unscoped global query.
    for payload in [&[0x0b][..], &[0x0b, 0xb8, 0x00][..], &[0, 0][..]] {
        h.handle(h.ctx(
            0,
            &[7],
            control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, payload),
        ))
        .await;
        h.assert_quiet();
    }
    assert_eq!(h.table.lock().await.len(), 2);
}

// --- Initialize-Routing-Table transactional envelope ---

#[tokio::test]
async fn init_update_query_and_garbage_envelopes() {
    let mut h = Harness::two_port();

    // Truncated entry: no table change, no ACK (previously ACK-on-partial).
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[1, 0x0b, 0xb8],
        ),
    ))
    .await;
    h.assert_quiet();
    assert_eq!(h.table.lock().await.len(), 2);

    // Trailing garbage after a zero count: not a query, no ACK.
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &[0, 0xff]),
    ))
    .await;
    h.assert_quiet();

    // Valid update: Port ID 1 names local port 0 (the wire mapping, not the
    // ingress port 1). Learns 3000 there; the ACK carries no table data and
    // is routed to the origin.
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[1, 0x0b, 0xb8, 1, 0],
        ),
    ))
    .await;
    assert!(h.drain(0).is_empty());
    let mut acks = h.drain(1);
    assert_eq!(acks.len(), 1);
    match acks.pop().unwrap() {
        SendRequest::Unicast { npdu, mac, .. } => {
            assert_eq!(mac.as_slice(), &[2]);
            let decoded = decode_npdu(npdu).unwrap();
            assert_eq!(
                decoded.message_type,
                Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK.to_raw())
            );
            assert!(decoded.payload.is_empty());
        }
        SendRequest::Broadcast { .. } => panic!("expected unicast ACK"),
    }
    let route = h.table.lock().await.lookup(3000).unwrap().clone();
    assert_eq!(route.port_index, 0);
    assert!(!route.directly_connected);
    assert_eq!(route.next_hop_mac.as_slice(), &[2]);

    // Valid query: one ACK carries the full table — ascending DNETs with
    // stable nonzero wire Port IDs (1000->1, 2000->2, 3000->1).
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &[0]),
    ))
    .await;
    let mut acks = h.drain(1);
    assert_eq!(acks.len(), 1);
    match acks.pop().unwrap() {
        SendRequest::Unicast { npdu, .. } => {
            let decoded = decode_npdu(npdu).unwrap();
            assert_eq!(
                decoded.payload.as_ref(),
                &[3, 0x03, 0xe8, 1, 0, 0x07, 0xd0, 2, 0, 0x0b, 0xb8, 1, 0]
            );
        }
        SendRequest::Broadcast { .. } => panic!("expected unicast ACK"),
    }
}

#[tokio::test]
async fn init_ack_truncated_entry_learns_nothing() {
    let mut h = Harness::two_port();
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
            &[2, 0x0b, 0xb8, 0, 0, 0x0f],
        ),
    ))
    .await;
    h.assert_quiet();
    let table = h.table.lock().await;
    assert_eq!(table.len(), 2);
    assert!(table.lookup(3000).is_none());
}

// --- Init query/ACK response path (relocated from tests.rs, RB-03) ---

#[tokio::test]
async fn initialize_routing_table_ack() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);

    let table = Arc::new(Mutex::new(table));

    let (tx, mut rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    // RB-03: a table query is Number of Ports == 0 (one octet), not a
    // missing field. An empty payload is a truncated envelope: no ACK.
    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE.to_raw()),
        payload: Bytes::from_static(&[0]),
        ..Npdu::default()
    };

    handle_network_message(
        &table,
        &send_txs,
        &IngressContext::test_local(0, 1000, &[0x0A], npdu),
        &control_policy::ControlGate::permissive(),
    )
    .await;

    let sent = rx.try_recv().unwrap();
    match sent {
        SendRequest::Unicast {
            npdu: data, mac, ..
        } => {
            assert_eq!(mac.as_slice(), &[0x0A]);
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.is_network_message);
            assert_eq!(
                decoded.message_type,
                Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK.to_raw())
            );
            assert_eq!(decoded.payload.len(), 9);
            assert_eq!(decoded.payload[0], 2);
            // Stable nonzero wire Port IDs in ascending-DNET order.
            assert_eq!(
                decoded.payload.as_ref(),
                &[2, 0x03, 0xe8, 1, 0, 0x07, 0xd0, 2, 0]
            );
        }
        _ => panic!("Expected Unicast response for Init-Routing-Table"),
    }
}

#[tokio::test]
async fn initialize_routing_table_empty_payload_sends_no_ack() {
    // RB-03: the Number of Ports octet is absent, so this is a truncated
    // envelope, not a query. No table change, no ACK.
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    let table = Arc::new(Mutex::new(table));

    let (tx, mut rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE.to_raw()),
        payload: Bytes::new(),
        ..Npdu::default()
    };

    handle_network_message(
        &table,
        &send_txs,
        &IngressContext::test_local(0, 1000, &[0x0A], npdu),
        &control_policy::ControlGate::permissive(),
    )
    .await;

    assert!(rx.try_recv().is_err());
    assert_eq!(table.lock().await.len(), 1);
}

// --- Routing-first dispatcher for directed controls ---

#[tokio::test]
async fn directed_control_routed_not_consumed() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_learned(3000, 1, MacAddr::from_slice(&[5]));
    let mut h = Harness::with_table(table);

    // I-Am addressed to a remote network: forwarded toward it, never learned.
    let mut npdu = control_npdu(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0x0f, 0xa0]);
    npdu.destination = Some(NpduAddress {
        network: 3000,
        mac_address: MacAddr::from_slice(&[9]),
    });
    npdu.hop_count = 255;
    let mut ctx = h.ctx(0, &[7], npdu);
    ctx.data_attributes = attributes();
    h.dispatch(ctx).await;

    assert!(h.drain(0).is_empty());
    let mut forwarded = h.drain(1);
    assert_eq!(forwarded.len(), 1);
    match forwarded.pop().unwrap() {
        SendRequest::Unicast {
            npdu,
            mac,
            data_attributes,
        } => {
            assert_eq!(mac.as_slice(), &[5]);
            let decoded = decode_npdu(npdu).unwrap();
            assert_eq!(
                decoded.message_type,
                Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw())
            );
            assert_eq!(
                decoded.destination,
                Some(NpduAddress {
                    network: 3000,
                    mac_address: MacAddr::from_slice(&[9]),
                })
            );
            assert_eq!(decoded.hop_count, 254);
            assert_eq!(decoded.payload.as_ref(), &[0x0f, 0xa0]);
            assert_eq!(data_attributes, attributes());
        }
        SendRequest::Broadcast { .. } => panic!("expected unicast route"),
    }
    // The payload network was forwarded, not learned.
    let table = h.table.lock().await;
    assert!(table.lookup(4000).is_none());
    assert_eq!(table.lookup(3000).unwrap().port_index, 1);
}

#[tokio::test]
async fn directed_control_to_own_network_handled_locally() {
    let mut h = Harness::two_port();

    // Who-Is for 2000 sent to our own network 1000: answered here, not routed.
    let mut npdu = control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x07, 0xd0]);
    npdu.destination = Some(NpduAddress {
        network: 1000,
        mac_address: MacAddr::from_slice(&[1]),
    });
    npdu.hop_count = 255;
    h.dispatch(h.ctx(0, &[7], npdu)).await;

    assert!(h.drain(1).is_empty());
    let replies = broadcast_data(h.drain(0));
    assert_eq!(replies.len(), 1);
    assert_eq!(
        decode_npdu(replies[0].clone()).unwrap().payload.as_ref(),
        &[0x07, 0xd0]
    );
}

#[tokio::test]
async fn directed_control_to_unknown_network_rejected() {
    let mut h = Harness::two_port();

    let mut npdu = control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x27, 0x0f]);
    npdu.destination = Some(NpduAddress {
        network: 9999,
        mac_address: MacAddr::from_slice(&[9]),
    });
    npdu.hop_count = 255;
    h.dispatch(h.ctx(0, &[7], npdu)).await;

    // RB-06: unknown first solicits (bounded Who-Is on the other port) then
    // fails the caller with the honest retryable reject; nothing is buffered.
    let solicited = broadcast_data(h.drain(1));
    assert_eq!(solicited.len(), 1);
    let who_is = decode_npdu(solicited[0].clone()).unwrap();
    assert_eq!(
        who_is.message_type,
        Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw())
    );
    assert_eq!(who_is.payload.as_ref(), &[0x27, 0x0f]);
    let mut rejects = h.drain(0);
    assert_eq!(rejects.len(), 1);
    match rejects.pop().unwrap() {
        SendRequest::Unicast { npdu, mac, .. } => {
            assert_eq!(mac.as_slice(), &[7]);
            let decoded = decode_npdu(npdu).unwrap();
            assert_eq!(
                decoded.message_type,
                Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw())
            );
            assert_eq!(
                decoded.payload.as_ref(),
                &[
                    RejectMessageReason::NOT_DIRECTLY_CONNECTED.to_raw(),
                    0x27,
                    0x0f
                ]
            );
        }
        SendRequest::Broadcast { .. } => panic!("expected unicast reject"),
    }
    assert_eq!(h.table.lock().await.len(), 2);
}

#[tokio::test]
async fn directed_who_is_remote_broadcast_forwarded_with_origin() {
    let mut h = Harness::two_port();

    // Locally-originated remote broadcast: origin added at forward time.
    let mut npdu = control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x0b, 0xb8]);
    npdu.destination = Some(NpduAddress {
        network: 2000,
        mac_address: MacAddr::new(),
    });
    npdu.hop_count = 255;
    h.dispatch(h.ctx(0, &[7], npdu)).await;
    assert!(h.drain(0).is_empty());
    let forwarded = broadcast_data(h.drain(1));
    assert_eq!(forwarded.len(), 1);
    let decoded = decode_npdu(forwarded[0].clone()).unwrap();
    assert!(decoded.destination.is_none());
    assert_eq!(
        decoded.source,
        Some(NpduAddress {
            network: 1000,
            mac_address: MacAddr::from_slice(&[7]),
        })
    );

    // Already-routed remote broadcast: origin retained.
    let mut npdu = control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x0b, 0xb8]);
    npdu.destination = Some(NpduAddress {
        network: 2000,
        mac_address: MacAddr::new(),
    });
    npdu.source = Some(NpduAddress {
        network: 900,
        mac_address: MacAddr::from_slice(&[9]),
    });
    npdu.hop_count = 254;
    h.dispatch(h.ctx(0, &[7], npdu)).await;
    let forwarded = broadcast_data(h.drain(1));
    assert_eq!(forwarded.len(), 1);
    assert_eq!(
        decode_npdu(forwarded[0].clone()).unwrap().source,
        Some(NpduAddress {
            network: 900,
            mac_address: MacAddr::from_slice(&[9]),
        })
    );
}

// --- Link-group provenance is not an effective-group verdict ---

#[tokio::test]
async fn link_broadcast_routed_unicast_control_stays_unicast() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_learned(3000, 1, MacAddr::from_slice(&[5]));
    let mut h = Harness::with_table(table);

    // A routed unicast arriving over a link broadcast still forwards unicast.
    let mut npdu = control_npdu(NetworkMessageType::ROUTER_BUSY_TO_NETWORK, &[0x0b, 0xb8]);
    npdu.destination = Some(NpduAddress {
        network: 3000,
        mac_address: MacAddr::from_slice(&[9]),
    });
    npdu.hop_count = 255;
    let mut ctx = h.ctx(0, &[7], npdu);
    ctx.link_layer_group = true;
    h.dispatch(ctx).await;

    assert!(h.drain(0).is_empty());
    match h.drain(1).pop() {
        Some(SendRequest::Unicast { .. }) => {}
        other => panic!("expected unicast forward, got {other:?}"),
    }
    // A transit control is never consumed: no busy mark from our own relay.
    assert_eq!(
        h.table.lock().await.effective_reachability(3000),
        Some(ReachabilityStatus::Reachable)
    );
}
