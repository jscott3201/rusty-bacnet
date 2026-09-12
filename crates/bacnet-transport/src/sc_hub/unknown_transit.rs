//! Unknown-only hub local rejection; not general BVLC admission.
use super::*;
use crate::sc_frame::{BROADCAST_VMAC, UNKNOWN_VMAC};

pub(super) fn local_nak(msg: &ScMessage) -> Option<ScMessage> {
    if msg.destination_vmac == Some(BROADCAST_VMAC)
        || matches!(msg.originating_vmac, Some(UNKNOWN_VMAC | BROADCAST_VMAC))
    {
        return None;
    }
    let mut nak = build_bvlc_result_nak(
        msg.message_id,
        msg.function,
        ErrorClass::COMMUNICATION,
        ErrorCode::BVLC_FUNCTION_UNKNOWN,
    );
    nak.destination_vmac = msg.originating_vmac;
    Some(nak)
}
