//! Initialize-Routing-Table messages that carry an SNET, at the hosted
//! router's gate: a query from another network is a protected control
//! (#2715), allowed by the default gate, refused by a hardened one with no
//! authorizer, and an authorizer's to decide.
//!
//! In-memory and deterministic, on [`super::envelope_harness`].

use std::sync::{Arc, Mutex};

use bacnet_encoding::npdu::{Npdu, NpduAddress};
use bacnet_npdu::NetworkMessageType;
use bacnet_types::MacAddr;

use super::control_policy::{
    ControlAuthContext, ControlAuthorizer, ControlGate, ControlPolicy, ControlServiceCounters,
};
use super::envelope_harness::*;

/// The network a remote requester claims.
const REMOTE: u16 = 81;

/// A routing-table query, a port count of zero, as a node on this network
/// sends it.
fn local_query() -> Npdu {
    control_npdu(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &[0])
}

/// `npdu` as a router relays it for a node on network [`REMOTE`].
fn from_remote(mut npdu: Npdu) -> Npdu {
    npdu.source = Some(NpduAddress {
        network: REMOTE,
        mac_address: MacAddr::from_slice(&[1]),
    });
    npdu
}

/// Decision totals: allowed, denied, and denied by policy alone.
fn totals(allow: u64, deny: u64, policy: u64) -> ControlServiceCounters {
    ControlServiceCounters {
        allow_total: allow,
        deny_total: deny,
        policy_deny_total: policy,
    }
}

/// The default gate answers a query from another network, as before, and
/// now counts it; a local query is answered without the gate.
#[tokio::test]
async fn the_default_gate_answers_a_remote_table_query_and_counts_it() {
    let mut h = Harness::two_port();
    h.handle(h.ctx(0, &[7], from_remote(local_query()))).await;
    assert_eq!(h.drain(0).len(), 1, "one Ack, to the link peer");
    assert_eq!(h.gate.snapshot().init_query, totals(1, 0, 0));

    h.handle(h.ctx(0, &[7], local_query())).await;
    assert_eq!(h.drain(0).len(), 1);
    assert_eq!(h.gate.snapshot().init_query, totals(1, 0, 0));
    assert_eq!(h.counters().denied, 0);
}

/// A hardened gate with no authorizer refuses a query from another network
/// like any protected control, silently, and still answers a local one.
#[tokio::test]
async fn a_hardened_gate_answers_table_queries_from_its_own_networks_only() {
    let gate = Arc::new(ControlGate::hardened());
    let mut h = Harness::with_capacities(gate, &[16, 16]);
    h.handle(h.ctx(0, &[7], from_remote(local_query()))).await;
    h.assert_quiet();
    assert_eq!(h.gate.snapshot().init_query, totals(0, 1, 1));
    assert_eq!(h.counters().denied, 1);

    h.handle(h.ctx(0, &[7], local_query())).await;
    assert_eq!(h.drain(0).len(), 1);
    assert_eq!(h.gate.snapshot().init_query, totals(0, 1, 1));
    assert_eq!(h.counters().denied, 1);
}

/// An authorizer is asked about a query from another network and can tell
/// it from an update: an Initialize-Routing-Table with an SNET and no
/// target networks. Refusing those leaves local queries and local updates
/// as they were.
#[tokio::test]
async fn an_authorizer_tells_a_remote_table_query_by_its_snet_and_empty_targets() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    let authorizer: ControlAuthorizer = Arc::new(move |context: &ControlAuthContext| {
        record.lock().unwrap().push(context.clone());
        let table = context.message_type == NetworkMessageType::INITIALIZE_ROUTING_TABLE;
        !(table && context.source_net.is_some() && context.target_nets.is_empty())
    });
    let gate = Arc::new(ControlGate::new(
        ControlPolicy::Permissive,
        Some(authorizer),
    ));
    let mut h = Harness::with_capacities(gate, &[16, 16]);

    h.handle(h.ctx(0, &[7], from_remote(local_query()))).await;
    h.assert_quiet();
    h.handle(h.ctx(0, &[7], local_query())).await;
    assert_eq!(h.drain(0).len(), 1, "a local query is not the gate's");
    let update = control_npdu(
        NetworkMessageType::INITIALIZE_ROUTING_TABLE,
        &[1, 0x0f, 0xa0, 2, 0],
    );
    h.handle(h.ctx(0, &[7], update)).await;
    assert_eq!(h.drain(0).len(), 1, "a local update is acknowledged");
    assert!(h.table().lookup(4000).is_some());

    let seen = seen.lock().unwrap();
    let [query, update] = &seen[..] else {
        panic!("{seen:?}")
    };
    assert_eq!((query.source_net, query.dest_net), (Some(REMOTE), None));
    assert_eq!(query.source_mac, MacAddr::from_slice(&[7]), "the link peer");
    assert!(query.target_nets.is_empty() && !query.targets_omitted);
    assert_eq!(
        (update.source_net, &update.target_nets[..]),
        (None, &[4000][..])
    );
    let snapshot = h.gate.snapshot();
    assert_eq!(snapshot.init_query, totals(0, 1, 0));
    assert_eq!(snapshot.init_mgmt, totals(1, 0, 0));
}
