use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bacnet_encoding::npdu::{encode_npdu, Npdu, NpduAddress};
use bacnet_types::enums::{NetworkMessageType, RejectMessageReason};
use bacnet_types::MacAddr;
use bytes::{BufMut, BytesMut};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, warn};

use crate::router_table::RouterTable;

use super::forwarding::send_reject;
use super::{IngressContext, SendRequest};

/// Validate an Initialize-Routing-Table / Initialize-Routing-Table-Ack data
/// portion without touching the table (RB-03).
///
/// Returns the advertised network numbers in order when the payload is
/// exactly `Number of Ports` entries — each `DNET(2) + Port ID(1) + Port Info
/// Length(1) + Port Info(N)` — with no truncation and no trailing bytes.
/// Returns `None` for any incomplete tail, including a missing `Number of
/// Ports` octet. Per-entry policy (reserved networks, existing routes, route
/// cap) stays in the apply phase; this only proves the envelope is complete.
fn parse_routing_table_entries(data: &[u8]) -> Option<Vec<u16>> {
    let count = usize::from(*data.first()?);
    let mut offset = 1usize;
    let mut networks = Vec::with_capacity(count);
    for _ in 0..count {
        if offset + 4 > data.len() {
            return None;
        }
        networks.push(u16::from_be_bytes([data[offset], data[offset + 1]]));
        let info_len = usize::from(data[offset + 3]);
        if offset + 4 + info_len > data.len() {
            return None;
        }
        offset += 4 + info_len;
    }
    if offset != data.len() {
        return None;
    }
    Some(networks)
}

/// Handle a network-layer message.
///
/// The caller ([`super::dispatch_network_message`]) has already applied the
/// routing-first rule: only link-local messages (no DNET or global
/// broadcast), messages for our own ingress network, never-routed controls
/// (What-Is / Network-Number-Is), and rejects reach this admission point.
///
/// Every learning path below validates its complete payload BEFORE acquiring
/// the table lock (RB-03, Clauses 6.2 / 6.4 / 6.5.4 / 6.6.3.2). A truncated
/// tail rejects the whole message: no table change, no partial forward, no
/// rebroadcast, and — for Initialize-Routing-Table — no ACK.
pub(super) async fn handle_network_message(
    table: &Arc<Mutex<RouterTable>>,
    send_txs: &[mpsc::Sender<SendRequest>],
    ctx: &IngressContext,
) {
    const MAX_LEARNED_ROUTES: usize = 256;

    let port_idx = ctx.port_idx;
    let port_network = ctx.port_network;
    let source_mac = ctx.source_mac.as_slice();
    let npdu: &Npdu = &ctx.npdu;
    let ingress_attributes = ctx.data_attributes.as_slice();

    let msg_type = match npdu.message_type {
        Some(t) => t,
        None => return,
    };

    if msg_type == NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw() {
        // Clause 6.4.1: optionally followed by one 2-octet network number.
        // Anything else (a 1-octet tail, trailing bytes) is malformed and
        // must not promote a scoped query into a global scan.
        let requested_network = match npdu.payload.len() {
            0 => None,
            2 => {
                let net = u16::from_be_bytes([npdu.payload[0], npdu.payload[1]]);
                if net == 0 {
                    return;
                }
                Some(net)
            }
            _ => return,
        };

        let table = table.lock().await;

        let networks: Vec<u16> = if let Some(net) = requested_network {
            // Only respond if the network is reachable via a different port.
            match table.lookup(net) {
                Some(entry) if entry.port_index != port_idx => vec![net],
                _ => {
                    // Unknown: forward Who-Is-Router to all other ports to discover the path.
                    drop(table);
                    // Clause 6.6.3.2 discovery relay: retain an already-routed
                    // SNET/SADR, else add the ingress network and the
                    // immediate source MAC (the originator is local). The
                    // immediate next hop stays implicit (our broadcast SA),
                    // never merged into the origin fields. Discovery relays
                    // are local broadcasts: no DNET travels with them.
                    let source = npdu.source.clone().unwrap_or(NpduAddress {
                        network: port_network,
                        mac_address: MacAddr::from_slice(source_mac),
                    });
                    let forward = Npdu {
                        is_network_message: true,
                        expecting_reply: npdu.expecting_reply,
                        priority: npdu.priority,
                        source: Some(source),
                        message_type: Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw()),
                        payload: npdu.payload.clone(),
                        ..Npdu::default()
                    };
                    let mut fwd_buf = BytesMut::with_capacity(8);
                    if let Ok(()) = encode_npdu(&mut fwd_buf, &forward) {
                        let frozen = fwd_buf.freeze();
                        for (i, tx) in send_txs.iter().enumerate() {
                            if i != port_idx {
                                let _ = tx.try_send(SendRequest::broadcast_with_attributes(
                                    frozen.clone(),
                                    ingress_attributes,
                                ));
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
        if let Err(e) = send_txs[port_idx].try_send(SendRequest::broadcast_with_attributes(
            buf.freeze(),
            ingress_attributes,
        )) {
            warn!(%e, "Router dropped I-Am-Router response: output channel full");
        }
    } else if msg_type == NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw() {
        // Clause 6.4.2: one or more 2-octet network numbers. A truncated
        // tail rejects the whole message — no prefix learning, no rebroadcast.
        let data = &npdu.payload;
        if data.is_empty() || data.len() % 2 != 0 {
            return;
        }

        let mut table = table.lock().await;

        let mut replacement_networks = HashSet::new();
        let mut offset = 0;
        while offset + 2 <= data.len() {
            let net = u16::from_be_bytes([data[offset], data[offset + 1]]);
            offset += 2;

            if table.len() >= MAX_LEARNED_ROUTES && table.lookup(net).is_none() {
                table.record_learning_cap();
                warn!("Router table learned routes cap ({MAX_LEARNED_ROUTES}) reached, ignoring further networks");
                break;
            }

            // Repeated entries in one packet are not independent arrivals.
            // Leave absent learning and same-port refresh entries unchanged.
            if table
                .lookup(net)
                .is_some_and(|entry| !entry.directly_connected && entry.port_index != port_idx)
                && !replacement_networks.insert(net)
            {
                continue;
            }
            if table.apply_learning_claim(
                net,
                port_idx,
                MacAddr::from_slice(source_mac),
                Instant::now(),
            ) {
                table.record_learned();
                debug!(
                    network = net,
                    port = port_idx,
                    "Learned route from I-Am-Router-To-Network"
                );
            }
        }
        drop(table);

        // Re-broadcast to all other ports unconditionally (spec 6.6.3.3).
        // Only reached for complete payloads; the payload is re-emitted
        // verbatim with the ingress attributes carried (Clause 6.5.4).
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
                    let _ = tx.try_send(SendRequest::broadcast_with_attributes(
                        frozen.clone(),
                        ingress_attributes,
                    ));
                }
            }
        }
    } else if msg_type == NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw() {
        // Clause 6.4.4: exactly one reason octet plus one 2-octet network.
        // Anything else is malformed: no table change, no relay.
        if npdu.payload.len() != 3 {
            return;
        }
        let reason = npdu.payload[0];
        let rejected_net = u16::from_be_bytes([npdu.payload[1], npdu.payload[2]]);
        warn!(
            network = rejected_net,
            reason = reason,
            "Received Reject-Message-To-Network"
        );
        {
            let mut tbl = table.lock().await;
            tbl.apply_reject(rejected_net, port_idx, reason, Instant::now());
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
                        let _ =
                            send_txs[dest_port].try_send(SendRequest::broadcast_with_attributes(
                                buf.freeze(),
                                ingress_attributes,
                            ));
                    } else {
                        let _ = send_txs[dest_port].try_send(SendRequest::unicast_with_attributes(
                            buf.freeze(),
                            dest_mac,
                            ingress_attributes,
                        ));
                    }
                }
            }
        }
    } else if msg_type == NetworkMessageType::ROUTER_BUSY_TO_NETWORK.to_raw() {
        // Clauses 6.4.5/6.6.3.6: an optional list of 2-octet networks. "If the
        // 2-octet network numbers are omitted, it means the router wishes to
        // stop the flow of messages to all the networks it normally serves":
        // the omitted scope is the via-peer set (learned routes egressing
        // this ingress port toward the immediate source MAC), never every
        // route and never none. Explicit lists intersect with that set, and
        // directly-attached paths are never overridden (see
        // `RouterTable::is_served_via_peer`). A truncated tail rejects the
        // whole message: no marks, no rebroadcast. Marks apply immediately
        // with a 30s deadline (no reject-style damping); learning freshness
        // (`last_seen`) is untouched. Propagation rebroadcasts the payload
        // verbatim out every other port; full queues drop locally (bounded,
        // no retry, no admission path) while per-route marks stay correct.
        let data = &npdu.payload;
        if data.len() % 2 != 0 {
            return;
        }
        let mut listed = Vec::with_capacity(data.len() / 2);
        let mut offset = 0;
        while offset + 2 <= data.len() {
            listed.push(u16::from_be_bytes([data[offset], data[offset + 1]]));
            offset += 2;
        }
        let peer = MacAddr::from_slice(source_mac);
        let deadline = Instant::now() + Duration::from_secs(30);
        {
            let mut tbl = table.lock().await;
            if listed.is_empty() {
                for net in tbl.routes_served_via_peer(port_idx, &peer) {
                    tbl.mark_busy(net, deadline);
                    debug!(
                        network = net,
                        "Router busy — marked network as congested (30s timer)"
                    );
                }
            } else {
                for net in listed {
                    if tbl.is_served_via_peer(net, port_idx, &peer) {
                        tbl.mark_busy(net, deadline);
                        debug!(
                            network = net,
                            "Router busy — marked network as congested (30s timer)"
                        );
                    }
                }
            }
        }
        // Re-broadcast to all other ports (spec 6.6.3.6)
        let rebroadcast = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::ROUTER_BUSY_TO_NETWORK.to_raw()),
            payload: npdu.payload.clone(),
            ..Npdu::default()
        };
        let mut buf = BytesMut::with_capacity(4 + npdu.payload.len());
        if let Ok(()) = encode_npdu(&mut buf, &rebroadcast) {
            let frozen = buf.freeze();
            for (i, tx) in send_txs.iter().enumerate() {
                if i != port_idx {
                    let _ = tx.try_send(SendRequest::broadcast_with_attributes(
                        frozen.clone(),
                        ingress_attributes,
                    ));
                }
            }
        }
    } else if msg_type == NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK.to_raw() {
        // Clauses 6.4.6/6.6.3.7: same scope rule as Router-Busy. "If the
        // 2-octet network numbers are omitted, the router wishes to re-enable
        // the flow of messages to all the networks it serves": the omitted
        // scope is the via-peer set, and explicit lists intersect with it.
        let data = &npdu.payload;
        if data.len() % 2 != 0 {
            return;
        }
        let mut listed = Vec::with_capacity(data.len() / 2);
        let mut offset = 0;
        while offset + 2 <= data.len() {
            listed.push(u16::from_be_bytes([data[offset], data[offset + 1]]));
            offset += 2;
        }
        let peer = MacAddr::from_slice(source_mac);
        {
            let mut tbl = table.lock().await;
            if listed.is_empty() {
                for net in tbl.routes_served_via_peer(port_idx, &peer) {
                    tbl.mark_available(net);
                    debug!(network = net, "Router available — cleared congestion");
                }
            } else {
                for net in listed {
                    if tbl.is_served_via_peer(net, port_idx, &peer) {
                        tbl.mark_available(net);
                        debug!(network = net, "Router available — cleared congestion");
                    }
                }
            }
        }
        // Re-broadcast to all other ports (spec 6.6.3.7)
        let rebroadcast = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK.to_raw()),
            payload: npdu.payload.clone(),
            ..Npdu::default()
        };
        let mut buf = BytesMut::with_capacity(4 + npdu.payload.len());
        if let Ok(()) = encode_npdu(&mut buf, &rebroadcast) {
            let frozen = buf.freeze();
            for (i, tx) in send_txs.iter().enumerate() {
                if i != port_idx {
                    let _ = tx.try_send(SendRequest::broadcast_with_attributes(
                        frozen.clone(),
                        ingress_attributes,
                    ));
                }
            }
        }
    } else if msg_type == NetworkMessageType::INITIALIZE_ROUTING_TABLE.to_raw() {
        // Clauses 6.4.7/6.6.3.8: Number of Ports plus exactly that many
        // entries, fully validated before the table lock. A truncated entry
        // or trailing bytes rejects the whole message: no table change and,
        // unlike the previous ACK-on-partial behavior, no ACK. A zero count
        // with no trailing bytes is a table query.
        let data = &npdu.payload;
        let Some(networks) = parse_routing_table_entries(data) else {
            return;
        };

        let is_query = data[0] == 0;

        if !is_query {
            let mut tbl = table.lock().await;
            for net in &networks {
                if *net == 0 || *net == 0xFFFF {
                    continue;
                }
                if tbl.lookup(*net).is_some() {
                    continue; // don't overwrite existing routes
                }
                if tbl.len() >= MAX_LEARNED_ROUTES {
                    tbl.record_learning_cap();
                    warn!("Init-Routing-Table: route cap reached, ignoring further entries");
                    break;
                }
                tbl.add_learned(*net, port_idx, MacAddr::from_slice(source_mac));
                tbl.record_learned();
                debug!(
                    network = net,
                    port = port_idx,
                    "Learned route from Init-Routing-Table"
                );
            }
            drop(tbl);
        }

        let mut payload = BytesMut::new();
        if is_query {
            let tbl = table.lock().await;
            let networks = tbl.networks();
            let count = networks.len().min(255);
            payload.put_u8(count as u8);
            for net in networks.iter().take(count) {
                if let Some(route) = tbl.lookup(*net) {
                    payload.put_u16(*net);
                    payload.put_u8(route.port_index as u8); // Port ID
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

        if let Err(e) = send_txs[port_idx].try_send(SendRequest::unicast_with_attributes(
            buf.freeze(),
            MacAddr::from_slice(source_mac),
            ingress_attributes,
        )) {
            warn!(%e, "Router dropped Init-Routing-Table-ACK: output channel full");
        }
    } else if msg_type == NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK.to_raw() {
        // Clause 6.4.3 / Figure 6-10: exactly DNET(2) + Performance Index(1).
        if npdu.payload.len() != 3 {
            return;
        }
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
    } else if msg_type == NetworkMessageType::ESTABLISH_CONNECTION_TO_NETWORK.to_raw() {
        // Clause 6.4.9 / Figure 6-6: exactly DNET(2) + Termination Time(1).
        if npdu.payload.len() != 3 {
            return;
        }
        let net = u16::from_be_bytes([npdu.payload[0], npdu.payload[1]]);
        let termination_time_min = npdu.payload[2];
        tracing::info!(
            network = net,
            termination_time_minutes = termination_time_min,
            "Received Establish-Connection-To-Network (PTP not implemented)"
        );
    } else if msg_type == NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK.to_raw() {
        // Clause 6.4.10: exactly one 2-octet network number.
        if npdu.payload.len() != 2 {
            return;
        }
        let net = u16::from_be_bytes([npdu.payload[0], npdu.payload[1]]);
        debug!(
            network = net,
            "Received Disconnect-Connection-To-Network (PTP not implemented; route removal ignored)"
        );
        let mut tbl = table.lock().await;
        tbl.record_disconnect_removal_ignored();
    } else if msg_type == NetworkMessageType::WHAT_IS_NETWORK_NUMBER.to_raw() {
        // Clause 6.4.14: never routed (enforced by dispatch); ignore when
        // SNET/SADR or DNET/DADR is present. X'12 defines no data octets, so
        // a non-empty payload is malformed. A link unicast or broadcast
        // What-Is may both be answered.
        if npdu.source.is_some() || npdu.destination.is_some() {
            return;
        }
        if !npdu.payload.is_empty() {
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

        if let Err(e) = send_txs[port_idx].try_send(SendRequest::broadcast_with_attributes(
            buf.freeze(),
            ingress_attributes,
        )) {
            warn!(%e, "Router dropped Network-Number-Is: output channel full");
        }
    } else if msg_type == NetworkMessageType::NETWORK_NUMBER_IS.to_raw() {
        // Clause 6.4.15: never routed (enforced by dispatch); ignore messages
        // that carry SNET/SADR or DNET/DADR, that arrive via link unicast, or
        // whose payload is not exactly network(2) + flag(1).
        if npdu.source.is_some() || npdu.destination.is_some() {
            return;
        }
        if !ctx.link_layer_group {
            return;
        }
        // Spec 6.6.3.12: process Network-Number-Is for conflict detection.
        if npdu.payload.len() != 3 {
            return;
        }
        let net = u16::from_be_bytes([npdu.payload[0], npdu.payload[1]]);
        let configured = npdu.payload[2];
        if net != port_network {
            if configured == 1 {
                warn!(
                    local_network = port_network,
                    peer_network = net,
                    "Network number conflict: port configured as {} but peer reports {} (configured)",
                    port_network, net
                );
            } else {
                debug!(
                    local_network = port_network,
                    peer_network = net,
                    "Network-Number-Is from peer (learned, differs from local)"
                );
            }
        }
    } else if msg_type == NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK.to_raw() {
        // Clauses 6.4.8/6.6.3.9: same data format as Initialize-Routing-Table;
        // learn routes from peer. An empty payload carries no entries and is
        // a no-op; any other malformed envelope is dropped without learning.
        let data = &npdu.payload;
        if data.is_empty() {
            return;
        }
        let Some(networks) = parse_routing_table_entries(data) else {
            return;
        };
        let mut table = table.lock().await;
        let mut replacement_networks = HashSet::new();
        for net in &networks {
            if *net == 0 || *net == 0xFFFF {
                continue;
            }
            if table.len() >= MAX_LEARNED_ROUTES {
                table.record_learning_cap();
                break;
            }
            if table
                .lookup(*net)
                .is_some_and(|entry| !entry.directly_connected && entry.port_index != port_idx)
                && !replacement_networks.insert(*net)
            {
                continue;
            }
            if table.apply_learning_claim(
                *net,
                port_idx,
                MacAddr::from_slice(source_mac),
                Instant::now(),
            ) {
                table.record_learned();
                debug!(
                    network = net,
                    port = port_idx,
                    "Learned route from Init-Routing-Table-Ack"
                );
            }
        }
    } else if (0x0A..=0x11).contains(&msg_type) {
        // Security messages — acknowledge but do not reject.
        debug!(
            message_type = msg_type,
            "Router received security network message (not implemented)"
        );
    } else {
        // Unknown message type — reject with reason 3 (spec 6.6.3.5).
        // Proprietary types (0x80+) arriving without a routable destination
        // take this explicit-reject path; they are never accepted as known
        // controls. Directed proprietary messages are routed opaquely by the
        // dispatcher and never reach here.
        debug!(
            message_type = msg_type,
            "Router rejecting unknown network message type"
        );
        send_reject(
            &send_txs[port_idx],
            source_mac,
            0,
            RejectMessageReason::UNKNOWN_MESSAGE_TYPE,
            ingress_attributes,
        );
    }
}

#[cfg(test)]
#[path = "corroboration_tests.rs"]
mod corroboration_tests;
