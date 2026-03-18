//! WritePropertyMultiple service per ASHRAE 135-2020 Clause 15.10.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::{BACnetPropertyValue, MAX_DECODED_ITEMS};

// ---------------------------------------------------------------------------
// WritePropertyMultipleRequest
// ---------------------------------------------------------------------------

/// A single object + list of property values to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteAccessSpecification {
    pub object_identifier: ObjectIdentifier,
    pub list_of_properties: Vec<BACnetPropertyValue>,
}

/// WritePropertyMultiple-Request service parameters.
///
/// Uses SimpleACK (no ACK struct needed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePropertyMultipleRequest {
    pub list_of_write_access_specs: Vec<WriteAccessSpecification>,
}

impl WritePropertyMultipleRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        for spec in &self.list_of_write_access_specs {
            primitives::encode_ctx_object_id(buf, 0, &spec.object_identifier);
            tags::encode_opening_tag(buf, 1);
            for prop_val in &spec.list_of_properties {
                prop_val.encode(buf);
            }
            tags::encode_closing_tag(buf, 1);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;
        let mut specs = Vec::new();

        while offset < data.len() {
            if specs.len() >= MAX_DECODED_ITEMS {
                return Err(Error::decoding(
                    offset,
                    "WPM request exceeds max decoded items",
                ));
            }

            // [0] object-identifier
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(pos, "WPM request truncated at object-id"));
            }
            let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
            offset = end;

            // [1] list-of-properties (opening tag 1)
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(1) {
                return Err(Error::decoding(
                    offset,
                    "WPM request expected opening tag 1",
                ));
            }
            offset = tag_end;

            let mut props = Vec::new();
            loop {
                if offset >= data.len() {
                    return Err(Error::decoding(offset, "WPM request missing closing tag 1"));
                }
                if props.len() >= MAX_DECODED_ITEMS {
                    return Err(Error::decoding(offset, "WPM properties exceeds max"));
                }
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if tag.is_closing_tag(1) {
                    offset = tag_end;
                    break;
                }
                let (pv, new_offset) = BACnetPropertyValue::decode(data, offset)?;
                props.push(pv);
                offset = new_offset;
            }

            specs.push(WriteAccessSpecification {
                object_identifier,
                list_of_properties: props,
            });
        }

        Ok(Self {
            list_of_write_access_specs: specs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::{ObjectType, PropertyIdentifier};

    #[test]
    fn request_round_trip() {
        let req = WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                    priority: Some(8),
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = WritePropertyMultipleRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn multi_object_round_trip() {
        let req = WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![
                WriteAccessSpecification {
                    object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
                    list_of_properties: vec![BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                        value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                        priority: None,
                    }],
                },
                WriteAccessSpecification {
                    object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 2).unwrap(),
                    list_of_properties: vec![BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                        value: vec![0x91, 0x01],
                        priority: Some(8),
                    }],
                },
            ],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = WritePropertyMultipleRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_wpm_request_truncated_1_byte() {
        let req = WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                    priority: Some(8),
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(WritePropertyMultipleRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_wpm_request_truncated_3_bytes() {
        let req = WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                    priority: Some(8),
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(WritePropertyMultipleRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_wpm_request_truncated_half() {
        let req = WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                    priority: Some(8),
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(WritePropertyMultipleRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_wpm_request_invalid_tag() {
        assert!(WritePropertyMultipleRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}
