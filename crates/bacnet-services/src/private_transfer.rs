//! ConfirmedPrivateTransfer / UnconfirmedPrivateTransfer services
//! per ASHRAE 135-2020 Clauses 15.19 and 16.10.6.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::error::Error;
use bytes::{BufMut, BytesMut};

// ---------------------------------------------------------------------------
// PrivateTransferRequest
// ---------------------------------------------------------------------------

/// Request parameters shared by ConfirmedPrivateTransfer and
/// UnconfirmedPrivateTransfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateTransferRequest {
    pub vendor_id: u32,
    pub service_number: u32,
    /// Vendor-defined payload (raw bytes, opaque to the stack).
    pub service_parameters: Option<Vec<u8>>,
}

impl PrivateTransferRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        // [0] vendorID
        primitives::encode_ctx_unsigned(buf, 0, self.vendor_id as u64);
        // [1] serviceNumber
        primitives::encode_ctx_unsigned(buf, 1, self.service_number as u64);
        // [2] serviceParameters (optional, opening/closing)
        if let Some(ref params) = self.service_parameters {
            tags::encode_opening_tag(buf, 2);
            buf.put_slice(params);
            tags::encode_closing_tag(buf, 2);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] vendorID
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(0) {
            return Err(Error::decoding(
                offset,
                "PrivateTransfer expected context tag 0 for vendorID",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "PrivateTransfer truncated at vendorID",
            ));
        }
        let vendor_id_raw = primitives::decode_unsigned(&data[pos..end])?;
        let vendor_id = u32::try_from(vendor_id_raw).map_err(|_| {
            Error::decoding(
                pos,
                format!("PrivateTransfer vendorID {vendor_id_raw} exceeds u32"),
            )
        })?;
        offset = end;

        // [1] serviceNumber
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(1) {
            return Err(Error::decoding(
                offset,
                "PrivateTransfer expected context tag 1 for serviceNumber",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "PrivateTransfer truncated at serviceNumber",
            ));
        }
        let service_number_raw = primitives::decode_unsigned(&data[pos..end])?;
        let service_number = u32::try_from(service_number_raw).map_err(|_| {
            Error::decoding(
                pos,
                format!("PrivateTransfer serviceNumber {service_number_raw} exceeds u32"),
            )
        })?;
        offset = end;

        // [2] serviceParameters (optional, opening/closing)
        let mut service_parameters = None;
        if offset < data.len() {
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_opening_tag(2) {
                let (value_bytes, new_offset) = tags::extract_context_value(data, tag_end, 2)?;
                service_parameters = Some(value_bytes.to_vec());
                offset = new_offset;
                let _ = offset;
            }
        }

        Ok(Self {
            vendor_id,
            service_number,
            service_parameters,
        })
    }
}

// ---------------------------------------------------------------------------
// PrivateTransferAck
// ---------------------------------------------------------------------------

/// ConfirmedPrivateTransfer-ACK service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateTransferAck {
    pub vendor_id: u32,
    pub service_number: u32,
    /// Vendor-defined result (raw bytes, opaque to the stack).
    pub result_block: Option<Vec<u8>>,
}

impl PrivateTransferAck {
    pub fn encode(&self, buf: &mut BytesMut) {
        // [0] vendorID
        primitives::encode_ctx_unsigned(buf, 0, self.vendor_id as u64);
        // [1] serviceNumber
        primitives::encode_ctx_unsigned(buf, 1, self.service_number as u64);
        // [2] resultBlock (optional, opening/closing)
        if let Some(ref block) = self.result_block {
            tags::encode_opening_tag(buf, 2);
            buf.put_slice(block);
            tags::encode_closing_tag(buf, 2);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] vendorID
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(0) {
            return Err(Error::decoding(
                offset,
                "PrivateTransferAck expected context tag 0 for vendorID",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "PrivateTransferAck truncated at vendorID",
            ));
        }
        let vendor_id_raw = primitives::decode_unsigned(&data[pos..end])?;
        let vendor_id = u32::try_from(vendor_id_raw).map_err(|_| {
            Error::decoding(
                pos,
                format!("PrivateTransferAck vendorID {vendor_id_raw} exceeds u32"),
            )
        })?;
        offset = end;

        // [1] serviceNumber
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(1) {
            return Err(Error::decoding(
                offset,
                "PrivateTransferAck expected context tag 1 for serviceNumber",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "PrivateTransferAck truncated at serviceNumber",
            ));
        }
        let service_number_raw = primitives::decode_unsigned(&data[pos..end])?;
        let service_number = u32::try_from(service_number_raw).map_err(|_| {
            Error::decoding(
                pos,
                format!("PrivateTransferAck serviceNumber {service_number_raw} exceeds u32"),
            )
        })?;
        offset = end;

        // [2] resultBlock (optional, opening/closing)
        let mut result_block = None;
        if offset < data.len() {
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_opening_tag(2) {
                let (value_bytes, new_offset) = tags::extract_context_value(data, tag_end, 2)?;
                result_block = Some(value_bytes.to_vec());
                offset = new_offset;
                let _ = offset;
            }
        }

        Ok(Self {
            vendor_id,
            service_number,
            result_block,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_header(vendor_id: u64, service_number: u64) -> BytesMut {
        let mut buf = BytesMut::new();
        primitives::encode_ctx_unsigned(&mut buf, 0, vendor_id);
        primitives::encode_ctx_unsigned(&mut buf, 1, service_number);
        buf
    }

    fn assert_both_decoders_reject(data: &[u8]) {
        assert!(PrivateTransferRequest::decode(data).is_err());
        assert!(PrivateTransferAck::decode(data).is_err());
    }

    #[test]
    fn request_round_trip() {
        let req = PrivateTransferRequest {
            vendor_id: 42,
            service_number: 7,
            service_parameters: Some(vec![0x21, 0x05]),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = PrivateTransferRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn request_no_params_round_trip() {
        let req = PrivateTransferRequest {
            vendor_id: 999,
            service_number: 1,
            service_parameters: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = PrivateTransferRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn ack_round_trip() {
        let ack = PrivateTransferAck {
            vendor_id: 42,
            service_number: 7,
            result_block: Some(vec![0x44, 0x42, 0x90, 0x00, 0x00]),
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = PrivateTransferAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    #[test]
    fn ack_no_result_round_trip() {
        let ack = PrivateTransferAck {
            vendor_id: 100,
            service_number: 3,
            result_block: None,
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = PrivateTransferAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    #[test]
    fn private_transfer_values_must_fit_u32() {
        let maximum = encode_header(u64::from(u32::MAX), u64::from(u32::MAX));
        let request = PrivateTransferRequest::decode(&maximum).unwrap();
        assert_eq!(request.vendor_id, u32::MAX);
        assert_eq!(request.service_number, u32::MAX);
        let ack = PrivateTransferAck::decode(&maximum).unwrap();
        assert_eq!(ack.vendor_id, u32::MAX);
        assert_eq!(ack.service_number, u32::MAX);

        let mut leading_zero = BytesMut::new();
        tags::encode_tag(&mut leading_zero, 0, tags::TagClass::Context, 5);
        leading_zero.extend_from_slice(&[0, 0xff, 0xff, 0xff, 0xff]);
        tags::encode_tag(&mut leading_zero, 1, tags::TagClass::Context, 5);
        leading_zero.extend_from_slice(&[0, 0xff, 0xff, 0xff, 0xff]);
        let request = PrivateTransferRequest::decode(&leading_zero).unwrap();
        assert_eq!(request.vendor_id, u32::MAX);
        assert_eq!(request.service_number, u32::MAX);
        let ack = PrivateTransferAck::decode(&leading_zero).unwrap();
        assert_eq!(ack.vendor_id, u32::MAX);
        assert_eq!(ack.service_number, u32::MAX);

        for value in [u64::from(u32::MAX) + 1, u64::MAX] {
            assert_both_decoders_reject(&encode_header(value, 1));
            assert_both_decoders_reject(&encode_header(1, value));
        }
    }

    #[test]
    fn private_transfer_requires_mandatory_context_tags() {
        let mut wrong_vendor_tag = BytesMut::new();
        primitives::encode_ctx_unsigned(&mut wrong_vendor_tag, 1, 42);
        primitives::encode_ctx_unsigned(&mut wrong_vendor_tag, 1, 7);
        assert_both_decoders_reject(&wrong_vendor_tag);

        let mut wrong_service_tag = BytesMut::new();
        primitives::encode_ctx_unsigned(&mut wrong_service_tag, 0, 42);
        primitives::encode_ctx_unsigned(&mut wrong_service_tag, 0, 7);
        assert_both_decoders_reject(&wrong_service_tag);

        let mut application_vendor = BytesMut::new();
        primitives::encode_app_unsigned(&mut application_vendor, 42);
        primitives::encode_ctx_unsigned(&mut application_vendor, 1, 7);
        assert_both_decoders_reject(&application_vendor);
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_request_empty_input() {
        assert!(PrivateTransferRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_request_truncated_1_byte() {
        let req = PrivateTransferRequest {
            vendor_id: 42,
            service_number: 7,
            service_parameters: Some(vec![0x21, 0x05]),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(PrivateTransferRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_request_truncated_half() {
        let req = PrivateTransferRequest {
            vendor_id: 42,
            service_number: 7,
            service_parameters: Some(vec![0x21, 0x05, 0x44, 0x42, 0x90, 0x00, 0x00]),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(PrivateTransferRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_request_invalid_tag() {
        assert!(PrivateTransferRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_ack_empty_input() {
        assert!(PrivateTransferAck::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_ack_truncated_1_byte() {
        let ack = PrivateTransferAck {
            vendor_id: 42,
            service_number: 7,
            result_block: Some(vec![0x44, 0x42]),
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        assert!(PrivateTransferAck::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_ack_invalid_tag() {
        assert!(PrivateTransferAck::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}
