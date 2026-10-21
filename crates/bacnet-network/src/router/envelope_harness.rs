//! Shared in-memory harness for the router's network-message suites: the
//! hosted router on the engine (#1679), two ports, no sockets and no tasks.
//!
//! [`Harness::handle`] and [`Harness::dispatch`] both encode the ingress
//! NPDU and hand it to the port's per-frame dispatch, which applies the
//! routing-first rule: every directed message the suites pass to `handle`
//! is a Reject, What-Is or Number-Is, which are always handled locally, so
//! no case depends on bypassing it. The port queues hold 16 requests, so
//! the full-queue cases still fill.

use std::cell::Cell;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_npdu::NetworkMessageType;
use bacnet_npdu::NpduLimits;
use bacnet_router_core::router::Options;
use bacnet_router_core::Instant;
use bacnet_transport::port::{DataAttribute, ReceivedNpdu, TransportProvenance};
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use super::control_policy::ControlGate;
use super::dispatch::{Outputs, PortDispatch, Shared};
use super::engine::{tokio_clock, Engine, OPTIONS};
use super::local_control::LocalControl;
use super::port_groups::PortGroups;
use super::send_queue;
use super::{RouterTableGuard, SendRequest};
use crate::layer::{AdmissionReceiver, ReceivedApdu, ReceivedNetworkControl};
use crate::router_table::{ClaimCounters, Convergence, PortId};
use bacnet_npdu::Mac;

pub(super) fn control_npdu(message_type: NetworkMessageType, payload: &[u8]) -> Npdu {
    Npdu {
        message_type: Some(message_type),
        payload: Bytes::copy_from_slice(payload),
        ..Npdu::default()
    }
}

/// The table's claim counts: zero but for what `set` changes.
/// `ClaimCounters` is `#[non_exhaustive]`, so a test can't spell it out.
pub(super) fn claim_counts(set: impl FnOnce(&mut ClaimCounters)) -> ClaimCounters {
    let mut counts = ClaimCounters::default();
    set(&mut counts);
    counts
}

pub(super) fn attributes() -> Vec<DataAttribute> {
    vec![DataAttribute {
        option_type: 31,
        must_understand: false,
        data: vec![0x12, 0x34],
    }]
}

/// What arrived on a port, as a test sets it up: the frame's facts and the
/// NPDU the harness encodes.
#[derive(Debug, Clone)]
pub(super) struct IngressContext {
    /// The arrival port's index.
    pub port_idx: usize,
    /// The link-layer source.
    pub source_mac: MacAddr,
    /// Whether it arrived as a link broadcast.
    pub link_layer_group: bool,
    /// The link's data attributes.
    pub data_attributes: Vec<DataAttribute>,
    /// How the transport learned of its origin.
    pub provenance: TransportProvenance,
    /// The NPDU.
    pub npdu: Npdu,
}

pub(super) struct Harness {
    networks: Vec<u16>,
    shared: Arc<Shared>,
    ports: Vec<PortDispatch>,
    pub(super) txs: Vec<send_queue::Sender>,
    pub(super) rxs: Vec<send_queue::Receiver>,
    pub(super) gate: Arc<ControlGate>,
    /// The router's local APDU queue.
    pub(super) apdus: AdmissionReceiver<ReceivedApdu>,
    local_tx: crate::layer::AdmissionSender<ReceivedApdu>,
    /// The write failures the test has taken note of.
    write_failures_seen: Cell<u64>,
}

/// The harness router's own MAC on each port: [1] on 1000, [2] on 2000.
/// More ports follow the pattern: [n + 1] on port n.
pub(super) const PORT_MACS: [u8; 2] = [0x01, 0x02];

/// The two-port harness's networks.
pub(super) const NETWORKS: [u16; 2] = [1000, 2000];

/// A routing table as a suite writes it before its router starts: direct
/// and learned routes, under standard or hardened convergence.
/// [`Harness::with_table`] starts the router with exactly these routes, the
/// ports' own direct ones included only when listed.
#[derive(Debug, Default)]
pub(super) struct TableSeed {
    hardened: bool,
    /// Network, port, and the next hop of a learned route.
    routes: Vec<(u16, u8, Option<MacAddr>)>,
}

impl TableSeed {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn new_hardened() -> Self {
        Self {
            hardened: true,
            ..Self::default()
        }
    }

    pub(super) fn add_direct(&mut self, network: u16, port: usize) {
        self.routes.push((network, port as u8, None));
    }

    pub(super) fn add_learned(&mut self, network: u16, port: usize, next_hop: MacAddr) {
        self.routes.push((network, port as u8, Some(next_hop)));
    }

    pub(super) fn len(&self) -> usize {
        self.routes.len()
    }
}

impl Harness {
    /// The two-port router with the permissive gate: direct routes to 1000
    /// on port 0 and 2000 on port 1.
    pub(super) fn two_port() -> Self {
        Self::with_capacities(Arc::new(ControlGate::permissive()), &[16, 16])
    }

    /// The two-port router, its table exactly `table`.
    pub(super) fn with_table(table: TableSeed) -> Self {
        Self::with_gate(table, Arc::new(ControlGate::permissive()))
    }

    /// The two-port router with `gate`, its table exactly `table`.
    pub(super) fn with_gate(table: TableSeed, gate: Arc<ControlGate>) -> Self {
        let harness = Self::with_capacities(gate, &[16, 16]);
        {
            let now = harness.now();
            let mut guard = harness.table();
            let networks: Vec<u16> = guard.networks().collect();
            for network in networks {
                guard.remove(network);
            }
            if table.hardened {
                guard.set_convergence(Convergence::Hardened);
            }
            for (network, port, next_hop) in table.routes {
                match next_hop {
                    None => guard.add_direct(network, PortId(port)),
                    Some(mac) => {
                        let mac = Mac::from_slice(&mac).unwrap();
                        guard.add_learned(network, PortId(port), mac, now)
                    }
                };
            }
        }
        harness
    }

    /// The two-port router with `gate` and these port queue capacities.
    pub(super) fn with_capacities(gate: Arc<ControlGate>, capacities: &[usize]) -> Self {
        Self::build(&NETWORKS, gate, capacities)
    }

    /// A router with a port on each of `networks`, the permissive gate and
    /// 16-request queues.
    pub(super) fn with_networks(networks: &[u16]) -> Self {
        let capacities = vec![16; networks.len()];
        Self::build(networks, Arc::new(ControlGate::permissive()), &capacities)
    }

    /// A router with a port on each of `networks`, carrying the NPDUs
    /// `limits` gives it, as its transport would report them, with the
    /// permissive gate and 16-request queues.
    pub(super) fn with_limits(networks: &[u16], limits: &[NpduLimits]) -> Self {
        let capacities = vec![16; networks.len()];
        let gate = Arc::new(ControlGate::permissive());
        Self::build_with(networks, limits, gate, &capacities, OPTIONS)
    }

    fn build(networks: &[u16], gate: Arc<ControlGate>, capacities: &[usize]) -> Self {
        let limits = vec![NpduLimits::default(); networks.len()];
        Self::build_with(networks, &limits, gate, capacities, OPTIONS)
    }

    fn build_with(
        networks: &[u16],
        limits: &[NpduLimits],
        gate: Arc<ControlGate>,
        capacities: &[usize],
        options: Options,
    ) -> Self {
        let configs: Vec<_> = (0u8..)
            .zip(networks.iter().zip(limits))
            .map(|(index, (&network, &limits))| {
                Engine::port(
                    network,
                    Mac::from_array([index + 1]),
                    limits,
                    PortGroups::default(),
                )
            })
            .collect();
        let engine = Engine::new(&configs, tokio_clock(), options).unwrap();
        let shared = Arc::new(Shared {
            engine,
            gate: Arc::clone(&gate),
            networks: networks.to_vec(),
            warnings: Default::default(),
        });
        let (txs, rxs) = capacities.iter().map(|&n| send_queue::channel(n)).unzip();
        let (local_tx, apdus, counters) = AdmissionReceiver::channel(true);
        let mut harness = Self {
            networks: networks.to_vec(),
            shared,
            ports: Vec::new(),
            txs,
            rxs,
            gate,
            apdus: AdmissionReceiver::from_apdu_parts(apdus, counters),
            local_tx,
            write_failures_seen: Cell::new(0),
        };
        harness.connect(None);
        harness
    }

    /// Point every port's dispatch at the queues, with `control` as the
    /// router's network-control consumer.
    fn connect(&mut self, control: Option<crate::layer::AdmissionSender<ReceivedNetworkControl>>) {
        let outputs = Arc::new(Outputs {
            replies: Default::default(),
            send_txs: self.txs.clone(),
            local_tx: self.local_tx.clone(),
            local_control: LocalControl::new(control, Arc::new(AtomicU64::new(0))),
        });
        self.ports = (0..self.networks.len())
            .map(|port| PortDispatch {
                shared: Arc::clone(&self.shared),
                outputs: Arc::clone(&outputs),
                port: port as u8,
            })
            .collect();
    }

    /// Send the startup announcements, as `start` does.
    pub(super) fn announce(&self) {
        self.shared.announce(&self.txs);
    }

    /// Give the router a network-control consumer and return its receiver.
    pub(super) fn network_control(&mut self) -> mpsc::Receiver<ReceivedNetworkControl> {
        let (tx, rx, _) = AdmissionReceiver::channel(false);
        self.connect(Some(tx));
        rx
    }

    /// RB-04 multi-peer fixture: direct 1000/0 + 2000/1, plus learned routes
    /// across three distinct ingress peers — 3000/3001 via port 1 peer [2],
    /// 4000 via port 1 peer [3] (same port, other peer), 5000 via port 0
    /// peer [9] (other port). Lets omitted/single/mixed Busy/Available lists
    /// assert per-route change AND no-change on one topology.
    pub(super) fn busy_scope() -> Self {
        let h = Self::two_port();
        h.learned(3000, 1, &[2]);
        h.learned(3001, 1, &[2]);
        h.learned(4000, 1, &[3]);
        h.learned(5000, 0, &[9]);
        h
    }

    /// Install `network` as learned through the router at `next_hop` on
    /// `port`, now.
    pub(super) fn learned(&self, network: u16, port: u8, next_hop: &[u8]) {
        let now = self.now();
        let next_hop = Mac::from_slice(next_hop).unwrap();
        assert!(self
            .table()
            .add_learned(network, PortId(port), next_hop, now));
    }

    /// Take claims under hardened convergence.
    pub(super) fn hardened(self) -> Self {
        self.table().set_convergence(Convergence::Hardened);
        self
    }

    /// The router's routing table.
    pub(super) fn table(&self) -> RouterTableGuard<'_> {
        RouterTableGuard {
            router: self.shared.engine.lock(),
        }
    }

    /// The engine's totals so far, as the router publishes them.
    pub(super) fn counters(&self) -> super::Counters {
        self.shared.engine.counters()
    }

    /// The rewrites the engine couldn't write so far, noted as expected:
    /// the harness checks, when it is dropped, that there were no others.
    pub(super) fn take_write_failures(&self) -> u64 {
        let failures = self.counters().write_failures;
        self.write_failures_seen.set(failures);
        failures
    }

    /// The router's time now.
    pub(super) fn now(&self) -> Instant {
        self.shared.engine.now()
    }

    pub(super) fn ctx(&self, port: usize, source_mac: &[u8], npdu: Npdu) -> IngressContext {
        self.ctx_with_provenance(port, source_mac, npdu, TransportProvenance::unverified())
    }

    pub(super) fn ctx_with_provenance(
        &self,
        port: usize,
        source_mac: &[u8],
        npdu: Npdu,
        provenance: TransportProvenance,
    ) -> IngressContext {
        IngressContext {
            port_idx: port,
            source_mac: MacAddr::from_slice(source_mac),
            link_layer_group: true,
            data_attributes: Vec::new(),
            provenance,
            npdu,
        }
    }

    /// Hand `ctx` to its port, as its transport would.
    pub(super) async fn handle(&mut self, ctx: IngressContext) {
        let mut npdu = BytesMut::new();
        encode_npdu(&mut npdu, &ctx.npdu).unwrap();
        self.ports[ctx.port_idx].dispatch(ReceivedNpdu {
            npdu: npdu.freeze(),
            source_mac: ctx.source_mac,
            link_layer_group: ctx.link_layer_group,
            data_attributes: ctx.data_attributes,
            provenance: ctx.provenance,
            direct_response: None,
            reply_tx: None,
        });
    }

    /// The same: the routing-first rule is the engine's either way.
    pub(super) async fn dispatch(&mut self, ctx: IngressContext) {
        self.handle(ctx).await;
    }

    pub(super) fn drain(&mut self, port: usize) -> Vec<SendRequest> {
        let mut out = Vec::new();
        while let Ok(req) = self.rxs[port].try_recv() {
            out.push(req);
        }
        out
    }

    pub(super) fn assert_quiet(&mut self) {
        for port in 0..self.rxs.len() {
            assert!(
                self.rxs[port].try_recv().is_err(),
                "expected no output on port {port}"
            );
        }
    }
}

/// With the differential test gone (#1679), every suite on the harness
/// checks what it used to: each call's sends were all polled, no rewrite
/// failed unless the test took note of it, and the hold-down pool never
/// overflowed.
impl Drop for Harness {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let counters = self.counters();
        assert_eq!(counters.undrained, 0, "every send was polled");
        assert_eq!(
            counters.write_failures,
            self.write_failures_seen.get(),
            "no rewrite failed unless the test expected it"
        );
        assert_eq!(
            self.table().claim_counters().hold_evicted,
            0,
            "no hold-down record was evicted"
        );
    }
}

pub(super) fn broadcast_data(requests: Vec<SendRequest>) -> Vec<Bytes> {
    requests
        .into_iter()
        .map(|req| match req {
            SendRequest::Broadcast { npdu, .. } => npdu,
            SendRequest::Unicast { .. } => panic!("expected broadcast"),
        })
        .collect()
}
