//! Established-node Address-Resolution admission and answering, not
//! discovery or dialing.
//!
//! Well-formed request bodies earn an ACK from [`maybe_answer`] carrying the
//! configured URI list (empty when unconfigured); well-formed response
//! bodies stay silently consumed with accepted activity (no NPDU, no state
//! change), per the response rule. Malformed request bodies draw the
//! validator's diagnostic as a connection-local NAK for locally-addressed
//! unicast; malformed response bodies are always silent because the response
//! rule forbids answering responses. Envelope routing (explicit
//! destinations, reserved origins, broadcast silence) matches the
//! Advertisement and Proprietary gates.

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::BytesMut;
use tokio::sync::Mutex;
use tracing::warn;

use super::connection::{ScConnection, ScConnectionState};
use super::data_attributes::build_bvlc_result_nak;
use super::rejection::{RejectionBudget, RejectionExpired};
use super::WebSocketPort;
use crate::sc_frame::{
    address_resolution_message_error, encode_sc_message,
    first_must_understand_destination_option_marker, is_valid_wss_uri, ScFunction, ScMessage, Vmac,
    BROADCAST_VMAC,
};

/// Largest answer envelope an Address-Resolution-ACK can need: base header
/// plus a destination VMAC. Advertised URI lists must leave room for it
/// inside the local receive budget.
const ADDRESS_RESOLUTION_ACK_ENVELOPE: usize = 10;

// The builder lives here rather than beside the other `with_*` setters so
// the transport loop file stays within the repository file-size cap; it is
// the same public builder surface either way.
impl<W: WebSocketPort> super::ScTransport<W> {
    /// Configure the direct-connection URIs this node advertises (builder-style).
    ///
    /// Each entry must have the secure WebSocket shape the receive-side
    /// validator accepts (scheme, host, visible characters), and the
    /// space-joined list must fit its own answer envelope inside the local
    /// receive budget. Accepted requests are answered with an
    /// Address-Resolution-ACK carrying the joined list; an unconfigured node
    /// answers with a valid empty list, never a refusal. Discovery, dialing,
    /// and hub behavior are unaffected.
    ///
    /// # Panics
    ///
    /// Panics naming the first offending URI when any entry is malformed, or
    /// when the joined list would not fit the local budget.
    pub fn with_advertised_uris(mut self, uris: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        self.advertised_uris = validate_advertised_uris(uris);
        self
    }
}

/// Validate builder-supplied advertised URIs with the receive-side shape
/// check and bound the joined list to the local BVLC budget.
///
/// Returns the owned list for transport storage. Panics naming the first
/// offending URI, or when the space-joined payload would not fit its own
/// answer envelope inside the local receive budget.
pub(super) fn validate_advertised_uris<I, S>(uris: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let collected: Vec<String> = uris
        .into_iter()
        .map(|uri| uri.as_ref().to_owned())
        .collect();
    for uri in &collected {
        assert!(
            is_valid_wss_uri(uri),
            "BACnet/SC advertised URI is not a valid direct-connection URI: {uri}"
        );
    }
    assert!(
        collected.join(" ").len() + ADDRESS_RESOLUTION_ACK_ENVELOPE
            <= crate::sc_limits::DEFAULT_MAX_BVLC_LENGTH as usize,
        "BACnet/SC advertised URI list exceeds the local BVLC budget"
    );
    collected
}

/// Answerable-request predicate for node Address-Resolution replies.
///
/// Returns the reply destination (the request origin, or `None` for a
/// hub-peer request so the reply stays peer-addressed) when the frame is a
/// well-formed, locally-addressed request. Malformed shapes keep the
/// existing NAK path; addressed or reserved-origin envelopes stay silent;
/// responses are never answered, per the response rule.
pub(super) fn answer_destination(msg: &ScMessage) -> Option<Option<Vmac>> {
    if msg.function != ScFunction::AddressResolution {
        return None;
    }
    if address_resolution_message_error(msg).is_some() {
        return None;
    }
    if msg.destination_vmac.is_some() {
        return None;
    }
    if matches!(msg.originating_vmac, Some(vmac) if vmac == [0; 6] || vmac == BROADCAST_VMAC) {
        return None;
    }
    Some(msg.originating_vmac)
}

/// Answer one accepted Address-Resolution request with an ACK carrying the
/// configured URI payload (empty when unconfigured).
///
/// Best-effort like Heartbeat-ACK: no rejection budget is spent on this
/// positive answer, and no separate rate gate is added — each valid unicast
/// request earns one answer, while the broadcast/response/addressed
/// envelopes that could amplify a storm stay silent and malformed shapes
/// keep the budgeted NAK path. The ACK copies the request message ID per
/// the response-ID rule (AB.2 response list, AB.2.7.1, AB.3.1.3); unlike a
/// solicited Advertisement it is a response message, so no fresh ID is
/// allocated. The Connected check shares one lock hold so an answer cannot
/// race a reconnect half-way.
pub(super) async fn maybe_answer<W: WebSocketPort>(
    msg: &ScMessage,
    conn: &Mutex<ScConnection>,
    ws: &W,
    advertised_payload: &[u8],
) {
    let destination = match answer_destination(msg) {
        Some(destination) => destination,
        None => return,
    };
    let reply = {
        let c = conn.lock().await;
        if c.state != ScConnectionState::Connected {
            return;
        }
        c.build_address_resolution_ack(msg.message_id, destination, advertised_payload)
    };
    let mut bytes = BytesMut::new();
    encode_sc_message(&mut bytes, &reply);
    if let Err(e) = ws.send(&bytes).await {
        warn!("BACnet/SC address-resolution ACK send error: {}", e);
    }
}

pub(super) async fn reject<W: WebSocketPort>(
    msg: &ScMessage,
    wire: &[u8],
    ws: &W,
    budget: RejectionBudget,
) -> Result<bool, RejectionExpired> {
    if !matches!(
        msg.function,
        ScFunction::AddressResolution | ScFunction::AddressResolutionAck
    ) {
        return Ok(false);
    }
    // Hub-connector AB.5.4: explicit destinations are not for the local
    // BVLL. Reserved origins are also silent; neither decision spends a
    // budget. Broadcast destinations arrive with an explicit address, so
    // the broadcast-silence rule needs no separate branch.
    if msg.destination_vmac.is_some()
        || matches!(msg.originating_vmac, Some(vmac) if vmac == [0; 6] || vmac == [0xFF; 6])
    {
        return Ok(true);
    }
    // Valid shapes are answered by `maybe_answer` (empty versus populated
    // URI list) and stay consumed with accepted activity. Discovery and
    // dialing remain later work; this gate only rejects malformed bodies
    // before dispatch.
    let Some(code) = address_resolution_message_error(msg) else {
        return Ok(false);
    };
    // Responses never draw a response, even when malformed. The malformed
    // response is discarded without activity, probe, or NPDU effects.
    if msg.function == ScFunction::AddressResolutionAck {
        return Ok(true);
    }
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
        warn!("BACnet/SC address-resolution NAK send error: {}", e);
    }
    Ok(true)
}
