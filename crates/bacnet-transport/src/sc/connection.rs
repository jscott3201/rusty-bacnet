//! BACnet/SC connection state machine.

use bacnet_types::error::Error;
use bytes::Bytes;
use tracing::{debug, warn};

use crate::port::DataAttribute;
use crate::sc_frame::{
    decode_sc_bvlc_result, is_broadcast_vmac, ScBvlcResult, ScFunction, ScMessage, Vmac,
};

use super::data_attributes;

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
}

impl ScConnection {
    pub fn new(local_vmac: Vmac, device_uuid: [u8; 16]) -> Self {
        Self {
            state: ScConnectionState::Disconnected,
            local_vmac,
            device_uuid,
            hub_vmac: None,
            max_bvlc_length: 1476,
            max_apdu_length: 1476,
            hub_max_bvlc_length: 1476,
            hub_max_apdu_length: 1476,
            next_message_id: 1,
            disconnect_ack_to_send: None,
            pending_connect_message_id: None,
            hub_device_uuid: None,
            connect_retry_allowed: true,
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
                warn!(
                    "ConnectAccept message_id {:#x} does not match request {:#x}",
                    msg.message_id, expected_id
                );
                return false;
            }
        }
        if msg.payload.len() != 26 {
            warn!(
                "ConnectAccept payload has {} bytes, expected 26",
                msg.payload.len()
            );
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
    /// Returns an error if not yet connected (no hub VMAC available).
    pub fn build_disconnect_request(&mut self) -> Result<ScMessage, Error> {
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
        Ok(ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: self.next_id(),
            originating_vmac: None,
            destination_vmac: Some(dest_vmac),
            dest_options: Vec::new(),
            data_options: data_attributes::to_data_options(data_attributes)?,
            payload: Bytes::copy_from_slice(npdu),
        })
    }

    /// Handle a received message. Returns NPDU data if it's an Encapsulated-NPDU for us.
    pub fn handle_received(&mut self, msg: &ScMessage) -> Option<(Bytes, Vmac)> {
        match msg.function {
            ScFunction::EncapsulatedNpdu => {
                if self.state != ScConnectionState::Connected {
                    debug!("Ignoring EncapsulatedNpdu in {:?} state", self.state);
                    return None;
                }
                if let Some(dest) = msg.destination_vmac {
                    if !is_broadcast_vmac(&dest) {
                        return None;
                    }
                }
                if msg.payload.len() > self.max_apdu_length as usize {
                    warn!(
                        "BACnet/SC NPDU ({} bytes) exceeds local Max-NPDU-Length ({}), dropping",
                        msg.payload.len(),
                        self.max_apdu_length
                    );
                    return None;
                }
                let source = msg.originating_vmac.unwrap_or([0; 6]);
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
                        warn!(
                            "BACnet/SC BVLC-Result NAK: function={:#x} \
                             error_class={} error_code={}",
                            result_for.to_raw(),
                            error_class,
                            error_code
                        );
                        self.state = ScConnectionState::Disconnected;
                    }
                    Err(e) => {
                        warn!("Malformed BACnet/SC BVLC-Result: {e}");
                        self.state = ScConnectionState::Disconnected;
                    }
                }
                None
            }
            _ => None,
        }
    }
}
