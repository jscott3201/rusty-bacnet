//! Established-node Proprietary-Message admission, not generic codec
//! validation and not vendor function dispatch.

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::BytesMut;
use tracing::warn;

use super::data_attributes::build_bvlc_result_nak;
use super::rejection::{RejectionBudget, RejectionExpired};
use super::WebSocketPort;
use crate::sc_frame::{
    encode_sc_message, first_must_understand_destination_option_marker, proprietary_message_error,
    ScFunction, ScMessage, BROADCAST_VMAC,
};

pub(super) async fn reject<W: WebSocketPort>(
    msg: &ScMessage,
    wire: &[u8],
    ws: &W,
    budget: RejectionBudget,
) -> Result<bool, RejectionExpired> {
    if msg.function != ScFunction::ProprietaryMessage {
        return Ok(false);
    }
    // Hub-connector AB.5.4: explicit unicast destinations are not for the
    // local BVLL. Broadcast and reserved origins are also silent per AB.2 /
    // AB.3.1.4 broadcast rules and local-matter drop; neither spends a budget.
    if msg.destination_vmac.is_some()
        || matches!(msg.originating_vmac, Some(vmac) if vmac == [0; 6] || vmac == BROADCAST_VMAC)
    {
        return Ok(true);
    }
    // Valid shapes are consumed silently with accepted activity (no NPDU, no
    // state change). Vendor dispatch is a local matter; unexpected but
    // well-formed messages are dropped here rather than NAKed.
    let Some(code) = proprietary_message_error(msg) else {
        return Ok(false);
    };
    let marker = if code == ErrorCode::HEADER_NOT_UNDERSTOOD {
        match first_must_understand_destination_option_marker(wire) {
            Some(marker) => marker,
            None => return Ok(true),
        }
    } else {
        0
    };
    let nak = build_bvlc_result_nak(
        msg.message_id,
        msg.function,
        marker,
        msg.originating_vmac,
        ErrorClass::COMMUNICATION,
        code,
    );
    let mut bytes = BytesMut::new();
    encode_sc_message(&mut bytes, &nak);
    if let Err(e) = budget.send(ws, &bytes).await? {
        warn!("BACnet/SC proprietary NAK send error: {}", e);
    }
    Ok(true)
}
