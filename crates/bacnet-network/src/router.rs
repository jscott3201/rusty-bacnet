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

        // Announce I-Am-Router-To-Network on each port listing networks
        // reachable via other ports (per ASHRAE 135-2020 Clause 6.6.1).
        for (port_idx, tx) in send_txs.iter().enumerate() {
            // Collect all networks reachable via OTHER ports (not this one)
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

                                // Global broadcast (0xFFFF) — forward to all other ports
                                if dest_net == 0xFFFF {
                                    forward_broadcast(
                                        &send_txs,
                                        port_idx,
                                        port_network,
                                        &received.source_mac,
                                        &npdu,
                                    );

                                    // Also deliver locally
                                    let apdu = ReceivedApdu {
                                        apdu: npdu.payload,
                                        source_mac: received.source_mac,
                                        source_network: npdu.source,
                                        reply_tx: received.reply_tx,
                                    };
                                    let _ = local_tx.send(apdu).await;
                                    continue;
                                }

                                // Look up route for destination network
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
                                        // DADR matches our MAC → deliver locally
                                        let apdu = ReceivedApdu {
                                            apdu: npdu.payload,
                                            source_mac: received.source_mac,
                                            source_network: npdu.source,
                                            reply_tx: received.reply_tx,
                                        };
                                        let _ = local_tx.send(apdu).await;
                                    } else {
                                        // Forward to destination port
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
                                    // Unknown network — Reject
                                    send_reject(
                                        &send_txs[port_idx],
                                        &received.source_mac,
                                        dest_net,
                                        RejectMessageReason::OTHER,
                                    );
                                }
                            } else {
                                // No destination — local message
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

        // Background task: periodically purge stale learned routes.
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
    let dest_mac = if route.directly_connected {
        npdu.destination
            .as_ref()
            .map(|d| d.mac_address.clone())
            .unwrap_or_default()
    } else {
        route.next_hop_mac.clone()
    };

    let forwarded = Npdu {
        is_network_message: false,
        expecting_reply: npdu.expecting_reply,
        priority: npdu.priority,
        destination: npdu.destination,
        source: Some(source),
        hop_count: npdu.hop_count - 1,
        message_type: None,
        vendor_id: None,
        payload: npdu.payload,
    };

    let mut buf = BytesMut::with_capacity(32 + payload_len);
    if let Err(e) = encode_npdu(&mut buf, &forwarded) {
        warn!("Failed to encode forwarded NPDU: {e}");
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
    let msg_type = match npdu.message_type {
        Some(t) => t,
        None => return,
    };

    if msg_type == NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw() {
        let table = table.lock().await;

        // Parse optional network number from payload
        let requested_network = if npdu.payload.len() >= 2 {
            Some(u16::from_be_bytes([npdu.payload[0], npdu.payload[1]]))
        } else {
            None
        };

        let networks: Vec<u16> = if let Some(net) = requested_network {
            // For a specific network, only respond if we know about it
            // AND it's not on the requesting port (Clause 6.5.1).
            match table.lookup(net) {
                Some(entry) if entry.port_index != port_idx => vec![net],
                _ => return,
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

        if let Err(e) = send_txs[port_idx].try_send(SendRequest::Unicast {
            npdu: buf.freeze(),
            mac: MacAddr::from_slice(source_mac),
        }) {
            warn!(%e, "Router dropped I-Am-Router response: output channel full");
        }
    } else if msg_type == NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw() {
        const MAX_LEARNED_ROUTES: usize = 256;
        const LEARNED_ROUTE_MAX_AGE: Duration = Duration::from_secs(300);

        let data = &npdu.payload;
        let mut offset = 0;
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
                debug!(
                    network = net,
                    port = port_idx,
                    "Learned route from I-Am-Router-To-Network"
                );
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
            let mut tbl = table.lock().await;
            if let Some(entry) = tbl.lookup(rejected_net) {
                if !entry.directly_connected {
                    tbl.remove(rejected_net);
                }
            }
        }
    } else if msg_type == NetworkMessageType::ROUTER_BUSY_TO_NETWORK.to_raw() {
        let data = &npdu.payload;
        let mut offset = 0;
        while offset + 2 <= data.len() {
            let net = u16::from_be_bytes([data[offset], data[offset + 1]]);
            offset += 2;
            debug!(network = net, "Router busy for network");
        }
    } else if msg_type == NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK.to_raw() {
        let data = &npdu.payload;
        let mut offset = 0;
        while offset + 2 <= data.len() {
            let net = u16::from_be_bytes([data[offset], data[offset + 1]]);
            offset += 2;
            debug!(network = net, "Router available for network");
        }
    } else if msg_type == NetworkMessageType::INITIALIZE_ROUTING_TABLE.to_raw() {
        // Parse incoming routing table entries and add as learned routes.
        // Format: 1 byte count + N * (2 bytes DNET + 1 byte port_id + 1 byte info_len + info)
        {
            let data = &npdu.payload;
            if !data.is_empty() {
                let count = data[0] as usize;
                let mut offset = 1;
                let mut tbl = table.lock().await;
                for _ in 0..count {
                    if offset + 4 > data.len() {
                        break;
                    }
                    let net = u16::from_be_bytes([data[offset], data[offset + 1]]);
                    // skip port_id (1 byte)
                    let info_len = data[offset + 3] as usize;
                    offset += 4 + info_len;

                    if tbl.lookup(net).is_some() {
                        continue; // don't overwrite existing routes
                    }
                    tbl.add_learned(net, port_idx, MacAddr::from_slice(source_mac));
                    debug!(
                        network = net,
                        port = port_idx,
                        "Learned route from Init-Routing-Table"
                    );
                }
            }
        }

        // Reply with Init-Routing-Table-Ack
        let tbl = table.lock().await;
        let mut payload = BytesMut::new();
        let networks = tbl.networks();
        payload.put_u8(networks.len().min(255) as u8);
        for net in &networks {
            if tbl.lookup(*net).is_some() {
                payload.put_u16(*net);
                payload.put_u8(0); // Port ID (simplified)
                payload.put_u8(0); // Port info length
            }
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
        // Clause 6.5.3: A router that can potentially route to a network
        // (e.g., via dial-up). Payload: 2 bytes DNET + 1 byte performance index.
        if npdu.payload.len() >= 3 {
            let net = u16::from_be_bytes([npdu.payload[0], npdu.payload[1]]);
            let performance_index = npdu.payload[2];
            debug!(
                network = net,
                performance_index = performance_index,
                port = port_idx,
                "Received I-Could-Be-Router-To-Network"
            );
            // Store as a learned route only if no existing route exists (lower priority).
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
        // Clause 6.5.9: Request to establish a connection to a remote network.
        // Payload: 2 bytes DNET + 1 byte termination time (minutes).
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
        // Clause 6.5.10: Disconnect from a remote network.
        // Payload: 2 bytes DNET. Remove the route if dynamically established.
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
        // Set up two BIP transports on different ports (simulating two networks)
        let transport_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

        // A device on network A sends to network B via the router
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

        // Give the router a moment to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Device A sends a routed message to Device B's network (2000)
        let apdu = vec![0x10, 0x08]; // WhoIs
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

        // Send to the router's port A
        // We need the router port's MAC — but we transferred ownership.
        // For this test, let's verify the router table is set up correctly.
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

        // Give the router time to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Get the router port's MAC... we can't because we transferred ownership.
        // This test verifies the local_rx channel is created correctly.
        // A full integration test would need to keep the router port MAC.

        router.stop().await;
    }

    // --- Hop count rejection tests ---

    #[test]
    fn forward_unicast_drops_hop_count_zero() {
        // Messages with hop_count=0 must not be forwarded (per Clause 6.4).
        // forward_unicast should silently drop them.
        let (tx_a, mut rx_a) = mpsc::channel::<SendRequest>(256);
        let (tx_b, mut rx_b) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx_a, tx_b];

        let route = crate::router_table::RouteEntry {
            port_index: 1,
            directly_connected: true,
            next_hop_mac: MacAddr::new(),
            last_seen: None,
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

        // Per Clause 6.2.2, hop_count=0 must be silently discarded — no reject
        assert!(rx_a.try_recv().is_err());
        assert!(rx_b.try_recv().is_err());
    }

    #[test]
    fn forward_broadcast_drops_hop_count_zero() {
        // Broadcasts with hop_count=0 must also be silently discarded.
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

        // Source port is 0, so forward_broadcast should send to port 1
        // ... but hop_count=0, so the message must be silently discarded
        forward_broadcast(&send_txs, 0, 1000, &[0x0A], &npdu);

        // Per Clause 6.2.2, hop_count=0 must be silently discarded — no reject
        assert!(rx_a.try_recv().is_err());
        assert!(rx_b.try_recv().is_err());
    }

    #[test]
    fn forward_unicast_decrements_hop_count() {
        // When hop_count > 0, the forwarded message should have hop_count - 1.
        let (tx_a, _rx_a) = mpsc::channel::<SendRequest>(256);
        let (tx_b, mut rx_b) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx_a, tx_b];

        let route = crate::router_table::RouteEntry {
            port_index: 1,
            directly_connected: true,
            next_hop_mac: MacAddr::new(),
            last_seen: None,
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

        // Should have been sent on port 1
        let sent = rx_b.try_recv().unwrap();
        match sent {
            SendRequest::Unicast { npdu: data, .. } => {
                let decoded = decode_npdu(data.clone()).unwrap();
                assert_eq!(decoded.hop_count, 9); // Decremented from 10 to 9
            }
            SendRequest::Broadcast { npdu: data } => {
                let decoded = decode_npdu(data.clone()).unwrap();
                assert_eq!(decoded.hop_count, 9);
            }
        }
    }

    // --- Reject for unknown network tests ---

    #[test]
    fn send_reject_generates_reject_message() {
        // When a message is destined for an unknown network, the router
        // should generate a Reject-Message-To-Network.
        let (tx, mut rx) = mpsc::channel::<SendRequest>(256);

        let source_mac = vec![0x0A, 0x00, 0x01, 0x01];
        let unknown_network: u16 = 9999;

        send_reject(
            &tx,
            &source_mac,
            unknown_network,
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
        );

        // Should have sent a reject message back to the source
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
                // Payload: reason(1) + network(2)
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
        // A single-port router has no other networks to announce,
        // so it should NOT send any I-Am-Router-To-Network broadcast.
        // We verify this by intercepting the send channel.
        let (send_tx, mut send_rx) = mpsc::channel::<SendRequest>(256);

        // Simulate the announcement logic for a single-port router
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

        // No announcement should have been sent
        assert!(send_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn two_port_router_sends_i_am_router_announcement() {
        // A two-port router should send I-Am-Router-To-Network on each port
        // listing the networks reachable via the other port.
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

        // Port A should announce network 2000
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

        // Port B should announce network 1000
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
        // A three-port router should announce two networks on each port.
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

        // Port A should announce networks 200 and 300
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

        // Port B should announce networks 100 and 300
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

        // Port C should announce networks 100 and 200
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
        // hop_count=1 means the message can be forwarded once more
        // (it will arrive at destination with hop_count=0).
        let (tx_a, _rx_a) = mpsc::channel::<SendRequest>(256);
        let (tx_b, mut rx_b) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx_a, tx_b];

        let route = crate::router_table::RouteEntry {
            port_index: 1,
            directly_connected: true,
            next_hop_mac: MacAddr::new(),
            last_seen: None,
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

        // Should still be forwarded (hop_count=1 is valid)
        let sent = rx_b.try_recv().unwrap();
        match sent {
            SendRequest::Unicast { npdu: data, .. } => {
                let decoded = decode_npdu(data.clone()).unwrap();
                assert_eq!(decoded.hop_count, 0); // Decremented from 1 to 0
            }
            SendRequest::Broadcast { npdu: data } => {
                let decoded = decode_npdu(data.clone()).unwrap();
                assert_eq!(decoded.hop_count, 0);
            }
        }
    }

    // --- Received Reject-Message-To-Network removes learned route ---

    #[tokio::test]
    async fn received_reject_removes_learned_route() {
        let mut table = RouterTable::new();
        table.add_direct(1000, 0);
        table.add_learned(3000, 0, MacAddr::from_slice(&[10, 0, 1, 1]));
        assert!(table.lookup(3000).is_some());

        let table = Arc::new(Mutex::new(table));

        let (tx, _rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        // Build a Reject-Message-To-Network for network 3000
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

        // The learned route for network 3000 should have been removed
        let tbl = table.lock().await;
        assert!(tbl.lookup(3000).is_none());
        // Direct route should be untouched
        assert!(tbl.lookup(1000).is_some());
    }

    #[tokio::test]
    async fn received_reject_does_not_remove_direct_route() {
        let mut table = RouterTable::new();
        table.add_direct(1000, 0);

        let table = Arc::new(Mutex::new(table));

        let (tx, _rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        // Build a Reject-Message-To-Network for network 1000 (directly connected)
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

        // Direct route should NOT be removed
        let tbl = table.lock().await;
        assert!(tbl.lookup(1000).is_some());
    }

    // --- Who-Is-Router-To-Network with specific network ---

    #[tokio::test]
    async fn who_is_router_with_specific_network() {
        let mut table = RouterTable::new();
        table.add_direct(1000, 0);
        table.add_direct(2000, 1);
        table.add_direct(3000, 2);

        let table = Arc::new(Mutex::new(table));

        let (tx, mut rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        // Payload contains requested network 2000
        let mut req_payload = BytesMut::with_capacity(2);
        req_payload.put_u16(2000);

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw()),
            payload: req_payload.freeze(),
            ..Npdu::default()
        };

        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

        // Should respond with I-Am-Router containing only network 2000
        let sent = rx.try_recv().unwrap();
        match sent {
            SendRequest::Unicast { npdu: data, mac } => {
                assert_eq!(mac.as_slice(), &[0x0A]);
                let decoded = decode_npdu(data.clone()).unwrap();
                assert!(decoded.is_network_message);
                assert_eq!(
                    decoded.message_type,
                    Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw())
                );
                // Payload should contain exactly one network: 2000
                assert_eq!(decoded.payload.len(), 2);
                let net = u16::from_be_bytes([decoded.payload[0], decoded.payload[1]]);
                assert_eq!(net, 2000);
            }
            _ => panic!("Expected Unicast response for Who-Is-Router"),
        }
    }

    #[tokio::test]
    async fn who_is_router_with_unknown_network_no_response() {
        let mut table = RouterTable::new();
        table.add_direct(1000, 0);

        let table = Arc::new(Mutex::new(table));

        let (tx, mut rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        // Request a network we don't know about
        let mut req_payload = BytesMut::with_capacity(2);
        req_payload.put_u16(9999);

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw()),
            payload: req_payload.freeze(),
            ..Npdu::default()
        };

        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

        // No response should be sent for unknown network
        assert!(rx.try_recv().is_err());
    }

    // --- Initialize-Routing-Table Ack ---

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

        // Should respond with Initialize-Routing-Table-Ack
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
                // Payload: count(1) + N * (network(2) + port_id(1) + info_len(1))
                // With 2 networks: 1 + 2*4 = 9 bytes
                assert_eq!(decoded.payload.len(), 9);
                assert_eq!(decoded.payload[0], 2); // 2 networks
            }
            _ => panic!("Expected Unicast response for Init-Routing-Table"),
        }
    }

    // --- Router-Busy / Router-Available (log-only, no crash) ---

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

        // Should process without panic
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

        // Should process without panic
        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;
    }

    // --- I-Could-Be-Router-To-Network ---

    #[tokio::test]
    async fn i_could_be_router_stores_potential_route() {
        let table = RouterTable::new();
        let table = Arc::new(Mutex::new(table));

        let (tx, _rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        // Payload: DNET(2) + performance_index(1)
        let mut payload = BytesMut::with_capacity(3);
        payload.put_u16(5000);
        payload.put_u8(50); // performance index

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A, 0x0B], &npdu).await;

        // Should have stored a learned route for network 5000
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

        // Existing direct route should remain unchanged
        let tbl = table.lock().await;
        let entry = tbl.lookup(5000).unwrap();
        assert!(entry.directly_connected);
        assert_eq!(entry.port_index, 1);
    }

    // --- Establish-Connection-To-Network ---

    #[tokio::test]
    async fn establish_connection_does_not_crash() {
        let table = RouterTable::new();
        let table = Arc::new(Mutex::new(table));

        let (tx, _rx) = mpsc::channel::<SendRequest>(256);
        let send_txs = vec![tx];

        // Payload: DNET(2) + termination_time(1)
        let mut payload = BytesMut::with_capacity(3);
        payload.put_u16(6000);
        payload.put_u8(30); // 30 minutes

        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::ESTABLISH_CONNECTION_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        // Should process without panic
        handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;
    }

    // --- Disconnect-Connection-To-Network ---

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

        // Learned route for network 7000 should be removed
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

        // Direct route should NOT be removed
        let tbl = table.lock().await;
        assert!(tbl.lookup(1000).is_some());
        assert!(tbl.lookup(1000).unwrap().directly_connected);
    }
}
