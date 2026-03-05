//! NPDU encoding and decoding per ASHRAE 135-2020 Clause 6.
//!
//! The Network Protocol Data Unit carries either an application-layer APDU
//! or a network-layer message, with optional source/destination routing
//! information for multi-hop BACnet internetworks.

use bacnet_types::enums::NetworkPriority;
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{BufMut, Bytes, BytesMut};

/// BACnet protocol version (always 1).
pub const BACNET_PROTOCOL_VERSION: u8 = 1;

// ---------------------------------------------------------------------------
// Address used in NPDU routing fields
// ---------------------------------------------------------------------------

/// Network-layer address: network number + MAC address.
///
/// Used for source/destination fields in routed NPDUs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NpduAddress {
    /// Network number (1-65534, or 0xFFFF for global broadcast destination).
    pub network: u16,
    /// MAC-layer address (variable length, empty for broadcast).
    pub mac_address: MacAddr,
}

// ---------------------------------------------------------------------------
// NPDU struct
// ---------------------------------------------------------------------------

/// Decoded Network Protocol Data Unit (Clause 6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Npdu {
    /// Whether this is a network-layer message (vs application-layer APDU).
    pub is_network_message: bool,
    /// Whether the sender expects a reply.
    pub expecting_reply: bool,
    /// Message priority.
    pub priority: NetworkPriority,
    /// Remote destination address, if routed.
    pub destination: Option<NpduAddress>,
    /// Originating source address (populated by routers).
    pub source: Option<NpduAddress>,
    /// Remaining hop count for routed messages (0-255).
    pub hop_count: u8,
    /// Network message type (when `is_network_message` is true).
    pub message_type: Option<u8>,
    /// Vendor ID for proprietary network messages (message_type >= 0x80).
    pub vendor_id: Option<u16>,
    /// Payload: either APDU bytes or network message data.
    pub payload: Bytes,
}

impl Default for Npdu {
    fn default() -> Self {
        Self {
            is_network_message: false,
            expecting_reply: false,
            priority: NetworkPriority::NORMAL,
            destination: None,
            source: None,
            hop_count: 255,
            message_type: None,
            vendor_id: None,
            payload: Bytes::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Encode an NPDU to wire format.
pub fn encode_npdu(buf: &mut BytesMut, npdu: &Npdu) -> Result<(), Error> {
    // Version
    buf.put_u8(BACNET_PROTOCOL_VERSION);

    // Control octet
    let mut control: u8 = npdu.priority.to_raw() & 0x03;
    if npdu.is_network_message {
        control |= 0x80;
    }
    if npdu.destination.is_some() {
        control |= 0x20;
    }
    if npdu.source.is_some() {
        control |= 0x08;
    }
    if npdu.expecting_reply {
        control |= 0x04;
    }
    buf.put_u8(control);

    // Destination (if present): DNET(2) + DLEN(1) + DADR(DLEN)
    if let Some(dest) = &npdu.destination {
        buf.put_u16(dest.network);
        if dest.mac_address.len() > 255 {
            return Err(Error::Encoding(
                "NPDU destination MAC address exceeds 255 bytes".into(),
            ));
        }
        buf.put_u8(dest.mac_address.len() as u8);
        buf.put_slice(&dest.mac_address);
    }

    // Source (if present): SNET(2) + SLEN(1) + SADR(SLEN)
    if let Some(src) = &npdu.source {
        buf.put_u16(src.network);
        if src.mac_address.len() > 255 {
            return Err(Error::Encoding(
                "NPDU source MAC address exceeds 255 bytes".into(),
            ));
        }
        buf.put_u8(src.mac_address.len() as u8);
        buf.put_slice(&src.mac_address);
    }

    // Hop count (only when destination present)
    if npdu.destination.is_some() {
        buf.put_u8(npdu.hop_count);
    }

    // Network message type or APDU payload
    if npdu.is_network_message {
        if let Some(msg_type) = npdu.message_type {
            buf.put_u8(msg_type);
            // Proprietary messages (0x80+) include a vendor ID
            if msg_type >= 0x80 {
                buf.put_u16(npdu.vendor_id.unwrap_or(0));
            }
        }
    }

    buf.put_slice(&npdu.payload);

    Ok(())
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Decode an NPDU from raw bytes.
///
/// Returns the decoded [`Npdu`]. The `payload` field contains either the
/// APDU bytes or network message data.
pub fn decode_npdu(data: Bytes) -> Result<Npdu, Error> {
    if data.len() < 2 {
        return Err(Error::buffer_too_short(2, data.len()));
    }

    let version = data[0];
    if version != BACNET_PROTOCOL_VERSION {
        return Err(Error::decoding(
            0,
            format!("unsupported BACnet protocol version: {version}"),
        ));
    }

    let control = data[1];
    let is_network_message = control & 0x80 != 0;
    let has_destination = control & 0x20 != 0;
    let has_source = control & 0x08 != 0;
    let expecting_reply = control & 0x04 != 0;
    let priority = NetworkPriority::from_raw(control & 0x03);

    if control & 0x50 != 0 {
        // Bits 4 (0x10) and 6 (0x40) are reserved per Clause 6.2.2
        tracing::warn!(
            control_byte = control,
            "NPDU control byte has reserved bits set (bits 4 or 6)"
        );
    }

    let mut offset = 2;
    let mut destination = None;
    let mut source = None;
    let mut hop_count: u8 = 255;

    // Destination
    if has_destination {
        if offset + 3 > data.len() {
            return Err(Error::decoding(
                offset,
                "NPDU too short for destination fields",
            ));
        }
        let dnet = u16::from_be_bytes([data[offset], data[offset + 1]]);
        offset += 2;
        let dlen = data[offset] as usize;
        offset += 1;

        if dlen > 0 && offset + dlen > data.len() {
            return Err(Error::decoding(
                offset,
                format!("NPDU destination address truncated: DLEN={dlen}"),
            ));
        }
        let dadr = MacAddr::from_slice(&data[offset..offset + dlen]);
        offset += dlen;

        if dnet == 0 {
            return Err(Error::decoding(
                offset - dlen - 3, // point back to DNET field
                "NPDU destination network 0 is invalid",
            ));
        }

        destination = Some(NpduAddress {
            network: dnet,
            mac_address: dadr,
        });
    }

    // Source
    if has_source {
        if offset + 3 > data.len() {
            return Err(Error::decoding(offset, "NPDU too short for source fields"));
        }
        let snet = u16::from_be_bytes([data[offset], data[offset + 1]]);
        offset += 2;
        let slen = data[offset] as usize;
        offset += 1;

        // SLEN=0 is invalid for source addresses per Clause 6.2.2
        // (source cannot be indeterminate — unlike DLEN=0 which means broadcast)
        if slen == 0 {
            return Err(Error::decoding(
                offset - 1,
                "NPDU source SLEN=0 is invalid (Clause 6.2.2)",
            ));
        }

        if slen > 0 && offset + slen > data.len() {
            return Err(Error::decoding(
                offset,
                format!("NPDU source address truncated: SLEN={slen}"),
            ));
        }
        let sadr = MacAddr::from_slice(&data[offset..offset + slen]);
        offset += slen;

        source = Some(NpduAddress {
            network: snet,
            mac_address: sadr,
        });

        if snet == 0 {
            return Err(Error::decoding(
                offset - slen - 3, // point back to SNET field
                "NPDU source network 0 is invalid",
            ));
        }
    }

    // Hop count (only when destination present)
    if has_destination {
        if offset >= data.len() {
            return Err(Error::decoding(offset, "NPDU too short for hop count"));
        }
        hop_count = data[offset];
        offset += 1;
    }

    // Network message type or remaining APDU
    let mut message_type = None;
    let mut vendor_id = None;

    if is_network_message {
        if offset >= data.len() {
            return Err(Error::decoding(
                offset,
                "NPDU too short for network message type",
            ));
        }
        let msg_type = data[offset];
        offset += 1;
        message_type = Some(msg_type);

        // Proprietary messages (0x80+) include a vendor ID
        if msg_type >= 0x80 {
            if offset + 2 > data.len() {
                return Err(Error::decoding(
                    offset,
                    "NPDU too short for proprietary vendor ID",
                ));
            }
            vendor_id = Some(u16::from_be_bytes([data[offset], data[offset + 1]]));
            offset += 2;
        }
    }

    let payload = data.slice(offset..);

    Ok(Npdu {
        is_network_message,
        expecting_reply,
        priority,
        destination,
        source,
        hop_count,
        message_type,
        vendor_id,
        payload,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_to_vec(npdu: &Npdu) -> Vec<u8> {
        let mut buf = BytesMut::with_capacity(64);
        encode_npdu(&mut buf, npdu).unwrap();
        buf.to_vec()
    }

    #[test]
    fn minimal_local_apdu() {
        // Simplest case: local, no routing, with APDU payload
        let npdu = Npdu {
            payload: Bytes::from_static(&[0x10, 0x08]), // UnconfirmedRequest WhoIs
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        // version=1, control=0x00 (normal priority, no flags), payload
        assert_eq!(encoded, vec![0x01, 0x00, 0x10, 0x08]);

        let decoded = decode_npdu(Bytes::from(encoded)).unwrap();
        assert_eq!(decoded, npdu);
    }

    #[test]
    fn expecting_reply_flag() {
        let npdu = Npdu {
            expecting_reply: true,
            payload: Bytes::from_static(&[0xAA]),
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        assert_eq!(encoded[1], 0x04); // control: expecting_reply bit
        let decoded = decode_npdu(Bytes::from(encoded)).unwrap();
        assert!(decoded.expecting_reply);
    }

    #[test]
    fn priority_encoding() {
        for (prio, expected_bits) in [
            (NetworkPriority::NORMAL, 0x00),
            (NetworkPriority::URGENT, 0x01),
            (NetworkPriority::CRITICAL_EQUIPMENT, 0x02),
            (NetworkPriority::LIFE_SAFETY, 0x03),
        ] {
            let npdu = Npdu {
                priority: prio,
                payload: Bytes::new(),
                ..Default::default()
            };
            let encoded = encode_to_vec(&npdu);
            assert_eq!(encoded[1] & 0x03, expected_bits);
            let decoded = decode_npdu(Bytes::from(encoded)).unwrap();
            assert_eq!(decoded.priority, prio);
        }
    }

    #[test]
    fn destination_only_round_trip() {
        let npdu = Npdu {
            destination: Some(NpduAddress {
                network: 1000,
                mac_address: MacAddr::from_slice(&[0x0A, 0x00, 0x01, 0x01, 0xBA, 0xC0]),
            }),
            hop_count: 254,
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        // control: has_destination = 0x20
        assert_eq!(encoded[1] & 0x20, 0x20);
        let decoded = decode_npdu(Bytes::from(encoded)).unwrap();
        assert_eq!(decoded, npdu);
    }

    #[test]
    fn destination_broadcast() {
        // Global broadcast: DNET=0xFFFF, DLEN=0 (no DADR)
        let npdu = Npdu {
            destination: Some(NpduAddress {
                network: 0xFFFF,
                mac_address: MacAddr::new(),
            }),
            hop_count: 255,
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        // version(1) + control(1) + DNET(2) + DLEN(1) + hop_count(1) + payload(2)
        assert_eq!(encoded.len(), 8);
        // DNET = 0xFFFF
        assert_eq!(&encoded[2..4], &[0xFF, 0xFF]);
        // DLEN = 0
        assert_eq!(encoded[4], 0);

        let decoded = decode_npdu(Bytes::from(encoded)).unwrap();
        assert_eq!(decoded, npdu);
    }

    #[test]
    fn source_only_round_trip() {
        let npdu = Npdu {
            source: Some(NpduAddress {
                network: 500,
                mac_address: MacAddr::from_slice(&[0x01]),
            }),
            payload: Bytes::from_static(&[0x30, 0x01, 0x0C]),
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        // control: has_source = 0x08
        assert_eq!(encoded[1] & 0x08, 0x08);
        // No hop count (no destination)
        let decoded = decode_npdu(Bytes::from(encoded)).unwrap();
        assert_eq!(decoded, npdu);
    }

    #[test]
    fn source_and_destination_round_trip() {
        let npdu = Npdu {
            expecting_reply: true,
            destination: Some(NpduAddress {
                network: 2000,
                mac_address: MacAddr::from_slice(&[0x0A, 0x00, 0x02, 0x01, 0xBA, 0xC0]),
            }),
            source: Some(NpduAddress {
                network: 1000,
                mac_address: MacAddr::from_slice(&[0x0A, 0x00, 0x01, 0x01, 0xBA, 0xC0]),
            }),
            hop_count: 250,
            payload: Bytes::from_static(&[0x00, 0x05, 0x01, 0x0C]),
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        // control: destination(0x20) | source(0x08) | expecting_reply(0x04) = 0x2C
        assert_eq!(encoded[1], 0x2C);
        let decoded = decode_npdu(Bytes::from(encoded)).unwrap();
        assert_eq!(decoded, npdu);
    }

    #[test]
    fn network_message_round_trip() {
        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(0x01), // I-Am-Router-To-Network
            payload: Bytes::from_static(&[0x03, 0xE8]), // network 1000
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        // control: network_message = 0x80
        assert_eq!(encoded[1] & 0x80, 0x80);
        let decoded = decode_npdu(Bytes::from(encoded)).unwrap();
        assert_eq!(decoded, npdu);
    }

    #[test]
    fn proprietary_network_message_round_trip() {
        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(0x80), // Proprietary range
            vendor_id: Some(999),
            payload: Bytes::from_static(&[0xDE, 0xAD]),
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        let decoded = decode_npdu(Bytes::from(encoded)).unwrap();
        assert_eq!(decoded, npdu);
    }

    #[test]
    fn wire_format_who_is_broadcast() {
        // Real-world WhoIs global broadcast:
        // Version=1, Control=0x20 (dest present), DNET=0xFFFF, DLEN=0,
        // HopCount=255, APDU=[0x10, 0x08]
        let wire = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];
        let decoded = decode_npdu(Bytes::copy_from_slice(&wire)).unwrap();
        assert!(!decoded.is_network_message);
        assert!(!decoded.expecting_reply);
        assert_eq!(decoded.priority, NetworkPriority::NORMAL);
        assert_eq!(
            decoded.destination,
            Some(NpduAddress {
                network: 0xFFFF,
                mac_address: MacAddr::new(),
            })
        );
        assert!(decoded.source.is_none());
        assert_eq!(decoded.hop_count, 255);
        assert_eq!(decoded.payload, vec![0x10, 0x08]);

        // Re-encode should match
        let reencoded = encode_to_vec(&decoded);
        assert_eq!(reencoded, wire);
    }

    #[test]
    fn decode_too_short() {
        assert!(decode_npdu(Bytes::new()).is_err());
        assert!(decode_npdu(Bytes::from_static(&[0x01])).is_err());
    }

    #[test]
    fn decode_wrong_version() {
        assert!(decode_npdu(Bytes::from_static(&[0x02, 0x00])).is_err());
    }

    #[test]
    fn decode_truncated_destination() {
        // Has destination flag but not enough bytes
        assert!(decode_npdu(Bytes::from_static(&[0x01, 0x20, 0xFF])).is_err());
    }

    #[test]
    fn decode_truncated_source() {
        // Has source flag but not enough bytes after destination
        assert!(decode_npdu(Bytes::from_static(&[0x01, 0x08, 0x00])).is_err());
    }

    // --- NPDU edge case tests ---

    #[test]
    fn npdu_network_zero() {
        // DNET=0 is invalid per Clause 6.2.2
        let npdu = Npdu {
            destination: Some(NpduAddress {
                network: 0,
                mac_address: MacAddr::from_slice(&[0x01]),
            }),
            hop_count: 255,
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        let result = decode_npdu(Bytes::from(encoded));
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("destination network 0"), "got: {err}");
    }

    #[test]
    fn npdu_network_fffe() {
        // 0xFFFE is the max non-broadcast network number
        // (0xFFFF is broadcast, 0xFFFE is the largest valid unicast network)
        let npdu = Npdu {
            destination: Some(NpduAddress {
                network: 0xFFFE,
                mac_address: MacAddr::from_slice(&[0x01, 0x02]),
            }),
            hop_count: 200,
            payload: Bytes::from_static(&[0xAA]),
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        let decoded = decode_npdu(Bytes::from(encoded)).unwrap();
        assert_eq!(decoded.destination.as_ref().unwrap().network, 0xFFFE);
        assert_eq!(decoded.hop_count, 200);
    }

    #[test]
    fn npdu_hop_count_zero() {
        // Hop count 0 is valid (means don't forward further)
        let npdu = Npdu {
            destination: Some(NpduAddress {
                network: 1000,
                mac_address: MacAddr::new(),
            }),
            hop_count: 0,
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        let decoded = decode_npdu(Bytes::from(encoded)).unwrap();
        assert_eq!(decoded.hop_count, 0);
    }

    #[test]
    fn npdu_source_with_empty_mac() {
        // SLEN=0 is invalid for source per Clause 6.2.2
        // (source cannot be indeterminate — unlike DLEN=0 which means broadcast)
        let npdu = Npdu {
            source: Some(NpduAddress {
                network: 500,
                mac_address: MacAddr::new(),
            }),
            payload: Bytes::from_static(&[0xBB]),
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        let result = decode_npdu(Bytes::from(encoded));
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("SLEN=0"), "got: {err}");
    }

    #[test]
    fn npdu_destination_dlen_zero_broadcast_accepted() {
        // DLEN=0 is valid for destination (broadcast) per Clause 6.2.2
        let npdu = Npdu {
            destination: Some(NpduAddress {
                network: 0xFFFF,
                mac_address: MacAddr::new(),
            }),
            hop_count: 255,
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        let decoded = decode_npdu(Bytes::from(encoded)).unwrap();
        assert_eq!(decoded.destination.as_ref().unwrap().network, 0xFFFF);
        assert!(decoded.destination.as_ref().unwrap().mac_address.is_empty());
    }

    #[test]
    fn npdu_destination_truncated_mac() {
        // DNET + DLEN present but MAC bytes are short
        // Version=1, Control=0x20 (dest present), DNET=1000, DLEN=6, only 2 MAC bytes
        let data = [0x01, 0x20, 0x03, 0xE8, 0x06, 0x01, 0x02];
        assert!(decode_npdu(Bytes::copy_from_slice(&data)).is_err());
    }

    #[test]
    fn npdu_source_truncated_mac() {
        // Source present but MAC bytes truncated
        let data = [0x01, 0x08, 0x01, 0xF4, 0x04, 0x01]; // SNET=500, SLEN=4, only 1 byte
        assert!(decode_npdu(Bytes::copy_from_slice(&data)).is_err());
    }

    #[test]
    fn npdu_missing_hop_count() {
        // Destination present but data ends before hop count
        // Version=1, Control=0x20, DNET=0xFFFF, DLEN=0
        let data = [0x01, 0x20, 0xFF, 0xFF, 0x00];
        assert!(decode_npdu(Bytes::copy_from_slice(&data)).is_err());
    }

    #[test]
    fn npdu_network_message_truncated_type() {
        // Network message flag set but no message type byte
        let data = [0x01, 0x80]; // is_network_message = true, but no type byte
        assert!(decode_npdu(Bytes::copy_from_slice(&data)).is_err());
    }

    #[test]
    fn npdu_proprietary_message_truncated_vendor() {
        // Proprietary message type (>=0x80) but vendor ID missing
        let data = [0x01, 0x80, 0x80]; // msg_type=0x80, need 2 more bytes for vendor
        assert!(decode_npdu(Bytes::copy_from_slice(&data)).is_err());
    }

    #[test]
    fn npdu_all_flags_round_trip() {
        // Maximum complexity: all flags set
        let npdu = Npdu {
            is_network_message: false,
            expecting_reply: true,
            priority: NetworkPriority::LIFE_SAFETY,
            destination: Some(NpduAddress {
                network: 2000,
                mac_address: MacAddr::from_slice(&[0x0A, 0x00, 0x02, 0x01, 0xBA, 0xC0]),
            }),
            source: Some(NpduAddress {
                network: 1000,
                mac_address: MacAddr::from_slice(&[0x0A, 0x00, 0x01, 0x01, 0xBA, 0xC0]),
            }),
            hop_count: 127,
            payload: Bytes::from_static(&[0xDE, 0xAD, 0xBE, 0xEF]),
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        let decoded = decode_npdu(Bytes::from(encoded)).unwrap();
        assert_eq!(decoded, npdu);
    }

    #[test]
    fn npdu_empty_payload() {
        // No payload at all
        let npdu = Npdu {
            payload: Bytes::new(),
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        let decoded = decode_npdu(Bytes::from(encoded)).unwrap();
        assert!(decoded.payload.is_empty());
    }

    #[test]
    fn reject_snet_zero() {
        // Source network 0 is invalid per Clause 6.2.2
        let npdu = Npdu {
            source: Some(NpduAddress {
                network: 0,
                mac_address: MacAddr::from_slice(&[0x01]),
            }),
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Default::default()
        };
        let encoded = encode_to_vec(&npdu);
        let result = decode_npdu(Bytes::from(encoded));
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("source network 0"), "got: {err}");
    }

    #[test]
    fn reserved_bits_warning_still_decodes() {
        // Reserved bits set in control byte should NOT cause decode failure
        // (warning only). Construct wire bytes manually with reserved bit 6 set.
        let mut data = vec![0x01, 0x40]; // version=1, control with reserved bit 6
                                         // Since no dest/source flags, just add payload
        data.extend_from_slice(&[0x10, 0x08]);

        // Should decode successfully (warning only, not error)
        let result = decode_npdu(Bytes::copy_from_slice(&data));
        assert!(
            result.is_ok(),
            "reserved bits should not cause decode failure"
        );
    }
}
