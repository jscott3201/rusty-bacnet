//! BVLL (BACnet Virtual Link Layer) encode/decode for BACnet/IP (Annex J).
//!
//! Wire format (standard):
//! ```text
//! [0x81] [function] [length_hi] [length_lo] [payload...]
//! ```
//!
//! Wire format (Forwarded-NPDU, function 0x04):
//! ```text
//! [0x81] [0x04] [length_hi] [length_lo] [ip0..ip3] [port_hi] [port_lo] [npdu...]
//! ```

use bacnet_types::enums::BvlcFunction;
use bacnet_types::error::Error;
use bytes::{BufMut, Bytes, BytesMut};

/// BVLC type byte for BACnet/IP (Annex J).
pub const BVLC_TYPE_BACNET_IP: u8 = 0x81;

/// Fixed BVLL header length: type(1) + function(1) + length(2).
pub const BVLL_HEADER_LENGTH: usize = 4;

/// Originating address length in Forwarded-NPDU: IPv4(4) + port(2).
pub const FORWARDED_ADDR_LENGTH: usize = 6;

/// A decoded BVLL message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BvllMessage {
    /// BVLC function code.
    pub function: BvlcFunction,
    /// Payload after the BVLL header (and originating address for forwarded).
    /// For NPDU-carrying functions, this is the raw NPDU bytes.
    /// For management functions, this is the function-specific data.
    pub payload: Bytes,
    /// Originating IPv4 address — only present for FORWARDED_NPDU.
    pub originating_ip: Option<[u8; 4]>,
    /// Originating port — only present for FORWARDED_NPDU.
    pub originating_port: Option<u16>,
}

/// Encode a standard BVLL frame (all functions except Forwarded-NPDU).
///
/// Returns an error if the total frame cannot be represented by the BVLC
/// 16-bit length field.
pub fn encode_bvll(
    buf: &mut BytesMut,
    function: BvlcFunction,
    payload: &[u8],
) -> Result<(), Error> {
    let total_length = BVLL_HEADER_LENGTH + payload.len();
    if total_length > u16::MAX as usize {
        return Err(Error::Encoding(format!(
            "BVLL frame length {total_length} exceeds 16-bit BVLC length field"
        )));
    }
    buf.reserve(total_length);
    buf.put_u8(BVLC_TYPE_BACNET_IP);
    buf.put_u8(function.to_raw());
    buf.put_u16(total_length as u16);
    buf.put_slice(payload);
    Ok(())
}

/// Encode a Forwarded-NPDU BVLL frame with originating address.
pub fn encode_bvll_forwarded(
    buf: &mut BytesMut,
    ip: [u8; 4],
    port: u16,
    npdu: &[u8],
) -> Result<(), Error> {
    let total_length = BVLL_HEADER_LENGTH + FORWARDED_ADDR_LENGTH + npdu.len();
    if total_length > u16::MAX as usize {
        return Err(Error::Encoding(format!(
            "BVLL Forwarded-NPDU length {total_length} exceeds 16-bit BVLC length field"
        )));
    }
    buf.reserve(total_length);
    buf.put_u8(BVLC_TYPE_BACNET_IP);
    buf.put_u8(BvlcFunction::FORWARDED_NPDU.to_raw());
    buf.put_u16(total_length as u16);
    buf.put_slice(&ip);
    buf.put_u16(port);
    buf.put_slice(npdu);
    Ok(())
}

/// Decode a BVLL frame from raw bytes.
pub fn decode_bvll(data: &[u8]) -> Result<BvllMessage, Error> {
    if data.len() < BVLL_HEADER_LENGTH {
        return Err(Error::decoding(0, "BVLL frame too short"));
    }

    if data[0] != BVLC_TYPE_BACNET_IP {
        return Err(Error::decoding(
            0,
            format!("BVLL expected type 0x81, got 0x{:02X}", data[0]),
        ));
    }

    let function = BvlcFunction::from_raw(data[1]);
    let length = u16::from_be_bytes([data[2], data[3]]) as usize;

    if length < BVLL_HEADER_LENGTH {
        return Err(Error::decoding(2, "BVLL length less than header size"));
    }
    if length > data.len() {
        return Err(Error::decoding(
            2,
            format!("BVLL length {} exceeds data length {}", length, data.len()),
        ));
    }

    if function == BvlcFunction::FORWARDED_NPDU {
        if length < BVLL_HEADER_LENGTH + FORWARDED_ADDR_LENGTH {
            return Err(Error::decoding(
                2,
                "BVLL Forwarded-NPDU too short for originating address",
            ));
        }
        let ip = [data[4], data[5], data[6], data[7]];
        let port = u16::from_be_bytes([data[8], data[9]]);
        let payload =
            Bytes::copy_from_slice(&data[BVLL_HEADER_LENGTH + FORWARDED_ADDR_LENGTH..length]);

        Ok(BvllMessage {
            function,
            payload,
            originating_ip: Some(ip),
            originating_port: Some(port),
        })
    } else {
        let payload = Bytes::copy_from_slice(&data[BVLL_HEADER_LENGTH..length]);

        Ok(BvllMessage {
            function,
            payload,
            originating_ip: None,
            originating_port: None,
        })
    }
}

/// Encode a 6-byte BACnet/IP MAC address from IPv4 + port.
pub fn encode_bip_mac(ip: [u8; 4], port: u16) -> [u8; 6] {
    let port_bytes = port.to_be_bytes();
    [ip[0], ip[1], ip[2], ip[3], port_bytes[0], port_bytes[1]]
}

/// Decode a 6-byte BACnet/IP MAC address into IPv4 + port.
pub fn decode_bip_mac(mac: &[u8]) -> Result<([u8; 4], u16), Error> {
    if mac.len() != 6 {
        return Err(Error::decoding(
            0,
            format!("BIP MAC must be 6 bytes, got {}", mac.len()),
        ));
    }
    let ip = [mac[0], mac[1], mac[2], mac[3]];
    let port = u16::from_be_bytes([mac[4], mac[5]]);
    Ok((ip, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BvlcCodecCase {
        name: &'static str,
        function: BvlcFunction,
        payload: &'static [u8],
        expected_payload: &'static [u8],
        expected_origin: Option<([u8; 4], u16)>,
    }

    const SAMPLE_NPDU: &[u8] = &[0x01, 0x20];
    const SAMPLE_BDT_ENTRY: &[u8] = &[192, 0, 2, 10, 0xBA, 0xC0, 255, 255, 255, 0];
    const SAMPLE_FDT_ENTRY: &[u8] = &[192, 0, 2, 20, 0xBA, 0xC0, 0x00, 0x3C, 0x00, 0x5A];
    const SAMPLE_BIP_ADDRESS: &[u8] = &[192, 0, 2, 30, 0xBA, 0xC0];

    fn encode_standard_frame(function: BvlcFunction, payload: &[u8]) -> BytesMut {
        let mut buf = BytesMut::new();
        encode_bvll(&mut buf, function, payload).expect("valid BVLL encoding");
        buf
    }

    #[test]
    fn annex_j_bvlc_function_constants_are_stable() {
        let expected_annex_j_functions = [
            ("BVLC_RESULT", BvlcFunction::BVLC_RESULT, 0x00),
            (
                "WRITE_BROADCAST_DISTRIBUTION_TABLE",
                BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE,
                0x01,
            ),
            (
                "READ_BROADCAST_DISTRIBUTION_TABLE",
                BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
                0x02,
            ),
            (
                "READ_BROADCAST_DISTRIBUTION_TABLE_ACK",
                BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
                0x03,
            ),
            ("FORWARDED_NPDU", BvlcFunction::FORWARDED_NPDU, 0x04),
            (
                "REGISTER_FOREIGN_DEVICE",
                BvlcFunction::REGISTER_FOREIGN_DEVICE,
                0x05,
            ),
            (
                "READ_FOREIGN_DEVICE_TABLE",
                BvlcFunction::READ_FOREIGN_DEVICE_TABLE,
                0x06,
            ),
            (
                "READ_FOREIGN_DEVICE_TABLE_ACK",
                BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
                0x07,
            ),
            (
                "DELETE_FOREIGN_DEVICE_TABLE_ENTRY",
                BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
                0x08,
            ),
            (
                "DISTRIBUTE_BROADCAST_TO_NETWORK",
                BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
                0x09,
            ),
            (
                "ORIGINAL_UNICAST_NPDU",
                BvlcFunction::ORIGINAL_UNICAST_NPDU,
                0x0A,
            ),
            (
                "ORIGINAL_BROADCAST_NPDU",
                BvlcFunction::ORIGINAL_BROADCAST_NPDU,
                0x0B,
            ),
        ];

        assert!(BvlcFunction::ALL_NAMED.len() >= expected_annex_j_functions.len());
        for ((actual_name, actual_function), (name, function, raw)) in BvlcFunction::ALL_NAMED
            .iter()
            .take(expected_annex_j_functions.len())
            .zip(expected_annex_j_functions)
        {
            assert_eq!(*actual_name, name);
            assert_eq!(*actual_function, function);
            assert_eq!(actual_function.to_raw(), raw);
            assert_eq!(BvlcFunction::from_raw(raw), function);
        }
    }

    #[test]
    fn deleted_secure_bvll_function_is_exposed_as_passthrough() {
        assert_eq!(BvlcFunction::SECURE_BVLL.to_raw(), 0x0C);
        assert_eq!(BvlcFunction::from_raw(0x0C), BvlcFunction::SECURE_BVLL);
    }

    #[test]
    fn decode_annex_j_bvlc_functions_and_deleted_secure_passthrough() {
        let cases = [
            BvlcCodecCase {
                name: "BVLC-Result",
                function: BvlcFunction::BVLC_RESULT,
                payload: &[0x00, 0x00],
                expected_payload: &[0x00, 0x00],
                expected_origin: None,
            },
            BvlcCodecCase {
                name: "Write-BDT",
                function: BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE,
                payload: SAMPLE_BDT_ENTRY,
                expected_payload: SAMPLE_BDT_ENTRY,
                expected_origin: None,
            },
            BvlcCodecCase {
                name: "Read-BDT",
                function: BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
                payload: &[],
                expected_payload: &[],
                expected_origin: None,
            },
            BvlcCodecCase {
                name: "Read-BDT-Ack",
                function: BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
                payload: &[],
                expected_payload: &[],
                expected_origin: None,
            },
            BvlcCodecCase {
                name: "Forwarded-NPDU",
                function: BvlcFunction::FORWARDED_NPDU,
                payload: SAMPLE_NPDU,
                expected_payload: SAMPLE_NPDU,
                expected_origin: Some(([192, 0, 2, 40], 0xBAC0)),
            },
            BvlcCodecCase {
                name: "Register-Foreign-Device",
                function: BvlcFunction::REGISTER_FOREIGN_DEVICE,
                payload: &[0x00, 0x3C],
                expected_payload: &[0x00, 0x3C],
                expected_origin: None,
            },
            BvlcCodecCase {
                name: "Read-FDT",
                function: BvlcFunction::READ_FOREIGN_DEVICE_TABLE,
                payload: &[],
                expected_payload: &[],
                expected_origin: None,
            },
            BvlcCodecCase {
                name: "Read-FDT-Ack",
                function: BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
                payload: SAMPLE_FDT_ENTRY,
                expected_payload: SAMPLE_FDT_ENTRY,
                expected_origin: None,
            },
            BvlcCodecCase {
                name: "Delete-FDT-Entry",
                function: BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
                payload: SAMPLE_BIP_ADDRESS,
                expected_payload: SAMPLE_BIP_ADDRESS,
                expected_origin: None,
            },
            BvlcCodecCase {
                name: "Distribute-Broadcast-To-Network",
                function: BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
                payload: SAMPLE_NPDU,
                expected_payload: SAMPLE_NPDU,
                expected_origin: None,
            },
            BvlcCodecCase {
                name: "Original-Unicast-NPDU",
                function: BvlcFunction::ORIGINAL_UNICAST_NPDU,
                payload: SAMPLE_NPDU,
                expected_payload: SAMPLE_NPDU,
                expected_origin: None,
            },
            BvlcCodecCase {
                name: "Original-Broadcast-NPDU",
                function: BvlcFunction::ORIGINAL_BROADCAST_NPDU,
                payload: SAMPLE_NPDU,
                expected_payload: SAMPLE_NPDU,
                expected_origin: None,
            },
            BvlcCodecCase {
                name: "Secure-BVLL passthrough",
                function: BvlcFunction::SECURE_BVLL,
                payload: &[],
                expected_payload: &[],
                expected_origin: None,
            },
        ];

        for case in cases {
            let frame = if let Some((ip, port)) = case.expected_origin {
                let mut buf = BytesMut::new();
                encode_bvll_forwarded(&mut buf, ip, port, case.payload)
                    .expect("valid Forwarded-NPDU encoding");
                buf
            } else {
                encode_standard_frame(case.function, case.payload)
            };
            let declared_length = u16::from_be_bytes([frame[2], frame[3]]) as usize;
            assert_eq!(declared_length, frame.len(), "{}", case.name);

            let msg = decode_bvll(&frame).unwrap_or_else(|err| panic!("{}: {err}", case.name));
            assert_eq!(msg.function, case.function, "{}", case.name);
            assert_eq!(msg.payload, case.expected_payload, "{}", case.name);
            assert_eq!(
                msg.originating_ip,
                case.expected_origin.map(|(ip, _)| ip),
                "{}",
                case.name
            );
            assert_eq!(
                msg.originating_port,
                case.expected_origin.map(|(_, port)| port),
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn decode_preserves_unknown_bvlc_function_code() {
        let msg = decode_bvll(&[0x81, 0x7F, 0x00, 0x06, 0xAA, 0xBB]).unwrap();
        assert_eq!(msg.function, BvlcFunction::from_raw(0x7F));
        assert_eq!(msg.payload, Bytes::from_static(&[0xAA, 0xBB]));
        assert!(msg.originating_ip.is_none());
        assert!(msg.originating_port.is_none());
    }

    #[test]
    fn decode_slices_payload_to_declared_length() {
        // Current decoder policy: ignore datagram bytes after the declared BVLC length.
        let msg =
            decode_bvll(&[0x81, 0x0A, 0x00, 0x06, 0x01, 0x00, 0xDE, 0xAD, 0xBE, 0xEF]).unwrap();

        assert_eq!(msg.function, BvlcFunction::ORIGINAL_UNICAST_NPDU);
        assert_eq!(msg.payload, Bytes::from_static(&[0x01, 0x00]));
    }

    #[test]
    fn encode_decode_unicast() {
        let npdu = vec![0x01, 0x00, 0x10, 0x02, 0x03];
        let mut buf = BytesMut::new();
        encode_bvll(&mut buf, BvlcFunction::ORIGINAL_UNICAST_NPDU, &npdu)
            .expect("valid BVLL encoding");

        assert_eq!(buf[0], 0x81);
        assert_eq!(buf[1], 0x0A);
        let length = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        assert_eq!(length, 4 + npdu.len());

        let msg = decode_bvll(&buf).unwrap();
        assert_eq!(msg.function, BvlcFunction::ORIGINAL_UNICAST_NPDU);
        assert_eq!(msg.payload, npdu);
        assert!(msg.originating_ip.is_none());
        assert!(msg.originating_port.is_none());
    }

    #[test]
    fn encode_decode_broadcast() {
        let npdu = vec![0x01, 0x00];
        let mut buf = BytesMut::new();
        encode_bvll(&mut buf, BvlcFunction::ORIGINAL_BROADCAST_NPDU, &npdu)
            .expect("valid BVLL encoding");

        let msg = decode_bvll(&buf).unwrap();
        assert_eq!(msg.function, BvlcFunction::ORIGINAL_BROADCAST_NPDU);
        assert_eq!(msg.payload, npdu);
    }

    #[test]
    fn encode_decode_forwarded_npdu() {
        let npdu = vec![0x01, 0x00, 0x55];
        let ip = [192, 168, 1, 100];
        let port = 0xBAC0;

        let mut buf = BytesMut::new();
        encode_bvll_forwarded(&mut buf, ip, port, &npdu).expect("valid BVLL forwarded encoding");

        assert_eq!(buf[0], 0x81);
        assert_eq!(buf[1], 0x04);
        let length = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        assert_eq!(length, 4 + 6 + npdu.len());
        assert_eq!(&buf[4..8], &ip);
        assert_eq!(u16::from_be_bytes([buf[8], buf[9]]), port);

        let msg = decode_bvll(&buf).unwrap();
        assert_eq!(msg.function, BvlcFunction::FORWARDED_NPDU);
        assert_eq!(msg.payload, npdu);
        assert_eq!(msg.originating_ip, Some(ip));
        assert_eq!(msg.originating_port, Some(port));
    }

    #[test]
    fn encode_decode_bvlc_result() {
        // BVLC-Result with 2-byte result code (successful completion)
        let result_code = 0x0000u16.to_be_bytes().to_vec();
        let mut buf = BytesMut::new();
        encode_bvll(&mut buf, BvlcFunction::BVLC_RESULT, &result_code)
            .expect("valid BVLL encoding");

        let msg = decode_bvll(&buf).unwrap();
        assert_eq!(msg.function, BvlcFunction::BVLC_RESULT);
        assert_eq!(msg.payload, result_code);
    }

    #[test]
    fn encode_decode_register_foreign_device() {
        // 2-byte TTL in seconds
        let ttl = 60u16.to_be_bytes().to_vec();
        let mut buf = BytesMut::new();
        encode_bvll(&mut buf, BvlcFunction::REGISTER_FOREIGN_DEVICE, &ttl)
            .expect("valid BVLL encoding");

        let msg = decode_bvll(&buf).unwrap();
        assert_eq!(msg.function, BvlcFunction::REGISTER_FOREIGN_DEVICE);
        assert_eq!(msg.payload, ttl);
    }

    #[test]
    fn encode_decode_empty_payload() {
        // Read-BDT has no payload
        let mut buf = BytesMut::new();
        encode_bvll(
            &mut buf,
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
            &[],
        )
        .expect("valid BVLL encoding");

        let msg = decode_bvll(&buf).unwrap();
        assert_eq!(
            msg.function,
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE
        );
        assert!(msg.payload.is_empty());
    }

    #[test]
    fn encode_bvll_oversized_payload_errors() {
        let payload = vec![0; u16::MAX as usize - BVLL_HEADER_LENGTH + 1];
        let mut buf = BytesMut::new();
        assert!(encode_bvll(&mut buf, BvlcFunction::ORIGINAL_UNICAST_NPDU, &payload).is_err());
    }

    #[test]
    fn encode_bvll_forwarded_oversized_payload_errors() {
        let payload = vec![0; u16::MAX as usize - BVLL_HEADER_LENGTH - FORWARDED_ADDR_LENGTH + 1];
        let mut buf = BytesMut::new();
        assert!(encode_bvll_forwarded(&mut buf, [127, 0, 0, 1], 0xBAC0, &payload).is_err());
    }

    #[test]
    fn decode_too_short() {
        assert!(decode_bvll(&[0x81, 0x0A]).is_err());
        assert!(decode_bvll(&[]).is_err());
    }

    #[test]
    fn decode_wrong_type() {
        assert!(decode_bvll(&[0x82, 0x0A, 0x00, 0x04]).is_err());
    }

    #[test]
    fn decode_length_exceeds_data() {
        // Claim length is 100, but only 4 bytes of data
        assert!(decode_bvll(&[0x81, 0x0A, 0x00, 0x64]).is_err());
    }

    #[test]
    fn decode_length_shorter_than_header_errors() {
        assert!(decode_bvll(&[0x81, 0x0A, 0x00, 0x03]).is_err());
    }

    #[test]
    fn decode_forwarded_too_short_for_address() {
        // Forwarded-NPDU with length 4 (no room for originating address)
        assert!(decode_bvll(&[0x81, 0x04, 0x00, 0x04]).is_err());
    }

    #[test]
    fn bip_mac_round_trip() {
        let ip = [10, 0, 1, 42];
        let port = 0xBAC0;
        let mac = encode_bip_mac(ip, port);
        let (decoded_ip, decoded_port) = decode_bip_mac(&mac).unwrap();
        assert_eq!(decoded_ip, ip);
        assert_eq!(decoded_port, port);
    }

    #[test]
    fn bip_mac_invalid_length() {
        assert!(decode_bip_mac(&[1, 2, 3]).is_err());
    }

    #[test]
    fn wire_format_original_broadcast_who_is() {
        // A real BACnet/IP Original-Broadcast-NPDU carrying a WhoIs
        // BVLL: 81 0B 00 08
        // NPDU: 01 20 FF FF 00 FF (version=1, dest=FFFF:broadcast, hop=255)
        let npdu = vec![0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF];
        let mut buf = BytesMut::new();
        encode_bvll(&mut buf, BvlcFunction::ORIGINAL_BROADCAST_NPDU, &npdu)
            .expect("valid BVLL encoding");

        assert_eq!(&buf[..4], &[0x81, 0x0B, 0x00, 0x0A]);
        assert_eq!(&buf[4..], &npdu);

        let msg = decode_bvll(&buf).unwrap();
        assert_eq!(msg.function, BvlcFunction::ORIGINAL_BROADCAST_NPDU);
        assert_eq!(msg.payload, npdu);
    }
}
