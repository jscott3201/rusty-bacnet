//! BACnet half-router — forwards APDUs between BACnet networks.
//!
//! Per ASHRAE 135-2020 Clause 6.4, a BACnet router connects two or more
//! BACnet networks. It forwards messages between them by manipulating
//! the NPDU source/destination fields and decrementing the hop count.
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
//! # Routing-claim trust model
//!
//! Classic BACnet routing claims are unauthenticated: an ingress port, next-hop
//! MAC or advertised network is not proof that a peer is authorized to control
//! that route. These local mitigations do not
//! prevent route poisoning or authenticate a claim (Clauses 6.4 and 6.6.3):
//! - Direct routes cannot be overwritten by learning or changed by rejects.
//! - I-Am-Router and Initialize-Routing-Table-ACK claims moving a learned route
//!   to a different port need two claims for the same (network, new port), no
//!   more than 60s apart (inclusive, aligned with the flap window). Repeats in
//!   separate messages from one router suffice; distinct sources are not required.
//!   Duplicate entries in one message cannot supply both votes. The old route keeps
//!   forwarding while pending. Absent-route learning and same-port refreshes
//!   remain immediate; Initialize-Routing-Table and I-Could-Be-Router remain
//!   absent-only and cannot corroborate replacements.
//! - One pending challenger is retained per learned network: a different new
//!   port replaces the slot and starts fresh. A slot older than 60s expires on
//!   the next learning claim. Applying, current-port refresh, removal, aging,
//!   direct-route installation and manual table edits clear the slot. Pending
//!   entries are bounded by the number of live learned routes.
//! - I-Am-Router and Initialize-Routing-Table/ACK retain their existing route-cap
//!   checks; stale learned routes age out, and rapid port changes warn.
//! - Disconnect-Connection-To-Network never removes routes: PTP connections are
//!   unimplemented, matching Establish-Connection-To-Network's no-op handling.
//!   Each well-formed ignored removal request is debug-logged and counted.
//! - Reject-driven table transitions are dampened per (ingress port, network),
//!   not by spoofable source MAC or routed source address. No-op rejects are
//!   ignored, without renewing busy deadlines. A first state-changing reject
//!   applies; further changes within 30s of the last applied reject are ignored.
//!   Ignored claims do not extend that window. Learning refresh/replacement,
//!   removal and aging re-arm all ingress keys for that network.
//! - [`RouterTable::claim_snapshot`] exposes saturating count-only outcomes;
//!   counters never influence learning or forwarding.
//!
//! The private 30s hold-down aligns with the existing reject-busy deadline: one
//! busy marking buys at most one accepted reject-driven churn event per key per
//! 30s without fresh learning; legitimate re-signals after the window can apply.
//! **Trade-off:** a legitimate unreachable-after-busy signal can be delayed up
//! to 30s. This is local hardening, not a protocol authentication mechanism.
//! Active cross-port alternation can delay legitimate convergence by continually
//! replacing the pending challenger. The old route keeps forwarding; if that
//! path is truly dead, traffic waits for attack pause/expiry and fresh learning.
//! Learning can still re-arm reject dampening even when the refresh is malicious.
//! Forwarding, reject relay and flap-warning thresholds are unchanged; operators
//! still need a trusted, appropriately isolated network.

use std::sync::Arc;
use std::time::Duration;

use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_transport::port::{DataAttribute, TransportPort};
use bacnet_types::enums::{NetworkMessageType, RejectMessageReason};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{BufMut, Bytes, BytesMut};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::layer::{is_group_delivery, AdmissionReceiver, QueueAdmissionCounters, ReceivedApdu};
use crate::router_table::{ReachabilityStatus, RouterTable};

mod control_messages;
mod forwarding;

use control_messages::handle_network_message;
use forwarding::{forward_broadcast, forward_unicast, send_reject};

/// A send request to be forwarded on a port.
#[derive(Debug)]
enum SendRequest {
    Unicast {
        npdu: Bytes,
        mac: MacAddr,
        data_attributes: Vec<DataAttribute>,
    },
    Broadcast {
        npdu: Bytes,
        data_attributes: Vec<DataAttribute>,
    },
}

impl SendRequest {
    fn unicast(npdu: Bytes, mac: MacAddr) -> Self {
        Self::Unicast {
            npdu,
            mac,
            data_attributes: Vec::new(),
        }
    }

    fn broadcast(npdu: Bytes) -> Self {
        Self::Broadcast {
            npdu,
            data_attributes: Vec::new(),
        }
    }
}

/// A router port: a transport bound to a specific BACnet network number.
pub struct RouterPort<T: TransportPort> {
    /// The transport for this port.
    pub transport: T,
    /// The network number assigned to this port.
    pub network_number: u16,
}

/// BACnet router connecting multiple networks.
///
/// The router holds multiple ports, each bound to a different BACnet network.
/// When an NPDU arrives on one port with a destination network that maps to
/// another port, the router forwards the message.
/// See the [receive-queue contract](crate::layer#receive-queue-admission) for
/// raw/tracked local receivers, admission limits, counters and lifecycle.
pub struct BACnetRouter {
    /// Shared routing table.
    table: Arc<Mutex<RouterTable>>,
    /// Dispatch tasks (one per port).
    dispatch_tasks: Vec<JoinHandle<()>>,
    /// Sender tasks (one per port, owns the transport for outgoing messages).
    sender_tasks: Vec<JoinHandle<()>>,
    /// Background task that purges stale learned routes.
    aging_task: Option<JoinHandle<()>>,
}

impl BACnetRouter {
    /// Create and start a router from a list of ports.
    ///
    /// Returns the router and a receiver for APDUs destined to local
    /// applications (messages without remote destination or where this
    /// router is the final hop).
    /// All ports share one 256-item local queue. Full/Closed admission drops the
    /// arriving APDU with its payload, metadata and reply sender without sending
    /// reply bytes or a wire rejection, never evicting an older item. Admission
    /// never waits for the consumer or stops forwarding/inline control handling.
    /// This raw receiver has no per-source quota, admission snapshot or
    /// depth/high-water tracking. Full/Closed drops are counted internally, with
    /// Closed > Full precedence.
    ///
    /// Closing retains queued items; dropping discards them. Both leave routing
    /// active, and each later local admission attempt counts as Closed.
    /// [`Self::stop`] also leaves queued items drainable. Use
    /// [`Self::start_with_admission`] for snapshots and per-source fairness; see
    /// the [receive-queue contract](crate::layer#receive-queue-admission) for the
    /// raw/tracked matrix and ownership/lifecycle details.
    pub async fn start<T: TransportPort + 'static>(
        ports: Vec<RouterPort<T>>,
    ) -> Result<(Self, mpsc::Receiver<ReceivedApdu>), Error> {
        Self::start_dispatch(ports, false)
            .await
            .map(|(router, rx, _)| (router, rx))
    }

    /// Start with a local APDU receiver exposing queue-admission snapshots.
    ///
    /// This is an alternative to [`Self::start`], with the same port and router
    /// lifecycle. All ports share one 256-item queue and one accounting state;
    /// [`AdmissionReceiver::counters`] returns cloneable count-only handles for
    /// depth/high-water and admission-drop totals, readable after receiver or
    /// router drop. Failed admission drops the arriving APDU, never an older one, and
    /// releases its payload, metadata and reply sender without reply bytes or
    /// wire rejections. Forwarding and inline control handling remain independent.
    /// Closing the receiver retains queued items for draining and counts each
    /// subsequent local arrival as a closed drop while forwarding continues.
    /// [`Self::stop`] also leaves queued items drainable. Dropping the
    /// receiver discards queued items and releases their reply channels without
    /// sending reply bytes; discarding queued items is not an admission drop.
    ///
    /// Each ([ingress port network number](ReceivedApdu::ingress_network),
    /// complete [`source_mac`](ReceivedApdu::source_mac) byte value) may hold at
    /// most 16 queued local APDUs, never keyed by routed NPDU SNET/SADR.
    /// Identical MAC bytes on different ports have separate quotas.
    /// Dequeuing releases one slot, even if the consumer retains the APDU.
    /// **Closed > fairness > Full** selects one drop reason; over-quota arrivals
    /// increment [`fairness_drops`](crate::layer::QueueAdmissionSnapshot::fairness_drops)
    /// even if the queue is also globally full, unless admission is closed.
    /// The raw [`Self::start`] receiver does not enforce a quota. See the
    /// [receive-queue contract](crate::layer#receive-queue-admission) for exact
    /// keys, the raw/tracked matrix and complete ownership/lifecycle rules.
    pub async fn start_with_admission<T: TransportPort + 'static>(
        ports: Vec<RouterPort<T>>,
    ) -> Result<(Self, AdmissionReceiver<ReceivedApdu>), Error> {
        let (router, rx, counters) = Self::start_dispatch(ports, true).await?;
        Ok((router, AdmissionReceiver::from_apdu_parts(rx, counters)))
    }

    async fn start_dispatch<T: TransportPort + 'static>(
        mut ports: Vec<RouterPort<T>>,
        track_depth: bool,
    ) -> Result<(Self, mpsc::Receiver<ReceivedApdu>, QueueAdmissionCounters), Error> {
        let mut table = RouterTable::new();

        // Reject duplicate network numbers
        {
            let mut seen = std::collections::HashSet::new();
            for port in &ports {
                if !seen.insert(port.network_number) {
                    return Err(Error::Encoding(format!(
                        "Duplicate network number {} in router ports",
                        port.network_number
                    )));
                }
            }
        }

        // Register directly-connected networks
        for (idx, port) in ports.iter().enumerate() {
            table.add_direct(port.network_number, idx);
        }

        let table = Arc::new(Mutex::new(table));
        let (local_tx, local_rx, counters) = AdmissionReceiver::channel(track_depth);

        // Start each transport, set up send channels
        let mut port_receivers = Vec::new();
        let mut send_txs: Vec<mpsc::Sender<SendRequest>> = Vec::new();
        let mut sender_tasks = Vec::new();
        let mut port_networks = Vec::new();
        let mut port_local_macs = Vec::new();

        for port in &mut ports {
            let rx = port.transport.start().await?;
            port_receivers.push(rx);
            port_networks.push(port.network_number);
            port_local_macs.push(MacAddr::from_slice(port.transport.local_mac()));
        }

        // Move transports into sender tasks
        for port in ports {
            let (send_tx, mut send_rx) = mpsc::channel::<SendRequest>(256);
            send_txs.push(send_tx);

            let transport = port.transport;
            let task = tokio::spawn(async move {
                while let Some(req) = send_rx.recv().await {
                    match req {
                        SendRequest::Unicast {
                            npdu,
                            mac,
                            data_attributes,
                        } => {
                            if let Err(e) = transport
                                .send_unicast_with_data_attributes(&npdu, &mac, &data_attributes)
                                .await
                            {
                                warn!(error = %e, "Router send_unicast failed");
                            }
                        }
                        SendRequest::Broadcast {
                            npdu,
                            data_attributes,
                        } => {
                            if let Err(e) = transport
                                .send_broadcast_with_data_attributes(&npdu, &data_attributes)
                                .await
                            {
                                warn!(error = %e, "Router send_broadcast failed");
                            }
                        }
                    }
                }
            });
            sender_tasks.push(task);
        }

        let send_txs = Arc::new(send_txs);

        // Announce I-Am-Router-To-Network on each port listing networks reachable via other ports.
        for (port_idx, tx) in send_txs.iter().enumerate() {
            let other_networks: Vec<u16> = port_networks
                .iter()
                .enumerate()
                .filter(|(idx, _)| *idx != port_idx)
                .map(|(_, net)| *net)
                .collect();

            if other_networks.is_empty() {
                continue;
            }

            let mut payload = BytesMut::with_capacity(other_networks.len() * 2);
            for net in &other_networks {
                payload.put_u16(*net);
            }

            let payload_len = payload.len();
            let response = Npdu {
                is_network_message: true,
                message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw()),
                payload: payload.freeze(),
                ..Npdu::default()
            };

            let mut buf = BytesMut::with_capacity(8 + payload_len);
            if let Err(e) = encode_npdu(&mut buf, &response) {
                warn!("Failed to encode I-Am-Router NPDU: {e}");
                continue;
            }

            if let Err(e) = tx.try_send(SendRequest::broadcast(buf.freeze())) {
                warn!(%e, "Router dropped I-Am-Router announcement: output channel full");
            }
        }

        let mut dispatch_tasks = Vec::new();

        for (port_idx, mut rx) in port_receivers.into_iter().enumerate() {
            let table = Arc::clone(&table);
            let local_tx = local_tx.clone();
            let send_txs = Arc::clone(&send_txs);
            let port_network = port_networks[port_idx];
            let local_mac = port_local_macs[port_idx].clone();

            let task = tokio::spawn(async move {
                while let Some(received) = rx.recv().await {
                    match decode_npdu(received.npdu.clone()) {
                        Ok(npdu) => {
                            if npdu.is_network_message {
                                // Proprietary network messages (type >= 0x80) with DNET
                                // should be forwarded, not processed locally.
                                let is_proprietary =
                                    npdu.message_type.map(|t| t >= 0x80).unwrap_or(false);
                                let has_remote_dest = npdu
                                    .destination
                                    .as_ref()
                                    .is_some_and(|d| d.network != 0xFFFF);
                                if is_proprietary && has_remote_dest {
                                    // Fall through to normal DNET routing below
                                } else {
                                    handle_network_message(
                                        &table,
                                        &send_txs,
                                        port_idx,
                                        port_network,
                                        &received.source_mac,
                                        &npdu,
                                    )
                                    .await;
                                    continue;
                                }
                            }

                            if let Some(ref dest) = npdu.destination {
                                let dest_net = dest.network;

                                // Global broadcast — forward to all other ports
                                if dest_net == 0xFFFF {
                                    forward_broadcast(
                                        &send_txs,
                                        port_idx,
                                        port_network,
                                        &received.source_mac,
                                        &npdu,
                                        &received.data_attributes,
                                    );

                                    // Deliver locally as well
                                    let apdu = ReceivedApdu {
                                        apdu: npdu.payload,
                                        source_mac: received.source_mac,
                                        ingress_network: Some(port_network),
                                        source_network: npdu.source,
                                        link_layer_group: received.link_layer_group,
                                        is_group: true,
                                        data_attributes: received.data_attributes,
                                        reply_tx: received.reply_tx,
                                    };
                                    let _ = local_tx.try_send_apdu(apdu);
                                    continue;
                                }

                                // Route lookup for destination network
                                let (route, reachability) = {
                                    let mut tbl = table.lock().await;
                                    let route = tbl.lookup(dest_net).cloned();
                                    let reachability = tbl.effective_reachability(dest_net);
                                    if route.is_some() {
                                        tbl.touch(dest_net);
                                    }
                                    (route, reachability)
                                };

                                if let Some(route) = route {
                                    // Check reachability before forwarding (spec 6.6.3.6)
                                    match reachability.unwrap_or(ReachabilityStatus::Reachable) {
                                        ReachabilityStatus::Busy => {
                                            send_reject(
                                                &send_txs[port_idx],
                                                &received.source_mac,
                                                dest_net,
                                                RejectMessageReason::ROUTER_BUSY,
                                            );
                                            continue;
                                        }
                                        ReachabilityStatus::Unreachable => {
                                            send_reject(
                                                &send_txs[port_idx],
                                                &received.source_mac,
                                                dest_net,
                                                RejectMessageReason::NOT_DIRECTLY_CONNECTED,
                                            );
                                            continue;
                                        }
                                        ReachabilityStatus::Reachable => {}
                                    }
                                    if route.port_index == port_idx && route.directly_connected {
                                        let dest_mac = npdu
                                            .destination
                                            .as_ref()
                                            .map(|d| &d.mac_address[..])
                                            .unwrap_or(&[]);
                                        if dest_mac == &local_mac[..] {
                                            // DADR matches our MAC: deliver locally
                                            let apdu = ReceivedApdu {
                                                apdu: npdu.payload,
                                                source_mac: received.source_mac,
                                                ingress_network: Some(port_network),
                                                source_network: npdu.source,
                                                link_layer_group: received.link_layer_group,
                                                is_group: false,
                                                data_attributes: received.data_attributes,
                                                reply_tx: received.reply_tx,
                                            };
                                            let _ = local_tx.try_send_apdu(apdu);
                                        } else {
                                            // Remote broadcast to our network (DLEN=0):
                                            // deliver locally AND forward
                                            if dest_mac.is_empty() {
                                                let apdu = ReceivedApdu {
                                                    apdu: npdu.payload.clone(),
                                                    source_mac: received.source_mac.clone(),
                                                    ingress_network: Some(port_network),
                                                    source_network: npdu.source.clone(),
                                                    link_layer_group: received.link_layer_group,
                                                    is_group: true,
                                                    data_attributes: received
                                                        .data_attributes
                                                        .clone(),
                                                    reply_tx: None,
                                                };
                                                let _ = local_tx.try_send_apdu(apdu);
                                            }
                                            forward_unicast(
                                                &send_txs,
                                                &route,
                                                port_network,
                                                &received.source_mac,
                                                npdu,
                                                port_idx,
                                                &received.data_attributes,
                                            );
                                        }
                                    } else {
                                        forward_unicast(
                                            &send_txs,
                                            &route,
                                            port_network,
                                            &received.source_mac,
                                            npdu,
                                            port_idx,
                                            &received.data_attributes,
                                        );
                                    }
                                } else {
                                    // Unknown network: send reject
                                    send_reject(
                                        &send_txs[port_idx],
                                        &received.source_mac,
                                        dest_net,
                                        RejectMessageReason::NOT_DIRECTLY_CONNECTED,
                                    );
                                }
                            } else {
                                let apdu = ReceivedApdu {
                                    apdu: npdu.payload,
                                    source_mac: received.source_mac,
                                    ingress_network: Some(port_network),
                                    source_network: npdu.source,
                                    link_layer_group: received.link_layer_group,
                                    is_group: is_group_delivery(received.link_layer_group, None),
                                    data_attributes: received.data_attributes,
                                    reply_tx: received.reply_tx,
                                };
                                let _ = local_tx.try_send_apdu(apdu);
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, port = port_idx, "Router decode failed");
                        }
                    }
                }
            });

            dispatch_tasks.push(task);
        }

        // Periodically purge stale learned routes.
        let aging_table = Arc::clone(&table);
        let aging_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            let max_age = Duration::from_secs(300); // 5 minutes
            loop {
                interval.tick().await;
                let mut tbl = aging_table.lock().await;
                let purged = tbl.purge_stale(max_age);
                tbl.clear_expired_busy();
                drop(tbl);
                for net in purged {
                    debug!(network = net, "Purged stale route");
                }
            }
        });

        Ok((
            Self {
                table,
                dispatch_tasks,
                sender_tasks,
                aging_task: Some(aging_task),
            },
            local_rx,
            counters,
        ))
    }

    /// Get a reference to the routing table.
    pub fn table(&self) -> &Arc<Mutex<RouterTable>> {
        &self.table
    }

    /// Stop the router.
    ///
    /// Aborts and awaits dispatch, sender and aging tasks without waiting for
    /// the local application consumer, even with a full queue. Queued local
    /// APDUs remain drainable, retaining their reply senders until consumed or
    /// the receiver is dropped. Stop is not an admission drop and does not clear
    /// depth/high-water/drop totals. See the
    /// [receive-queue contract](crate::layer#receive-queue-admission).
    pub async fn stop(&mut self) {
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
    }
}

#[cfg(test)]
mod admission_tests;
#[cfg(test)]
mod claim_tests;
#[cfg(test)]
mod tests;
