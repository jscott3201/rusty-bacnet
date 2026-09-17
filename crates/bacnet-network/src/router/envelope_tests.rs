//! RB-03 focused tests: router control envelopes and discovery origin.
//!
//! Clause map (ASHRAE 135-2020, supplied PDF): 6.2 (NPCI/envelope rules),
//! 6.4 (per-message data formats), 6.5.4 (remote-traffic relay: retain routed
//! SNET/SADR, else add ingress net + SA; forward data attributes), 6.6.3.2
//! (Who-Is discovery relay), 6.4.14/6.4.15 + 6.6.3.12 (What-Is/Number-Is
//! non-routed restrictions).
//!
//! In-memory ports only: no sockets, no timing. `Harness::handle` exercises
//! the local-control admission point; `Harness::dispatch` exercises the
//! routing-first dispatcher.

use super::*;
use bacnet_encoding::npdu::NpduAddress;

fn control_npdu(message_type: NetworkMessageType, payload: &[u8]) -> Npdu {
    Npdu {
        is_network_message: true,
        message_type: Some(message_type.to_raw()),
        payload: Bytes::copy_from_slice(payload),
        ..Npdu::default()
    }
}

fn attributes() -> Vec<DataAttribute> {
    vec![DataAttribute {
        option_type: 31,
        must_understand: false,
        data: vec![0x12, 0x34],
    }]
}

struct Harness {
    table: Arc<Mutex<RouterTable>>,
    txs: Vec<mpsc::Sender<SendRequest>>,
    rxs: Vec<mpsc::Receiver<SendRequest>>,
}

impl Harness {
    fn two_port() -> Self {
        let mut table = RouterTable::new();
        table.add_direct(1000, 0);
        table.add_direct(2000, 1);
        Self::with_table(table)
    }

    fn with_table(table: RouterTable) -> Self {
        let (tx0, rx0) = mpsc::channel(16);
        let (tx1, rx1) = mpsc::channel(16);
        Self {
            table: Arc::new(Mutex::new(table)),
            txs: vec![tx0, tx1],
            rxs: vec![rx0, rx1],
        }
    }

    fn ctx(&self, port: usize, source_mac: &[u8], npdu: Npdu) -> IngressContext {
        let _ = &self;
        IngressContext {
            port_idx: port,
            port_network: if port == 0 { 1000 } else { 2000 },
            source_mac: MacAddr::from_slice(source_mac),
            link_layer_group: true,
            data_attributes: Vec::new(),
            npdu,
        }
    }

    async fn handle(&mut self, ctx: IngressContext) {
        handle_network_message(&self.table, &self.txs, &ctx).await;
    }

    async fn dispatch(&mut self, ctx: IngressContext) {
        dispatch_network_message(&self.table, &self.txs, &ctx).await;
    }

    fn drain(&mut self, port: usize) -> Vec<SendRequest> {
        let mut out = Vec::new();
        while let Ok(req) = self.rxs[port].try_recv() {
            out.push(req);
        }
        out
    }

    fn assert_quiet(&mut self) {
        for port in 0..self.rxs.len() {
            assert!(
                self.rxs[port].try_recv().is_err(),
                "expected no output on port {port}"
            );
        }
    }
}

fn broadcast_data(requests: Vec<SendRequest>) -> Vec<Bytes> {
    requests
        .into_iter()
        .map(|req| match req {
            SendRequest::Broadcast { npdu, .. } => npdu,
            SendRequest::Unicast { .. } => panic!("expected broadcast"),
        })
        .collect()
}

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

    // Valid update: learns 3000, ACK without table data routed to the origin.
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
            assert_eq!(decoded.payload.as_ref(), &[0]);
        }
        SendRequest::Broadcast { .. } => panic!("expected unicast ACK"),
    }
    assert_eq!(h.table.lock().await.lookup(3000).unwrap().port_index, 1);

    // Valid query: ACK carries the full table, now including 3000.
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
            assert_eq!(decoded.payload[0], 3);
            assert_eq!(decoded.payload.len(), 13);
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

    assert!(h.drain(1).is_empty());
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
