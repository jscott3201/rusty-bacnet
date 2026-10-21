//! BACnet half-router — forwards APDUs between BACnet networks.
//!
//! A BACnet router (ASHRAE 135-2020 Clause 6.6) sits on several BACnet
//! networks at once and relays messages among them, rewriting the NPDU
//! source/destination fields and decrementing the hop count as it goes.
//!
//! This implementation supports:
//! - Forwarding APDUs between directly-connected networks
//! - Who-Is-Router-To-Network / I-Am-Router-To-Network messages
//! - Reject-Message-To-Network for unknown routes
//! - Learned routes from I-Am-Router-To-Network announcements
//!
//! Local application delivery follows the shared
//! [receive-queue contract](crate::layer#receive-queue-admission), independently
//! of forwarding and inline network-message handling.
//!
//! Each outgoing port holds at most 256 queued sends, plus one in progress.
//! Among queued sends, higher network priority goes first; equal priorities
//! keep their arrival order (Clause 6.5.4). A full queue refuses the new send
//! without evicting an earlier one. A send in progress is not preempted, and
//! sustained higher-priority traffic can delay lower-priority traffic.
//!
//! # Routing-claim trust model (RB-06)
//!
//! Classic BACnet routing claims are unauthenticated: an ingress port, next-hop
//! MAC or advertised network is not proof that a peer is authorized to control
//! that route. These local mitigations do not prevent route poisoning or
//! authenticate a claim (Clauses 6.4 and 6.6.3):
//! - Direct routes cannot be overwritten by learning, changed by rejects, or
//!   changed by Initialize-Routing-Table management updates.
//! - Standard convergence applies 6.6.3.2 last-wins immediately: each new
//!   I-Am-Router / Init-ACK advertisement updates routing information, with
//!   flap warnings retained. Same-port refreshes and absent learning stay
//!   immediate; I-Could-Be-Router remains absent-only, within the route cap.
//! - Hardened convergence ([`Convergence::Hardened`](crate::router_table::Convergence)
//!   set through [`BACnetRouter::table`], explicit opt-in only)
//!   holds cross-port learned moves for a second same-(network, port) claim
//!   within 60s inclusive. Repeats in separate messages suffice; duplicates in
//!   one message cannot corroborate. The old route forwards while pending.
//!   Alternation resets the slot; single-shot advertisers need the 60s reaper
//!   plus a fresh claim. Corroboration is dampening, not identity assurance.
//! - Initialize-Routing-Table updates are management writes (6.4.7/6.6.3.8):
//!   nonzero Port ID replaces/appends the learned entry, Port ID 0 purges it,
//!   immediately in wire order. They never create/alter direct routes. Wire
//!   controls pass [`control_policy::ControlGate`] (RB-09); direct immunity
//!   stays safety scoping, not auth. LOCAL table calls bypass policy.
//! - A query (Number of Ports 0) never mutates and answers with the complete
//!   table in ascending-DNET bounded portions (6.6.3.9); wire Port IDs are
//!   `port_index + 1` (0 stays the purge trigger).
//! - Hardened pending holds one challenger per learned network (bounded by live
//!   learned routes). Slots expire lazily on the next claim plus the 60s aging
//!   reaper; apply/refresh/removal/aging/direct/manual edits clear the slot.
//! - Freshness is split: `last_seen` (control confirmation) drives 300s expiry;
//!   `last_used` (forwarding use) never extends it. Busy/Available touch
//!   neither. Traffic no longer pins a dead route past the rescue.
//! - Unknown DNETs solicit once per 5s per DNET (coalesced, max 256 pending,
//!   30s entry age, reserved never solicited) then fail the caller with
//!   NOT_DIRECTLY_CONNECTED (6.6.3.1/6.5 retryable); packets are never buffered.
//!   `stop` cancels pending discovery.
//! - Disconnect never removes routes (PTP unimplemented). Reject transitions
//!   are dampened per (ingress port, network) with a 30s hold-down; no-op
//!   rejects never extend busy deadlines. The table's
//!   [`claim_counters`](crate::router_table::Table::claim_counters) stay
//!   count-only saturating and never affect decisions.

mod actions;
pub mod control_policy;
mod counters;
mod dispatch;
mod engine;
mod hosted_host;
mod limits;
mod local_control;
mod local_delivery;
mod options;
mod originate;
mod port_groups;
mod replies;
mod send_queue;
mod send_warning;
mod sender;
pub use replies::RouterResponse;

use std::ops::{Deref, DerefMut};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use bacnet_npdu::Mac;
pub use bacnet_router_core::router::{
    Congestion, Duplicates, OriginateError, Request, SourceSelection,
};
use bacnet_router_core::{Instant, PortId};
use bacnet_transport::port::{DataAttribute, TransportPort};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::Bytes;
pub use counters::Counters;
pub use originate::OriginateFailure;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::layer::ReceivedNetworkControl;
use crate::layer::{AdmissionReceiver, AdmissionSender, QueueAdmissionCounters, ReceivedApdu};

pub use control_policy::{ControlAuthContext, ControlAuthorizer, ControlClass};
pub use control_policy::{ControlDecisionCounters, ControlGate, ControlPolicy};
pub use control_policy::{ControlServiceCounters, ControlTrust};
use dispatch::{Outputs, PortDispatch, Shared};
use engine::{tokio_clock, Engine, EngineGuard, PORTS};
pub use options::{LocalApduReceiver, NetworkControlReceiver, RouterOptions, StartedRouter};
use port_groups::PortGroups;

// The suites that start a router take these names from here, as before the
// engine.
#[cfg(test)]
use {
    bacnet_encoding::npdu::{encode_npdu, Npdu},
    bacnet_npdu::NetworkMessageType,
    bacnet_transport::port::TransportProvenance,
    bytes::BytesMut,
    std::time::Duration,
};

/// A send request to be forwarded on a port.
#[derive(Debug)]
enum SendRequest {
    Unicast {
        npdu: Bytes,
        mac: MacAddr,
        data_attributes: Vec<DataAttribute>,
        reject_origin: Option<bacnet_router_core::router::RejectOrigin>,
        response: Option<replies::ReplyCompletion>,
    },
    Broadcast {
        npdu: Bytes,
        data_attributes: Vec<DataAttribute>,
        reject_origin: Option<bacnet_router_core::router::RejectOrigin>,
    },
}

/// A router port: a transport bound to a specific BACnet network number.
///
/// [`BACnetRouter::start`] calls
/// [`TransportPort::prepare_for_routing`] on each port's transport before it
/// starts it, so an MS/TP port takes and sends extended frames whatever its
/// `MstpConfig` says: Clause 9.3 requires them of a routing node. Once the
/// transport has started, it reads [`TransportPort::max_npdu`], and answers
/// an NPDU too long for the port with reject reason 4 (#1665). It follows
/// the limit's changes after that, for a transport that publishes them
/// ([`TransportPort::max_npdu_changes`], #1696).
pub struct RouterPort<T: TransportPort> {
    /// The transport for this port.
    pub transport: T,
    /// The network number assigned to this port: 1 to 65534.
    /// [`BACnetRouter::start`] refuses 0, which is no network, and 0xFFFF,
    /// the global broadcast DNET, which can't be the SNET the router puts
    /// in what it relays from the port (Clause 6.2.2.1).
    pub network_number: u16,
}

/// BACnet router connecting multiple networks.
///
/// The router holds multiple ports, each bound to a different BACnet network.
/// When an NPDU arrives on one port with a destination network that maps to
/// another port, the router forwards the message.
/// See the [receive-queue contract](crate::layer#receive-queue-admission) for
/// raw/tracked local receivers, admission limits, counters and lifecycle.
///
/// Every routing decision is `bacnet-router-core`'s engine (#1679), which
/// the router keeps behind one lock: each port's dispatch task takes it for
/// one frame at a time, as the sweep and [`Self::table`] do, and nothing
/// holds it across an await. The engine's timers run on Tokio time from the
/// router's start. It follows Clause 6 where the router before #1679 didn't
/// (R1 to R7 of #1679): egress limits, rejects that keep routes,
/// network numbers at startup, reachable-only and arrival-port-aware
/// route answers, the hop count, and the router's own address on any port.
pub struct BACnetRouter {
    replies: Arc<replies::ReplyLifetime>,
    /// The port queues, for the router's own requests ([`Self::originate`]):
    /// held weakly, as replies hold them, so stopping the dispatch tasks
    /// still closes the local receivers.
    outputs: std::sync::Weak<Outputs>,
    /// The engine, the gate and the ports' groups.
    shared: Arc<Shared>,
    /// Dispatch tasks (one per port).
    dispatch_tasks: Vec<JoinHandle<()>>,
    /// Sender tasks (one per port, owns the transport for outgoing messages).
    sender_tasks: Vec<JoinHandle<()>>,
    /// Background task that runs the engine's sweep when it is due.
    aging_task: Option<JoinHandle<()>>,
    /// Tasks that follow a port's NPDU limit (one per port whose transport
    /// publishes its changes).
    limit_tasks: Vec<JoinHandle<()>>,
    /// Last sequence given to a control for the router's own consumer.
    network_control_ingress_sequence: Arc<AtomicU64>,
}

/// The routing table of a running [`BACnetRouter`], borrowed for local
/// management (see [`BACnetRouter::table`]).
///
/// While it lives, every port's dispatch and the sweep wait for it: read or
/// change what you need, then drop it. It can't be held across an await in
/// a spawned task.
///
/// The lock behind it isn't reentrant. Don't call [`BACnetRouter::table`]
/// again while holding one, nor from a [`ControlAuthorizer`], which runs
/// while that lock is held: the call waits for the lock already held, and
/// on the same thread that is forever. The router's other `&self` methods
/// take no lock, so they are safe to call meanwhile: the counts
/// ([`BACnetRouter::counters`] and the four drop counts),
/// [`BACnetRouter::now`] and [`BACnetRouter::control_snapshot`].
pub struct RouterTableGuard<'a> {
    router: EngineGuard<'a>,
}

impl Deref for RouterTableGuard<'_> {
    type Target = crate::router_table::RouterTable;

    fn deref(&self) -> &Self::Target {
        self.router.table()
    }
}

impl DerefMut for RouterTableGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.router.table_mut()
    }
}

impl std::fmt::Debug for RouterTableGuard<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.router.table().fmt(f)
    }
}

impl BACnetRouter {
    /// Create and start a router from a list of ports, set up as `options`
    /// asks (#1220).
    ///
    /// Returns the router with its local APDU receiver, for messages without
    /// a remote destination or for which this router is the final hop, plus
    /// the network-control receiver when the options ask for it.
    /// [`RouterOptions::new`] is the plain router; see [`RouterOptions`] for
    /// the tracked APDU receiver, the wire-control policy and the
    /// network-control receiver, which combine freely.
    ///
    /// Fails if there are no ports or more than 255 (a routing table's Port
    /// ID names at most 255), if a port's network number is 0 or 0xFFFF
    /// ([`RouterPort::network_number`]), or if two ports share one, all
    /// before any transport starts; and if a transport fails to start, or
    /// if a transport's own MAC is longer than
    /// [`MAX_MAC_LEN`](bacnet_npdu::MAX_MAC_LEN)
    /// octets.
    pub async fn start<T, A, C>(
        ports: Vec<RouterPort<T>>,
        options: RouterOptions<A, C>,
    ) -> Result<StartedRouter<A, C>, Error>
    where
        T: TransportPort + 'static,
        A: LocalApduReceiver,
        C: NetworkControlReceiver,
    {
        let (control_tx, network_control) = if options.wants_network_control() {
            let (tx, rx, counters) = AdmissionReceiver::channel(C::TRACKED);
            (Some(tx), Some(C::from_queue(rx, counters)))
        } else {
            (None, None)
        };
        let gate = Arc::new(options.control_gate());
        let (router, rx, counters) = Self::start_dispatch(
            ports,
            A::TRACKED,
            gate,
            control_tx,
            options.engine_options(),
        )
        .await?;
        Ok(StartedRouter {
            router,
            apdus: A::from_queue(rx, counters),
            network_control,
        })
    }

    /// Count-only RB-09 decision totals (allow/deny/policy-deny per class).
    pub fn control_snapshot(&self) -> control_policy::ControlDecisionCounters {
        self.shared.gate.snapshot()
    }

    /// The engine's counts since start: the four drop counts below, and
    /// what else the engine counts, such as the frames it refused as
    /// malformed, the relays a spent hop count stopped, the rejects it sent,
    /// the sends too long for their egress port (`egress_too_long`: the
    /// limit its transport's [`TransportPort::max_npdu`] reports, #1665, or
    /// a definite pre-send length refusal reported by the transport, including
    /// BACnet/SC frame attributes and direct-peer bounds, #1716),
    /// and the rewrites it couldn't write (`write_failures`: longer than its
    /// 65,556-octet scratch, or with a field no NPDU may carry). They
    /// saturate and name no peer. The table's own counts are
    /// [`claim_counters`](crate::router_table::Table::claim_counters).
    ///
    /// They are the totals as of the last frame, send refusal, sweep or
    /// table change the engine finished: the engine's own counts are `u32`s
    /// it starts again each time they are taken, and the router takes them,
    /// and adds them to these `u64` totals, before it lets the engine's lock
    /// go (#1767). So reading them takes no lock: it is safe while a
    /// [`RouterTableGuard`] lives, and so are the four drop counts below,
    /// which read them.
    pub fn counters(&self) -> Counters {
        self.shared.engine.counters()
    }

    /// NPDUs refused on any port since start because an address in them was
    /// past [`MAX_MAC_LEN`](bacnet_npdu::MAX_MAC_LEN):
    /// their DLEN or SLEN (#1141), or the link-layer source MAC the port's
    /// transport reported (#1198). Saturates at `u64::MAX`.
    ///
    /// Such an NPDU is neither forwarded nor delivered locally, and a network
    /// message in it changes no route. When its DLEN or SLEN is the long one
    /// and it names a specific DNET, the router also answers the sender with
    /// Reject-Message-To-Network reason 6, `ADDRESSING_ERROR` (Clause 6.4.4),
    /// the way it rejects a DNET it cannot reach. A global broadcast, or an
    /// NPDU without a DNET, is only dropped. So is a frame from an over-long
    /// link-layer source MAC: that is a fault in the transport, not in the
    /// NPDU, and the router sends nothing back to such a MAC. No built-in
    /// transport reports one.
    pub fn address_length_drops(&self) -> u64 {
        self.counters().address_length_drops
    }

    /// NPDUs dropped on any port since start because their DNET was 0xFFFF
    /// and they also carried a DADR (#1379). Saturates at `u64::MAX`.
    ///
    /// DNET 0xFFFF already names every device on every network (Clauses
    /// 6.2.2 and 6.3.2), so a DADR beside it contradicts it. The router does
    /// not pass such an NPDU on to its other ports, deliver it locally,
    /// act on a network message in it, or answer it with a reject: a
    /// global broadcast never draws one. The address lengths are within
    /// bounds, so the drop is counted here and not in
    /// [`Self::address_length_drops`].
    pub fn global_broadcast_dadr_drops(&self) -> u64 {
        self.counters().global_broadcast_dadr_drops
    }

    /// NPDUs dropped on any port since start because they were addressed to
    /// a broadcast, global (DNET 0xFFFF) or remote (a DNET with DLEN 0), and
    /// carried an APDU other than an Unconfirmed-Request (#1491). Saturates
    /// at `u64::MAX`.
    ///
    /// Only an Unconfirmed-Request may go to a broadcast network address
    /// (Clause 6.3). Anything else belongs to one peer's transaction, so the
    /// router neither passes it on to any network nor delivers it locally,
    /// and answers it with no reject. The PDU type comes from the first APDU
    /// octet, with no decode; network messages are not affected.
    pub fn broadcast_pdu_type_drops(&self) -> u64 {
        self.counters().broadcast_pdu_type_drops
    }

    /// NPDUs dropped on any port since start because the router would have
    /// delivered them, on a directly connected port, to a DADR that is a
    /// group destination there, such as a B/IP multicast address, and their
    /// APDU isn't an Unconfirmed-Request (#1504). Saturates at `u64::MAX`.
    ///
    /// Sent as one unicast, such an NPDU would reach every node in the
    /// group without the broadcast network addresses that
    /// [`Self::broadcast_pdu_type_drops`] filters: a confirmed request would
    /// draw an answer from each node, and an answer would reach nodes that
    /// asked nothing. The port's rule is
    /// [`TransportPort::group_destinations`],
    /// so a custom transport that keeps the default recognizes none. The
    /// router delivers it nowhere and answers it with no reject. A network
    /// message routed to such a DADR, or a Reject-Message-To-Network relayed
    /// to one, is dropped and counted too: it must not reach every router on
    /// that network as a broadcast. An Unconfirmed-Request for one of the
    /// groups the port's broadcast stands for
    /// ([`TransportPort::local_broadcast_destinations`]) isn't counted: it
    /// goes out on that port as a local broadcast, and reaches this router's
    /// application where a DLEN 0 broadcast would (#1677). On B/IP, one for a
    /// group at another UDP port, another B/IP network, is counted.
    pub fn group_dadr_drops(&self) -> u64 {
        self.counters().group_dadr_drops
    }

    async fn start_dispatch<T: TransportPort + 'static>(
        mut ports: Vec<RouterPort<T>>,
        track_depth: bool,
        control: Arc<control_policy::ControlGate>,
        network_control: Option<AdmissionSender<ReceivedNetworkControl>>,
        engine_options: bacnet_router_core::router::Options,
    ) -> Result<(Self, mpsc::Receiver<ReceivedApdu>, QueueAdmissionCounters), Error> {
        if ports.is_empty() || ports.len() > PORTS {
            return Err(Error::Encoding(format!(
                "A router needs 1 to {PORTS} ports, not {}",
                ports.len()
            )));
        }
        // Refuse reserved and duplicate network numbers
        {
            let mut seen = std::collections::HashSet::new();
            for port in &ports {
                if matches!(port.network_number, 0 | 0xFFFF) {
                    return Err(Error::Encoding(format!(
                        "A router port can't be on network {}: ports take 1 to 65534",
                        port.network_number
                    )));
                }
                if !seen.insert(port.network_number) {
                    return Err(Error::Encoding(format!(
                        "Duplicate network number {} in router ports",
                        port.network_number
                    )));
                }
            }
        }

        let (local_tx, local_rx, counters) = AdmissionReceiver::channel(track_depth);

        // Start each transport, and note what the engine and its hooks need.
        let mut port_receivers = Vec::new();
        let mut configs = Vec::new();
        let mut networks = Vec::new();
        let mut limit_changes = Vec::new();

        for port in &mut ports {
            // A routing node may owe its data link more than other nodes:
            // MS/TP turns on extended frames here (Clause 9.3).
            port.transport.prepare_for_routing();
            let rx = port.transport.start().await?;
            port_receivers.push(rx);
            let mac = Mac::from_slice(port.transport.local_mac()).map_err(|_| {
                Error::Encoding(format!(
                    "Router port on network {} has a MAC longer than {} octets",
                    port.network_number,
                    bacnet_npdu::MAX_MAC_LEN
                ))
            })?;
            // Read once the transport has started: MS/TP's grows with the
            // extended frames routing turned on, and BACnet/SC's comes from
            // its hub connection.
            configs.push(Engine::port(
                port.network_number,
                mac,
                port.transport.max_npdu(),
                PortGroups {
                    groups: port.transport.group_destinations(),
                    local_broadcasts: port.transport.local_broadcast_destinations(),
                },
            ));
            // Taken after that read; the task that follows it applies the
            // value it holds first, so a change in between isn't lost.
            limit_changes.push(port.transport.max_npdu_changes());
            networks.push(port.network_number);
        }
        let engine = Engine::new(&configs, tokio_clock(), engine_options)
            .map_err(|error| Error::Encoding(error.to_string()))?;

        let network_control_ingress_sequence = Arc::new(AtomicU64::new(0));
        let shared = Arc::new(Shared {
            engine,
            gate: control,
            networks,
            warnings: Default::default(),
        });

        let (send_txs, send_rxs): (Vec<_>, Vec<_>) = (0..ports.len())
            .map(|_| send_queue::channel(send_queue::CAPACITY))
            .unzip();
        let replies = Arc::new(replies::ReplyLifetime::default());
        let outputs = Arc::new(Outputs {
            replies: Arc::clone(&replies),
            send_txs,
            local_tx,
            local_control: local_control::LocalControl::new(
                network_control,
                Arc::clone(&network_control_ingress_sequence),
            ),
        });
        // Senders may return a late rejection through these queues, but
        // must not keep them or the local consumers alive after dispatch.
        let sender_tasks = ports
            .into_iter()
            .zip(send_rxs)
            .enumerate()
            .map(|(index, (port, send_rx))| {
                tokio::spawn(sender::run(
                    port.transport,
                    // At most 255 ports, checked above.
                    PortId(index as u8),
                    send_rx,
                    Arc::clone(&shared),
                    Arc::downgrade(&outputs),
                ))
            })
            .collect();

        // Follow the limits of the ports whose transports publish them.
        let limit_tasks = limit_changes
            .into_iter()
            .enumerate()
            .filter_map(|(index, changes)| {
                // At most 255 ports, checked above.
                let port = PortId(index as u8);
                let task = limits::follow(Arc::clone(&shared), port, changes?);
                Some(tokio::spawn(task))
            })
            .collect();

        // Startup: each port hears its own network number, in a
        // Network-Number-Is (R3, Clause 6.4.20), then, in an I-Am-Router,
        // which networks lie beyond the other ports.
        shared.announce(&outputs.send_txs);

        let dispatch_tasks = port_receivers
            .into_iter()
            .enumerate()
            .map(|(index, rx)| {
                let port = PortDispatch {
                    shared: Arc::clone(&shared),
                    outputs: Arc::clone(&outputs),
                    // At most 255 ports, checked above.
                    port: index as u8,
                };
                tokio::spawn(port.run(rx))
            })
            .collect();

        // Run the engine's sweep at each of its deadlines: it purges stale
        // learned routes, lapsed busy marks, hardened pending slots and
        // discovery entries. `last_used` never extends `last_seen` expiry.
        //
        // The task sleeps until the deadline it last read, and nothing
        // wakes it early. That holds because only the sweep itself
        // (`on_deadline`) moves the engine's `next_deadline`; if the core
        // ever lets another call move it, that call's site must wake this
        // task (a `Notify`, say), or the sweep runs late.
        let aging = Arc::clone(&shared);
        let aging_task = tokio::spawn(async move {
            let clock = aging.engine.clock();
            // A deadline past the host clock's range never comes.
            let wake = |next: Option<Instant>| next.and_then(|at| clock.to_std(at));
            let mut next = wake(aging.engine.lock().next_deadline());
            loop {
                match next {
                    Some(at) => tokio::time::sleep_until(at.into()).await,
                    None => std::future::pending().await,
                }
                next = wake(aging.sweep());
            }
        });

        Ok((
            Self {
                replies,
                outputs: Arc::downgrade(&outputs),
                shared,
                dispatch_tasks,
                sender_tasks,
                aging_task: Some(aging_task),
                limit_tasks,
                network_control_ingress_sequence,
            },
            local_rx,
            counters,
        ))
    }

    /// The routing table, locked for local management: read routes, or
    /// change them with no wire-control policy in the way.
    ///
    /// Every port's dispatch waits while the guard lives, so drop it soon,
    /// and never hold it across an await. Times in it are the router's own
    /// ([`Self::now`]).
    ///
    /// Don't call it again while a guard lives on the same thread, or from
    /// a [`ControlAuthorizer`]: the lock isn't reentrant, so that call
    /// never returns. The counts, [`Self::now`] and
    /// [`Self::control_snapshot`] take no lock and are safe to call while
    /// the guard lives.
    pub fn table(&self) -> RouterTableGuard<'_> {
        RouterTableGuard {
            router: self.shared.engine.lock(),
        }
    }

    /// The router's time now: the clock its table and timers run on, Tokio
    /// time since the router started.
    pub fn now(&self) -> Instant {
        self.shared.engine.now()
    }

    /// Stop the router.
    ///
    /// Aborts and awaits dispatch, sender, aging and limit tasks without
    /// waiting for the local application consumer, even with a full queue.
    /// Queued local APDUs remain drainable, retaining their reply senders
    /// until consumed or the receiver is dropped. Stop is not an admission
    /// drop and does not clear depth/high-water/drop totals. See the
    /// [receive-queue contract](crate::layer#receive-queue-admission).
    pub async fn stop(&mut self) {
        self.replies.seal();
        for task in self.dispatch_tasks.drain(..) {
            task.abort();
            let _ = task.await;
        }
        for task in self.sender_tasks.drain(..) {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.aging_task.take() {
            task.abort();
            let _ = task.await;
        }
        for task in self.limit_tasks.drain(..) {
            task.abort();
            let _ = task.await;
        }
        // Cancel pending unknown-destination discovery so no further
        // solicitation is emitted and coalesced waiters observe shutdown.
        self.shared.engine.lock().cancel_discovery();
    }

    /// How many unknown networks the router is looking for.
    #[cfg(test)]
    fn searches(&self) -> (usize, bool) {
        let router = self.shared.engine.lock();
        (router.discovery().len(), router.discovery().is_cancelled())
    }
}

#[cfg(test)]
mod admission_tests;
#[cfg(test)]
mod bip_group_tests;
#[cfg(test)]
mod busy_scope_tests;
#[cfg(test)]
mod claim_tests;
#[cfg(test)]
mod congestion_tests;
#[cfg(test)]
mod corroboration_tests;
#[cfg(test)]
mod cutover_tests;
#[cfg(test)]
mod driven_port;
#[cfg(test)]
mod envelope_control_tests;
#[cfg(test)]
mod envelope_discovery_tests;
#[cfg(test)]
mod envelope_harness;
#[cfg(test)]
mod fixes_tests;
#[cfg(test)]
mod hardening_tests;
#[cfg(test)]
mod ingress_harness;
#[cfg(test)]
mod init_routing_table_tests;
#[cfg(test)]
mod late_refusal_tests;
#[cfg(test)]
mod limit_tests;
#[cfg(test)]
mod mstp_extended_tests;
#[cfg(test)]
mod originate_tests;
#[cfg(test)]
mod priority_harness;
#[cfg(test)]
mod priority_tests;
#[cfg(test)]
mod r7_reply_tests;
#[cfg(test)]
mod rb06_convergence_tests;
#[cfg(test)]
mod rb09_control_policy_tests;
#[cfg(test)]
mod reject_tests;
#[cfg(test)]
mod tests;

impl Drop for BACnetRouter {
    fn drop(&mut self) {
        self.replies.seal();
    }
}
