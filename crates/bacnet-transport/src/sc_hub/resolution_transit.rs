//! Selected hub-only family policy, not AB.3.3 node URI discovery support.
use super::*;
use crate::sc_frame::{BROADCAST_VMAC, UNKNOWN_VMAC};

pub(super) fn local_nak(msg: &ScMessage) -> Option<ScMessage> {
    // An ACK is a response even with an empty URI list or invalid local fields.
    // AB.2 forbids responding to responses and broadcasts.
    if msg.function != ScFunction::AddressResolution
        || msg.destination_vmac == Some(BROADCAST_VMAC)
        || matches!(msg.originating_vmac, Some(UNKNOWN_VMAC | BROADCAST_VMAC))
    {
        return None;
    }
    // Preserve the unsupported hub-local request policy, not the optional
    // direct-connection endpoint's AB.3.3 diagnostic/URI semantics.
    let mut nak = build_bvlc_result_nak(
        msg.message_id,
        msg.function,
        ErrorClass::COMMUNICATION,
        ErrorCode::UNEXPECTED_DATA,
    );
    nak.destination_vmac = msg.originating_vmac;
    Some(nak)
}
