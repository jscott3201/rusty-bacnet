//! Hub-connector admission after source and Must Understand rejection.
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::BytesMut;
use tracing::warn;

use super::rejection::{RejectionBudget, RejectionExpired};
use super::{data_attributes::build_bvlc_result_nak, WebSocketPort};
use crate::sc_frame::{encode_sc_message, missing_npdu_payload, ScMessage};

pub(super) async fn reject<W: WebSocketPort>(
    msg: &ScMessage,
    ws: &W,
    budget: RejectionBudget,
) -> Result<bool, RejectionExpired> {
    if !missing_npdu_payload(msg) {
        return Ok(false);
    }
    // Earlier source/MU decisions retain their precedence. For this new fault,
    // broadcast is silent (AB.2); explicit unicast destinations are dropped by
    // the hub connector (AB.5.4), not answered. Source admission already passed.
    if msg.destination_vmac.is_none() {
        let nak = build_bvlc_result_nak(
            msg.message_id,
            msg.function,
            0,
            msg.originating_vmac,
            ErrorClass::COMMUNICATION,
            ErrorCode::PAYLOAD_EXPECTED,
        );
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &nak);
        if let Err(e) = budget.send(ws, &buf).await? {
            warn!("BACnet/SC missing NPDU payload NAK send error: {}", e);
        }
    }
    Ok(true)
}
