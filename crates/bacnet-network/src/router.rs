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

use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{NetworkMessageType, RejectMessageReason};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{BufMut, Bytes, BytesMut};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::layer::ReceivedApdu;
use crate::router_table::RouterTable;

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
                                let route = {
                                    let tbl = table.lock().await;
                                    tbl.lookup(dest_net).cloned()
                                };

                                if let Some(route) = route {
                                    if route.port_index == port_idx
                                        && route.directly_connected
                                        && npdu
                                            .destination
                                            .as_ref()
                                            .is_some_and(|d| d.mac_address == local_mac)
                                    {
                                        // DADR matches our MAC: deliver locally
                                        let apdu = ReceivedApdu {
                                            apdu: npdu.payload,
                                            source_mac: received.source_mac,
                                            source_network: npdu.source,
                                            reply_tx: received.reply_tx,
                                        };
                                        let _ = local_tx.send(apdu).await;
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
                let purged = aging_table.lock().await.purge_stale(max_age);
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

/// Build the source NpduAddress for a forwarded message.
fn build_source(npdu: &Npdu, source_network: u16, source_mac: &[u8]) -> NpduAddress {
    npdu.source.clone().unwrap_or(NpduAddress {
        network: source_network,
        mac_address: MacAddr::from_slice(source_mac),
    })
}

/// Forward a message to a specific destination via the route entry.
fn forward_unicast(
    send_txs: &[mpsc::Sender<SendRequest>],
    route: &crate::router_table::RouteEntry,
    source_network: u16,
    source_mac: &[u8],
    npdu: Npdu,
    _source_port_idx: usize,
) {
    if npdu.hop_count == 0 {
        warn!("Discarding NPDU with hop_count=0");
        return;
    }

    let payload_len = npdu.payload.len();
    let source = build_source(&npdu, source_network, source_mac);
    let dest_mac;
    let forwarded_dest;
    let forwarded_hop_count;

    if route.directly_connected {
        // Directly connected: strip DNET/DADR/Hop Count from NPCI, send to DADR.
        dest_mac = npdu
            .destination
            .as_ref()
            .map(|d| d.mac_address.clone())
            .unwrap_or_default();
        forwarded_dest = None;
        forwarded_hop_count = 0; // not used without destination
    } else {
        dest_mac = route.next_hop_mac.clone();
        forwarded_dest = npdu.destination;
        forwarded_hop_count = npdu.hop_count - 1;
    };

    let forwarded = Npdu {
        is_network_message: false,
        expecting_reply: npdu.expecting_reply,
        priority: npdu.priority,
        destination: forwarded_dest,
        source: Some(source),
        hop_count: forwarded_hop_count,
        message_type: None,
        vendor_id: None,
        payload: npdu.payload,
    };

    let mut buf = BytesMut::with_capacity(32 + payload_len);
    if let Err(e) = encode_npdu(&mut buf, &forwarded) {
        warn!("Failed to encode forwarded NPDU: {e}");
        return;
    }

    if route.port_index >= send_txs.len() {
        warn!(
            port = route.port_index,
            "Route references invalid port index"
        );
        return;
    }
    if dest_mac.is_empty() {
        if let Err(e) =
            send_txs[route.port_index].try_send(SendRequest::Broadcast { npdu: buf.freeze() })
        {
            warn!(%e, "Router dropped message: output channel full");
        }
    } else if let Err(e) = send_txs[route.port_index].try_send(SendRequest::Unicast {
        npdu: buf.freeze(),
        mac: dest_mac,
    }) {
        warn!(%e, "Router dropped message: output channel full");
    }
}

/// Forward a global broadcast to all ports except the source port.
fn forward_broadcast(
    send_txs: &[mpsc::Sender<SendRequest>],
    source_port: usize,
    source_network: u16,
    source_mac: &[u8],
    npdu: &Npdu,
) {
    if npdu.hop_count == 0 {
        warn!("Discarding NPDU with hop_count=0");
        return;
    }

    let forwarded = Npdu {
        is_network_message: false,
        expecting_reply: npdu.expecting_reply,
        priority: npdu.priority,
        destination: npdu.destination.clone(),
        source: Some(build_source(npdu, source_network, source_mac)),
        hop_count: npdu.hop_count - 1,
        message_type: None,
        vendor_id: None,
        payload: npdu.payload.clone(),
    };

    let mut buf = BytesMut::with_capacity(32 + npdu.payload.len());
    if let Err(e) = encode_npdu(&mut buf, &forwarded) {
        warn!("Failed to encode forwarded broadcast NPDU: {e}");
        return;
    }

    let encoded = buf.freeze();
    for (idx, tx) in send_txs.iter().enumerate() {
        if idx == source_port {
            continue;
        }
        if let Err(e) = tx.try_send(SendRequest::Broadcast {
            npdu: encoded.clone(),
        }) {
            warn!(%e, "Router dropped broadcast: output channel full");
        }
    }
}

/// Handle a network-layer message.
async fn handle_network_message(
    table: &Arc<Mutex<RouterTable>>,
    send_txs: &[mpsc::Sender<SendRequest>],
    port_idx: usize,
    port_network: u16,
    source_mac: &[u8],
    npdu: &Npdu,
) {
    const MAX_LEARNED_ROUTES: usize = 256;

    let msg_type = match npdu.message_type {
        Some(t) => t,
        None => return,
    };

    if msg_type == NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw() {
        let table = table.lock().await;

        let requested_network = if npdu.payload.len() >= 2 {
            Some(u16::from_be_bytes([npdu.payload[0], npdu.payload[1]]))
        } else {
            None
        };

        let networks: Vec<u16> = if let Some(net) = requested_network {
            // Only respond if the network is reachable via a different port.
            match table.lookup(net) {
                Some(entry) if entry.port_index != port_idx => vec![net],
                _ => {
                    // Unknown: forward Who-Is-Router to all other ports to discover the path.
                    drop(table);
                    let forward = Npdu {
                        is_network_message: true,
                        message_type: Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw()),
                        payload: npdu.payload.clone(),
                        ..Npdu::default()
                    };
                    let mut fwd_buf = BytesMut::with_capacity(8);
                    if let Ok(()) = encode_npdu(&mut fwd_buf, &forward) {
                        let frozen = fwd_buf.freeze();
                        for (i, tx) in send_txs.iter().enumerate() {
                            if i != port_idx {
                                let _ = tx.try_send(SendRequest::Broadcast {
                                    npdu: frozen.clone(),
                                });
                            }
                        }
                    }
                    return;
                }
            }
        } else {
            table.networks_not_on_port(port_idx)
        };

        if networks.is_empty() {
            return;
        }

        let mut payload = BytesMut::with_capacity(networks.len() * 2);
        for net in &networks {
            payload.put_u16(*net);
        }

        let payload_len = payload.len();
        let response = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(4 + payload_len);
        if let Err(e) = encode_npdu(&mut buf, &response) {
            warn!("Failed to encode I-Am-Router response NPDU: {e}");
            return;
        }

        // I-Am-Router-To-Network is always broadcast.
        if let Err(e) = send_txs[port_idx].try_send(SendRequest::Broadcast { npdu: buf.freeze() }) {
            warn!(%e, "Router dropped I-Am-Router response: output channel full");
        }
    } else if msg_type == NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw() {
        const LEARNED_ROUTE_MAX_AGE: Duration = Duration::from_secs(300);

        let data = &npdu.payload;
        let mut offset = 0;
        let mut any_new = false;
        let mut table = table.lock().await;

        while offset + 2 <= data.len() {
            let net = u16::from_be_bytes([data[offset], data[offset + 1]]);
            offset += 2;

            if table.len() >= MAX_LEARNED_ROUTES && table.lookup(net).is_none() {
                warn!("Router table learned routes cap ({MAX_LEARNED_ROUTES}) reached, ignoring further networks");
                break;
            }

            if table.add_learned_stable(
                net,
                port_idx,
                MacAddr::from_slice(source_mac),
                LEARNED_ROUTE_MAX_AGE,
            ) {
                any_new = true;
                debug!(
                    network = net,
                    port = port_idx,
                    "Learned route from I-Am-Router-To-Network"
                );
            }
        }
        drop(table);

        // Re-broadcast to other ports only if new routes were learned (prevents loops).
        if any_new && !npdu.payload.is_empty() {
            let rebroadcast = Npdu {
                is_network_message: true,
                message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw()),
                payload: npdu.payload.clone(),
                ..Npdu::default()
            };
            let mut buf = BytesMut::with_capacity(4 + npdu.payload.len());
            if let Ok(()) = encode_npdu(&mut buf, &rebroadcast) {
                let frozen = buf.freeze();
                for (i, tx) in send_txs.iter().enumerate() {
                    if i != port_idx {
                        let _ = tx.try_send(SendRequest::Broadcast {
                            npdu: frozen.clone(),
                        });
                    }
                }
            }
        }
    } else if msg_type == NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw() {
        if npdu.payload.len() >= 3 {
            let reason = npdu.payload[0];
            let rejected_net = u16::from_be_bytes([npdu.payload[1], npdu.payload[2]]);
            warn!(
                network = rejected_net,
                reason = reason,
                "Received Reject-Message-To-Network"
            );
            {
                let mut tbl = table.lock().await;
                if let Some(entry) = tbl.lookup(rejected_net) {
                    if !entry.directly_connected {
                        tbl.remove(rejected_net);
                    }
                }
            }

            // Relay the reject to the originating node if SNET/SADR is present.
            if let Some(ref source) = npdu.source {
                let tbl = table.lock().await;
                if let Some(route) = tbl.lookup(source.network) {
                    let dest_port = route.port_index;
                    let dest_mac = if route.directly_connected {
                        source.mac_address.clone()
                    } else {
                        route.next_hop_mac.clone()
                    };
                    drop(tbl);

                    let forwarded = Npdu {
                        is_network_message: true,
                        message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw()),
                        destination: Some(NpduAddress {
                            network: source.network,
                            mac_address: source.mac_address.clone(),
                        }),
                        hop_count: 255,
                        payload: npdu.payload.clone(),
                        ..Npdu::default()
                    };
                    let mut buf = BytesMut::with_capacity(32);
                    if let Ok(()) = encode_npdu(&mut buf, &forwarded) {
                        if dest_mac.is_empty() {
                            let _ = send_txs[dest_port]
                                .try_send(SendRequest::Broadcast { npdu: buf.freeze() });
                        } else {
                            let _ = send_txs[dest_port].try_send(SendRequest::Unicast {
                                npdu: buf.freeze(),
                                mac: dest_mac,
                            });
                        }
                    }
                }
            }
        }
    } else if msg_type == NetworkMessageType::ROUTER_BUSY_TO_NETWORK.to_raw() {
        let data = &npdu.payload;
        let mut offset = 0;
        let mut tbl = table.lock().await;
        while offset + 2 <= data.len() {
            let net = u16::from_be_bytes([data[offset], data[offset + 1]]);
            offset += 2;
            if let Some(entry) = tbl.lookup_mut(net) {
                entry.reachability = crate::router_table::ReachabilityStatus::Busy;
            }
            debug!(network = net, "Router busy — marked network as congested");
        }
    } else if msg_type == NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK.to_raw() {
        let data = &npdu.payload;
        let mut offset = 0;
        let mut tbl = table.lock().await;
        while offset + 2 <= data.len() {
            let net = u16::from_be_bytes([data[offset], data[offset + 1]]);
            offset += 2;
            if let Some(entry) = tbl.lookup_mut(net) {
                entry.reachability = crate::router_table::ReachabilityStatus::Reachable;
            }
            debug!(network = net, "Router available — cleared congestion");
        }
    } else if msg_type == NetworkMessageType::INITIALIZE_ROUTING_TABLE.to_raw() {
        let data = &npdu.payload;
        let count = if data.is_empty() { 0 } else { data[0] as usize };

        let is_query = count == 0;

        if !is_query {
            let mut offset = 1usize;
            let mut tbl = table.lock().await;
            for _ in 0..count {
                if offset + 4 > data.len() {
                    break;
                }
                let net = u16::from_be_bytes([data[offset], data[offset + 1]]);
                // skip port_id (1 byte)
                let info_len = data[offset + 3] as usize;
                if offset + 4 + info_len > data.len() {
                    break;
                }
                offset += 4 + info_len;

                if net == 0 || net == 0xFFFF {
                    continue;
                }
                if tbl.lookup(net).is_some() {
                    continue; // don't overwrite existing routes
                }
                if tbl.len() >= MAX_LEARNED_ROUTES {
                    warn!("Init-Routing-Table: route cap reached, ignoring further entries");
                    break;
                }
                tbl.add_learned(net, port_idx, MacAddr::from_slice(source_mac));
                debug!(
                    network = net,
                    port = port_idx,
                    "Learned route from Init-Routing-Table"
                );
            }
        }

        let mut payload = BytesMut::new();
        if is_query {
            let tbl = table.lock().await;
            let networks = tbl.networks();
            let count = networks.len().min(255);
            payload.put_u8(count as u8);
            for net in networks.iter().take(count) {
                if tbl.lookup(*net).is_some() {
                    payload.put_u16(*net);
                    payload.put_u8(0); // Port ID (simplified)
                    payload.put_u8(0); // Port info length
                }
            }
        } else {
            payload.put_u8(0);
        }

        let payload_len = payload.len();
        let response = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(8 + payload_len);
        if let Err(e) = encode_npdu(&mut buf, &response) {
            warn!("Failed to encode Init-Routing-Table-ACK NPDU: {e}");
            return;
        }

        if let Err(e) = send_txs[port_idx].try_send(SendRequest::Unicast {
            npdu: buf.freeze(),
            mac: MacAddr::from_slice(source_mac),
        }) {
            warn!(%e, "Router dropped Init-Routing-Table-ACK: output channel full");
        }
    } else if msg_type == NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK.to_raw() {
        if npdu.payload.len() >= 3 {
            let net = u16::from_be_bytes([npdu.payload[0], npdu.payload[1]]);
            let performance_index = npdu.payload[2];
            debug!(
                network = net,
                performance_index = performance_index,
                port = port_idx,
                "Received I-Could-Be-Router-To-Network"
            );
            // Store only if no existing route (lower priority than direct/learned).
            let mut tbl = table.lock().await;
            if tbl.lookup(net).is_none() {
                tbl.add_learned(net, port_idx, MacAddr::from_slice(source_mac));
                debug!(
                    network = net,
                    port = port_idx,
                    "Stored potential route from I-Could-Be-Router-To-Network"
                );
            }
        }
    } else if msg_type == NetworkMessageType::ESTABLISH_CONNECTION_TO_NETWORK.to_raw() {
        if npdu.payload.len() >= 3 {
            let net = u16::from_be_bytes([npdu.payload[0], npdu.payload[1]]);
            let termination_time_min = npdu.payload[2];
            debug!(
                network = net,
                termination_time_minutes = termination_time_min,
                "Received Establish-Connection-To-Network"
            );
        }
    } else if msg_type == NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK.to_raw() {
        if npdu.payload.len() >= 2 {
            let net = u16::from_be_bytes([npdu.payload[0], npdu.payload[1]]);
            debug!(network = net, "Received Disconnect-Connection-To-Network");
            let mut tbl = table.lock().await;
            if let Some(entry) = tbl.lookup(net) {
                if !entry.directly_connected {
                    tbl.remove(net);
                    debug!(
                        network = net,
                        "Removed dynamically established route on disconnect"
                    );
                }
            }
        }
    } else if msg_type == NetworkMessageType::WHAT_IS_NETWORK_NUMBER.to_raw() {
        // Ignore if SNET/SADR or DNET/DADR is present.
        if npdu.source.is_some() || npdu.destination.is_some() {
            return;
        }
        let mut payload = BytesMut::with_capacity(3);
        payload.put_u16(port_network);
        payload.put_u8(1); // configured

        let response = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::NETWORK_NUMBER_IS.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(8);
        if let Err(e) = encode_npdu(&mut buf, &response) {
            warn!("Failed to encode Network-Number-Is NPDU: {e}");
            return;
        }

        if let Err(e) = send_txs[port_idx].try_send(SendRequest::Broadcast { npdu: buf.freeze() }) {
            warn!(%e, "Router dropped Network-Number-Is: output channel full");
        }
    } else {
        debug!(
            message_type = msg_type,
            "Router ignoring unhandled network message"
        );
    }
}

/// Send a Reject-Message-To-Network.
fn send_reject(
    send_tx: &mpsc::Sender<SendRequest>,
    source_mac: &[u8],
    rejected_network: u16,
    reason: RejectMessageReason,
) {
    let mut payload = BytesMut::with_capacity(3);
    payload.put_u8(reason.to_raw());
    payload.put_u16(rejected_network);

    let reject = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    let mut buf = BytesMut::with_capacity(8);
    if let Err(e) = encode_npdu(&mut buf, &reject) {
        warn!("Failed to encode Reject-Message NPDU: {e}");
        return;
    }

    if let Err(e) = send_tx.try_send(SendRequest::Unicast {
        npdu: buf.freeze(),
        mac: MacAddr::from_slice(source_mac),
    }) {
        warn!(%e, "Router dropped reject message: output channel full");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_transport::bip::BipTransport;
    use std::net::Ipv4Addr;
    use tokio::time::Duration;

    #[tokio::test]
    async fn router_forwards_between_networks() {
        let transport_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

        let mut device_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let mut device_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

        let _rx_b = device_b.start().await.unwrap();
        let _rx_a = device_a.start().await.unwrap();

        let port_a = RouterPort {
            transport: transport_a,
            network_number: 1000,
        };
        let port_b = RouterPort {
            transport: transport_b,
            network_number: 2000,
        };

        let (mut router, _local_rx) = BACnetRouter::start(vec![port_a, port_b]).await.unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let apdu = vec![0x10, 0x08];
        let npdu = Npdu {
            is_network_message: false,
            expecting_reply: false,
            priority: bacnet_types::enums::NetworkPriority::NORMAL,
            destination: Some(NpduAddress {
                network: 2000,
                mac_address: MacAddr::from_slice(device_b.local_mac()),
            }),
            source: None,
            hop_count: 255,
            payload: Bytes::copy_from_slice(&apdu),
            ..Npdu::default()
        };

        let mut buf = BytesMut::new();
        encode_npdu(&mut buf, &npdu).unwrap();

        let table = router.table().lock().await;
        assert_eq!(table.len(), 2);
        assert!(table.lookup(1000).unwrap().directly_connected);
        assert!(table.lookup(2000).unwrap().directly_connected);
        drop(table);

        router.stop().await;
    }

    #[tokio::test]
    async fn router_table_populated_on_start() {
        let transport_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let transport_c = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

        let ports = vec![
            RouterPort {
                transport: transport_a,
                network_number: 100,
            },
            RouterPort {
                transport: transport_b,
                network_number: 200,
            },
            RouterPort {
                transport: transport_c,
                network_number: 300,
            },
        ];

        let (mut router, _local_rx) = BACnetRouter::start(ports).await.unwrap();

        let table = router.table().lock().await;
        assert_eq!(table.len(), 3);
        assert_eq!(table.lookup(100).unwrap().port_index, 0);
        assert_eq!(table.lookup(200).unwrap().port_index, 1);
        assert_eq!(table.lookup(300).unwrap().port_index, 2);
        drop(table);

        router.stop().await;
    }

    #[tokio::test]
    async fn local_message_delivered_to_application() {
        let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let mut sender = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let _sender_rx = sender.start().await.unwrap();

        let router_port = RouterPort {
            transport,
            network_number: 1000,
        };

        let (mut router, _local_rx) = BACnetRouter::start(vec![router_port]).await.unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        router.stop().await;
    }

    #[test]
    fn forward_unicast_drops_hop_count_zero() {
        let (tx_a, mut rx_a) = mpsc::channel::<SendRequest>(256);
        let (tx_b, mut rx_b) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx_a, tx_b];

        let route = crate::router_table::RouteEntry {
            port_index: 1,
            directly_connected: true,
            next_hop_mac: MacAddr::new(),
            last_seen: None,
            reachability: crate::router_table::ReachabilityStatus::Reachable,
        };

        let npdu = Npdu {
            is_network_message: false,
            expecting_reply: false,
            priority: bacnet_types::enums::NetworkPriority::NORMAL,
            destination: Some(NpduAddress {
                network: 2000,
                mac_address: MacAddr::from_slice(&[0x01, 0x02]),
            }),
            source: None,
            hop_count: 0, // Should cause the message to be dropped
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Npdu::default()
        };

        forward_unicast(&send_txs, &route, 1000, &[0x0A], npdu, 0);

        assert!(rx_a.try_recv().is_err());
        assert!(rx_b.try_recv().is_err());
    }

    #[test]
    fn forward_broadcast_drops_hop_count_zero() {
        let (tx_a, mut rx_a) = mpsc::channel::<SendRequest>(256);
        let (tx_b, mut rx_b) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx_a, tx_b];

        let npdu = Npdu {
            is_network_message: false,
            expecting_reply: false,
            priority: bacnet_types::enums::NetworkPriority::NORMAL,
            destination: Some(NpduAddress {
                network: 0xFFFF,
                mac_address: MacAddr::new(),
            }),
            source: None,
            hop_count: 0, // Should cause broadcast to be dropped
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Npdu::default()
        };

        forward_broadcast(&send_txs, 0, 1000, &[0x0A], &npdu);

        assert!(rx_a.try_recv().is_err());
        assert!(rx_b.try_recv().is_err());
    }

    #[test]
    fn forward_unicast_decrements_hop_count() {
        let (tx_a, _rx_a) = mpsc::channel::<SendRequest>(256);
        let (tx_b, mut rx_b) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx_a, tx_b];

        let route = crate::router_table::RouteEntry {
            port_index: 1,
            directly_connected: true,
            next_hop_mac: MacAddr::new(),
            last_seen: None,
            reachability: crate::router_table::ReachabilityStatus::Reachable,
        };

        let npdu = Npdu {
            is_network_message: false,
            expecting_reply: false,
            priority: bacnet_types::enums::NetworkPriority::NORMAL,
            destination: Some(NpduAddress {
                network: 2000,
                mac_address: MacAddr::from_slice(&[0x01, 0x02]),
            }),
            source: None,
            hop_count: 10,
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Npdu::default()
        };

        forward_unicast(&send_txs, &route, 1000, &[0x0A], npdu, 0);

        let sent = rx_b.try_recv().unwrap();
        match sent {
            SendRequest::Unicast { npdu: data, .. } => {
                let decoded = decode_npdu(data.clone()).unwrap();
                assert!(decoded.destination.is_none());
                assert!(decoded.source.is_some());
            }
            SendRequest::Broadcast { npdu: data } => {
                let decoded = decode_npdu(data.clone()).unwrap();
                assert!(decoded.destination.is_none());
            }
        }
    }

    #[test]
    fn send_reject_generates_reject_message() {
        let (tx, mut rx) = mpsc::channel::<SendRequest>(256);

        let source_mac = vec![0x0A, 0x00, 0x01, 0x01];
        let unknown_network: u16 = 9999;

        send_reject(
            &tx,
            &source_mac,
            unknown_network,
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
        );

        let sent = rx.try_recv().unwrap();
        match sent {
            SendRequest::Unicast { npdu: data, mac } => {
                assert_eq!(mac.as_slice(), &source_mac[..]);
                let decoded = decode_npdu(data.clone()).unwrap();
                assert!(decoded.is_network_message);
                assert_eq!(
                    decoded.message_type,
                    Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw())
                );
                assert_eq!(decoded.payload.len(), 3);
                assert_eq!(
                    decoded.payload[0],
                    RejectMessageReason::NOT_DIRECTLY_CONNECTED.to_raw()
                );
                let rejected_net = u16::from_be_bytes([decoded.payload[1], decoded.payload[2]]);
                assert_eq!(rejected_net, 9999);
            }
            _ => panic!("Expected Unicast send for reject message"),
        }
    }

    #[tokio::test]
    async fn single_port_router_no_i_am_router_announcement() {
        let (send_tx, mut send_rx) = mpsc::channel::<SendRequest>(256);

        let port_networks: Vec<u16> = vec![1000];
        let send_txs = [send_tx];

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
            encode_npdu(&mut buf, &response).unwrap();

            let _ = tx.try_send(SendRequest::Broadcast { npdu: buf.freeze() });
        }

        assert!(send_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn two_port_router_sends_i_am_router_announcement() {
        let (tx_a, mut rx_a) = mpsc::channel::<SendRequest>(256);
        let (tx_b, mut rx_b) = mpsc::channel::<SendRequest>(256);

        let port_networks: Vec<u16> = vec![1000, 2000];
        let send_txs = [tx_a, tx_b];

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
            encode_npdu(&mut buf, &response).unwrap();

            let _ = tx.try_send(SendRequest::Broadcast { npdu: buf.freeze() });
        }

        let sent_a = rx_a.try_recv().unwrap();
        match sent_a {
            SendRequest::Broadcast { npdu: data } => {
                let decoded = decode_npdu(data.clone()).unwrap();
                assert!(decoded.is_network_message);
                assert_eq!(
                    decoded.message_type,
                    Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw())
                );
                assert_eq!(decoded.payload.len(), 2);
                let net = u16::from_be_bytes([decoded.payload[0], decoded.payload[1]]);
                assert_eq!(net, 2000);
            }
            _ => panic!("Expected Broadcast for I-Am-Router announcement on port A"),
        }

        let sent_b = rx_b.try_recv().unwrap();
        match sent_b {
            SendRequest::Broadcast { npdu: data } => {
                let decoded = decode_npdu(data.clone()).unwrap();
                assert!(decoded.is_network_message);
                assert_eq!(
                    decoded.message_type,
                    Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw())
                );
                assert_eq!(decoded.payload.len(), 2);
                let net = u16::from_be_bytes([decoded.payload[0], decoded.payload[1]]);
                assert_eq!(net, 1000);
            }
            _ => panic!("Expected Broadcast for I-Am-Router announcement on port B"),
        }
    }

    #[tokio::test]
    async fn three_port_router_announces_multiple_networks() {
        let (tx_a, mut rx_a) = mpsc::channel::<SendRequest>(256);
        let (tx_b, mut rx_b) = mpsc::channel::<SendRequest>(256);
        let (tx_c, mut rx_c) = mpsc::channel::<SendRequest>(256);

        let port_networks: Vec<u16> = vec![100, 200, 300];
        let send_txs = [tx_a, tx_b, tx_c];

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
            encode_npdu(&mut buf, &response).unwrap();

            let _ = tx.try_send(SendRequest::Broadcast { npdu: buf.freeze() });
        }

        let sent_a = rx_a.try_recv().unwrap();
        match sent_a {
            SendRequest::Broadcast { npdu: data } => {
                let decoded = decode_npdu(data.clone()).unwrap();
                assert!(decoded.is_network_message);
                assert_eq!(decoded.payload.len(), 4); // two u16 values
                let net1 = u16::from_be_bytes([decoded.payload[0], decoded.payload[1]]);
                let net2 = u16::from_be_bytes([decoded.payload[2], decoded.payload[3]]);
                assert_eq!(net1, 200);
                assert_eq!(net2, 300);
            }
            _ => panic!("Expected Broadcast on port A"),
        }

        let sent_b = rx_b.try_recv().unwrap();
        match sent_b {
            SendRequest::Broadcast { npdu: data } => {
                let decoded = decode_npdu(data.clone()).unwrap();
                assert_eq!(decoded.payload.len(), 4);
                let net1 = u16::from_be_bytes([decoded.payload[0], decoded.payload[1]]);
                let net2 = u16::from_be_bytes([decoded.payload[2], decoded.payload[3]]);
                assert_eq!(net1, 100);
                assert_eq!(net2, 300);
            }
            _ => panic!("Expected Broadcast on port B"),
        }

        let sent_c = rx_c.try_recv().unwrap();
        match sent_c {
            SendRequest::Broadcast { npdu: data } => {
                let decoded = decode_npdu(data.clone()).unwrap();
                assert_eq!(decoded.payload.len(), 4);
                let net1 = u16::from_be_bytes([decoded.payload[0], decoded.payload[1]]);
                let net2 = u16::from_be_bytes([decoded.payload[2], decoded.payload[3]]);
                assert_eq!(net1, 100);
                assert_eq!(net2, 200);
            }
            _ => panic!("Expected Broadcast on port C"),
        }
    }

    #[test]
    fn forward_unicast_with_hop_count_one_still_forwards() {
        let (tx_a, _rx_a) = mpsc::channel::<SendRequest>(256);
        let (tx_b, mut rx_b) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx_a, tx_b];

        let route = crate::router_table::RouteEntry {
            port_index: 1,
            directly_connected: true,
            next_hop_mac: MacAddr::new(),
            last_seen: None,
            reachability: crate::router_table::ReachabilityStatus::Reachable,
        };

        let npdu = Npdu {
            is_network_message: false,
            expecting_reply: false,
            priority: bacnet_types::enums::NetworkPriority::NORMAL,
            destination: Some(NpduAddress {
                network: 2000,
                mac_address: MacAddr::from_slice(&[0x01, 0x02]),
            }),
            source: None,
            hop_count: 1,
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Npdu::default()
        };

        forward_unicast(&send_txs, &route, 1000, &[0x0A], npdu, 0);

        let sent = rx_b.try_recv().unwrap();
        match sent {
            SendRequest::Unicast { npdu: data, .. } => {
                let decoded = decode_npdu(data.clone()).unwrap();
                assert!(decoded.destination.is_none());
                assert!(decoded.source.is_some());
            }
            SendRequest::Broadcast { npdu: data } => {
                let decoded = decode_npdu(data.clone()).unwrap();
                assert!(decoded.destination.is_none());
            }
        }
    }

    #[tokio::test]
    async fn received_reject_removes_learned_route() {
        let mut table = RouterTable::new();
        table.add_direct(1000, 0);
        table.add_learned(3000, 0, MacAddr::from_slice(&[10, 0, 1, 1]));
        assert!(table.lookup(3000).is_some());

        let table = Arc::new(Mutex::new(table));

        let (tx, _rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        let mut payload = BytesMut::with_capacity(3);
        payload.put_u8(RejectMessageReason::OTHER.to_raw());
        payload.put_u16(3000);

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

        let tbl = table.lock().await;
        assert!(tbl.lookup(3000).is_none());
        assert!(tbl.lookup(1000).is_some());
    }

    #[tokio::test]
    async fn received_reject_does_not_remove_direct_route() {
        let mut table = RouterTable::new();
        table.add_direct(1000, 0);

        let table = Arc::new(Mutex::new(table));

        let (tx, _rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];
        let mut payload = BytesMut::with_capacity(3);
        payload.put_u8(RejectMessageReason::OTHER.to_raw());
        payload.put_u16(1000);

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

        let tbl = table.lock().await;
        assert!(tbl.lookup(1000).is_some());
    }

    #[tokio::test]
    async fn who_is_router_with_specific_network() {
        let mut table = RouterTable::new();
        table.add_direct(1000, 0);
        table.add_direct(2000, 1);
        table.add_direct(3000, 2);

        let table = Arc::new(Mutex::new(table));

        let (tx, mut rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        let mut req_payload = BytesMut::with_capacity(2);
        req_payload.put_u16(2000);

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw()),
            payload: req_payload.freeze(),
            ..Npdu::default()
        };

        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

        let sent = rx.try_recv().unwrap();
        match sent {
            SendRequest::Broadcast { npdu: data } => {
                let decoded = decode_npdu(data.clone()).unwrap();
                assert!(decoded.is_network_message);
                assert_eq!(
                    decoded.message_type,
                    Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw())
                );
                assert_eq!(decoded.payload.len(), 2);
                let net = u16::from_be_bytes([decoded.payload[0], decoded.payload[1]]);
                assert_eq!(net, 2000);
            }
            _ => panic!("Expected Broadcast response for I-Am-Router"),
        }
    }

    #[tokio::test]
    async fn who_is_router_with_unknown_network_no_response() {
        let mut table = RouterTable::new();
        table.add_direct(1000, 0);

        let table = Arc::new(Mutex::new(table));

        let (tx, mut rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        let mut req_payload = BytesMut::with_capacity(2);
        req_payload.put_u16(9999);

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw()),
            payload: req_payload.freeze(),
            ..Npdu::default()
        };

        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn initialize_routing_table_ack() {
        let mut table = RouterTable::new();
        table.add_direct(1000, 0);
        table.add_direct(2000, 1);

        let table = Arc::new(Mutex::new(table));

        let (tx, mut rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE.to_raw()),
            payload: Bytes::new(),
            ..Npdu::default()
        };

        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

        let sent = rx.try_recv().unwrap();
        match sent {
            SendRequest::Unicast { npdu: data, mac } => {
                assert_eq!(mac.as_slice(), &[0x0A]);
                let decoded = decode_npdu(data.clone()).unwrap();
                assert!(decoded.is_network_message);
                assert_eq!(
                    decoded.message_type,
                    Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK.to_raw())
                );
                assert_eq!(decoded.payload.len(), 9);
                assert_eq!(decoded.payload[0], 2);
            }
            _ => panic!("Expected Unicast response for Init-Routing-Table"),
        }
    }

    #[tokio::test]
    async fn router_busy_does_not_crash() {
        let table = RouterTable::new();
        let table = Arc::new(Mutex::new(table));

        let (tx, _rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        let mut payload = BytesMut::with_capacity(4);
        payload.put_u16(1000);
        payload.put_u16(2000);

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::ROUTER_BUSY_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;
    }

    #[tokio::test]
    async fn router_available_does_not_crash() {
        let table = RouterTable::new();
        let table = Arc::new(Mutex::new(table));

        let (tx, _rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        let mut payload = BytesMut::with_capacity(4);
        payload.put_u16(1000);
        payload.put_u16(2000);

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;
    }

    #[tokio::test]
    async fn i_could_be_router_stores_potential_route() {
        let table = RouterTable::new();
        let table = Arc::new(Mutex::new(table));

        let (tx, _rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        let mut payload = BytesMut::with_capacity(3);
        payload.put_u16(5000);
        payload.put_u8(50);

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A, 0x0B], &npdu).await;

        let tbl = table.lock().await;
        let entry = tbl.lookup(5000).unwrap();
        assert!(!entry.directly_connected);
        assert_eq!(entry.port_index, 0);
        assert_eq!(entry.next_hop_mac.as_slice(), &[0x0A, 0x0B]);
    }

    #[tokio::test]
    async fn i_could_be_router_does_not_overwrite_existing_route() {
        let mut table = RouterTable::new();
        table.add_direct(5000, 1);
        let table = Arc::new(Mutex::new(table));

        let (tx, _rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        let mut payload = BytesMut::with_capacity(3);
        payload.put_u16(5000);
        payload.put_u8(50);

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

        let tbl = table.lock().await;
        let entry = tbl.lookup(5000).unwrap();
        assert!(entry.directly_connected);
        assert_eq!(entry.port_index, 1);
    }

    #[tokio::test]
    async fn establish_connection_does_not_crash() {
        let table = RouterTable::new();
        let table = Arc::new(Mutex::new(table));

        let (tx, _rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        let mut payload = BytesMut::with_capacity(3);
        payload.put_u16(6000);
        payload.put_u8(30);

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::ESTABLISH_CONNECTION_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;
    }

    #[tokio::test]
    async fn disconnect_removes_learned_route() {
        let mut table = RouterTable::new();
        table.add_learned(7000, 0, MacAddr::from_slice(&[10, 0, 1, 1]));
        let table = Arc::new(Mutex::new(table));

        let (tx, _rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        let mut payload = BytesMut::with_capacity(2);
        payload.put_u16(7000);

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

        let tbl = table.lock().await;
        assert!(tbl.lookup(7000).is_none());
    }

    #[tokio::test]
    async fn disconnect_does_not_remove_direct_route() {
        let mut table = RouterTable::new();
        table.add_direct(1000, 0);
        let table = Arc::new(Mutex::new(table));

        let (tx, _rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        let mut payload = BytesMut::with_capacity(2);
        payload.put_u16(1000);

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

        let tbl = table.lock().await;
        assert!(tbl.lookup(1000).is_some());
        assert!(tbl.lookup(1000).unwrap().directly_connected);
    }
}
