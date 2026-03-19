//! BACnet/SC connection state machine for WASM.
//!
//! Ported from `bacnet-transport/src/sc.rs` — pure sync logic with no tokio
//! dependencies. Manages the Connect → Connected → Disconnect lifecycle.

use bytes::Bytes;

use crate::sc_frame::{is_broadcast_vmac, ScFunction, ScMessage, Vmac};
use bacnet_types::error::Error;

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
    pub max_apdu_length: u16,
    pub hub_max_apdu_length: u16,
    next_message_id: u16,
    pub disconnect_ack_to_send: Option<ScMessage>,
}

impl ScConnection {
    pub fn new(local_vmac: Vmac) -> Self {
        Self {
            state: ScConnectionState::Disconnected,
            local_vmac,
            device_uuid: [0u8; 16],
            hub_vmac: None,
            max_apdu_length: 1476,
            hub_max_apdu_length: 1476,
            next_message_id: 1,
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
        let mut payload_buf = Vec::with_capacity(26);
        payload_buf.extend_from_slice(&self.local_vmac);
        payload_buf.extend_from_slice(&self.device_uuid);
        payload_buf.extend_from_slice(&1476u16.to_be_bytes());
        payload_buf.extend_from_slice(&self.max_apdu_length.to_be_bytes());
        ScMessage {
            function: ScFunction::ConnectRequest,
            message_id: self.next_id(),
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
        // AB.2.11.1: VMAC(6) + Device_UUID(16) + Max-BVLC-Length(2) + Max-NPDU-Length(2) = 26
        if msg.payload.len() >= 26 {
            let mut hub_vmac = [0u8; 6];
            hub_vmac.copy_from_slice(&msg.payload[0..6]);
            self.hub_vmac = Some(hub_vmac);
            self.hub_max_apdu_length = u16::from_be_bytes([msg.payload[24], msg.payload[25]]);
        } else if msg.payload.len() >= 6 {
            let mut hub_vmac = [0u8; 6];
            hub_vmac.copy_from_slice(&msg.payload[0..6]);
            self.hub_vmac = Some(hub_vmac);
        }
        self.state = ScConnectionState::Connected;
        true
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

    /// Build an Encapsulated-NPDU message.
    pub fn build_encapsulated_npdu(&mut self, dest_vmac: Vmac, npdu: &[u8]) -> ScMessage {
        ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: self.next_id(),
            originating_vmac: Some(self.local_vmac),
            destination_vmac: Some(dest_vmac),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::copy_from_slice(npdu),
        }
    }

    /// Handle a received message. Returns NPDU data + source VMAC if it's an Encapsulated-NPDU.
    pub fn handle_received(&mut self, msg: &ScMessage) -> Option<(Bytes, Vmac)> {
        match msg.function {
            ScFunction::EncapsulatedNpdu => {
                if self.state != ScConnectionState::Connected {
                    return None;
                }
                if let Some(dest) = msg.destination_vmac {
                    if dest != self.local_vmac && !is_broadcast_vmac(&dest) {
                        return None;
                    }
                }
                let source = msg.originating_vmac.unwrap_or([0; 6]);
                Some((msg.payload.clone(), source))
            }
            ScFunction::HeartbeatRequest => None,
            ScFunction::DisconnectRequest => {
                self.state = ScConnectionState::Disconnected;
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
                    self.state = ScConnectionState::Disconnected;
                }
                None
            }
            ScFunction::Result => {
                let is_error = !msg.payload.is_empty();
                if is_error {
                    self.state = ScConnectionState::Disconnected;
                }
                None
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_handshake() {
        let vmac = [1, 2, 3, 4, 5, 6];
        let mut conn = ScConnection::new(vmac);
        assert_eq!(conn.state, ScConnectionState::Disconnected);

        let req = conn.build_connect_request();
        assert_eq!(conn.state, ScConnectionState::Connecting);
        assert_eq!(req.function, ScFunction::ConnectRequest);
        // AB.2.10.1: no VMACs, 26-byte payload
        assert!(req.originating_vmac.is_none());
        assert_eq!(req.payload.len(), 26);

        // Simulate ConnectAccept with 26-byte payload
        let hub_vmac = [7, 8, 9, 10, 11, 12];
        let mut accept_payload = Vec::with_capacity(26);
        accept_payload.extend_from_slice(&hub_vmac);
        accept_payload.extend_from_slice(&[0u8; 16]); // Device UUID
        accept_payload.extend_from_slice(&1476u16.to_be_bytes());
        accept_payload.extend_from_slice(&1476u16.to_be_bytes());
        let accept = ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: req.message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(accept_payload),
        };
        assert!(conn.handle_connect_accept(&accept));
        assert_eq!(conn.state, ScConnectionState::Connected);
        assert_eq!(conn.hub_vmac, Some(hub_vmac));
        assert_eq!(conn.hub_max_apdu_length, 1476);
    }

    #[test]
    fn connect_accept_wrong_state() {
        let mut conn = ScConnection::new([1; 6]);
        // Not in Connecting state
        let msg = ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: 1,
            originating_vmac: Some([2; 6]),
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(vec![0; 10]),
        };
        assert!(!conn.handle_connect_accept(&msg));
    }

    #[test]
    fn disconnect_request_and_ack() {
        let vmac = [1; 6];
        let hub_vmac = [2; 6];
        let mut conn = ScConnection::new(vmac);
        conn.state = ScConnectionState::Connected;
        conn.hub_vmac = Some(hub_vmac);

        let req = conn.build_disconnect_request().unwrap();
        assert_eq!(conn.state, ScConnectionState::Disconnecting);
        assert_eq!(req.function, ScFunction::DisconnectRequest);
        // AB.2.12.1: no VMACs
        assert!(req.originating_vmac.is_none());
        assert!(req.destination_vmac.is_none());

        // Receive DisconnectAck
        let ack = ScMessage {
            function: ScFunction::DisconnectAck,
            message_id: req.message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        };
        conn.handle_received(&ack);
        assert_eq!(conn.state, ScConnectionState::Disconnected);
    }

    #[test]
    fn disconnect_without_hub_vmac() {
        let mut conn = ScConnection::new([1; 6]);
        assert!(conn.build_disconnect_request().is_err());
    }

    #[test]
    fn encapsulated_npdu_round_trip() {
        let vmac = [1; 6];
        let hub_vmac = [2; 6];
        let mut conn = ScConnection::new(vmac);
        conn.state = ScConnectionState::Connected;
        conn.hub_vmac = Some(hub_vmac);

        let npdu = vec![0x01, 0x04, 0x00];
        let msg = conn.build_encapsulated_npdu([3; 6], &npdu);
        assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
        assert_eq!(msg.destination_vmac, Some([3; 6]));
        assert_eq!(msg.payload.as_ref(), &npdu[..]);
    }

    #[test]
    fn handle_encapsulated_npdu_for_us() {
        let vmac = [1; 6];
        let mut conn = ScConnection::new(vmac);
        conn.state = ScConnectionState::Connected;

        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 42,
            originating_vmac: Some([2; 6]),
            destination_vmac: Some(vmac),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x01, 0x04]),
        };
        let result = conn.handle_received(&msg);
        assert!(result.is_some());
        let (data, source) = result.unwrap();
        assert_eq!(data.as_ref(), &[0x01, 0x04]);
        assert_eq!(source, [2; 6]);
    }

    #[test]
    fn handle_encapsulated_npdu_not_for_us() {
        let vmac = [1; 6];
        let mut conn = ScConnection::new(vmac);
        conn.state = ScConnectionState::Connected;

        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 42,
            originating_vmac: Some([2; 6]),
            destination_vmac: Some([3; 6]), // not for us
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x01]),
        };
        assert!(conn.handle_received(&msg).is_none());
    }

    #[test]
    fn handle_encapsulated_npdu_broadcast() {
        let vmac = [1; 6];
        let mut conn = ScConnection::new(vmac);
        conn.state = ScConnectionState::Connected;

        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 42,
            originating_vmac: Some([2; 6]),
            destination_vmac: Some([0xFF; 6]), // broadcast
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x01]),
        };
        assert!(conn.handle_received(&msg).is_some());
    }

    #[test]
    fn handle_disconnect_request_generates_ack() {
        let vmac = [1; 6];
        let mut conn = ScConnection::new(vmac);
        conn.state = ScConnectionState::Connected;

        let msg = ScMessage {
            function: ScFunction::DisconnectRequest,
            message_id: 99,
            originating_vmac: Some([2; 6]),
            destination_vmac: Some(vmac),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        };
        conn.handle_received(&msg);
        assert_eq!(conn.state, ScConnectionState::Disconnected);
        let ack = conn.disconnect_ack_to_send.take().unwrap();
        assert_eq!(ack.function, ScFunction::DisconnectAck);
        assert_eq!(ack.message_id, 99);
        // AB.2.13.1: no VMACs on DisconnectAck
        assert!(ack.originating_vmac.is_none());
        assert!(ack.destination_vmac.is_none());
    }

    #[test]
    fn handle_error_result_disconnects() {
        let mut conn = ScConnection::new([1; 6]);
        conn.state = ScConnectionState::Connected;

        let msg = ScMessage {
            function: ScFunction::Result,
            message_id: 1,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x06, 0x00, 0x01, 0x00, 0x01]), // error payload
        };
        conn.handle_received(&msg);
        assert_eq!(conn.state, ScConnectionState::Disconnected);
    }

    #[test]
    fn heartbeat() {
        let vmac = [1; 6];
        let hub_vmac = [2; 6];
        let mut conn = ScConnection::new(vmac);
        conn.hub_vmac = Some(hub_vmac);

        let hb = conn.build_heartbeat();
        assert_eq!(hb.function, ScFunction::HeartbeatRequest);
        // AB.2.14.1: no VMACs on HeartbeatRequest
        assert!(hb.originating_vmac.is_none());
        assert!(hb.destination_vmac.is_none());
        assert!(hb.payload.is_empty());
    }

    #[test]
    fn message_id_wraps() {
        let mut conn = ScConnection::new([1; 6]);
        conn.next_message_id = u16::MAX;
        assert_eq!(conn.next_id(), u16::MAX);
        assert_eq!(conn.next_id(), 0);
        assert_eq!(conn.next_id(), 1);
    }
}
