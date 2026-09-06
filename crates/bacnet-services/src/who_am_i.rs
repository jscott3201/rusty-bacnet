//! Who-Am-I and You-Are services per ASHRAE 135-2020 Clause 16.10.9 / 16.10.10.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

// ---------------------------------------------------------------------------
// WhoAmIRequest
// ---------------------------------------------------------------------------

/// Who-Am-I-Request (empty APDU, no parameters).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoAmIRequest;

impl WhoAmIRequest {
    pub fn encode(&self, _buf: &mut BytesMut) {}

    pub fn decode(_data: &[u8]) -> Result<Self, Error> {
        Ok(Self)
    }
}

// ---------------------------------------------------------------------------
// YouAreRequest
// ---------------------------------------------------------------------------

/// You-Are-Request service parameters.
///
/// ```text
/// YouAreRequest ::= SEQUENCE {
///     vendorID           [0] Unsigned16,
///     modelName          [1] CharacterString,
///     serialNumber       [2] CharacterString,
///     deviceIdentifier   [3] ObjectIdentifier OPTIONAL,
///     deviceMACAddress   [4] OctetString OPTIONAL
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YouAreRequest {
    pub vendor_id: u16,
    pub model_name: String,
    pub serial_number: String,
    pub device_identifier: Option<ObjectIdentifier>,
    pub device_mac_address: Option<Vec<u8>>,
}

impl YouAreRequest {
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] vendorID
        primitives::encode_ctx_unsigned(buf, 0, self.vendor_id as u64);
        // [1] modelName
        primitives::encode_ctx_character_string(buf, 1, &self.model_name)?;
        // [2] serialNumber
        primitives::encode_ctx_character_string(buf, 2, &self.serial_number)?;
        // [3] deviceIdentifier OPTIONAL
        if let Some(ref oid) = self.device_identifier {
            primitives::encode_ctx_object_id(buf, 3, oid);
        }
        // [4] deviceMACAddress OPTIONAL
        if let Some(ref mac) = self.device_mac_address {
            primitives::encode_ctx_octet_string(buf, 4, mac);
        }
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] vendorID
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(0) {
            return Err(Error::decoding(offset, "YouAre expected context tag 0"));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "YouAre truncated at vendor-id"));
        }
        let vendor_id = u16::try_from(primitives::decode_unsigned(&data[pos..end])?)
            .map_err(|_| Error::decoding(pos, "YouAre vendor-id exceeds u16"))?;
        offset = end;

        // [1] modelName
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(1) {
            return Err(Error::decoding(offset, "YouAre expected context tag 1"));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "YouAre truncated at model-name"));
        }
        let model_name = primitives::decode_character_string(&data[pos..end])?;
        offset = end;

        // [2] serialNumber
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(2) {
            return Err(Error::decoding(offset, "YouAre expected context tag 2"));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "YouAre truncated at serial-number"));
        }
        let serial_number = primitives::decode_character_string(&data[pos..end])?;
        offset = end;

        // [3] deviceIdentifier OPTIONAL
        let mut device_identifier = None;
        if offset < data.len() {
            let (opt, new_off) = tags::decode_optional_context(data, offset, 3)?;
            if let Some(content) = opt {
                device_identifier = Some(ObjectIdentifier::decode(content)?);
                offset = new_off;
            }
        }

        // [4] deviceMACAddress OPTIONAL
        let mut device_mac_address = None;
        if offset < data.len() {
            let (opt, new_off) = tags::decode_optional_context(data, offset, 4)?;
            if let Some(content) = opt {
                device_mac_address = Some(content.to_vec());
                offset = new_off;
            }
        }
        let _ = offset;

        Ok(Self {
            vendor_id,
            model_name,
            serial_number,
            device_identifier,
            device_mac_address,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    fn encode_required_fields(
        vendor_tag: u8,
        vendor_id: u64,
        model_tag: u8,
        serial_tag: u8,
    ) -> BytesMut {
        let mut buf = BytesMut::new();
        primitives::encode_ctx_unsigned(&mut buf, vendor_tag, vendor_id);
        primitives::encode_ctx_character_string(&mut buf, model_tag, "M").unwrap();
        primitives::encode_ctx_character_string(&mut buf, serial_tag, "S").unwrap();
        buf
    }

    #[test]
    fn who_am_i_round_trip() {
        let req = WhoAmIRequest;
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(buf.is_empty());
        let decoded = WhoAmIRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn who_am_i_ignores_trailing_data() {
        let decoded = WhoAmIRequest::decode(&[0xFF, 0x01, 0x02]).unwrap();
        assert_eq!(WhoAmIRequest, decoded);
    }

    #[test]
    fn you_are_round_trip() {
        let req = YouAreRequest {
            vendor_id: 42,
            model_name: "TestDevice".to_string(),
            serial_number: "SN-12345".to_string(),
            device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap()),
            device_mac_address: Some(vec![0xDE, 0xAD, 0xBE, 0xEF]),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = YouAreRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn you_are_minimal() {
        let req = YouAreRequest {
            vendor_id: 1,
            model_name: "M".to_string(),
            serial_number: "S".to_string(),
            device_identifier: None,
            device_mac_address: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = YouAreRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn you_are_vendor_id_must_fit_u16() {
        let maximum = encode_required_fields(0, u64::from(u16::MAX), 1, 2);
        assert_eq!(YouAreRequest::decode(&maximum).unwrap().vendor_id, u16::MAX);

        let mut leading_zero = BytesMut::new();
        tags::encode_tag(&mut leading_zero, 0, tags::TagClass::Context, 3);
        leading_zero.extend_from_slice(&[0x00, 0xff, 0xff]);
        primitives::encode_ctx_character_string(&mut leading_zero, 1, "M").unwrap();
        primitives::encode_ctx_character_string(&mut leading_zero, 2, "S").unwrap();
        assert_eq!(
            YouAreRequest::decode(&leading_zero).unwrap().vendor_id,
            u16::MAX
        );

        for value in [u64::from(u16::MAX) + 1, 65_537, u64::MAX] {
            let data = encode_required_fields(0, value, 1, 2);
            assert!(YouAreRequest::decode(&data).is_err());
        }
    }

    #[test]
    fn you_are_requires_mandatory_context_tags() {
        for (vendor_tag, model_tag, serial_tag) in [(1, 1, 2), (0, 2, 2), (0, 1, 1)] {
            let data = encode_required_fields(vendor_tag, 42, model_tag, serial_tag);
            assert!(YouAreRequest::decode(&data).is_err());
        }
    }

    #[test]
    fn you_are_empty_input() {
        assert!(YouAreRequest::decode(&[]).is_err());
    }

    #[test]
    fn you_are_truncated() {
        let req = YouAreRequest {
            vendor_id: 42,
            model_name: "Test".to_string(),
            serial_number: "SN".to_string(),
            device_identifier: None,
            device_mac_address: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(YouAreRequest::decode(&buf[..2]).is_err());
    }
}
