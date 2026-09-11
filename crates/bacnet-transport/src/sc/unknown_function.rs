//! Established-node unknown-function admission, not generic codec validation.

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::BytesMut;
use tracing::warn;

use super::data_attributes::build_bvlc_result_nak;
use super::rejection::{RejectionBudget, RejectionExpired};
use super::WebSocketPort;
use crate::sc_frame::{encode_sc_message, ScFunction, ScMessage, BROADCAST_VMAC};

pub(super) async fn reject<W: WebSocketPort>(
    msg: &ScMessage,
    ws: &W,
    budget: RejectionBudget,
) -> Result<bool, RejectionExpired> {
    // The receive loop supplies successful wire decodes: 0x00..0x0C are
    // canonical known variants, including known-but-unhandled functions.
    if !matches!(msg.function, ScFunction::Unknown(_)) {
        return Ok(false);
    }
    // Node-side explicit destinations are not response targets. Reserved
    // origins are also silent; neither decision spends a rejection budget.
    if msg.destination_vmac.is_some()
        || matches!(msg.originating_vmac, Some(vmac) if vmac == [0; 6] || vmac == BROADCAST_VMAC)
    {
        return Ok(true);
    }
    // AB.3.1.5: unknown unicast gets 7/143 and is discarded. The owner's
    // unknown-first diagnostic policy does not interpret options or payload.
    // An absent origin denotes the connection peer, not a missing NPDU source.
    let nak = build_bvlc_result_nak(
        msg.message_id,
        msg.function,
        0,
        msg.originating_vmac,
        ErrorClass::COMMUNICATION,
        ErrorCode::BVLC_FUNCTION_UNKNOWN,
    );
    let mut bytes = BytesMut::new();
    encode_sc_message(&mut bytes, &nak);
    if let Err(e) = budget.send(ws, &bytes).await? {
        warn!("BACnet/SC unknown-function NAK send error: {}", e);
    }
    Ok(true)
}
