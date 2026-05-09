use bacnet_encoding::npdu::{encode_npdu, Npdu, NpduAddress};
use bacnet_types::enums::{NetworkMessageType, RejectMessageReason};
use bacnet_types::MacAddr;
use bytes::{BufMut, BytesMut};
use tokio::sync::mpsc;
use tracing::warn;

use super::SendRequest;

/// Build the source NpduAddress for a forwarded message.
fn build_source(npdu: &Npdu, source_network: u16, source_mac: &[u8]) -> NpduAddress {
    npdu.source.clone().unwrap_or(NpduAddress {
        network: source_network,
        mac_address: MacAddr::from_slice(source_mac),
    })
}

/// Forward a message to a specific destination via the route entry.
pub(super) fn forward_unicast(
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
        is_network_message: npdu.is_network_message,
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
pub(super) fn forward_broadcast(
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
        is_network_message: npdu.is_network_message,
        expecting_reply: npdu.expecting_reply,
        priority: npdu.priority,
        destination: npdu.destination.clone(),
        source: Some(build_source(npdu, source_network, source_mac)),
        hop_count: npdu.hop_count - 1,
        message_type: npdu.message_type,
        vendor_id: npdu.vendor_id,
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

/// Send a Reject-Message-To-Network.
pub(super) fn send_reject(
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
