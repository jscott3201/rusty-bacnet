//! Initialize-Routing-Table messages that carry an SNET, at the hosted
//! router. A query from another network is a protected control (#2715),
//! allowed by the default gate, refused by a hardened one with no
//! authorizer, and an authorizer's to decide. An update from another
//! network is dropped before the gate unless
//! `RouterOptions::remote_table_updates` is on (#2717).
//!
//! In-memory and deterministic, on [`super::envelope_harness`].

use std::sync::{Arc, Mutex};

use bacnet_encoding::npdu::{Npdu, NpduAddress};
use bacnet_npdu::NetworkMessageType;
use bacnet_types::MacAddr;

use super::control_policy::{
    ControlAuthContext, ControlAuthorizer, ControlClass, ControlGate, ControlPolicy,
    ControlServiceCounters,
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
/// it from an update by its class, `InitQuery`, as well as by its SNET and
/// empty target networks. Refusing those leaves local queries and local
/// updates as they were.
#[tokio::test]
async fn an_authorizer_tells_a_remote_table_query_by_its_class() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    let authorizer: ControlAuthorizer = Arc::new(move |context: &ControlAuthContext| {
        record.lock().unwrap().push(context.clone());
        context.class != ControlClass::InitQuery
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
        query.message_type,
        NetworkMessageType::INITIALIZE_ROUTING_TABLE
    );
    assert_eq!(
        (query.class, update.class),
        (ControlClass::InitQuery, ControlClass::InitMgmt)
    );
    assert_eq!(
        (update.source_net, &update.target_nets[..]),
        (None, &[4000][..])
    );
    let snapshot = h.gate.snapshot();
    assert_eq!(snapshot.init_query, totals(0, 1, 0));
    assert_eq!(snapshot.init_mgmt, totals(1, 0, 0));
}

/// An update for network 4000 through port 1, and a purge of network 3000.
const ADD_4000: [u8; 5] = [1, 0x0f, 0xa0, 2, 0];
const PURGE_3000: [u8; 5] = [1, 0x0b, 0xb8, 0, 0];

fn update(data: &[u8]) -> Npdu {
    control_npdu(NetworkMessageType::INITIALIZE_ROUTING_TABLE, data)
}

/// By default the router drops a routing-table update that carries an SNET
/// (#2717): no change, no Ack, and the gate is never asked, so its totals
/// stay at zero and the drop shows in the router's own `denied` count. The
/// same updates from the router's own network apply.
#[tokio::test]
async fn a_table_update_from_another_network_is_refused_by_default() {
    let mut h = Harness::two_port();
    h.learned(3000, 1, &[2]);
    for data in [ADD_4000, PURGE_3000] {
        h.handle(h.ctx(0, &[7], from_remote(update(&data)))).await;
        h.assert_quiet();
    }
    assert!(h.table().lookup(4000).is_none());
    assert!(h.table().lookup(3000).is_some());
    assert_eq!(h.gate.snapshot().init_mgmt, totals(0, 0, 0));
    assert_eq!(h.counters().denied, 2);

    for data in [ADD_4000, PURGE_3000] {
        h.handle(h.ctx(0, &[7], update(&data))).await;
        assert_eq!(h.drain(0).len(), 1, "the data-less Ack");
    }
    assert!(h.table().lookup(4000).is_some());
    assert!(h.table().lookup(3000).is_none());
    assert_eq!(h.gate.snapshot().init_mgmt, totals(2, 0, 0));
    assert_eq!(h.counters().denied, 2);
}

/// An authorizer that would allow the update is not asked about it either:
/// the option is the switch, and the authorizer only narrows it.
#[tokio::test]
async fn an_authorizer_alone_does_not_admit_a_remote_table_update() {
    let allow_all: ControlAuthorizer = Arc::new(|_| true);
    let gate = Arc::new(ControlGate::new(ControlPolicy::Hardened, Some(allow_all)));
    let mut h = Harness::with_capacities(gate, &[16, 16]);
    h.handle(h.ctx(0, &[7], from_remote(update(&ADD_4000))))
        .await;
    h.assert_quiet();
    assert!(h.table().lookup(4000).is_none());
    assert_eq!(h.gate.snapshot().init_mgmt, totals(0, 0, 0));
    assert_eq!(h.counters().denied, 1);
}

/// With the option on, an update from another network is the gate's to
/// decide, as it was before #2717: the default gate applies it and
/// acknowledges the link peer, a hardened one refuses it, and an authorizer
/// sees its SNET.
#[tokio::test]
async fn with_the_option_a_remote_table_update_goes_to_the_gate() {
    let on = |options: &mut bacnet_router_core::router::Options| {
        options.remote_table_updates = true;
    };
    let mut h = Harness::with_options(Arc::new(ControlGate::permissive()), on);
    h.handle(h.ctx(0, &[7], from_remote(update(&ADD_4000))))
        .await;
    assert_eq!(h.drain(0).len(), 1, "the data-less Ack");
    assert!(h.table().lookup(4000).is_some());
    assert_eq!(h.gate.snapshot().init_mgmt, totals(1, 0, 0));
    assert_eq!(h.counters().denied, 0);

    let mut h = Harness::with_options(Arc::new(ControlGate::hardened()), on);
    h.handle(h.ctx(0, &[7], from_remote(update(&ADD_4000))))
        .await;
    h.assert_quiet();
    assert!(h.table().lookup(4000).is_none());
    assert_eq!(h.gate.snapshot().init_mgmt, totals(0, 1, 1));

    let local_only: ControlAuthorizer = Arc::new(|context| context.source_net.is_none());
    let gate = Arc::new(ControlGate::new(
        ControlPolicy::Permissive,
        Some(local_only),
    ));
    let mut h = Harness::with_options(gate, on);
    h.handle(h.ctx(0, &[7], from_remote(update(&ADD_4000))))
        .await;
    h.assert_quiet();
    h.handle(h.ctx(0, &[7], update(&ADD_4000))).await;
    assert_eq!(h.drain(0).len(), 1);
    assert_eq!(h.gate.snapshot().init_mgmt, totals(1, 1, 0));
}

/// `RouterOptions::remote_table_updates` is what turns the engine's option
/// on; the plain options leave it off.
#[test]
fn the_router_option_sets_the_engines() {
    use super::RouterOptions;
    assert!(!RouterOptions::new().engine_options().remote_table_updates);
    let on = RouterOptions::new().remote_table_updates();
    assert!(on.engine_options().remote_table_updates);
}
