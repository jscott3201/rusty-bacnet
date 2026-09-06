//! Originating VMAC admission for NPDUs received through a hub connector.

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::BytesMut;
use tracing::warn;

use crate::sc_frame::{encode_sc_message, ScFunction, ScMessage, Vmac, BROADCAST_VMAC};

use super::{data_attributes::build_bvlc_result_nak, WebSocketPort};

pub(super) fn hub_source(msg: &ScMessage) -> Option<Vmac> {
    msg.originating_vmac
        .filter(|source| *source != [0; 6] && *source != BROADCAST_VMAC)
}

pub(super) async fn reject_invalid_npdu_source<W: WebSocketPort>(msg: &ScMessage, ws: &W) -> bool {
    if msg.function != ScFunction::EncapsulatedNpdu || hub_source(msg).is_some() {
        return false;
    }

    // A hub-relayed NPDU carries the originating node's VMAC (AB.5.3.2–3).
    // Classifying omission as PARAMETER_OUT_OF_RANGE is an AB.3.1.5 inference.
    // Reply connection-locally to an omitted source, except for broadcasts.
    // Explicit zero/broadcast sources have no valid unicast return address:
    // suppress those replies as local hardening, not a Standard exception.
    if msg.originating_vmac.is_none() && msg.destination_vmac != Some(BROADCAST_VMAC) {
        let nak = build_bvlc_result_nak(
            msg.message_id,
            msg.function,
            0,
            None,
            ErrorClass::COMMUNICATION,
            ErrorCode::PARAMETER_OUT_OF_RANGE,
        );
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &nak);
        if let Err(e) = ws.send(&buf).await {
            warn!("BACnet/SC source admission NAK send error: {}", e);
        }
    }

    true
}
