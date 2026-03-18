//! BACnet/SC (Secure Connect) frame encode/decode per Addendum bj / ASHRAE 135-2020 Annex AB.
//!
//! BACnet/SC uses WebSocket binary messages. Each message contains a BVLC-SC
//! header followed by optional payload data.
//!
//! Wire format:
//! ```text
//! [bvlc_function] [control] [msg_id(2)] [orig_vmac(6)?] [dest_vmac(6)?] [dest_opts?]
//! [data_opts?] [payload...]
//! ```

use bacnet_types::error::Error;
use bytes::{BufMut, Bytes, BytesMut};

/// BACnet/SC BVLC function codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ScFunction {
    /// BVLC-Result
    Result = 0x00,
    /// Encapsulated-NPDU — carries BACnet NPDU data.
    EncapsulatedNpdu = 0x01,
    /// Address-Resolution
    AddressResolution = 0x02,
    /// Address-Resolution-ACK
    AddressResolutionAck = 0x03,
    /// Advertisement
    Advertisement = 0x04,
    /// Advertisement-Solicitation
    AdvertisementSolicitation = 0x05,
    /// Connect-Request
    ConnectRequest = 0x06,
    /// Connect-Accept
    ConnectAccept = 0x07,
    /// Disconnect-Request
    DisconnectRequest = 0x08,
    /// Disconnect-ACK
    DisconnectAck = 0x09,
    /// Heartbeat-Request
    HeartbeatRequest = 0x0A,
    /// Heartbeat-ACK
    HeartbeatAck = 0x0B,
    /// Proprietary-Message
    ProprietaryMessage = 0x0C,
    /// Unknown function code.
    Unknown(u8),
}

impl ScFunction {
    pub fn from_raw(val: u8) -> Self {
        match val {
            0x00 => Self::Result,
            0x01 => Self::EncapsulatedNpdu,
            0x02 => Self::AddressResolution,
            0x03 => Self::AddressResolutionAck,
            0x04 => Self::Advertisement,
            0x05 => Self::AdvertisementSolicitation,
            0x06 => Self::ConnectRequest,
            0x07 => Self::ConnectAccept,
            0x08 => Self::DisconnectRequest,
            0x09 => Self::DisconnectAck,
            0x0A => Self::HeartbeatRequest,
            0x0B => Self::HeartbeatAck,
            0x0C => Self::ProprietaryMessage,
            v => Self::Unknown(v),
        }
    }

    pub fn to_raw(self) -> u8 {
        match self {
            Self::Result => 0x00,
            Self::EncapsulatedNpdu => 0x01,
            Self::AddressResolution => 0x02,
            Self::AddressResolutionAck => 0x03,
            Self::Advertisement => 0x04,
            Self::AdvertisementSolicitation => 0x05,
            Self::ConnectRequest => 0x06,
            Self::ConnectAccept => 0x07,
            Self::DisconnectRequest => 0x08,
            Self::DisconnectAck => 0x09,
            Self::HeartbeatRequest => 0x0A,
            Self::HeartbeatAck => 0x0B,
            Self::ProprietaryMessage => 0x0C,
            Self::Unknown(v) => v,
        }
    }
}

/// BVLC-SC control flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScControl {
    /// Originating Virtual Address present.
    pub has_originating_vmac: bool,
    /// Destination Virtual Address present.
    pub has_destination_vmac: bool,
    /// Destination options present.
    pub has_dest_options: bool,
    /// Data options present.
    pub has_data_options: bool,
}

impl ScControl {
    /// Encode control flags to a byte. Bits 7-4 are reserved (zero); bits 3-0 carry the flags.
    pub fn to_byte(self) -> u8 {
        let mut b = 0u8;
        if self.has_originating_vmac {
            b |= 0x08; // bit 3
        }
        if self.has_destination_vmac {
            b |= 0x04; // bit 2
        }
        if self.has_dest_options {
            b |= 0x02; // bit 1
        }
        if self.has_data_options {
            b |= 0x01; // bit 0
        }
        b
    }

    /// Decode control flags from a byte.
    pub fn from_byte(b: u8) -> Self {
        Self {
            has_originating_vmac: b & 0x08 != 0, // bit 3
            has_destination_vmac: b & 0x04 != 0, // bit 2
            has_dest_options: b & 0x02 != 0,     // bit 1
            has_data_options: b & 0x01 != 0,     // bit 0
        }
    }
}

/// Virtual MAC address (6 bytes).
pub type Vmac = [u8; 6];

/// Broadcast VMAC (all 0xFF).
pub const BROADCAST_VMAC: Vmac = [0xFF; 6];

/// Unknown/uninitialized VMAC (all 0x00).
pub const UNKNOWN_VMAC: Vmac = [0x00; 6];

/// Check if a VMAC is the broadcast address (all 0xFF).
pub fn is_broadcast_vmac(vmac: &Vmac) -> bool {
    *vmac == BROADCAST_VMAC
}

/// A single BACnet/SC header option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScOption {
    /// Option type (bits 4..0, values 1..31).
    pub option_type: u8,
    /// If true, recipient must understand this option or reject the message.
    pub must_understand: bool,
    /// Option data (variable length). Empty for options with no data.
    pub data: Vec<u8>,
}

/// A decoded BACnet/SC BVLC message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScMessage {
    pub function: ScFunction,
    pub message_id: u16,
    pub originating_vmac: Option<Vmac>,
    pub destination_vmac: Option<Vmac>,
    /// Destination options (TLV-encoded).
    pub dest_options: Vec<ScOption>,
    /// Data options (TLV-encoded).
    pub data_options: Vec<ScOption>,
    /// Payload data (NPDU for EncapsulatedNpdu, function-specific otherwise).
    pub payload: Bytes,
}

/// Minimum BVLC-SC header: function(1) + control(1) + message_id(2) = 4.
const SC_MIN_HEADER: usize = 4;

/// Encode a BACnet/SC BVLC message into the buffer.
pub fn encode_sc_message(buf: &mut BytesMut, msg: &ScMessage) {
    let control = ScControl {
        has_originating_vmac: msg.originating_vmac.is_some(),
        has_destination_vmac: msg.destination_vmac.is_some(),
        has_dest_options: !msg.dest_options.is_empty(),
        has_data_options: !msg.data_options.is_empty(),
    };

    // Header
    buf.put_u8(msg.function.to_raw());
    buf.put_u8(control.to_byte());
    buf.put_u16(msg.message_id);

    // Optional VMACs
    if let Some(ref vmac) = msg.originating_vmac {
        buf.put_slice(vmac);
    }
    if let Some(ref vmac) = msg.destination_vmac {
        buf.put_slice(vmac);
    }

    // Destination options
    if !msg.dest_options.is_empty() {
        encode_sc_options(buf, &msg.dest_options);
    }

    // Data options
    if !msg.data_options.is_empty() {
        encode_sc_options(buf, &msg.data_options);
    }

    // Payload
    buf.put_slice(&msg.payload);
}

/// Encode SC header options.
fn encode_sc_options(buf: &mut BytesMut, options: &[ScOption]) {
    for (i, opt) in options.iter().enumerate() {
        let more_follows = i + 1 < options.len();
        let has_data = !opt.data.is_empty();
        let mut marker = opt.option_type & 0x1F; // bits 4..0
        if more_follows {
            marker |= 0x80; // bit 7
        }
        if opt.must_understand {
            marker |= 0x40; // bit 6
        }
        if has_data {
            marker |= 0x20; // bit 5 (Header Data Flag)
        }
        buf.put_u8(marker);
        if has_data {
            debug_assert!(
                opt.data.len() <= u16::MAX as usize,
                "SC option data too large"
            );
            let data_len = (opt.data.len() as u64).min(u16::MAX as u64) as u16;
            buf.put_u16(data_len);
            buf.put_slice(&opt.data);
        }
    }
}

/// Decode a BACnet/SC BVLC message from raw bytes.
pub fn decode_sc_message(data: &[u8]) -> Result<ScMessage, Error> {
    if data.len() < SC_MIN_HEADER {
        return Err(Error::decoding(0, "BACnet/SC message too short"));
    }

    let function = ScFunction::from_raw(data[0]);
    let control = ScControl::from_byte(data[1]);
    let message_id = u16::from_be_bytes([data[2], data[3]]);

    let mut offset = SC_MIN_HEADER;

    // Originating VMAC
    let originating_vmac = if control.has_originating_vmac {
        if data.len() < offset + 6 {
            return Err(Error::decoding(offset, "truncated originating VMAC"));
        }
        let mut vmac = [0u8; 6];
        vmac.copy_from_slice(&data[offset..offset + 6]);
        offset += 6;
        Some(vmac)
    } else {
        None
    };

    // Destination VMAC
    let destination_vmac = if control.has_destination_vmac {
        if data.len() < offset + 6 {
            return Err(Error::decoding(offset, "truncated destination VMAC"));
        }
        let mut vmac = [0u8; 6];
        vmac.copy_from_slice(&data[offset..offset + 6]);
        offset += 6;
        Some(vmac)
    } else {
        None
    };

    // Decode destination options and data options (TLV-encoded, variable length)
    let dest_options = if control.has_dest_options {
        decode_sc_options(data, &mut offset)?
    } else {
        Vec::new()
    };
    let data_options = if control.has_data_options {
        decode_sc_options(data, &mut offset)?
    } else {
        Vec::new()
    };

    // Remaining data is payload
    let payload = Bytes::copy_from_slice(&data[offset..]);

    Ok(ScMessage {
        function,
        message_id,
        originating_vmac,
        destination_vmac,
        dest_options,
        data_options,
        payload,
    })
}

/// Decode SC header options.
fn decode_sc_options(data: &[u8], offset: &mut usize) -> Result<Vec<ScOption>, Error> {
    const MAX_SC_OPTIONS: usize = 64;
    let mut options = Vec::new();
    loop {
        if *offset >= data.len() {
            return Err(Error::decoding(*offset, "SC option truncated"));
        }
        let marker = data[*offset];
        let option_type = marker & 0x1F; // bits 4..0
        let must_understand = marker & 0x40 != 0; // bit 6
        let has_data = marker & 0x20 != 0; // bit 5
        let more_follows = marker & 0x80 != 0; // bit 7
        *offset += 1;

        let option_data = if has_data {
            if *offset + 2 > data.len() {
                return Err(Error::decoding(*offset, "SC option length truncated"));
            }
            let length = u16::from_be_bytes([data[*offset], data[*offset + 1]]) as usize;
            *offset += 2;
            if *offset + length > data.len() {
                return Err(Error::decoding(*offset, "SC option data truncated"));
            }
            let d = data[*offset..*offset + length].to_vec();
            *offset += length;
            d
        } else {
            Vec::new()
        };

        if options.len() >= MAX_SC_OPTIONS {
            return Err(Error::decoding(*offset, "too many SC options"));
        }
        options.push(ScOption {
            option_type,
            must_understand,
            data: option_data,
        });
        if !more_follows {
            break;
        }
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_round_trip() {
        for raw in 0x00..=0x0C {
            let f = ScFunction::from_raw(raw);
            assert_eq!(f.to_raw(), raw);
        }
        let f = ScFunction::from_raw(0x42);
        assert_eq!(f.to_raw(), 0x42);
        assert_eq!(f, ScFunction::Unknown(0x42));
    }

    #[test]
    fn control_round_trip() {
        let ctrl = ScControl {
            has_originating_vmac: true,
            has_destination_vmac: false,
            has_dest_options: true,
            has_data_options: false,
        };
        let b = ctrl.to_byte();
        assert_eq!(b, 0x0A);
        let decoded = ScControl::from_byte(b);
        assert_eq!(decoded, ctrl);
    }

    #[test]
    fn control_all_flags() {
        let ctrl = ScControl {
            has_originating_vmac: true,
            has_destination_vmac: true,
            has_dest_options: true,
            has_data_options: true,
        };
        assert_eq!(ctrl.to_byte(), 0x0F);
        assert_eq!(ScControl::from_byte(0x0F), ctrl);
    }

    #[test]
    fn control_no_flags() {
        let ctrl = ScControl::default();
        assert_eq!(ctrl.to_byte(), 0x00);
    }

    #[test]
    fn encapsulated_npdu_round_trip() {
        let npdu = vec![0x01, 0x00, 0x10, 0x02];
        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 42,
            originating_vmac: Some([0x01, 0x02, 0x03, 0x04, 0x05, 0x06]),
            destination_vmac: Some([0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F]),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(npdu.clone()),
        };

        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);

        let decoded = decode_sc_message(&buf).unwrap();
        assert_eq!(decoded.function, ScFunction::EncapsulatedNpdu);
        assert_eq!(decoded.message_id, 42);
        assert_eq!(
            decoded.originating_vmac,
            Some([0x01, 0x02, 0x03, 0x04, 0x05, 0x06])
        );
        assert_eq!(
            decoded.destination_vmac,
            Some([0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F])
        );
        assert_eq!(decoded.payload, npdu);
    }

    #[test]
    fn heartbeat_no_vmacs() {
        let msg = ScMessage {
            function: ScFunction::HeartbeatRequest,
            message_id: 1,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        };

        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);
        // Minimum: function(1) + control(1) + msg_id(2) = 4
        assert_eq!(buf.len(), 4);

        let decoded = decode_sc_message(&buf).unwrap();
        assert_eq!(decoded.function, ScFunction::HeartbeatRequest);
        assert_eq!(decoded.message_id, 1);
        assert!(decoded.originating_vmac.is_none());
        assert!(decoded.destination_vmac.is_none());
        assert!(decoded.payload.is_empty());
    }

    #[test]
    fn originating_vmac_only() {
        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 100,
            originating_vmac: Some([0xAA; 6]),
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x01, 0x20]),
        };

        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);

        let decoded = decode_sc_message(&buf).unwrap();
        assert_eq!(decoded.originating_vmac, Some([0xAA; 6]));
        assert!(decoded.destination_vmac.is_none());
        assert_eq!(decoded.payload, vec![0x01, 0x20]);
    }

    #[test]
    fn destination_vmac_only() {
        let msg = ScMessage {
            function: ScFunction::AddressResolution,
            message_id: 200,
            originating_vmac: None,
            destination_vmac: Some(BROADCAST_VMAC),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        };

        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);

        let decoded = decode_sc_message(&buf).unwrap();
        assert!(decoded.originating_vmac.is_none());
        assert_eq!(decoded.destination_vmac, Some(BROADCAST_VMAC));
    }

    #[test]
    fn connect_request_round_trip() {
        let msg = ScMessage {
            function: ScFunction::ConnectRequest,
            message_id: 0xFFFF,
            originating_vmac: Some([0x01; 6]),
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x00, 0x01]), // VMAC of requested hub
        };

        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);

        let decoded = decode_sc_message(&buf).unwrap();
        assert_eq!(decoded.function, ScFunction::ConnectRequest);
        assert_eq!(decoded.message_id, 0xFFFF);
        assert_eq!(decoded.payload, vec![0x00, 0x01]);
    }

    #[test]
    fn decode_too_short() {
        assert!(decode_sc_message(&[0x01, 0x00]).is_err());
    }

    #[test]
    fn decode_truncated_originating_vmac() {
        // Has originating VMAC flag (bit 3) but only 2 bytes after header
        let data = [0x01, 0x08, 0x00, 0x01, 0xAA, 0xBB];
        assert!(decode_sc_message(&data).is_err());
    }

    #[test]
    fn decode_truncated_destination_vmac() {
        // Has destination VMAC flag (bit 2) but only 2 bytes after header
        let data = [0x01, 0x04, 0x00, 0x01, 0xAA, 0xBB];
        assert!(decode_sc_message(&data).is_err());
    }

    #[test]
    fn wire_format_check() {
        // No VMACs — control byte should be 0x00 (no flags set).
        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 0x0042,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x01, 0x00]),
        };

        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);

        assert_eq!(buf[0], 0x01); // EncapsulatedNpdu
        assert_eq!(buf[1], 0x00); // no flags
        assert_eq!(buf[2], 0x00); // msg_id high
        assert_eq!(buf[3], 0x42); // msg_id low
        assert_eq!(&buf[4..], &[0x01, 0x00]); // payload
    }

    #[test]
    fn wire_format_check_both_vmacs() {
        // Both VMACs present: bits 3 (originating) + 2 (destination) = 0x0C.
        let orig = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
        let dest = [0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F];
        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 0x0001,
            originating_vmac: Some(orig),
            destination_vmac: Some(dest),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0xFF]),
        };

        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);

        assert_eq!(buf[0], 0x01); // EncapsulatedNpdu
        assert_eq!(buf[1], 0x0C);
        assert_eq!(buf[2], 0x00); // msg_id high
        assert_eq!(buf[3], 0x01); // msg_id low
        assert_eq!(&buf[4..10], &orig);
        assert_eq!(&buf[10..16], &dest);
        assert_eq!(&buf[16..], &[0xFF]); // payload
    }

    #[test]
    fn sc_options_round_trip() {
        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 42,
            originating_vmac: Some([0x01; 6]),
            destination_vmac: Some([0x02; 6]),
            dest_options: vec![ScOption {
                option_type: 1,
                must_understand: false,
                data: vec![0xAA, 0xBB],
            }],
            data_options: vec![ScOption {
                option_type: 2,
                must_understand: false,
                data: vec![0xCC],
            }],
            payload: Bytes::from_static(&[0x01, 0x00]),
        };
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);
        let decoded = decode_sc_message(&buf).unwrap();
        assert_eq!(decoded.dest_options, msg.dest_options);
        assert_eq!(decoded.data_options, msg.data_options);
        assert_eq!(decoded.payload, msg.payload);
    }

    #[test]
    fn sc_options_empty_round_trip() {
        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 1,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x01, 0x00]),
        };
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);
        let decoded = decode_sc_message(&buf).unwrap();
        assert!(decoded.dest_options.is_empty());
        assert!(decoded.data_options.is_empty());
    }

    #[test]
    fn sc_options_multiple_chained() {
        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 99,
            originating_vmac: Some([0x01; 6]),
            destination_vmac: None,
            dest_options: vec![
                ScOption {
                    option_type: 1,
                    must_understand: false,
                    data: vec![0x10],
                },
                ScOption {
                    option_type: 2,
                    must_understand: true,
                    data: vec![0x20, 0x21],
                },
            ],
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0xFF]),
        };
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);
        let decoded = decode_sc_message(&buf).unwrap();
        assert_eq!(decoded.dest_options.len(), 2);
        assert_eq!(decoded.dest_options[0].option_type, 1);
        assert!(!decoded.dest_options[0].must_understand);
        assert_eq!(decoded.dest_options[1].option_type, 2);
        assert!(decoded.dest_options[1].must_understand);
    }
}
