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
use super::SendRequest;

/// Handle a network-layer message.
pub(super) async fn handle_network_message(
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

            if table.add_learned_with_flap_detection(net, port_idx, MacAddr::from_slice(source_mac))
            {
                debug!(
                    network = net,
                    port = port_idx,
                    "Learned route from I-Am-Router-To-Network"
                );
            }
        }
        drop(table);

        // Re-broadcast to all other ports unconditionally (spec 6.6.3.3).
        if !npdu.payload.is_empty() {
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
                        match reason {
                            1 => tbl.mark_unreachable(rejected_net),
                            2 => tbl
                                .mark_busy(rejected_net, Instant::now() + Duration::from_secs(30)),
                            _ => {
                                tbl.remove(rejected_net);
                            }
                        }
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
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut tbl = table.lock().await;
        while offset + 2 <= data.len() {
            let net = u16::from_be_bytes([data[offset], data[offset + 1]]);
            offset += 2;
            tbl.mark_busy(net, deadline);
            debug!(
                network = net,
                "Router busy — marked network as congested (30s timer)"
            );
        }
        drop(tbl);
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
                    let _ = tx.try_send(SendRequest::Broadcast {
                        npdu: frozen.clone(),
                    });
                }
            }
        }
    } else if msg_type == NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK.to_raw() {
        let data = &npdu.payload;
        let mut offset = 0;
        let mut tbl = table.lock().await;
        while offset + 2 <= data.len() {
            let net = u16::from_be_bytes([data[offset], data[offset + 1]]);
            offset += 2;
            tbl.mark_available(net);
            debug!(network = net, "Router available — cleared congestion");
        }
        drop(tbl);
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
                    let _ = tx.try_send(SendRequest::Broadcast {
                        npdu: frozen.clone(),
                    });
                }
            }
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
            tracing::info!(
                network = net,
                termination_time_minutes = termination_time_min,
                "Received Establish-Connection-To-Network (PTP not implemented)"
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
    } else if msg_type == NetworkMessageType::NETWORK_NUMBER_IS.to_raw() {
        // Spec 6.6.3.12: process Network-Number-Is for conflict detection.
        if npdu.payload.len() >= 3 {
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
        }
    } else if msg_type == NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK.to_raw() {
        // Spec 6.4.8: process Init-Routing-Table-Ack — learn routes from peer.
        let data = &npdu.payload;
        if data.is_empty() {
            return;
        }
        let count = data[0] as usize;
        let mut offset = 1usize;
        let mut table = table.lock().await;
        for _ in 0..count {
            if offset + 4 > data.len() {
                break;
            }
            let net = u16::from_be_bytes([data[offset], data[offset + 1]]);
            let info_len = data[offset + 3] as usize;
            if offset + 4 + info_len > data.len() {
                break;
            }
            offset += 4 + info_len;
            if net == 0 || net == 0xFFFF {
                continue;
            }
            if table.len() >= MAX_LEARNED_ROUTES {
                break;
            }
            table.add_learned_with_flap_detection(net, port_idx, MacAddr::from_slice(source_mac));
            debug!(
                network = net,
                port = port_idx,
                "Learned route from Init-Routing-Table-Ack"
            );
        }
    } else if (0x0A..=0x11).contains(&msg_type) {
        // Security messages — acknowledge but do not reject.
        debug!(
            message_type = msg_type,
            "Router received security network message (not implemented)"
        );
    } else {
        // Unknown message type — reject with reason 3 (spec 6.6.3.5).
        debug!(
            message_type = msg_type,
            "Router rejecting unknown network message type"
        );
        send_reject(
            &send_txs[port_idx],
            source_mac,
            0,
            RejectMessageReason::UNKNOWN_MESSAGE_TYPE,
        );
    }
}
