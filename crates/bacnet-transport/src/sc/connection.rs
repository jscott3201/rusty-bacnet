//! BACnet/SC connection state machine.

use bacnet_types::error::Error;
use bytes::Bytes;
use tracing::{debug, warn};

use crate::port::DataAttribute;
use crate::sc_frame::{
    decode_sc_bvlc_result, is_broadcast_vmac, ScBvlcResult, ScFunction, ScMessage, Vmac,
};

use super::diagnostic_throttle::DiagnosticThrottle;
use super::{data_attributes, source_admission};

/// BACnet/SC connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScConnectionState {
    /// Not connected.
    Disconnected,
    /// Connect-Request sent, waiting for Connect-Accept.
    Connecting,
    /// Connected and operational.
    Connected,
    /// Disconnect requested.
    Disconnecting,
}

/// BACnet/SC hub connection manager.
#[derive(Clone)]
pub struct ScConnection {
    pub state: ScConnectionState,
    pub local_vmac: Vmac,
    /// Device UUID (16 bytes, RFC 4122).
    pub device_uuid: [u8; 16],
    pub hub_vmac: Option<Vmac>,
    /// Maximum encoded BACnet/SC BVLC message length this node can accept.
    pub max_bvlc_length: u16,
    /// Maximum NPDU length this node can accept (sent in ConnectRequest).
    pub max_apdu_length: u16,
    /// Maximum encoded BACnet/SC BVLC message length the hub can accept.
    pub hub_max_bvlc_length: u16,
    /// Maximum NPDU length the hub can accept (learned from ConnectAccept).
    pub hub_max_apdu_length: u16,
    pub(super) next_message_id: u16,
    /// Pending Disconnect-ACK to send after receiving a Disconnect-Request.
    pub disconnect_ack_to_send: Option<ScMessage>,
    /// Message ID of the last ConnectRequest sent (for response verification).
    pub(super) pending_connect_message_id: Option<u16>,
    /// Device UUID of the connected hub.
    pub hub_device_uuid: Option<[u8; 16]>,
    /// Whether the last connect failure permits another connection attempt.
    pub(super) connect_retry_allowed: bool,
    /// Owner-local throttle for connection-path malformed diagnostics only.
    ///
    /// Logging-only: never affects state transitions, NPDU delivery, or NAK
    /// decisions. At most one diagnostic per second per connection; bursts
    /// count as suppressed for the next summary. Ignored by transactional
    /// field comparisons (logging state, not protocol state).
    malformed_diag: DiagnosticThrottle,
}

impl ScConnection {
    pub fn new(local_vmac: Vmac, device_uuid: [u8; 16]) -> Self {
        Self {
            state: ScConnectionState::Disconnected,
            local_vmac,
            device_uuid,
            hub_vmac: None,
            max_bvlc_length: crate::sc_limits::DEFAULT_MAX_BVLC_LENGTH,
            max_apdu_length: 1476,
            hub_max_bvlc_length: 1476,
            hub_max_apdu_length: 1476,
            next_message_id: 1,
            disconnect_ack_to_send: None,
            pending_connect_message_id: None,
            hub_device_uuid: None,
            connect_retry_allowed: true,
            malformed_diag: DiagnosticThrottle::new(),
        }
    }

    pub(super) fn connect_probe(&self) -> Self {
        let mut probe = Self::new(self.local_vmac, self.device_uuid);
        probe.max_bvlc_length = self.max_bvlc_length;
        probe.max_apdu_length = self.max_apdu_length;
        probe
    }

    pub(super) fn absorb_failed_probe(&mut self, probe: &Self) {
        self.local_vmac = probe.local_vmac;
        if !probe.connect_retry_allowed {
            self.connect_retry_allowed = false;
        }
    }

    /// Test-only suppressed diagnostic count for the connection throttle.
    ///
    /// Exposes the logging throttle without affecting wire decisions, so
    /// burst tests can assert O(1) diagnostics alongside bit-for-bit
    /// accept/NAK/silence behavior.
    #[cfg(test)]
    pub(super) fn malformed_diag_suppressed(&self) -> u64 {
        self.malformed_diag.suppressed()
    }

    /// Generate the next message ID.
    pub fn next_id(&mut self) -> u16 {
        let id = self.next_message_id;
        self.next_message_id = self.next_message_id.wrapping_add(1);
        id
    }

    /// Build a Connect-Request message (26-byte payload, no VMACs).
    pub fn build_connect_request(&mut self) -> ScMessage {
        self.state = ScConnectionState::Connecting;
        let mut payload_buf = Vec::with_capacity(26);
        payload_buf.extend_from_slice(&self.local_vmac);
        payload_buf.extend_from_slice(&self.device_uuid);
        payload_buf.extend_from_slice(&self.max_bvlc_length.to_be_bytes());
        payload_buf.extend_from_slice(&self.max_apdu_length.to_be_bytes());
        let msg_id = self.next_id();
        self.pending_connect_message_id = Some(msg_id);
        ScMessage {
            function: ScFunction::ConnectRequest,
            message_id: msg_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(payload_buf),
        }
    }

    /// Handle a received Connect-Accept (26-byte payload).
    pub fn handle_connect_accept(&mut self, msg: &ScMessage) -> bool {
        if self.state != ScConnectionState::Connecting {
            return false;
        }
        if msg.function != ScFunction::ConnectAccept {
            return false;
        }
        if let Some(expected_id) = self.pending_connect_message_id {
            if msg.message_id != expected_id {
                if self.malformed_diag.should_emit_now() {
                    let suppressed = self.malformed_diag.take_suppressed();
                    if suppressed > 0 {
                        warn!(
                            "ConnectAccept message_id {:#x} does not match request {:#x} (suppressed {suppressed} similar diagnostics)",
                            msg.message_id, expected_id
                        );
                    } else {
                        warn!(
                            "ConnectAccept message_id {:#x} does not match request {:#x}",
                            msg.message_id, expected_id
                        );
                    }
                }
                return false;
            }
        }
        if crate::sc_frame::connect_message_error(msg).is_some() {
            return false;
        }
        self.pending_connect_message_id = None;
        let mut hub_vmac = [0u8; 6];
        hub_vmac.copy_from_slice(&msg.payload[0..6]);
        self.hub_vmac = Some(hub_vmac);
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&msg.payload[6..22]);
        self.hub_device_uuid = Some(uuid);
        self.hub_max_bvlc_length = u16::from_be_bytes([msg.payload[22], msg.payload[23]]);
        self.hub_max_apdu_length = u16::from_be_bytes([msg.payload[24], msg.payload[25]]);
        self.state = ScConnectionState::Connected;
        true
    }

    /// Build a Disconnect-Request message (no VMACs).
    ///
    /// Returns an error if not connected.
    pub fn build_disconnect_request(&mut self) -> Result<ScMessage, Error> {
        if self.state != ScConnectionState::Connected {
            return Err(Error::Encoding(
                "cannot build DisconnectRequest: connection is not connected".into(),
            ));
        }
        if self.hub_vmac.is_none() {
            return Err(Error::Encoding(
                "cannot build DisconnectRequest: no hub VMAC (not connected)".into(),
            ));
        }
        self.state = ScConnectionState::Disconnecting;
        Ok(ScMessage {
            function: ScFunction::DisconnectRequest,
            message_id: self.next_id(),
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        })
    }

    /// Build a Heartbeat-Request message (no VMACs).
    pub fn build_heartbeat(&mut self) -> ScMessage {
        ScMessage {
            function: ScFunction::HeartbeatRequest,
            message_id: self.next_id(),
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        }
    }

    /// Build a Heartbeat-ACK message. Per Annex AB.2.15, no VMACs.
    pub fn build_heartbeat_ack(&self, request_message_id: u16) -> ScMessage {
        ScMessage {
            function: ScFunction::HeartbeatAck,
            message_id: request_message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        }
    }

    /// Build an Encapsulated-NPDU message.
    pub fn build_encapsulated_npdu(&mut self, dest_vmac: Vmac, npdu: &[u8]) -> ScMessage {
        self.build_encapsulated_npdu_with_data_attributes(dest_vmac, npdu, &[])
            .expect("empty data attributes are valid")
    }

    /// Build an Encapsulated-NPDU message with BACnet/SC Data Options.
    pub fn build_encapsulated_npdu_with_data_attributes(
        &mut self,
        dest_vmac: Vmac,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<ScMessage, Error> {
        let data_options = data_attributes::to_data_options(data_attributes)?;
        Ok(ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: self.next_id(),
            originating_vmac: None,
            destination_vmac: Some(dest_vmac),
            dest_options: Vec::new(),
            data_options,
            payload: Bytes::copy_from_slice(npdu),
        })
    }

    /// Build a solicited Advertisement reply (AB.2.8.1 content, AB.3.2 trigger).
    ///
    /// The caller supplies the destination selected by the request-addressing
    /// rule (`None` for a hub-peer solicitation, otherwise the solicitation
    /// origin) and the hub-connection status derived from live transport
    /// state (1 = primary hub, 2 = failover hub). The message ID is always
    /// fresh: a solicited Advertisement is not a "response message" and must
    /// not copy the solicitation ID (AB.3.1.3). Accept-direct is always 0 —
    /// this transport has no direct-connection accept path — and the two
    /// maxima echo the local receive configuration. No Data Options.
    pub fn build_solicited_advertisement(
        &mut self,
        destination_vmac: Option<Vmac>,
        hub_status: u8,
    ) -> ScMessage {
        debug_assert!(
            hub_status == 1 || hub_status == 2,
            "solicited Advertisement status must be 1 (primary) or 2 (failover)"
        );
        let mut payload = Vec::with_capacity(6);
        payload.push(hub_status);
        payload.push(0);
        payload.extend_from_slice(&self.max_bvlc_length.to_be_bytes());
        payload.extend_from_slice(&self.max_apdu_length.to_be_bytes());
        ScMessage {
            function: ScFunction::Advertisement,
            message_id: self.next_id(),
            originating_vmac: None,
            destination_vmac,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(payload),
        }
    }

    /// Build an Address-Resolution request for on-demand direct discovery.
    ///
    /// The destination names the target node; the origin is omitted because
    /// the sender is the originator. The payload is empty and no Data
    /// Options are present. The message ID is fresh from the shared counter
    /// so the later ACK can be correlated by ID. Only the ID counter moves.
    pub fn build_address_resolution_request(&mut self, destination_vmac: Vmac) -> ScMessage {
        ScMessage {
            function: ScFunction::AddressResolution,
            message_id: self.next_id(),
            originating_vmac: None,
            destination_vmac: Some(destination_vmac),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        }
    }

    /// Build a direct-connection Encapsulated-NPDU (peer-addressed, no VMACs).
    ///
    /// Used only for unicast over an established direct WebSocket to the
    /// connection peer: both address parameters are omitted. Hub sends keep
    /// the destination address. Only the ID counter moves besides the
    /// returned message.
    pub fn build_direct_encapsulated_npdu(
        &mut self,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<ScMessage, Error> {
        let data_options = data_attributes::to_data_options(data_attributes)?;
        Ok(ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: self.next_id(),
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options,
            payload: Bytes::copy_from_slice(npdu),
        })
    }

    /// Build an Address-Resolution-ACK reply for one accepted request.
    ///
    /// The destination mirrors the request origin (`None` for a hub-peer
    /// request so the reply stays peer-addressed, otherwise the requesting
    /// node) and the payload carries the configured space-joined URI list,
    /// or zero octets when unconfigured. The message ID is always fresh: an
    /// ACK answers the request but travels as its own message, matching the
    /// solicited-Advertisement precedent. No Data Options. The caller
    /// supplies already-validated payload bytes; only the ID counter moves.
    pub fn build_address_resolution_ack(
        &mut self,
        destination_vmac: Option<Vmac>,
        uri_payload: &[u8],
    ) -> ScMessage {
        ScMessage {
            function: ScFunction::AddressResolutionAck,
            message_id: self.next_id(),
            originating_vmac: None,
            destination_vmac,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::copy_from_slice(uri_payload),
        }
    }

    /// Handle a received message. Returns NPDU data if it's an Encapsulated-NPDU for us.
    /// Hub-relayed NPDUs must include a non-reserved Originating VMAC.
    pub fn handle_received(&mut self, msg: &ScMessage) -> Option<(Bytes, Vmac)> {
        if crate::sc_frame::control_envelope_error(msg).is_some() {
            return None;
        }
        match msg.function {
            ScFunction::EncapsulatedNpdu => {
                if self.state != ScConnectionState::Connected {
                    if self.malformed_diag.should_emit_now() {
                        let suppressed = self.malformed_diag.take_suppressed();
                        if suppressed > 0 {
                            debug!("Ignoring EncapsulatedNpdu in {:?} state (suppressed {suppressed} similar diagnostics)", self.state);
                        } else {
                            debug!("Ignoring EncapsulatedNpdu in {:?} state", self.state);
                        }
                    }
                    return None;
                }
                if let Some(dest) = msg.destination_vmac {
                    if !is_broadcast_vmac(&dest) {
                        return None;
                    }
                }
                if msg.payload.len() > self.max_apdu_length as usize {
                    if self.malformed_diag.should_emit_now() {
                        let suppressed = self.malformed_diag.take_suppressed();
                        if suppressed > 0 {
                            warn!(
                                "BACnet/SC NPDU ({} bytes) exceeds local Max-NPDU-Length ({}), dropping (suppressed {suppressed} similar diagnostics)",
                                msg.payload.len(),
                                self.max_apdu_length
                            );
                        } else {
                            warn!(
                                "BACnet/SC NPDU ({} bytes) exceeds local Max-NPDU-Length ({}), dropping",
                                msg.payload.len(),
                                self.max_apdu_length
                            );
                        }
                    }
                    return None;
                }
                let source = source_admission::hub_source(msg)?;
                if crate::sc_frame::missing_npdu_payload(msg) {
                    return None;
                }
                Some((msg.payload.clone(), source))
            }
            ScFunction::HeartbeatRequest => None,
            ScFunction::DisconnectRequest => {
                self.state = ScConnectionState::Disconnected;
                self.disconnect_ack_to_send = Some(ScMessage {
                    function: ScFunction::DisconnectAck,
                    message_id: msg.message_id,
                    originating_vmac: None,
                    destination_vmac: None,
                    dest_options: Vec::new(),
                    data_options: Vec::new(),
                    payload: Bytes::new(),
                });
                None
            }
            ScFunction::DisconnectAck => {
                if self.state == ScConnectionState::Disconnecting {
                    self.state = ScConnectionState::Disconnected;
                }
                None
            }
            ScFunction::Result => {
                match decode_sc_bvlc_result(msg) {
                    Ok(ScBvlcResult::Ack { .. }) => {}
                    Ok(ScBvlcResult::Nak {
                        result_for,
                        error_class,
                        error_code,
                        ..
                    }) => {
                        if self.malformed_diag.should_emit_now() {
                            let suppressed = self.malformed_diag.take_suppressed();
                            if suppressed > 0 {
                                warn!(
                                    "BACnet/SC BVLC-Result NAK: function={:#x} \
                                     error_class={} error_code={} (suppressed {suppressed} similar diagnostics)",
                                    result_for.to_raw(),
                                    error_class,
                                    error_code
                                );
                            } else {
                                warn!(
                                    "BACnet/SC BVLC-Result NAK: function={:#x} \
                                     error_class={} error_code={}",
                                    result_for.to_raw(),
                                    error_class,
                                    error_code
                                );
                            }
                        }
                        if result_for != ScFunction::EncapsulatedNpdu {
                            // Discovery negatives relayed from a target node
                            // (origin present) are normal: the peer does not
                            // support direct connections or knows no URIs.
                            // Stay connected so the sender can fall back to
                            // hub delivery. Hub-peer NAKs (origin absent)
                            // keep the existing fatal policy.
                            let discovery_negative = matches!(
                                result_for,
                                ScFunction::AddressResolution | ScFunction::AddressResolutionAck
                            ) && msg.originating_vmac.is_some();
                            if !discovery_negative {
                                self.state = ScConnectionState::Disconnected;
                            }
                        }
                    }
                    Err(e) => {
                        if self.malformed_diag.should_emit_now() {
                            let suppressed = self.malformed_diag.take_suppressed();
                            if suppressed > 0 {
                                warn!("Malformed BACnet/SC BVLC-Result: {e} (suppressed {suppressed} similar diagnostics)");
                            } else {
                                warn!("Malformed BACnet/SC BVLC-Result: {e}");
                            }
                        }
                        self.state = ScConnectionState::Disconnected;
                    }
                }
                None
            }
            ScFunction::Advertisement | ScFunction::AdvertisementSolicitation => {
                // Validated before activity by the rejection gate. Received
                // Advertisements need no local peer store and stay consumed
                // without NPDU delivery or state change. Solicited replies to
                // accepted solicitations are originated by the transport loop
                // (which owns the hub role, rate clock, and socket), so this
                // handler stays pure.
                None
            }
            ScFunction::ProprietaryMessage => {
                // Validated before activity by the rejection gate. Vendor
                // dispatch is a local matter; the frame is consumed without
                // NPDU delivery or state change.
                None
            }
            ScFunction::AddressResolution | ScFunction::AddressResolutionAck => {
                // Validated before activity by the rejection gate. Replies to
                // accepted requests are originated by the transport loop
                // (which owns the socket and the advertised URIs), so this
                // handler stays pure; discovery and dialing remain later
                // work. Well-formed bodies stay consumed without NPDU
                // delivery or state change.
                None
            }
            _ => None,
        }
    }
}
