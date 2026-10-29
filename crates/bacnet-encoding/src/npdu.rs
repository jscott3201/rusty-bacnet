//! The owned NPDU (ASHRAE 135-2020 Clause 6) and its codec.
//!
//! The Network Protocol Data Unit carries either an application-layer APDU
//! or a network-layer message, with optional source/destination routing
//! information for multi-hop BACnet internetworks.
//!
//! [`Npdu`] holds its addresses and payload in owned buffers. It decodes
//! and encodes through `bacnet-npdu`'s borrowed view, the one NPDU codec of
//! the stack, so the router core, the non-router network layer and every
//! layer above them accept and refuse the same frames. Both directions hold
//! DADR and SADR to [`MAX_MAC_LEN`] octets (#1141), so everything above the
//! network layer only ever sees an address that some data link could carry.

use bacnet_npdu::{Address, EncodeError, MAX_MAC_LEN, NetworkMessageType, NetworkPriority};
use bacnet_types::MacAddr;
use bacnet_types::error::{Error, RangeError};
use bytes::{Bytes, BytesMut};

mod decode_error;
pub use decode_error::NpduDecodeError;

/// `bacnet-npdu`, whose types this module's API speaks (the priority, the
/// message type, the decode faults and the address bound), re-exported so
/// its version can't drift apart from this crate's: the view this codec
/// runs on is [`view::Npdu`].
pub use bacnet_npdu as view;

// ---------------------------------------------------------------------------
// Address used in NPDU routing fields
// ---------------------------------------------------------------------------

/// Network-layer address: network number + MAC address.
///
/// Used for source/destination fields in routed NPDUs. The MAC holds at most
/// [`MAX_MAC_LEN`] octets on the wire: the decoder refuses a longer one and
/// the encoder refuses to write it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NpduAddress {
    /// Network number (1-65534, or 0xFFFF for global broadcast destination).
    pub network: u16,
    /// MAC-layer address (variable length, empty for broadcast).
    pub mac_address: MacAddr,
}

impl NpduAddress {
    /// The borrowed form `bacnet-npdu`'s view uses.
    fn view(&self) -> Address<'_> {
        Address {
            network: self.network,
            mac: &self.mac_address,
        }
    }
}

/// An owned copy of a borrowed address.
impl From<Address<'_>> for NpduAddress {
    fn from(address: Address<'_>) -> Self {
        Self {
            network: address.network,
            mac_address: MacAddr::from_slice(address.mac),
        }
    }
}

// ---------------------------------------------------------------------------
// NPDU struct
// ---------------------------------------------------------------------------

/// Decoded Network Protocol Data Unit (Clause 6.2).
///
/// A network-layer message is one whose [`message_type`](Self::message_type)
/// is set; without it the payload is an APDU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Npdu {
    /// Whether the sender expects a reply.
    pub expecting_reply: bool,
    /// Message priority.
    pub priority: NetworkPriority,
    /// Remote destination address, if routed.
    pub destination: Option<NpduAddress>,
    /// Originating source address (populated by routers).
    pub source: Option<NpduAddress>,
    /// Remaining hop count for routed messages (0-255). Only an NPDU with a
    /// destination carries one: without it this is 255, and the encoder
    /// leaves it out.
    pub hop_count: u8,
    /// The network-layer message type, or `None` for an APDU.
    pub message_type: Option<NetworkMessageType>,
    /// Vendor ID for a proprietary network message type
    /// ([`NetworkMessageType::is_proprietary`]).
    pub vendor_id: Option<u16>,
    /// Payload: either APDU bytes or network message data.
    pub payload: Bytes,
}

impl Npdu {
    /// Whether it carries a network-layer message rather than an APDU.
    pub fn is_network_message(&self) -> bool {
        self.message_type.is_some()
    }

    /// An owned copy of `view`'s control information, with an empty
    /// payload. The payload is the caller's to attach: the frame the view
    /// was read from, advanced by [`payload_offset`](view::Npdu::payload_offset),
    /// shares the frame's buffer instead of copying it.
    pub fn header_from(view: &view::Npdu<'_>) -> Self {
        Self {
            expecting_reply: view.expecting_reply,
            priority: view.priority,
            destination: view.destination.map(NpduAddress::from),
            source: view.source.map(NpduAddress::from),
            hop_count: view.hop_count,
            message_type: view.message_type,
            vendor_id: view.vendor_id,
            payload: Bytes::new(),
        }
    }

    /// The borrowed form `bacnet-npdu`'s encoder writes.
    fn view(&self) -> bacnet_npdu::Npdu<'_> {
        bacnet_npdu::Npdu {
            expecting_reply: self.expecting_reply,
            priority: self.priority,
            destination: self.destination.as_ref().map(NpduAddress::view),
            source: self.source.as_ref().map(NpduAddress::view),
            hop_count: self.hop_count,
            message_type: self.message_type,
            vendor_id: self.vendor_id,
            payload: &self.payload,
            payload_offset: 0,
        }
    }
}

impl Default for Npdu {
    fn default() -> Self {
        Self {
            expecting_reply: false,
            priority: NetworkPriority::Normal,
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

/// Encode an NPDU to wire format, appending it to `buf`.
///
/// Nothing is appended when the NPDU is one the decoder would refuse: a
/// DNET of 0, an SNET of 0 or 0xFFFF, an empty SADR, or a DADR or SADR past
/// [`MAX_MAC_LEN`].
pub fn encode_npdu(buf: &mut BytesMut, npdu: &Npdu) -> Result<(), Error> {
    let view = npdu.view();
    let start = buf.len();
    buf.resize(start + view.encoded_len(), 0);
    match view.encode_into(&mut buf[start..]) {
        Ok(_) => Ok(()),
        Err(refused) => {
            buf.truncate(start);
            Err(encode_error(npdu, refused))
        }
    }
}

/// An encoder refusal as a static [`Error::Range`] naming the field it
/// concerns, formatted only when displayed (#1777).
fn encode_error(npdu: &Npdu, refused: EncodeError) -> Error {
    let destination = npdu.destination.as_ref();
    let source = npdu.source.as_ref();
    match refused {
        EncodeError::ReservedNetwork if destination.is_some_and(|d| d.network == 0) => {
            RangeError::new("NPDU DNET must not be 0")
        }
        EncodeError::ReservedNetwork => RangeError::new("NPDU SNET must be 1..65534, got {}")
            .with_value(source.map_or(0, |s| s.network).into()),
        EncodeError::EmptySource => RangeError::new("NPDU SLEN must not be 0"),
        EncodeError::AddressTooLong => {
            let (label, address) = match destination {
                Some(d) if d.mac_address.len() > MAX_MAC_LEN => (
                    "NPDU destination address of {} octets exceeds the {}-octet limit",
                    d,
                ),
                _ => (
                    "NPDU source address of {} octets exceeds the {}-octet limit",
                    source.expect("an over-long address is one of the two"),
                ),
            };
            RangeError::new(label).with_values(address.mac_address.len() as u64, MAX_MAC_LEN as u64)
        }
        // The buffer is sized to the NPDU, so nothing else is refused.
        _ => RangeError::new("NPDU encoding refused"),
    }
    .into()
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Decode an NPDU from raw bytes.
///
/// Returns the decoded [`Npdu`]. The `payload` field contains either the
/// APDU bytes or network message data, sharing `data`'s buffer.
///
/// A DLEN or SLEN past [`MAX_MAC_LEN`] fails with
/// [`NpduDecodeError::AddressTooLong`], raised without reading the address
/// octets, so a caller can tell it apart from every other malformation
/// ([`NpduDecodeError::Malformed`]). For an over-long DADR the error still
/// carries the SNET/SADR behind it, when the frame holds a valid one, so a
/// router can address its reject (#1158).
pub fn decode_npdu(data: Bytes) -> Result<Npdu, NpduDecodeError> {
    // Only hosted builds log; tracing is a std-only dependency.
    #[cfg(feature = "std")]
    if let Some(&control) = data.get(1)
        && control & 0x50 != 0
    {
        tracing::warn!(
            control_byte = control,
            "NPDU control byte has reserved bits set (bits 4 or 6)"
        );
    }
    let view = match bacnet_npdu::Npdu::decode(&data) {
        Ok(view) => view,
        Err(bacnet_npdu::DecodeError::AddressTooLong {
            field,
            length,
            dnet,
            source,
        }) => {
            return Err(NpduDecodeError::AddressTooLong {
                field,
                length,
                dnet,
                source: source.map(NpduAddress::from),
            });
        }
        Err(bacnet_npdu::DecodeError::Malformed(fault)) => {
            return Err(NpduDecodeError::Malformed(fault));
        }
    };
    let mut npdu = Npdu::header_from(&view);
    // Advanced, not sliced: a slice is a clone, and the first clone of a
    // frame received into a plain buffer allocates its shared header (#1713).
    let offset = view.payload_offset;
    let mut payload = data;
    bytes::Buf::advance(&mut payload, offset);
    npdu.payload = payload;
    Ok(npdu)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod address_bound_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_npdu::Malformed;

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
            (NetworkPriority::Normal, 0x00),
            (NetworkPriority::Urgent, 0x01),
            (NetworkPriority::CriticalEquipment, 0x02),
            (NetworkPriority::LifeSafety, 0x03),
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
            message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK),
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
            message_type: Some(NetworkMessageType::from_raw(0x80)), // Proprietary range
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
        assert!(!decoded.is_network_message());
        assert!(!decoded.expecting_reply);
        assert_eq!(decoded.priority, NetworkPriority::Normal);
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
        assert_eq!(
            decode_npdu(Bytes::new()),
            Err(NpduDecodeError::Malformed(Malformed::Truncated))
        );
        assert_eq!(
            decode_npdu(Bytes::from_static(&[0x01])),
            Err(NpduDecodeError::Malformed(Malformed::Truncated))
        );
    }

    #[test]
    fn decode_wrong_version() {
        assert_eq!(
            decode_npdu(Bytes::from_static(&[0x02, 0x00])),
            Err(NpduDecodeError::Malformed(Malformed::Version))
        );
    }

    #[test]
    fn decode_truncated_destination() {
        // Has destination flag but not enough bytes
        assert_eq!(
            decode_npdu(Bytes::from_static(&[0x01, 0x20, 0xFF])),
            Err(NpduDecodeError::Malformed(Malformed::Truncated))
        );
    }

    #[test]
    fn decode_truncated_source() {
        // Has source flag but not enough bytes after destination
        assert_eq!(
            decode_npdu(Bytes::from_static(&[0x01, 0x08, 0x00])),
            Err(NpduDecodeError::Malformed(Malformed::Truncated))
        );
    }

    // --- NPDU edge case tests ---

    #[test]
    fn npdu_network_zero() {
        // DNET=0 is invalid per Clause 6.2.2 — rejected at encode time
        let npdu = Npdu {
            destination: Some(NpduAddress {
                network: 0,
                mac_address: MacAddr::from_slice(&[0x01]),
            }),
            hop_count: 255,
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Default::default()
        };
        let mut buf = BytesMut::new();
        let result = encode_npdu(&mut buf, &npdu);
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("DNET"), "got: {err}");
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
        // SLEN=0 is invalid for source per Clause 6.2.2 — rejected at encode time
        let npdu = Npdu {
            source: Some(NpduAddress {
                network: 500,
                mac_address: MacAddr::new(),
            }),
            payload: Bytes::from_static(&[0xBB]),
            ..Default::default()
        };
        let mut buf = BytesMut::new();
        let result = encode_npdu(&mut buf, &npdu);
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("SLEN"), "got: {err}");
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
        assert_eq!(
            decode_npdu(Bytes::copy_from_slice(&data)),
            Err(NpduDecodeError::Malformed(Malformed::Truncated))
        );
    }

    #[test]
    fn npdu_source_truncated_mac() {
        // Source present but MAC bytes truncated
        let data = [0x01, 0x08, 0x01, 0xF4, 0x04, 0x01]; // SNET=500, SLEN=4, only 1 byte
        assert_eq!(
            decode_npdu(Bytes::copy_from_slice(&data)),
            Err(NpduDecodeError::Malformed(Malformed::Truncated))
        );
    }

    #[test]
    fn npdu_missing_hop_count() {
        // Destination present but data ends before hop count
        // Version=1, Control=0x20, DNET=0xFFFF, DLEN=0
        let data = [0x01, 0x20, 0xFF, 0xFF, 0x00];
        assert_eq!(
            decode_npdu(Bytes::copy_from_slice(&data)),
            Err(NpduDecodeError::Malformed(Malformed::Truncated))
        );
    }

    #[test]
    fn npdu_network_message_truncated_type() {
        // Network message flag set but no message type byte
        let data = [0x01, 0x80]; // is_network_message = true, but no type byte
        assert_eq!(
            decode_npdu(Bytes::copy_from_slice(&data)),
            Err(NpduDecodeError::Malformed(Malformed::Truncated))
        );
    }

    #[test]
    fn npdu_proprietary_message_truncated_vendor() {
        // Proprietary message type (>=0x80) but vendor ID missing
        let data = [0x01, 0x80, 0x80]; // msg_type=0x80, need 2 more bytes for vendor
        assert_eq!(
            decode_npdu(Bytes::copy_from_slice(&data)),
            Err(NpduDecodeError::Malformed(Malformed::Truncated))
        );
    }

    #[test]
    fn npdu_all_flags_round_trip() {
        // Maximum complexity: all flags set
        let npdu = Npdu {
            expecting_reply: true,
            priority: NetworkPriority::LifeSafety,
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
        // Source network 0 is invalid per Clause 6.2.2 — rejected at encode time
        let npdu = Npdu {
            source: Some(NpduAddress {
                network: 0,
                mac_address: MacAddr::from_slice(&[0x01]),
            }),
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Default::default()
        };
        let mut buf = BytesMut::new();
        let result = encode_npdu(&mut buf, &npdu);
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("SNET"), "got: {err}");
    }

    #[test]
    fn reserved_bits_warning_still_decodes() {
        // Reserved bits set in control byte should NOT cause decode failure
        // (warning only). Construct wire bytes manually with reserved bit 6 set.
        let mut data = vec![0x01, 0x40]; // version=1, control with reserved bit 6
        data.extend_from_slice(&[0x10, 0x08]);

        // Should decode successfully (warning only, not error)
        let result = decode_npdu(Bytes::copy_from_slice(&data));
        assert!(
            result.is_ok(),
            "reserved bits should not cause decode failure"
        );
    }
}
