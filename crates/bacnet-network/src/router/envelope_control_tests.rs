//! RB-03 control-envelope tests: exact lengths, lists, and opacity.
//!
//! Clause map (ASHRAE 135-2020, supplied PDF): 6.2 (envelope rules), 6.4
//! (per-message data formats), 6.2.4/6.5.4 (proprietary opacity in relay),
//! 6.4.14/6.4.15 + 6.6.3.12 (What-Is/Number-Is non-routed restrictions).
//!
//! Shares [`super::envelope_harness`] with the discovery module.

use super::envelope_harness::*;
use super::*;
use bacnet_encoding::npdu::NpduAddress;

// --- I-Am / Busy / Available complete-list validation ---

#[tokio::test]
async fn i_am_empty_and_truncated_payloads_learn_nothing() {
    let mut h = Harness::two_port();
    for payload in [&[][..], &[0x0b][..], &[0x0b, 0xb8, 0x0b][..]] {
        h.handle(h.ctx(
            1,
            &[2],
            control_npdu(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, payload),
        ))
        .await;
        h.assert_quiet();
    }
    let table = h.table.lock().await;
    assert_eq!(table.len(), 2);
    assert!(table.lookup(3000).is_none());
}

#[tokio::test]
async fn busy_available_empty_valid_and_odd_payloads() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_learned(3000, 0, MacAddr::from_slice(&[1]));
    let mut h = Harness::with_table(table);

    // Truncated tail: no marks, no rebroadcast.
    for kind in [
        NetworkMessageType::ROUTER_BUSY_TO_NETWORK,
        NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK,
    ] {
        h.handle(h.ctx(1, &[2], control_npdu(kind, &[0x0b]))).await;
        h.assert_quiet();
        assert_eq!(
            h.table.lock().await.effective_reachability(3000),
            Some(ReachabilityStatus::Reachable)
        );
    }

    // Empty (omitted) list keeps its existing meaning: rebroadcast, no marks.
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(NetworkMessageType::ROUTER_BUSY_TO_NETWORK, &[]),
    ))
    .await;
    assert_eq!(broadcast_data(h.drain(0)).len(), 1);
    assert!(h.drain(1).is_empty());
    assert_eq!(
        h.table.lock().await.effective_reachability(3000),
        Some(ReachabilityStatus::Reachable)
    );

    // Valid list: marked busy and rebroadcast verbatim.
    let mut ctx = h.ctx(
        1,
        &[2],
        control_npdu(NetworkMessageType::ROUTER_BUSY_TO_NETWORK, &[0x0b, 0xb8]),
    );
    ctx.data_attributes = attributes();
    h.handle(ctx).await;
    assert_eq!(
        h.table.lock().await.effective_reachability(3000),
        Some(ReachabilityStatus::Busy)
    );
    let rebroadcasts = broadcast_data(h.drain(0));
    assert_eq!(rebroadcasts.len(), 1);
    assert_eq!(
        decode_npdu(rebroadcasts[0].clone())
            .unwrap()
            .payload
            .as_ref(),
        &[0x0b, 0xb8]
    );

    // Valid Available clears the mark.
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(
            NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK,
            &[0x0b, 0xb8],
        ),
    ))
    .await;
    assert_eq!(
        h.table.lock().await.effective_reachability(3000),
        Some(ReachabilityStatus::Reachable)
    );
    assert_eq!(broadcast_data(h.drain(0)).len(), 1);
}

// --- Exact-length fixed controls ---

#[tokio::test]
async fn reject_requires_exact_length_for_table_change_and_relay() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1]));
    table.add_direct(4000, 1);
    let mut h = Harness::with_table(table);

    // Short and long tails: no table change, no relay.
    for payload in [&[1, 0x0b][..], &[1, 0x0b, 0xb8, 0xff][..]] {
        let mut npdu = control_npdu(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, payload);
        npdu.source = Some(NpduAddress {
            network: 4000,
            mac_address: MacAddr::from_slice(&[7]),
        });
        h.handle(h.ctx(0, &[1], npdu)).await;
        h.assert_quiet();
        assert_eq!(
            h.table.lock().await.effective_reachability(3000),
            Some(ReachabilityStatus::Reachable)
        );
    }

    // Exact envelope: table changes and the reject relays toward the origin.
    let mut npdu = control_npdu(
        NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
        &[1, 0x0b, 0xb8],
    );
    npdu.source = Some(NpduAddress {
        network: 4000,
        mac_address: MacAddr::from_slice(&[7]),
    });
    let mut ctx = h.ctx(0, &[1], npdu);
    ctx.data_attributes = attributes();
    h.handle(ctx).await;
    assert_eq!(
        h.table.lock().await.effective_reachability(3000),
        Some(ReachabilityStatus::Unreachable)
    );
    let mut relayed = h.drain(1);
    assert_eq!(relayed.len(), 1);
    match relayed.pop().unwrap() {
        SendRequest::Unicast {
            npdu,
            mac,
            data_attributes,
        } => {
            assert_eq!(mac.as_slice(), &[7]);
            let decoded = decode_npdu(npdu).unwrap();
            assert_eq!(
                decoded.destination,
                Some(NpduAddress {
                    network: 4000,
                    mac_address: MacAddr::from_slice(&[7]),
                })
            );
            assert_eq!(data_attributes, attributes());
        }
        SendRequest::Broadcast { .. } => panic!("expected unicast relay"),
    }
}

#[tokio::test]
async fn fixed_length_controls_ignore_malformed_tails() {
    let mut h = Harness::two_port();

    // I-Could-Be needs exactly DNET(2) + index(1).
    h.handle(h.ctx(
        0,
        &[1],
        control_npdu(
            NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK,
            &[0x0b, 0xb8, 0x00, 0xff],
        ),
    ))
    .await;
    h.assert_quiet();
    assert!(h.table.lock().await.lookup(3000).is_none());

    // Establish needs exactly DNET(2) + termination(1); Disconnect exactly 2.
    h.handle(h.ctx(
        0,
        &[1],
        control_npdu(
            NetworkMessageType::ESTABLISH_CONNECTION_TO_NETWORK,
            &[0x0b, 0xb8],
        ),
    ))
    .await;
    h.handle(h.ctx(
        0,
        &[1],
        control_npdu(
            NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK,
            &[0x0b, 0xb8, 0x00],
        ),
    ))
    .await;
    h.assert_quiet();
    assert_eq!(
        h.table
            .lock()
            .await
            .claim_snapshot()
            .disconnect_removal_ignored,
        0
    );

    // The exact Disconnect envelope still counts.
    h.handle(h.ctx(
        0,
        &[1],
        control_npdu(
            NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK,
            &[0x0b, 0xb8],
        ),
    ))
    .await;
    assert_eq!(
        h.table
            .lock()
            .await
            .claim_snapshot()
            .disconnect_removal_ignored,
        1
    );
}

// --- What-Is / Network-Number-Is non-routed restrictions ---

#[tokio::test]
async fn what_is_guards_and_reply() {
    let mut h = Harness::two_port();

    // SNET, DNET, or defined-data payloads are ignored, never answered.
    let mut addressed = control_npdu(NetworkMessageType::WHAT_IS_NETWORK_NUMBER, &[]);
    addressed.source = Some(NpduAddress {
        network: 900,
        mac_address: MacAddr::from_slice(&[9]),
    });
    h.handle(h.ctx(0, &[7], addressed)).await;
    let mut directed = control_npdu(NetworkMessageType::WHAT_IS_NETWORK_NUMBER, &[]);
    directed.destination = Some(NpduAddress {
        network: 1000,
        mac_address: MacAddr::new(),
    });
    h.handle(h.ctx(0, &[7], directed)).await;
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::WHAT_IS_NETWORK_NUMBER, &[0]),
    ))
    .await;
    h.assert_quiet();

    // Valid local query: local-broadcast Number-Is on the ingress port.
    let mut ctx = h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::WHAT_IS_NETWORK_NUMBER, &[]),
    );
    ctx.data_attributes = attributes();
    h.handle(ctx).await;
    assert!(h.drain(1).is_empty());
    let mut replies = h.drain(0);
    assert_eq!(replies.len(), 1);
    match replies.pop().unwrap() {
        SendRequest::Broadcast {
            npdu,
            data_attributes,
        } => {
            let decoded = decode_npdu(npdu).unwrap();
            assert_eq!(
                decoded.message_type,
                Some(NetworkMessageType::NETWORK_NUMBER_IS.to_raw())
            );
            assert_eq!(decoded.payload.as_ref(), &[0x03, 0xe8, 1]);
            assert_eq!(data_attributes, attributes());
        }
        SendRequest::Unicast { .. } => panic!("expected broadcast Number-Is"),
    }
}

#[tokio::test]
async fn number_is_non_routed_restrictions() {
    let mut h = Harness::two_port();

    // SNET/SADR, DNET/DADR, link-unicast, or short payloads are ignored.
    let mut with_source = control_npdu(NetworkMessageType::NETWORK_NUMBER_IS, &[0x07, 0xd0, 1]);
    with_source.source = Some(NpduAddress {
        network: 900,
        mac_address: MacAddr::from_slice(&[9]),
    });
    h.handle(h.ctx(0, &[7], with_source)).await;
    let mut with_dest = control_npdu(NetworkMessageType::NETWORK_NUMBER_IS, &[0x07, 0xd0, 1]);
    with_dest.destination = Some(NpduAddress {
        network: 1000,
        mac_address: MacAddr::new(),
    });
    h.handle(h.ctx(0, &[7], with_dest)).await;
    let mut unicast = h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::NETWORK_NUMBER_IS, &[0x07, 0xd0, 1]),
    );
    unicast.link_layer_group = false;
    h.handle(unicast).await;
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::NETWORK_NUMBER_IS, &[0x07, 0xd0]),
    ))
    .await;
    h.assert_quiet();

    // Valid link-broadcast Number-Is is processed without emitting traffic.
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::NETWORK_NUMBER_IS, &[0x07, 0xd0, 1]),
    ))
    .await;
    h.assert_quiet();
}

#[tokio::test]
async fn what_is_and_number_is_directed_are_never_routed() {
    let mut h = Harness::two_port();

    // Even with a reachable remote DNET, these stay local (and are ignored
    // there by the address guard): never forwarded, never rejected.
    for kind in [
        NetworkMessageType::WHAT_IS_NETWORK_NUMBER,
        NetworkMessageType::NETWORK_NUMBER_IS,
    ] {
        let mut npdu = control_npdu(kind, &[]);
        npdu.destination = Some(NpduAddress {
            network: 2000,
            mac_address: MacAddr::new(),
        });
        npdu.hop_count = 255;
        h.dispatch(h.ctx(0, &[7], npdu)).await;
        h.assert_quiet();
    }
    assert_eq!(h.table.lock().await.len(), 2);
}

// --- Proprietary opacity (Clauses 6.2.4 / 6.5.4) ---

#[tokio::test]
async fn proprietary_unicast_forward_preserves_identity() {
    let (tx0, _rx0) = mpsc::channel::<SendRequest>(16);
    let (tx1, mut rx1) = mpsc::channel::<SendRequest>(16);
    let send_txs = vec![tx0, tx1];

    let mut npdu = Npdu {
        is_network_message: true,
        message_type: Some(0x80),
        vendor_id: Some(1234),
        destination: Some(NpduAddress {
            network: 3000,
            mac_address: MacAddr::from_slice(&[9]),
        }),
        hop_count: 255,
        payload: Bytes::from_static(&[1, 2, 3]),
        ..Npdu::default()
    };

    // Remote route: destination, identity, and attributes all preserved.
    let route = crate::router_table::RouteEntry {
        port_index: 1,
        directly_connected: false,
        next_hop_mac: MacAddr::from_slice(&[5]),
        last_seen: None,
        reachability: ReachabilityStatus::Reachable,
        busy_until: None,
        flap_count: 0,
        last_port_change: None,
    };
    forward_unicast(
        &send_txs,
        &route,
        1000,
        &[7],
        npdu.clone(),
        0,
        &attributes(),
    );
    match rx1.try_recv().unwrap() {
        SendRequest::Unicast {
            npdu,
            mac,
            data_attributes,
        } => {
            assert_eq!(mac.as_slice(), &[5]);
            let decoded = decode_npdu(npdu).unwrap();
            assert_eq!(decoded.message_type, Some(0x80));
            assert_eq!(decoded.vendor_id, Some(1234));
            assert_eq!(
                decoded.destination,
                Some(NpduAddress {
                    network: 3000,
                    mac_address: MacAddr::from_slice(&[9]),
                })
            );
            assert_eq!(decoded.hop_count, 254);
            assert_eq!(data_attributes, attributes());
        }
        SendRequest::Broadcast { .. } => panic!("expected unicast"),
    }

    // Direct route: DNET stripped per 6.5.4, identity still preserved.
    npdu.destination = Some(NpduAddress {
        network: 2000,
        mac_address: MacAddr::from_slice(&[9]),
    });
    let route = crate::router_table::RouteEntry {
        port_index: 1,
        directly_connected: true,
        next_hop_mac: MacAddr::new(),
        last_seen: None,
        reachability: ReachabilityStatus::Reachable,
        busy_until: None,
        flap_count: 0,
        last_port_change: None,
    };
    forward_unicast(&send_txs, &route, 1000, &[7], npdu, 0, &[]);
    match rx1.try_recv().unwrap() {
        SendRequest::Unicast { npdu, mac, .. } => {
            assert_eq!(mac.as_slice(), &[9]);
            let decoded = decode_npdu(npdu).unwrap();
            assert!(decoded.destination.is_none());
            assert_eq!(decoded.message_type, Some(0x80));
            assert_eq!(decoded.vendor_id, Some(1234));
        }
        SendRequest::Broadcast { .. } => panic!("expected unicast"),
    }
}

#[tokio::test]
async fn proprietary_directed_is_routed_opaquely_not_consumed() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_learned(3000, 1, MacAddr::from_slice(&[5]));
    let mut h = Harness::with_table(table);

    let mut npdu = Npdu {
        is_network_message: true,
        message_type: Some(0x90),
        vendor_id: Some(999),
        destination: Some(NpduAddress {
            network: 3000,
            mac_address: MacAddr::from_slice(&[9]),
        }),
        hop_count: 255,
        payload: Bytes::from_static(&[0xde, 0xad]),
        ..Npdu::default()
    };
    npdu.source = Some(NpduAddress {
        network: 1000,
        mac_address: MacAddr::from_slice(&[7]),
    });
    h.dispatch(h.ctx(0, &[7], npdu)).await;

    assert!(h.drain(0).is_empty());
    let mut forwarded = h.drain(1);
    assert_eq!(forwarded.len(), 1);
    match forwarded.pop().unwrap() {
        SendRequest::Unicast { npdu, .. } => {
            let decoded = decode_npdu(npdu).unwrap();
            assert_eq!(decoded.message_type, Some(0x90));
            assert_eq!(decoded.vendor_id, Some(999));
            assert_eq!(decoded.payload.as_ref(), &[0xde, 0xad]);
        }
        SendRequest::Broadcast { .. } => panic!("expected unicast"),
    }
}

#[tokio::test]
async fn unknown_local_control_is_explicitly_rejected() {
    let mut h = Harness::two_port();
    let mut ctx = h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::from_raw(0x14), &[]),
    );
    ctx.data_attributes = attributes();
    h.handle(ctx).await;

    assert!(h.drain(1).is_empty());
    let mut rejects = h.drain(0);
    assert_eq!(rejects.len(), 1);
    match rejects.pop().unwrap() {
        SendRequest::Unicast {
            npdu,
            mac,
            data_attributes,
        } => {
            assert_eq!(mac.as_slice(), &[7]);
            let decoded = decode_npdu(npdu).unwrap();
            assert_eq!(
                decoded.payload.as_ref(),
                &[RejectMessageReason::UNKNOWN_MESSAGE_TYPE.to_raw(), 0, 0]
            );
            assert_eq!(data_attributes, attributes());
        }
        SendRequest::Broadcast { .. } => panic!("expected unicast reject"),
    }
}
