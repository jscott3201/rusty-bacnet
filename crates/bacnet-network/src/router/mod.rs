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

use std::sync::Arc;
use std::time::Duration;

use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{NetworkMessageType, RejectMessageReason};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{BufMut, Bytes, BytesMut};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::layer::ReceivedApdu;
use crate::router_table::{ReachabilityStatus, RouterTable};

mod control_messages;
mod forwarding;

use control_messages::handle_network_message;
use forwarding::{forward_broadcast, forward_unicast, send_reject};

/// A send request to be forwarded on a port.
#[derive(Debug)]
enum SendRequest {
    Unicast { npdu: Bytes, mac: MacAddr },
    Broadcast { npdu: Bytes },
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
    pub async fn start<T: TransportPort + 'static>(
        mut ports: Vec<RouterPort<T>>,
    ) -> Result<(Self, mpsc::Receiver<ReceivedApdu>), Error> {
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
        let (local_tx, local_rx) = mpsc::channel(256);

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
                        SendRequest::Unicast { npdu, mac } => {
                            if let Err(e) = transport.send_unicast(&npdu, &mac).await {
                                warn!(error = %e, "Router send_unicast failed");
                            }
                        }
                        SendRequest::Broadcast { npdu } => {
                            if let Err(e) = transport.send_broadcast(&npdu).await {
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

            if let Err(e) = tx.try_send(SendRequest::Broadcast { npdu: buf.freeze() }) {
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
                                    );

                                    // Deliver locally as well
                                    let apdu = ReceivedApdu {
                                        apdu: npdu.payload,
                                        source_mac: received.source_mac,
                                        source_network: npdu.source,
                                        reply_tx: received.reply_tx,
                                    };
                                    let _ = local_tx.send(apdu).await;
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
                                                source_network: npdu.source,
                                                reply_tx: received.reply_tx,
                                            };
                                            let _ = local_tx.send(apdu).await;
                                        } else {
                                            // Remote broadcast to our network (DLEN=0):
                                            // deliver locally AND forward
                                            if dest_mac.is_empty() {
                                                let apdu = ReceivedApdu {
                                                    apdu: npdu.payload.clone(),
                                                    source_mac: received.source_mac.clone(),
                                                    source_network: npdu.source.clone(),
                                                    reply_tx: None,
                                                };
                                                let _ = local_tx.send(apdu).await;
                                            }
                                            forward_unicast(
                                                &send_txs,
                                                &route,
                                                port_network,
                                                &received.source_mac,
                                                npdu,
                                                port_idx,
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
                                    source_network: npdu.source,
                                    reply_tx: received.reply_tx,
                                };
                                let _ = local_tx.send(apdu).await;
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
        ))
    }

    /// Get a reference to the routing table.
    pub fn table(&self) -> &Arc<Mutex<RouterTable>> {
        &self.table
    }

    /// Stop the router.
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
mod tests;
