//! Established-node Advertisement/Solicitation admission, not generic codec
//! validation and not AB.3.2 status tracking or solicited responses.

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::BytesMut;
use tracing::warn;

use super::data_attributes::build_bvlc_result_nak;
use super::rejection::{RejectionBudget, RejectionExpired};
use super::WebSocketPort;
use crate::sc_frame::{
    advertisement_message_error, encode_sc_message,
    first_must_understand_destination_option_marker, ScFunction, ScMessage, BROADCAST_VMAC,
};

pub(super) async fn reject<W: WebSocketPort>(
    msg: &ScMessage,
    wire: &[u8],
    ws: &W,
    budget: RejectionBudget,
) -> Result<bool, RejectionExpired> {
    if !matches!(
        msg.function,
        ScFunction::Advertisement | ScFunction::AdvertisementSolicitation
    ) {
        return Ok(false);
    }
    // Hub-connector AB.5.4: explicit destinations are not for the local BVLL.
    // Reserved origins are also silent; neither decision spends a budget.
    if msg.destination_vmac.is_some()
        || matches!(msg.originating_vmac, Some(vmac) if vmac == [0; 6] || vmac == BROADCAST_VMAC)
    {
        return Ok(true);
    }
    // Valid shapes are consumed silently with accepted activity (no NPDU, no
    // state change). AB.3.2 status updates and solicited Advertisement
    // transmissions are deferred transmission behavior, not admission.
    let Some(code) = advertisement_message_error(msg) else {
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
        warn!("BACnet/SC advertisement NAK send error: {}", e);
    }
    Ok(true)
}
