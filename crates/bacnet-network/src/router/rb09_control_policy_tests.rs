//! RB-09 routing-control admission: compat/permissive default + hardened opt-in.
//!
//! In-memory, deterministic, no sleeps. Uses [`super::envelope_harness`] and
//! `ctx_with_provenance`. Deny is silent: no mutation, relay, ACK, or Reject.

use super::control_policy::{ControlAuthorizer, ControlGate, ControlPolicy};
use super::envelope_harness::*;
use super::*;
use bacnet_encoding::npdu::NpduAddress;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn hardened() -> Arc<ControlGate> {
    Arc::new(ControlGate::hardened())
}

fn hardened_with(auth: ControlAuthorizer) -> Arc<ControlGate> {
    Arc::new(ControlGate::new(ControlPolicy::Hardened, Some(auth)))
}

fn permissive_with(auth: ControlAuthorizer) -> Arc<ControlGate> {
    Arc::new(ControlGate::new(ControlPolicy::Permissive, Some(auth)))
}

fn allow_all() -> ControlAuthorizer {
    Arc::new(|_| true)
}

#[tokio::test]
async fn permissive_default_matches_today() {
    let mut h = Harness::two_port();
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0x0b, 0xb8]),
    ))
    .await;
    assert!(h.table.lock().await.lookup(3000).is_some());
    assert_eq!(broadcast_data(h.drain(0)).len(), 1);
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[1, 0x0f, 0xa0, 2, 0],
        ),
    ))
    .await;
    assert_eq!(h.table.lock().await.lookup(4000).unwrap().port_index, 1);
    assert_eq!(h.drain(0).len(), 1);
    assert_eq!(h.gate.snapshot().i_am.allow_total, 1);
    assert_eq!(h.gate.snapshot().init_mgmt.allow_total, 1);
}

#[tokio::test]
async fn hardened_denies_protected_silently() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_learned(3000, 1, MacAddr::from_slice(&[2]));
    let mut h = Harness::with_gate(table, hardened());
    // I-Am learning denied: no mutation, no rebroadcast.
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0x0b, 0xb9]),
    ))
    .await;
    h.assert_quiet();
    assert!(h.table.lock().await.lookup(3001).is_none());
    // Init update + purge denied: no mutation, no empty ACK.
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[1, 0x0f, 0xa0, 2, 0],
        ),
    ))
    .await;
    h.assert_quiet();
    assert!(h.table.lock().await.lookup(4000).is_none());
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[1, 0x0b, 0xb8, 0, 0],
        ),
    ))
    .await;
    h.assert_quiet();
    assert!(h.table.lock().await.lookup(3000).is_some());
    // Init-ACK learning denied.
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
            &[1, 0x0f, 0xa0, 1, 0],
        ),
    ))
    .await;
    h.assert_quiet();
    // Busy/Available (listed + omitted) denied: no marks, no rebroadcast.
    for (kind, payload) in [
        (NetworkMessageType::ROUTER_BUSY_TO_NETWORK, vec![0x0b, 0xb8]),
        (NetworkMessageType::ROUTER_BUSY_TO_NETWORK, vec![]),
        (
            NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK,
            vec![0x0b, 0xb8],
        ),
        (NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK, vec![]),
    ] {
        h.handle(h.ctx(1, &[2], control_npdu(kind, &payload))).await;
        h.assert_quiet();
    }
    assert_eq!(
        h.table.lock().await.effective_reachability(3000),
        Some(ReachabilityStatus::Reachable)
    );
    // Reject transition denied: no table change, no relay.
    let mut npdu = control_npdu(
        NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
        &[1, 0x0b, 0xb8],
    );
    npdu.source = Some(NpduAddress {
        network: 2000,
        mac_address: MacAddr::from_slice(&[7]),
    });
    h.handle(h.ctx(1, &[2], npdu)).await;
    h.assert_quiet();
    assert_eq!(
        h.table.lock().await.effective_reachability(3000),
        Some(ReachabilityStatus::Reachable)
    );
    // I-Could-Be denied: no insert.
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(
            NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK,
            &[0x13, 0x88, 50],
        ),
    ))
    .await;
    h.assert_quiet();
    assert!(h.table.lock().await.lookup(5000).is_none());
    let snap = h.gate.snapshot();
    assert_eq!(snap.i_am.deny_total, 1);
    assert_eq!(snap.i_am.policy_deny_total, 1);
    assert_eq!(snap.init_mgmt.deny_total, 2);
    assert_eq!(snap.init_ack.deny_total, 1);
    assert_eq!(snap.busy.deny_total, 2);
    assert_eq!(snap.available.deny_total, 2);
    assert_eq!(snap.reject.deny_total, 1);
    assert_eq!(snap.i_could_be.deny_total, 1);
}

#[tokio::test]
async fn hardened_allows_read_only_discovery() {
    let mut h = Harness::with_gate(
        {
            let mut t = RouterTable::new();
            t.add_direct(1000, 0);
            t.add_direct(2000, 1);
            t.add_learned(3000, 1, MacAddr::from_slice(&[2]));
            t
        },
        hardened(),
    );
    // Who-Is answer still works for unknown authority.
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x0b, 0xb8]),
    ))
    .await;
    assert_eq!(broadcast_data(h.drain(0)).len(), 1);
    // Init query still answered, table untouched.
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &[0]),
    ))
    .await;
    assert_eq!(h.drain(0).len(), 1);
    assert_eq!(h.table.lock().await.len(), 3);
    // What-Is answered; Number-Is quiet; Establish/Disconnect no-ops.
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::WHAT_IS_NETWORK_NUMBER, &[]),
    ))
    .await;
    assert_eq!(h.drain(0).len(), 1);
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::NETWORK_NUMBER_IS, &[0x03, 0xe8, 1]),
    ))
    .await;
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(
            NetworkMessageType::ESTABLISH_CONNECTION_TO_NETWORK,
            &[0x0b, 0xb8, 30],
        ),
    ))
    .await;
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(
            NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK,
            &[0x0b, 0xb8],
        ),
    ))
    .await;
    h.assert_quiet();
    // Security ack-only: quiet. Proprietary: explicit reject (allowed).
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::from_raw(0x0B), &[]),
    ))
    .await;
    h.assert_quiet();
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::from_raw(0x90), &[1]),
    ))
    .await;
    assert_eq!(h.drain(0).len(), 1);
    // Directed opaque forwarding needs no policy: still forwards.
    let mut npdu = control_npdu(NetworkMessageType::from_raw(0x90), &[0xde, 0xad]);
    npdu.destination = Some(NpduAddress {
        network: 3000,
        mac_address: MacAddr::from_slice(&[9]),
    });
    npdu.hop_count = 255;
    h.dispatch(h.ctx(0, &[7], npdu)).await;
    assert!(h.drain(0).is_empty());
    assert_eq!(h.drain(1).len(), 1);
}

#[tokio::test]
async fn hardened_authorized_update_across_two_nets() {
    let auth: ControlAuthorizer =
        Arc::new(|ctx| ctx.target_nets.iter().all(|n| *n == 3000 || *n == 4000));
    let mut h = Harness::with_gate(
        {
            let mut t = RouterTable::new();
            t.add_direct(1000, 0);
            t.add_direct(2000, 1);
            t
        },
        hardened_with(auth),
    );
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0x0b, 0xb8]),
    ))
    .await;
    assert!(h.table.lock().await.lookup(3000).is_some());
    assert_eq!(broadcast_data(h.drain(0)).len(), 1);
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[1, 0x0f, 0xa0, 2, 0],
        ),
    ))
    .await;
    assert!(h.table.lock().await.lookup(4000).is_some());
    assert_eq!(h.drain(0).len(), 1);
    // Outside the two controlled nets: denied silently.
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0x13, 0x88]),
    ))
    .await;
    h.assert_quiet();
    assert!(h.table.lock().await.lookup(5000).is_none());
    let snap = h.gate.snapshot();
    assert_eq!(snap.i_am.allow_total, 1);
    assert_eq!(snap.init_mgmt.allow_total, 1);
    assert_eq!(snap.i_am.deny_total, 1);
}

#[tokio::test]
async fn authorizer_deny_and_panic_fail_closed() {
    for (name, gate) in [
        ("refuse", permissive_with(Arc::new(|_| false))),
        (
            "panic",
            permissive_with(Arc::new(|_| panic!("policy failure"))),
        ),
        ("hardened-refuse", hardened_with(Arc::new(|_| false))),
    ] {
        let mut h = Harness::with_gate(
            {
                let mut t = RouterTable::new();
                t.add_direct(1000, 0);
                t.add_direct(2000, 1);
                t
            },
            gate,
        );
        h.handle(h.ctx(
            0,
            &[9],
            control_npdu(
                NetworkMessageType::INITIALIZE_ROUTING_TABLE,
                &[1, 0x0b, 0xb8, 2, 0],
            ),
        ))
        .await;
        h.assert_quiet();
        assert!(h.table.lock().await.lookup(3000).is_none(), "{name}");
        let snap = h.gate.snapshot();
        assert_eq!(snap.init_mgmt.deny_total, 1, "{name}");
        assert_eq!(snap.init_mgmt.allow_total, 0, "{name}");
    }
    // Hardened absent-authorizer denies are policy-denies, not authorizer denials.
    let mut h = Harness::with_gate(RouterTable::new(), hardened());
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0x0b, 0xb8]),
    ))
    .await;
    let snap = h.gate.snapshot();
    assert_eq!(snap.i_am.policy_deny_total, 1);
}

#[tokio::test]
async fn init_query_answered_in_hardened_full_queue_bounded() {
    let mut h = Harness::with_gate(
        {
            let mut t = RouterTable::new();
            t.add_direct(1000, 0);
            t.add_direct(2000, 1);
            t
        },
        hardened_with(allow_all()),
    );
    for _ in 0..16 {
        h.txs[0]
            .try_send(SendRequest::Broadcast {
                npdu: bytes::Bytes::new(),
                data_attributes: Vec::new(),
            })
            .unwrap();
    }
    // Allowed update still applies; ACK drops bounded on a full queue.
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[1, 0x0b, 0xb8, 2, 0],
        ),
    ))
    .await;
    assert!(h.table.lock().await.lookup(3000).is_some());
    assert_eq!(h.drain(0).len(), 16);
    // Denied control on a full queue stays quiet without panic.
    let mut denied = Harness::with_gate(
        {
            let mut t = RouterTable::new();
            t.add_direct(1000, 0);
            t.add_direct(2000, 1);
            t
        },
        hardened(),
    );
    for _ in 0..16 {
        denied.txs[0]
            .try_send(SendRequest::Broadcast {
                npdu: bytes::Bytes::new(),
                data_attributes: Vec::new(),
            })
            .unwrap();
    }
    denied
        .handle(denied.ctx(
            0,
            &[9],
            control_npdu(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0x0b, 0xb8]),
        ))
        .await;
    assert!(denied.table.lock().await.lookup(3000).is_none());
    assert_eq!(denied.drain(0).len(), 16);
}

#[tokio::test]
async fn standard_vs_hardened_matrix() {
    for (permissive, expect_learn) in [(false, false), (true, true)] {
        let gate = if permissive {
            Arc::new(ControlGate::permissive())
        } else {
            hardened()
        };
        let mut h = Harness::with_gate(
            {
                let mut t = RouterTable::new();
                t.add_direct(1000, 0);
                t.add_direct(2000, 1);
                t
            },
            gate,
        );
        h.handle(h.ctx(
            1,
            &[2],
            control_npdu(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0x0b, 0xb8]),
        ))
        .await;
        assert_eq!(
            h.table.lock().await.lookup(3000).is_some(),
            expect_learn,
            "permissive={permissive}"
        );
        if expect_learn {
            assert_eq!(broadcast_data(h.drain(0)).len(), 1);
        } else {
            h.assert_quiet();
        }
    }
}

#[tokio::test]
async fn convergence_recovers_after_deny_and_busy_expiry() {
    let auth: ControlAuthorizer = Arc::new(|ctx| ctx.port_idx == 1);
    let mut h = Harness::with_gate(
        {
            let mut t = RouterTable::new();
            t.add_direct(1000, 0);
            t.add_direct(2000, 1);
            t.add_learned(3000, 1, MacAddr::from_slice(&[2]));
            t
        },
        hardened_with(auth),
    );
    // Unknown port denied: route stays Reachable (APDU path unaffected).
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(NetworkMessageType::ROUTER_BUSY_TO_NETWORK, &[0x0b, 0xb8]),
    ))
    .await;
    h.assert_quiet();
    assert_eq!(
        h.table.lock().await.effective_reachability(3000),
        Some(ReachabilityStatus::Reachable)
    );
    // Authorized Busy marks via the announcing peer.
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(NetworkMessageType::ROUTER_BUSY_TO_NETWORK, &[0x0b, 0xb8]),
    ))
    .await;
    assert_eq!(broadcast_data(h.drain(0)).len(), 1);
    assert_eq!(
        h.table.lock().await.effective_reachability(3000),
        Some(ReachabilityStatus::Busy)
    );
    // Busy expiry via table-seeded past deadline (no sleeps): reachable again.
    {
        let mut tbl = h.table.lock().await;
        tbl.mark_busy(3000, Instant::now() - Duration::from_secs(1));
        tbl.clear_expired_busy();
    }
    assert_eq!(
        h.table.lock().await.effective_reachability(3000),
        Some(ReachabilityStatus::Reachable)
    );
    // Fresh authorized learning converges after the clear.
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0x0b, 0xb8]),
    ))
    .await;
    assert!(h.table.lock().await.lookup(3000).is_some());
}

#[tokio::test]
async fn reject_deny_sends_no_success_and_no_loop() {
    let mut h = Harness::with_gate(
        {
            let mut t = RouterTable::new();
            t.add_learned(3000, 0, MacAddr::from_slice(&[1]));
            t.add_direct(4000, 1);
            t
        },
        hardened(),
    );
    let mut npdu = control_npdu(
        NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
        &[1, 0x0b, 0xb8],
    );
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
    // Authorized reject applies once and relays once: no invented success,
    // no response loop (exactly one relay, no second reject).
    let mut allowed = Harness::with_gate(
        {
            let mut t = RouterTable::new();
            t.add_learned(3000, 0, MacAddr::from_slice(&[1]));
            t.add_direct(4000, 1);
            t
        },
        hardened_with(allow_all()),
    );
    let mut npdu = control_npdu(
        NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
        &[1, 0x0b, 0xb8],
    );
    npdu.source = Some(NpduAddress {
        network: 4000,
        mac_address: MacAddr::from_slice(&[7]),
    });
    allowed.handle(allowed.ctx(0, &[1], npdu)).await;
    assert_eq!(
        allowed.table.lock().await.effective_reachability(3000),
        Some(ReachabilityStatus::Unreachable)
    );
    assert_eq!(allowed.drain(1).len(), 1);
    assert!(allowed.drain(0).is_empty());
    assert!(allowed.drain(1).is_empty());
}

#[tokio::test]
async fn authorizer_sees_only_protected_and_replaced_peer_differs() {
    let calls = Arc::new(AtomicUsize::new(0));
    let auth: ControlAuthorizer = {
        let calls = Arc::clone(&calls);
        Arc::new(move |ctx: &ControlAuthContext| {
            calls.fetch_add(1, Ordering::Relaxed);
            ctx.source_mac.as_slice() == [2] && ctx.port_idx == 1
        })
    };
    let mut h = Harness::with_gate(
        {
            let mut t = RouterTable::new();
            t.add_direct(1000, 0);
            t.add_direct(2000, 1);
            t
        },
        hardened_with(auth),
    );
    // Read-only never consults the callback.
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[]),
    ))
    .await;
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &[0]),
    ))
    .await;
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    // Peer [2] allowed, replaced peer [3] denied for the same net.
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0x0b, 0xb8]),
    ))
    .await;
    assert!(h.table.lock().await.lookup(3000).is_some());
    h.handle(h.ctx(
        1,
        &[3],
        control_npdu(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0x0b, 0xb9]),
    ))
    .await;
    assert!(h.table.lock().await.lookup(3001).is_none());
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[test]
fn control_context_debug_is_redacted() {
    use bacnet_transport::port::TransportProvenance;
    let gate = ControlGate::hardened();
    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw()),
        vendor_id: Some(999),
        source: Some(NpduAddress {
            network: 900,
            mac_address: MacAddr::from_slice(&[0xAA, 0xBB]),
        }),
        payload: bytes::Bytes::from_static(&[0x0b, 0xb8, 0xde, 0xad]),
        ..Npdu::default()
    };
    let ingress = IngressContext::test_local_with_provenance(
        1,
        2000,
        &[0xAA, 0xBB, 0xCC],
        npdu,
        TransportProvenance::unverified(),
    );
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let auth: ControlAuthorizer = {
        let seen = Arc::clone(&seen);
        Arc::new(move |ctx: &ControlAuthContext| {
            seen.lock().unwrap().push(format!("{:?}", ctx));
            false
        })
    };
    let gate = ControlGate::new(ControlPolicy::Hardened, Some(auth));
    assert!(!gate.authorize(
        &ingress,
        super::control_policy::ControlClass::IAm,
        &[3000],
        false
    ));
    let rendered = seen.lock().unwrap().pop().unwrap();
    assert!(rendered.contains("3000"));
    assert!(rendered.contains("unverified"));
    assert!(!rendered.contains("AA"));
    assert!(!rendered.contains("DE"));
    assert!(!rendered.contains("ad"));
    let _ = gate;
}
