//! BACnet/SC connection state machine for WASM.
//!
//! Ported from `bacnet-transport/src/sc.rs` — pure sync logic with no tokio
//! dependencies. Manages the Connect → Connected → Disconnect lifecycle.

use bytes::Bytes;

use crate::data_attributes::{self, DataAttribute};
use crate::sc_frame::{
    decode_sc_bvlc_result, is_broadcast_vmac, ScBvlcResult, ScFunction, ScMessage, Vmac,
};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;

const DEFAULT_MAX_BVLC_LENGTH: u16 = 1476;
pub(crate) const DEFAULT_HEARTBEAT_INTERVAL_MS: u64 = 30_000;
pub(crate) const DEFAULT_HEARTBEAT_TIMEOUT_MS: u64 = 60_000;

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
pub struct ScConnection {
    pub state: ScConnectionState,
    pub local_vmac: Vmac,
    /// Device UUID (16 bytes, RFC 4122) per AB.1.5.3.
    pub device_uuid: [u8; 16],
    pub hub_vmac: Option<Vmac>,
    /// Maximum encoded BACnet/SC BVLC message length this node can accept.
    pub max_bvlc_length: u16,
    /// Maximum NPDU length this node can accept.
    pub max_apdu_length: u16,
    /// Maximum encoded BACnet/SC BVLC message length the hub can accept.
    pub hub_max_bvlc_length: u16,
    /// Maximum NPDU length the hub can accept.
    pub hub_max_apdu_length: u16,
    next_message_id: u16,
    pending_connect_message_id: Option<u16>,
    last_bvlc_received_ms: Option<u64>,
    pending_heartbeat_message_id: Option<u16>,
    pub disconnect_ack_to_send: Option<ScMessage>,
}

/// BACnet/SC NPDU received by a WASM/browser client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedScNpdu {
    /// Raw NPDU bytes.
    pub npdu: Bytes,
    /// Source VMAC, or the unknown VMAC when absent.
    pub source_vmac: Vmac,
    /// BACnet/SC Data Options exposed as data attributes.
    pub data_attributes: Vec<DataAttribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScHeartbeatAction {
    None,
    Send(ScMessage),
    Disconnect,
}

impl ScConnection {
    pub fn new(local_vmac: Vmac) -> Self {
        Self::new_with_device_uuid(local_vmac, [0u8; 16])
    }

    pub fn new_with_device_uuid(local_vmac: Vmac, device_uuid: [u8; 16]) -> Self {
        Self {
            state: ScConnectionState::Disconnected,
            local_vmac,
            device_uuid,
            hub_vmac: None,
            max_bvlc_length: DEFAULT_MAX_BVLC_LENGTH,
            max_apdu_length: DEFAULT_MAX_BVLC_LENGTH,
            hub_max_bvlc_length: DEFAULT_MAX_BVLC_LENGTH,
            hub_max_apdu_length: DEFAULT_MAX_BVLC_LENGTH,
            next_message_id: 1,
            pending_connect_message_id: None,
            last_bvlc_received_ms: None,
            pending_heartbeat_message_id: None,
            disconnect_ack_to_send: None,
        }
    }

    pub fn next_id(&mut self) -> u16 {
        let id = self.next_message_id;
        self.next_message_id = self.next_message_id.wrapping_add(1);
        id
    }

    /// Build a Connect-Request message.
    ///
    /// AB.2.10.1: VMAC(6) + Device_UUID(16) + Max-BVLC-Length(2) + Max-NPDU-Length(2) = 26 bytes.
    /// No Originating/Destination Virtual Address.
    pub fn build_connect_request(&mut self) -> ScMessage {
        self.state = ScConnectionState::Connecting;
        let message_id = self.next_id();
        self.pending_connect_message_id = Some(message_id);
        let mut payload_buf = Vec::with_capacity(26);
        payload_buf.extend_from_slice(&self.local_vmac);
        payload_buf.extend_from_slice(&self.device_uuid);
        payload_buf.extend_from_slice(&self.max_bvlc_length.to_be_bytes());
        payload_buf.extend_from_slice(&self.max_apdu_length.to_be_bytes());
        ScMessage {
            function: ScFunction::ConnectRequest,
            message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(payload_buf),
        }
    }

    /// Handle a received Connect-Accept (AB.2.11.1).
    pub fn handle_connect_accept(&mut self, msg: &ScMessage) -> bool {
        if self.state != ScConnectionState::Connecting {
            return false;
        }
        if msg.function != ScFunction::ConnectAccept {
            return false;
        }
        if self.pending_connect_message_id != Some(msg.message_id) {
            return false;
        }
        if msg.payload.len() != 26 {
            return false;
        }
        self.pending_connect_message_id = None;
        let mut hub_vmac = [0u8; 6];
        hub_vmac.copy_from_slice(&msg.payload[0..6]);
        self.hub_vmac = Some(hub_vmac);
        self.hub_max_bvlc_length = u16::from_be_bytes([msg.payload[22], msg.payload[23]]);
        self.hub_max_apdu_length = u16::from_be_bytes([msg.payload[24], msg.payload[25]]);
        self.state = ScConnectionState::Connected;
        true
    }

    pub fn abort_connect(&mut self) {
        self.mark_disconnected();
    }

    pub(crate) fn mark_disconnected(&mut self) {
        self.state = ScConnectionState::Disconnected;
        self.pending_connect_message_id = None;
        self.pending_heartbeat_message_id = None;
        self.last_bvlc_received_ms = None;
        self.disconnect_ack_to_send = None;
    }

    /// Handle a BVLC-Result received while waiting for Connect-Accept.
    ///
    /// Returns true when AB.6.2.2 duplicate-VMAC recovery installed the
    /// supplied replacement Random-48 VMAC.
    pub fn handle_connect_result(
        &mut self,
        result_message_id: u16,
        result: &ScBvlcResult,
        replacement_vmac: Option<Vmac>,
    ) -> Result<bool, Error> {
        let duplicate_vmac = self.connect_result_requires_random48_vmac(result_message_id, result);
        self.abort_connect();

        if duplicate_vmac {
            let replacement_vmac = replacement_vmac.ok_or_else(|| {
                Error::Encoding("duplicate VMAC recovery requires a replacement VMAC".into())
            })?;
            self.local_vmac = replacement_vmac;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn connect_result_requires_random48_vmac(
        &self,
        result_message_id: u16,
        result: &ScBvlcResult,
    ) -> bool {
        if self.pending_connect_message_id != Some(result_message_id) {
            return false;
        }

        let ScBvlcResult::Nak {
            result_for,
            error_class,
            error_code,
            ..
        } = result
        else {
            return false;
        };

        *result_for == ScFunction::ConnectRequest
            && *error_class == ErrorClass::COMMUNICATION.to_raw()
            && *error_code == ErrorCode::NODE_DUPLICATE_VMAC.to_raw()
    }

    /// Build a Disconnect-Request message.
    /// AB.2.12.1: No Originating/Destination Virtual Address.
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

    /// Build a Heartbeat-Request message.
    /// AB.2.14.1: No Originating/Destination Virtual Address.
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

    /// Build a Heartbeat-ACK message.
    /// AB.2.15.1: No Originating/Destination Virtual Address.
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

    pub(crate) fn start_heartbeat_tracking(&mut self, now_ms: u64) {
        self.last_bvlc_received_ms = Some(now_ms);
        self.pending_heartbeat_message_id = None;
    }

    pub(crate) fn record_heartbeat_activity(&mut self, now_ms: u64) {
        if self.state == ScConnectionState::Connected {
            self.last_bvlc_received_ms = Some(now_ms);
            self.pending_heartbeat_message_id = None;
        }
    }

    pub(crate) fn handle_heartbeat_ack(&mut self, msg: &ScMessage, now_ms: u64) -> bool {
        if self.heartbeat_ack_matches_outstanding(msg) {
            self.record_heartbeat_activity(now_ms);
            true
        } else {
            false
        }
    }

    pub(crate) fn next_heartbeat_action(
        &mut self,
        now_ms: u64,
        interval_ms: u64,
        timeout_ms: u64,
    ) -> ScHeartbeatAction {
        if self.state != ScConnectionState::Connected {
            return ScHeartbeatAction::None;
        }

        let Some(last_received) = self.last_bvlc_received_ms else {
            self.start_heartbeat_tracking(now_ms);
            return ScHeartbeatAction::None;
        };

        if now_ms < last_received {
            self.mark_disconnected();
            return ScHeartbeatAction::Disconnect;
        }

        let idle_ms = now_ms - last_received;
        if idle_ms > timeout_ms {
            self.mark_disconnected();
            return ScHeartbeatAction::Disconnect;
        }

        if idle_ms >= interval_ms && self.pending_heartbeat_message_id.is_none() {
            let heartbeat = self.build_heartbeat();
            self.pending_heartbeat_message_id = Some(heartbeat.message_id);
            return ScHeartbeatAction::Send(heartbeat);
        }

        ScHeartbeatAction::None
    }

    fn heartbeat_ack_matches_outstanding(&self, msg: &ScMessage) -> bool {
        msg.function == ScFunction::HeartbeatAck
            && self
                .pending_heartbeat_message_id
                .is_some_and(|message_id| msg.message_id == message_id)
            && msg.originating_vmac.is_none()
            && msg.destination_vmac.is_none()
            && msg.dest_options.is_empty()
            && msg.data_options.is_empty()
            && msg.payload.is_empty()
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

    /// Return the fail-closed Result response for an unsupported Must Understand Data Option.
    pub fn unsupported_must_understand_result(&self, msg: &ScMessage) -> Option<Option<ScMessage>> {
        data_attributes::unsupported_must_understand_result(msg)
    }

    /// Handle a received message. Returns NPDU data when it's an Encapsulated-NPDU for us.
    pub fn handle_received(&mut self, msg: &ScMessage) -> Option<ReceivedScNpdu> {
        match msg.function {
            ScFunction::EncapsulatedNpdu => {
                if self.state != ScConnectionState::Connected {
                    return None;
                }
                if let Some(dest) = msg.destination_vmac {
                    if !is_broadcast_vmac(&dest) {
                        return None;
                    }
                }
                if msg.payload.len() > self.max_apdu_length as usize {
                    return None;
                }
                let source = msg.originating_vmac.unwrap_or([0; 6]);
                Some(ReceivedScNpdu {
                    npdu: msg.payload.clone(),
                    source_vmac: source,
                    data_attributes: data_attributes::from_data_options(msg),
                })
            }
            ScFunction::HeartbeatRequest => None,
            ScFunction::DisconnectRequest => {
                self.mark_disconnected();
                // AB.2.13.1: Disconnect-ACK has no VMACs
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
                    self.mark_disconnected();
                }
                None
            }
            ScFunction::Result => {
                match decode_sc_bvlc_result(msg) {
                    Ok(ScBvlcResult::Ack { .. }) => {}
                    Ok(ScBvlcResult::Nak { .. }) | Err(_) => {
                        self.mark_disconnected();
                    }
                }
                None
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
