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
pub fn encode_bvll(buf: &mut BytesMut, function: BvlcFunction, payload: &[u8]) {
    let total_length = BVLL_HEADER_LENGTH + payload.len();
    buf.reserve(total_length);
    buf.put_u8(BVLC_TYPE_BACNET_IP);
    buf.put_u8(function.to_raw());
    buf.put_u16(total_length as u16);
    buf.put_slice(payload);
}

/// Encode a Forwarded-NPDU BVLL frame with originating address.
pub fn encode_bvll_forwarded(buf: &mut BytesMut, ip: [u8; 4], port: u16, npdu: &[u8]) {
    let total_length = BVLL_HEADER_LENGTH + FORWARDED_ADDR_LENGTH + npdu.len();
    buf.reserve(total_length);
    buf.put_u8(BVLC_TYPE_BACNET_IP);
    buf.put_u8(BvlcFunction::FORWARDED_NPDU.to_raw());
    buf.put_u16(total_length as u16);
    buf.put_slice(&ip);
    buf.put_u16(port);
    buf.put_slice(npdu);
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

    #[test]
    fn encode_decode_unicast() {
        let npdu = vec![0x01, 0x00, 0x10, 0x02, 0x03];
        let mut buf = BytesMut::new();
        encode_bvll(&mut buf, BvlcFunction::ORIGINAL_UNICAST_NPDU, &npdu);

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
        encode_bvll(&mut buf, BvlcFunction::ORIGINAL_BROADCAST_NPDU, &npdu);

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
        encode_bvll_forwarded(&mut buf, ip, port, &npdu);

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
        encode_bvll(&mut buf, BvlcFunction::BVLC_RESULT, &result_code);

        let msg = decode_bvll(&buf).unwrap();
        assert_eq!(msg.function, BvlcFunction::BVLC_RESULT);
        assert_eq!(msg.payload, result_code);
    }

    #[test]
    fn encode_decode_register_foreign_device() {
        // 2-byte TTL in seconds
        let ttl = 60u16.to_be_bytes().to_vec();
        let mut buf = BytesMut::new();
        encode_bvll(&mut buf, BvlcFunction::REGISTER_FOREIGN_DEVICE, &ttl);

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
        );

        let msg = decode_bvll(&buf).unwrap();
        assert_eq!(
            msg.function,
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE
        );
        assert!(msg.payload.is_empty());
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
        encode_bvll(&mut buf, BvlcFunction::ORIGINAL_BROADCAST_NPDU, &npdu);

        assert_eq!(&buf[..4], &[0x81, 0x0B, 0x00, 0x0A]);
        assert_eq!(&buf[4..], &npdu);

        let msg = decode_bvll(&buf).unwrap();
        assert_eq!(msg.function, BvlcFunction::ORIGINAL_BROADCAST_NPDU);
        assert_eq!(msg.payload, npdu);
    }
}
