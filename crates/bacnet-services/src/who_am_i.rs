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
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "YouAre truncated at vendor-id"));
        }
        let vendor_id = primitives::decode_unsigned(&data[pos..end])? as u16;
        offset = end;

        // [1] modelName
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "YouAre truncated at model-name"));
        }
        let model_name = primitives::decode_character_string(&data[pos..end])?;
        offset = end;

        // [2] serialNumber
        let (tag, pos) = tags::decode_tag(data, offset)?;
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
