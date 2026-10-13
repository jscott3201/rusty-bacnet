//! Routing-claim counting and dampening (RB-06): rejects and their
//! hold-down, learning at the route cap, management writes and malformed
//! tails, on the router's engine through [`super::envelope_harness`]. The
//! harness router has direct routes to 1000 on port 0 and 2000 on port 1.

use bacnet_encoding::npdu::{decode_npdu, Npdu, NpduAddress};
use bacnet_types::enums::NetworkMessageType;
use bacnet_types::MacAddr;
use bytes::Bytes;

use super::envelope_harness::*;
use super::SendRequest;
use crate::router_table::{ClaimCounters, PortId, Reachability};

// Invoke the real dispatch with in-memory port queues; no sleeps, sockets or
// background aging.
async fn deliver(
    h: &mut Harness,
    port: usize,
    source_mac: &[u8],
    message_type: NetworkMessageType,
    payload: &[u8],
) -> [Vec<SendRequest>; 2] {
    let ctx = h.ctx(port, source_mac, control_npdu(message_type, payload));
    h.handle(ctx).await;
    [h.drain(0), h.drain(1)]
}

async fn reject(h: &mut Harness, port: usize, source_mac: &[u8], reason: u8) {
    let output = deliver(
        h,
        port,
        source_mac,
        NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
        &[reason, 0x0b, 0xb8],
    )
    .await;
    assert!(output.iter().all(Vec::is_empty));
}

fn reachability(h: &Harness, network: u16) -> Option<Reachability> {
    h.table().effective_reachability(network, h.now())
}

/// Reasons 1 and 2 apply once and then count as changing nothing; any
/// other reason never changes the route (R2, Clause 6.6.3.5), where the
/// router before #1679's fix removed it.
#[tokio::test]
async fn first_rejects_apply_once_and_other_reasons_never() {
    for reason in [0, 1, 2, 4] {
        let mut h = Harness::two_port();
        h.learned(3000, 0, &[1]);
        for _ in 0..12 {
            reject(&mut h, 0, &[1], reason).await;
        }
        let (state, applied) = match reason {
            1 => (Reachability::Unreachable, 1),
            2 => (Reachability::Busy, 1),
            _ => (Reachability::Reachable, 0),
        };
        assert_eq!(reachability(&h, 3000), Some(state));
        // Reasons 1 and 2 apply once, then change nothing; the others are
        // kept every time.
        let (dampened, kept) = if applied == 1 { (11, 0) } else { (0, 12) };
        assert_eq!(
            h.table().claim_counters(),
            claim_counts(|c| {
                c.reject_applied = applied;
                c.reject_dampened = dampened;
                c.reject_dampened_same_state = dampened;
                c.reject_kept = kept;
            })
        );
    }
}

#[tokio::test]
async fn spoofing_mac_does_not_bypass_hold_down_but_another_ingress_is_independent() {
    let mut h = Harness::two_port();
    h.learned(3000, 0, &[1]);
    reject(&mut h, 0, &[1], 2).await;
    for mac in 1..=10 {
        reject(&mut h, 0, &[mac], 1).await;
    }
    assert_eq!(reachability(&h, 3000), Some(Reachability::Busy));
    reject(&mut h, 1, &[1], 1).await;
    assert_eq!(reachability(&h, 3000), Some(Reachability::Unreachable));
    assert_eq!(
        h.table().claim_counters(),
        claim_counts(|c| {
            c.reject_applied = 2;
            c.reject_dampened = 10;
            c.reject_dampened_hold_down = 10;
        })
    );
}

#[tokio::test]
async fn i_am_refresh_rearms_rejects_and_corroborated_learning_still_warns() {
    // Hardened gate: cross-port moves need two claims; same-port refreshes
    // re-arm reject hold-down immediately in both modes.
    let mut h = Harness::two_port().hardened();
    for (index, port) in [0, 0, 1, 0, 1, 0].into_iter().enumerate() {
        if index >= 2 {
            deliver(
                &mut h,
                port,
                &[index as u8],
                NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
                &[0x0b, 0xb8],
            )
            .await;
        }
        let output = deliver(
            &mut h,
            port,
            &[index as u8],
            NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
            &[0x0b, 0xb8],
        )
        .await;
        assert!(output[port].is_empty());
        assert_eq!(output[1 - port].len(), 1); // Existing unconditional rebroadcast.
        {
            let table = h.table();
            let route = table.lookup(3000).unwrap();
            assert_eq!(route.port(), PortId(port as u8));
            assert_eq!(route.next_hop().as_slice(), &[index as u8]);
            assert_eq!(route.reachability(), Reachability::Reachable);
        }
        reject(&mut h, 0, &[1], 1).await;
        assert_eq!(reachability(&h, 3000), Some(Reachability::Unreachable));
    }
    let table = h.table();
    assert_eq!(table.lookup(3000).unwrap().flap_count(), 4);
    assert_eq!(
        table.claim_counters(),
        claim_counts(|c| {
            c.learned_ok = 6;
            c.reject_applied = 6;
            c.flap_warned = 2;
            c.pending_started = 4;
            c.corroborated_applied = 4;
        })
    );
}

#[tokio::test]
async fn truncated_tails_change_nothing_and_send_no_ack() {
    // RB-03 correction (Clauses 6.2/6.4/6.6.3.2): complete-payload validation
    // precedes every table-mutation path. Previously a trailing octet was
    // silently ignored — prefix entries were applied and the rebroadcast
    // (I-Am) or ACK (Init) was still emitted. Now the whole message is
    // dropped: no table change, no forward, no rebroadcast, no ACK.
    let mut h = Harness::two_port();

    // I-Am with a 1-octet tail: 3000 must not be learned, nothing rebroadcast.
    let output = deliver(
        &mut h,
        0,
        &[1],
        NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
        &[0, 0, 0xff, 0xff, 0x03, 0xe8, 0x0b, 0xb8, 0],
    )
    .await;
    assert!(output.iter().all(Vec::is_empty));
    assert_eq!(h.table().len(), 2);

    // Init with a truncated tail is rejected whole: nothing applied, no ACK.
    let init = [
        6, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 0x03, 0xe8, 0, 0, 0x0b, 0xb8, 0, 0, 0x0f, 0xa0, 0, 0, 0x13,
    ];
    let output = deliver(
        &mut h,
        1,
        &[2],
        NetworkMessageType::INITIALIZE_ROUTING_TABLE,
        &init,
    )
    .await;
    assert!(output.iter().all(Vec::is_empty)); // No ACK for an invalid envelope.
    assert_eq!(h.table().len(), 2);

    // The same truncation in an ACK still learns nothing.
    deliver(
        &mut h,
        1,
        &[2],
        NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
        &init,
    )
    .await;
    let table = h.table();
    assert_eq!(table.len(), 2);
    assert!(table.lookup(1000).unwrap().is_direct());
    assert!(table.lookup(3000).is_none());
    assert!(table.lookup(4000).is_none());
    assert_eq!(table.claim_counters(), ClaimCounters::default());
}

/// The router's two direct routes and 254 learned ones on port 0 fill the
/// table to its 256-route cap.
fn full(h: &Harness) {
    for net in 1..=254 {
        h.learned(net, 0, &[1]);
    }
    assert_eq!(h.table().len(), 256);
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
        let mut h = Harness::two_port().hardened();
        full(&h);
        // The unknown 3000 triggers the cap; the existing 1 behind it stays unexamined.
        let payload: &[u8] = if message_type == NetworkMessageType::I_AM_ROUTER_TO_NETWORK {
            &[0x0b, 0xb8, 0, 1]
        } else {
            &[2, 0x0b, 0xb8, 0, 0, 0, 1, 0, 0]
        };
        deliver(&mut h, 1, &[2], message_type, payload).await;
        {
            let table = h.table();
            assert_eq!(table.len(), 256);
            assert!(table.lookup(3000).is_none());
            assert_eq!(table.lookup(1).unwrap().port(), PortId(0));
            assert_eq!(
                table.claim_counters(),
                claim_counts(|c| c.learned_cap_ignored = 1)
            );
        }
        // Existing per-message cap behavior is preserved, including ACK's cap
        // before refresh.
        let existing: &[u8] = if message_type == NetworkMessageType::I_AM_ROUTER_TO_NETWORK {
            &[0, 1]
        } else {
            &[1, 0, 1, 0, 0]
        };
        deliver(&mut h, 1, &[2], message_type, existing).await;
        if message_type == NetworkMessageType::I_AM_ROUTER_TO_NETWORK {
            assert_eq!(h.table().lookup(1).unwrap().port(), PortId(0));
            deliver(&mut h, 1, &[2], message_type, existing).await;
        }
        let table = h.table();
        let refreshed = message_type == NetworkMessageType::I_AM_ROUTER_TO_NETWORK;
        assert_eq!(table.lookup(1).unwrap().port(), PortId(u8::from(refreshed)));
        assert_eq!(table.claim_counters().learned_ok, u64::from(refreshed));
        assert_eq!(
            table.claim_counters().learned_cap_ignored,
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
    let mut h = Harness::two_port();
    full(&h);
    // Two unknown networks via Port ID 2 (local port 1): the first trips the
    // cap and stops the message; the second stays unexamined.
    deliver(
        &mut h,
        1,
        &[2],
        NetworkMessageType::INITIALIZE_ROUTING_TABLE,
        &[2, 0x0b, 0xb8, 2, 0, 0x0b, 0xb9, 2, 0],
    )
    .await;
    {
        let table = h.table();
        assert_eq!(table.len(), 256);
        assert!(table.lookup(3000).is_none());
        assert!(table.lookup(3001).is_none());
        assert_eq!(
            table.claim_counters(),
            claim_counts(|c| c.learned_cap_ignored = 1)
        );
    }
    // Replacement of an existing learned entry bypasses the cap and rewrites
    // the port mapping immediately (no corroboration gate, no learned_ok:
    // management writes are not learning claims).
    deliver(
        &mut h,
        1,
        &[2],
        NetworkMessageType::INITIALIZE_ROUTING_TABLE,
        &[1, 0, 1, 2, 0],
    )
    .await;
    let table = h.table();
    let route = table.lookup(1).unwrap();
    assert_eq!(route.port(), PortId(1));
    assert!(!route.is_direct());
    assert_eq!(route.next_hop().as_slice(), &[2]);
    assert_eq!(
        table.claim_counters(),
        claim_counts(|c| c.learned_cap_ignored = 1)
    );
}

#[tokio::test]
async fn direct_route_immunity_and_malformed_rejects_under_spam() {
    let mut h = Harness::two_port();
    h.table().add_direct(3000, PortId(0));
    for _ in 0..10 {
        for reason in [0, 1, 2] {
            reject(&mut h, 1, &[1], reason).await;
        }
    }
    for payload in [&[][..], &[1][..], &[1, 0x0b][..]] {
        deliver(
            &mut h,
            0,
            &[1],
            NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
            payload,
        )
        .await;
    }
    assert_eq!(reachability(&h, 3000), Some(Reachability::Reachable));
    let table = h.table();
    assert!(table.lookup(3000).unwrap().is_direct());
    assert_eq!(
        table.claim_counters(),
        claim_counts(|c| {
            c.reject_dampened = 30;
            c.reject_dampened_same_state = 30;
        })
    );
}

#[tokio::test]
async fn init_replace_and_ack_refresh_both_rearm_reject_hold_down() {
    // RB-05: a management replace installs a fresh learned entry (clearing
    // reject records like fresh learning), so a later reject applies again;
    // the ACK same-port refresh re-arms the same way.
    let mut h = Harness::two_port();
    h.learned(3000, 0, &[1]);
    reject(&mut h, 0, &[1], 2).await;
    // Port ID 1 names local port 0: replaces 3000 in place, re-arming.
    deliver(
        &mut h,
        1,
        &[2],
        NetworkMessageType::INITIALIZE_ROUTING_TABLE,
        &[1, 0x0b, 0xb8, 1, 0],
    )
    .await;
    reject(&mut h, 0, &[1], 1).await;
    assert_eq!(reachability(&h, 3000), Some(Reachability::Unreachable));
    deliver(
        &mut h,
        0,
        &[2],
        NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
        &[1, 0x0b, 0xb8, 0, 0],
    )
    .await;
    reject(&mut h, 0, &[1], 2).await;
    assert_eq!(reachability(&h, 3000), Some(Reachability::Busy));
    let table = h.table();
    let route = table.lookup(3000).unwrap();
    assert_eq!(route.port(), PortId(0));
    assert_eq!(route.next_hop().as_slice(), &[2]);
    assert_eq!(
        table.claim_counters(),
        claim_counts(|c| {
            c.learned_ok = 1;
            c.reject_applied = 3;
        })
    );
}

#[tokio::test]
async fn dampened_table_transition_still_relays_every_reject() {
    let mut h = Harness::two_port();
    h.learned(3000, 0, &[1]);
    h.table().add_direct(4000, PortId(1));
    // The reject names the originator, 4000/[7], as its DNET/DADR (#1158).
    let origin = NpduAddress {
        network: 4000,
        mac_address: MacAddr::from_slice(&[7]),
    };
    for reason in [1, 1, 2] {
        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw()),
            destination: Some(origin.clone()),
            payload: Bytes::from(vec![reason, 0x0b, 0xb8]),
            ..Default::default()
        };
        h.handle(h.ctx(0, &[1], npdu.clone())).await;
        let SendRequest::Unicast {
            npdu: data, mac, ..
        } = h.rxs[1].try_recv().unwrap()
        else {
            panic!("expected original reject relay");
        };
        // 4000 is directly connected: the DADR gets the reject, and the
        // ingress network and link sender become its SNET/SADR.
        assert_eq!(mac, origin.mac_address);
        let relayed = decode_npdu(data).unwrap();
        assert!(relayed.destination.is_none());
        assert_eq!(relayed.payload, npdu.payload);
        assert_eq!(relayed.message_type, npdu.message_type);
        assert_eq!(
            relayed.source,
            Some(NpduAddress {
                network: 1000,
                mac_address: MacAddr::from_slice(&[1]),
            })
        );
        h.assert_quiet();
    }
    assert_eq!(
        h.table().claim_counters(),
        claim_counts(|c| {
            c.reject_applied = 1;
            c.reject_dampened = 2;
            // The trailing Busy claim on a permanently Unreachable route is
            // a same-state no-op (it can never resurrect via auto-clear), not
            // a held-down change — while every reject still relays above.
            c.reject_dampened_same_state = 2;
        })
    );
}
