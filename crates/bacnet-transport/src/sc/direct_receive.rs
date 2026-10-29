//! Shared direct-connection wire admission for accepted and initiated peers.
use crate::sc_frame::{
    BROADCAST_VMAC, ScFunction, ScMessage, Vmac, first_must_understand_destination_option_marker,
};
use bytes::Bytes;
use tracing::warn;

/// Must-Understand Destination Option decision for an inbound direct NPDU.
///
/// Hub/node parity (`sc/data_attributes.rs` +
/// `sc/rejection.rs::unsupported_must_understand_destination_option`): an
/// Encapsulated-NPDU carrying any Must-Understand Destination Option is
/// never delivered. Unicast-shaped frames answer with a connection-local
/// BVLC-Result NAK (`COMMUNICATION`/`HEADER_NOT_UNDERSTOOD` carrying the
/// wire marker); broadcast-shaped frames drop silently. A frame whose wire
/// marker cannot be recovered also drops silently without a NAK, matching
/// the hub gate. Runs before the direct shape/payload gates so an MU
/// option is never lost to an earlier silent drop.
pub(crate) enum DirectMuDecision {
    /// No unsupported MU Destination Option: continue through the direct gates.
    Pass,
    /// Drop without delivery and without a NAK.
    Drop,
    /// Drop without delivery after sending this NAK on the direct socket.
    Nak(ScMessage),
}

pub(crate) fn direct_must_understand_decision(msg: &ScMessage, wire: &[u8]) -> DirectMuDecision {
    if msg.function != ScFunction::EncapsulatedNpdu {
        return DirectMuDecision::Pass;
    }
    if msg
        .dest_options
        .iter()
        .all(|option| !option.must_understand)
    {
        return DirectMuDecision::Pass;
    }
    if msg.destination_vmac == Some(BROADCAST_VMAC) {
        return DirectMuDecision::Drop;
    }
    match first_must_understand_destination_option_marker(wire) {
        Some(marker) => DirectMuDecision::Nak(direct_must_understand_nak(
            msg.message_id,
            marker,
            msg.originating_vmac,
        )),
        None => {
            warn!("direct NPDU with unsupported Destination Option lost its wire marker, dropping");
            DirectMuDecision::Drop
        }
    }
}

/// Connection-local BVLC-Result NAK for an unsupported MU Destination Option.
///
/// Mirrors `sc/data_attributes.rs::build_bvlc_result_nak` for the direct
/// socket: the NAK answers on the same connection, so a well-formed direct
/// NPDU (both VMACs omitted) yields a peer-addressed NAK with neither
/// address parameter, exactly like the hub mapping with an absent origin.
fn direct_must_understand_nak(
    message_id: u16,
    error_header_marker: u8,
    destination_vmac: Option<Vmac>,
) -> ScMessage {
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    let class = ErrorClass::COMMUNICATION.to_raw().to_be_bytes();
    let code = ErrorCode::HEADER_NOT_UNDERSTOOD.to_raw().to_be_bytes();
    ScMessage {
        function: ScFunction::Result,
        message_id,
        originating_vmac: None,
        destination_vmac,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(vec![
            ScFunction::EncapsulatedNpdu.to_raw(),
            0x01,
            error_header_marker,
            class[0],
            class[1],
            code[0],
            code[1],
        ]),
    }
}

/// Direct NPDU admission: unicast only with both addresses omitted.
///
/// Returns the NPDU bytes when the frame is a well-formed direct
/// Encapsulated-NPDU within local limits; otherwise `None` and the frame
/// is dropped without delivery or state change.
#[cfg(feature = "sc-tls")]
pub(crate) fn direct_npdu(msg: &ScMessage, max_npdu: u16) -> Option<Bytes> {
    if msg.originating_vmac.is_some() || msg.destination_vmac.is_some() {
        return None;
    }
    if msg.payload.is_empty() {
        return None;
    }
    if msg.payload.len() > max_npdu as usize {
        return None;
    }
    Some(msg.payload.clone())
}
