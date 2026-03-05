//! Shared BACnet service data types per ASHRAE 135-2020 Clause 21.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bytes::{BufMut, BytesMut};

/// Safety limit for decoded sequences to prevent unbounded allocations.
pub const MAX_DECODED_ITEMS: usize = 10_000;

// ---------------------------------------------------------------------------
// PropertyReference
// ---------------------------------------------------------------------------

/// BACnetPropertyReference per Clause 21.
///
/// ```text
/// BACnetPropertyReference ::= SEQUENCE {
///     propertyIdentifier  [0] BACnetPropertyIdentifier,
///     propertyArrayIndex  [1] Unsigned OPTIONAL
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyReference {
    pub property_identifier: PropertyIdentifier,
    pub property_array_index: Option<u32>,
}

impl PropertyReference {
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_ctx_unsigned(buf, 0, self.property_identifier.to_raw() as u64);
        if let Some(idx) = self.property_array_index {
            primitives::encode_ctx_unsigned(buf, 1, idx as u64);
        }
    }

    pub fn decode(data: &[u8], offset: usize) -> Result<(Self, usize), Error> {
        // [0] propertyIdentifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "PropertyReference truncated at property-id",
            ));
        }
        let prop_id = primitives::decode_unsigned(&data[pos..end])? as u32;
        let mut offset = end;

        // [1] propertyArrayIndex (optional)
        let mut array_index = None;
        if offset < data.len() {
            let (tag, new_pos) = tags::decode_tag(data, offset)?;
            if tag.is_context(1) {
                let end = new_pos + tag.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        new_pos,
                        "PropertyReference truncated at array-index",
                    ));
                }
                array_index = Some(primitives::decode_unsigned(&data[new_pos..end])? as u32);
                offset = end;
            }
        }

        Ok((
            Self {
                property_identifier: PropertyIdentifier::from_raw(prop_id),
                property_array_index: array_index,
            },
            offset,
        ))
    }
}

// ---------------------------------------------------------------------------
// BACnetPropertyValue
// ---------------------------------------------------------------------------

/// BACnetPropertyValue per Clause 21.
///
/// ```text
/// BACnetPropertyValue ::= SEQUENCE {
///     propertyIdentifier  [0] BACnetPropertyIdentifier,
///     propertyArrayIndex  [1] Unsigned OPTIONAL,
///     value               [2] ABSTRACT-SYNTAX.&Type,
///     priority            [3] Unsigned (1..16) OPTIONAL
/// }
/// ```
///
/// The `value` field contains raw application-tagged bytes. The application
/// layer interprets the value based on the property type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetPropertyValue {
    pub property_identifier: PropertyIdentifier,
    pub property_array_index: Option<u32>,
    pub value: Vec<u8>,
    pub priority: Option<u8>,
}

impl BACnetPropertyValue {
    pub fn encode(&self, buf: &mut BytesMut) {
        // [0] propertyIdentifier
        primitives::encode_ctx_unsigned(buf, 0, self.property_identifier.to_raw() as u64);
        // [1] propertyArrayIndex (optional)
        if let Some(idx) = self.property_array_index {
            primitives::encode_ctx_unsigned(buf, 1, idx as u64);
        }
        // [2] value (opening/closing)
        tags::encode_opening_tag(buf, 2);
        buf.put_slice(&self.value);
        tags::encode_closing_tag(buf, 2);
        // [3] priority (optional)
        if let Some(prio) = self.priority {
            primitives::encode_ctx_unsigned(buf, 3, prio as u64);
        }
    }

    pub fn decode(data: &[u8], offset: usize) -> Result<(Self, usize), Error> {
        // [0] propertyIdentifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "BACnetPropertyValue truncated at property-id",
            ));
        }
        let prop_id = primitives::decode_unsigned(&data[pos..end])? as u32;
        let mut offset = end;

        // [1] propertyArrayIndex (optional) — peek to see if it's tag 1
        let mut array_index = None;
        if offset < data.len() {
            let (tag, new_pos) = tags::decode_tag(data, offset)?;
            if tag.is_context(1) {
                let end = new_pos + tag.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        new_pos,
                        "BACnetPropertyValue truncated at array-index",
                    ));
                }
                array_index = Some(primitives::decode_unsigned(&data[new_pos..end])? as u32);
                offset = end;
            }
        }

        // [2] value — extract between opening/closing tag 2
        // We need to skip the opening tag we already peeked at
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(2) {
            return Err(Error::decoding(
                offset,
                "BACnetPropertyValue expected opening tag 2",
            ));
        }
        let (value_bytes, offset) = tags::extract_context_value(data, tag_end, 2)?;
        let value = value_bytes.to_vec();

        // [3] priority (optional)
        let mut priority = None;
        if offset < data.len() {
            let (tag, new_pos) = tags::decode_tag(data, offset)?;
            if tag.is_context(3) {
                let end = new_pos + tag.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        new_pos,
                        "BACnetPropertyValue truncated at priority",
                    ));
                }
                let prio = primitives::decode_unsigned(&data[new_pos..end])? as u8;
                if !(1..=16).contains(&prio) {
                    return Err(Error::decoding(
                        new_pos,
                        format!("BACnetPropertyValue priority {prio} out of range 1-16"),
                    ));
                }
                priority = Some(prio);
                return Ok((
                    Self {
                        property_identifier: PropertyIdentifier::from_raw(prop_id),
                        property_array_index: array_index,
                        value,
                        priority,
                    },
                    end,
                ));
            }
        }

        Ok((
            Self {
                property_identifier: PropertyIdentifier::from_raw(prop_id),
                property_array_index: array_index,
                value,
                priority,
            },
            offset,
        ))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_reference_round_trip() {
        let pr = PropertyReference {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        };
        let mut buf = BytesMut::new();
        pr.encode(&mut buf);
        let (decoded, _) = PropertyReference::decode(&buf, 0).unwrap();
        assert_eq!(pr, decoded);
    }

    #[test]
    fn property_reference_with_index_round_trip() {
        let pr = PropertyReference {
            property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
            property_array_index: Some(8),
        };
        let mut buf = BytesMut::new();
        pr.encode(&mut buf);
        let (decoded, _) = PropertyReference::decode(&buf, 0).unwrap();
        assert_eq!(pr, decoded);
    }

    #[test]
    fn bacnet_property_value_round_trip() {
        let pv = BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x44, 0x42, 0x90, 0x00, 0x00], // app-tagged Real 72.5
            priority: None,
        };
        let mut buf = BytesMut::new();
        pv.encode(&mut buf);
        let (decoded, _) = BACnetPropertyValue::decode(&buf, 0).unwrap();
        assert_eq!(pv, decoded);
    }

    #[test]
    fn bacnet_property_value_with_all_fields() {
        let pv = BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: Some(5),
            value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: Some(8),
        };
        let mut buf = BytesMut::new();
        pv.encode(&mut buf);
        let (decoded, _) = BACnetPropertyValue::decode(&buf, 0).unwrap();
        assert_eq!(pv, decoded);
    }

    #[test]
    fn bacnet_property_value_priority_validation() {
        let pv = BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x10], // app boolean true
            priority: Some(8),
        };
        let mut buf = BytesMut::new();
        pv.encode(&mut buf);

        // Manually corrupt priority to 0
        let data = buf.to_vec();
        let mut corrupted = data.clone();
        // Priority is the last encoded byte — find and change it
        let last = corrupted.len() - 1;
        corrupted[last] = 0; // set priority value to 0
        assert!(BACnetPropertyValue::decode(&corrupted, 0).is_err());
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_property_reference_empty_input() {
        assert!(PropertyReference::decode(&[], 0).is_err());
    }

    #[test]
    fn test_decode_property_reference_truncated_1_byte() {
        let pr = PropertyReference {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: Some(8),
        };
        let mut buf = BytesMut::new();
        pr.encode(&mut buf);
        assert!(PropertyReference::decode(&buf[..1], 0).is_err());
    }

    #[test]
    fn test_decode_property_reference_invalid_tag() {
        assert!(PropertyReference::decode(&[0xFF, 0xFF, 0xFF], 0).is_err());
    }

    #[test]
    fn test_decode_bacnet_property_value_empty_input() {
        assert!(BACnetPropertyValue::decode(&[], 0).is_err());
    }

    #[test]
    fn test_decode_bacnet_property_value_truncated_1_byte() {
        let pv = BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: None,
        };
        let mut buf = BytesMut::new();
        pv.encode(&mut buf);
        assert!(BACnetPropertyValue::decode(&buf[..1], 0).is_err());
    }

    #[test]
    fn test_decode_bacnet_property_value_truncated_2_bytes() {
        let pv = BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: None,
        };
        let mut buf = BytesMut::new();
        pv.encode(&mut buf);
        assert!(BACnetPropertyValue::decode(&buf[..2], 0).is_err());
    }

    #[test]
    fn test_decode_bacnet_property_value_truncated_3_bytes() {
        let pv = BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: None,
        };
        let mut buf = BytesMut::new();
        pv.encode(&mut buf);
        assert!(BACnetPropertyValue::decode(&buf[..3], 0).is_err());
    }

    #[test]
    fn test_decode_bacnet_property_value_invalid_tag() {
        assert!(BACnetPropertyValue::decode(&[0xFF, 0xFF, 0xFF], 0).is_err());
    }

    #[test]
    fn test_decode_bacnet_property_value_oversized_length() {
        // Tag byte with extended length that exceeds data
        assert!(BACnetPropertyValue::decode(&[0x05, 0xFF], 0).is_err());
    }
}
