//! Shared BACnet service data types per ASHRAE 135-2020 Clause 21.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bytes::{BufMut, BytesMut};

/// Safety limit for decoded sequences to prevent unbounded allocations.
pub const MAX_DECODED_ITEMS: usize = 10_000;

pub(crate) fn decode_context<'a>(
    data: &'a [u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> Result<(&'a [u8], usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if !tag.is_context(expected_tag) {
        return Err(Error::decoding(
            offset,
            format!("{field} expected context tag {expected_tag}"),
        ));
    }
    let end = pos
        .checked_add(tag.length as usize)
        .ok_or_else(|| Error::decoding(pos, format!("{field} length overflow")))?;
    if end > data.len() {
        return Err(Error::decoding(pos, format!("{field} truncated")));
    }
    Ok((&data[pos..end], end))
}

pub(crate) fn decode_context_u32(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> Result<(u32, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    let value = primitives::decode_unsigned(content)?;
    let value = u32::try_from(value)
        .map_err(|_| Error::decoding(offset, format!("{field} exceeds u32")))?;
    Ok((value, end))
}

pub(crate) fn decode_context_bool(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> Result<(bool, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    let value = match content {
        [0] => false,
        [1] => true,
        _ => {
            return Err(Error::decoding(
                offset,
                format!("{field} expected Boolean 0 or 1"),
            ));
        }
    };
    Ok((value, end))
}

#[derive(Clone, Copy)]
pub(crate) enum PropertyValueBoundary {
    End,
    Context(u8),
    ContextToEnd(u8),
    Closing(u8),
}

fn matches_property_boundary(data: &[u8], offset: usize, boundary: PropertyValueBoundary) -> bool {
    match boundary {
        PropertyValueBoundary::End => offset == data.len(),
        PropertyValueBoundary::Context(number) => {
            tags::decode_tag(data, offset).is_ok_and(|(tag, _)| tag.is_context(number))
        }
        PropertyValueBoundary::ContextToEnd(number) => {
            tags::decode_tag(data, offset).is_ok_and(|(tag, content_start)| {
                tag.is_context(number)
                    && content_start
                        .checked_add(tag.length as usize)
                        .is_some_and(|end| end == data.len())
            })
        }
        PropertyValueBoundary::Closing(number) => {
            tags::decode_tag(data, offset).is_ok_and(|(tag, _)| tag.is_closing_tag(number))
        }
    }
}

pub(crate) fn extract_property_value<'a>(
    data: &'a [u8],
    offset: usize,
    closing_tag: u8,
    property: PropertyIdentifier,
    boundaries: &[PropertyValueBoundary],
) -> Result<(&'a [u8], usize), Error> {
    if property == PropertyIdentifier::EVENT_PARAMETERS
        && data.get(offset..offset.saturating_add(2)) == Some(&[0xfe, 0xff])
    {
        // Before EventParameter framing, tag 255 wrapped arbitrary octets. Use
        // the enclosing service grammar to distinguish a payload marker from
        // the wrapper terminator without consuming a sibling property. Reject
        // multiple valid boundaries because the old format cannot disambiguate them.
        let outer_close = (closing_tag << 4) | 0x0f;
        let mut candidate = None;
        for pos in offset.saturating_add(4)..data.len() {
            let end = pos + 1;
            if data[pos] == outer_close
                && data.get(pos - 2..pos) == Some(&[0xff, 0xff])
                && boundaries
                    .iter()
                    .any(|boundary| matches_property_boundary(data, end, *boundary))
            {
                if candidate.is_some() {
                    return Err(Error::decoding(
                        offset,
                        "legacy EventParameters value has ambiguous closing tags",
                    ));
                }
                candidate = Some((pos, end));
            }
        }
        if let Some((pos, end)) = candidate {
            return Ok((&data[offset..pos], end));
        }
        return Err(Error::decoding(
            offset,
            "legacy EventParameters value is missing its closing tags",
        ));
    }

    tags::extract_context_value(data, offset, closing_tag)
}

// ---------------------------------------------------------------------------
// PropertyReference
// ---------------------------------------------------------------------------

/// BACnetPropertyReference.
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
        let (prop_id, mut offset) =
            decode_context_u32(data, offset, 0, "PropertyReference property-id")?;

        // [1] propertyArrayIndex (optional)
        let mut array_index = None;
        if offset < data.len() {
            let (tag, _) = tags::decode_tag(data, offset)?;
            if tag.is_context(1) {
                let (value, end) =
                    decode_context_u32(data, offset, 1, "PropertyReference array-index")?;
                array_index = Some(value);
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

/// BACnetPropertyValue.
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
        Self::decode_with_boundaries(
            data,
            offset,
            &[
                PropertyValueBoundary::End,
                PropertyValueBoundary::ContextToEnd(3),
            ],
        )
    }

    pub(crate) fn decode_in_list(
        data: &[u8],
        offset: usize,
        closing_tag: u8,
    ) -> Result<(Self, usize), Error> {
        Self::decode_with_boundaries(
            data,
            offset,
            &[
                PropertyValueBoundary::Context(3),
                PropertyValueBoundary::Context(0),
                PropertyValueBoundary::Closing(closing_tag),
            ],
        )
    }

    fn decode_with_boundaries(
        data: &[u8],
        offset: usize,
        boundaries: &[PropertyValueBoundary],
    ) -> Result<(Self, usize), Error> {
        // [0] propertyIdentifier
        let (prop_id, mut offset) =
            decode_context_u32(data, offset, 0, "BACnetPropertyValue property-id")?;

        // [1] propertyArrayIndex (optional)
        let mut array_index = None;
        if offset < data.len() {
            let (tag, _) = tags::decode_tag(data, offset)?;
            if tag.is_context(1) {
                let (value, end) =
                    decode_context_u32(data, offset, 1, "BACnetPropertyValue array-index")?;
                array_index = Some(value);
                offset = end;
            }
        }

        // [2] value
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(2) {
            return Err(Error::decoding(
                offset,
                "BACnetPropertyValue expected opening tag 2",
            ));
        }
        let property_identifier = PropertyIdentifier::from_raw(prop_id);
        let (value_bytes, offset) =
            extract_property_value(data, tag_end, 2, property_identifier, boundaries)?;
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
                let prio = primitives::decode_unsigned(&data[new_pos..end])?;
                if !(1..=16).contains(&prio) {
                    return Err(Error::decoding(
                        new_pos,
                        format!("BACnetPropertyValue priority {prio} out of range 1-16"),
                    ));
                }
                priority = Some(u8::try_from(prio).map_err(|_| {
                    Error::decoding(new_pos, "BACnetPropertyValue priority conversion failed")
                })?);
                return Ok((
                    Self {
                        property_identifier,
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
                property_identifier,
                property_array_index: array_index,
                value,
                priority,
            },
            offset,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_encoding::tags::TagClass;

    fn encode_context_bytes(buf: &mut BytesMut, tag: u8, value: &[u8]) {
        tags::encode_tag(
            buf,
            tag,
            TagClass::Context,
            u32::try_from(value.len()).unwrap(),
        );
        buf.put_slice(value);
    }

    fn append_null_value(buf: &mut BytesMut) {
        tags::encode_opening_tag(buf, 2);
        primitives::encode_app_null(buf);
        tags::encode_closing_tag(buf, 2);
    }

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
    fn bacnet_property_value_preserves_legacy_event_parameters() {
        let pv = BACnetPropertyValue {
            property_identifier: PropertyIdentifier::EVENT_PARAMETERS,
            property_array_index: None,
            value: vec![0xfe, 0xff, 1, 0xff, 0xff, 0x2f, 2, 0xff, 0xff],
            priority: None,
        };
        let mut buf = BytesMut::new();
        pv.encode(&mut buf);

        let (decoded, consumed) = BACnetPropertyValue::decode(&buf, 0).unwrap();
        assert_eq!(decoded, pv);
        assert_eq!(consumed, buf.len());
    }

    #[test]
    fn bacnet_property_value_rejects_unclosed_legacy_event_parameters() {
        let pv = BACnetPropertyValue {
            property_identifier: PropertyIdentifier::EVENT_PARAMETERS,
            property_array_index: None,
            value: vec![0xfe, 0xff, 1, 2],
            priority: None,
        };
        let mut buf = BytesMut::new();
        pv.encode(&mut buf);

        assert!(BACnetPropertyValue::decode(&buf, 0).is_err());
    }

    #[test]
    fn bacnet_property_value_priority_validation() {
        let pv = BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x10], // app boolean true
            priority: None,
        };
        let mut base = BytesMut::new();
        pv.encode(&mut base);

        for priority in [0, 17, 257, 272, u64::MAX] {
            let mut encoded = base.clone();
            primitives::encode_ctx_unsigned(&mut encoded, 3, priority);
            let error = BACnetPropertyValue::decode(&encoded, 0).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("priority {priority} out of range 1-16")),
                "unexpected error for priority {priority}: {error}"
            );
        }

        let mut leading_zero = base;
        // Context tag 3 with a two-octet leading-zero encoding of numeric 1.
        leading_zero.extend_from_slice(&[0x3A, 0x00, 0x01]);
        let (decoded, consumed) = BACnetPropertyValue::decode(&leading_zero, 0).unwrap();
        assert_eq!(decoded.priority, Some(1));
        assert_eq!(consumed, leading_zero.len());
    }

    #[test]
    fn shared_property_values_must_fit_u32() {
        let max_with_leading_zero = [0, 0xFF, 0xFF, 0xFF, 0xFF];

        let mut reference = BytesMut::new();
        encode_context_bytes(&mut reference, 0, &max_with_leading_zero);
        encode_context_bytes(&mut reference, 1, &max_with_leading_zero);
        let (decoded, consumed) = PropertyReference::decode(&reference, 0).unwrap();
        assert_eq!(decoded.property_identifier.to_raw(), u32::MAX);
        assert_eq!(decoded.property_array_index, Some(u32::MAX));
        assert_eq!(consumed, reference.len());

        let mut property_value = BytesMut::new();
        encode_context_bytes(&mut property_value, 0, &max_with_leading_zero);
        encode_context_bytes(&mut property_value, 1, &max_with_leading_zero);
        append_null_value(&mut property_value);
        let (decoded, consumed) = BACnetPropertyValue::decode(&property_value, 0).unwrap();
        assert_eq!(decoded.property_identifier.to_raw(), u32::MAX);
        assert_eq!(decoded.property_array_index, Some(u32::MAX));
        assert_eq!(consumed, property_value.len());

        for overflow in [u32::MAX as u64 + 1, u64::MAX] {
            let mut reference_property = BytesMut::new();
            primitives::encode_ctx_unsigned(&mut reference_property, 0, overflow);
            assert!(PropertyReference::decode(&reference_property, 0).is_err());

            let mut reference_index = BytesMut::new();
            primitives::encode_ctx_unsigned(&mut reference_index, 0, 1);
            primitives::encode_ctx_unsigned(&mut reference_index, 1, overflow);
            assert!(PropertyReference::decode(&reference_index, 0).is_err());

            let mut value_property = BytesMut::new();
            primitives::encode_ctx_unsigned(&mut value_property, 0, overflow);
            append_null_value(&mut value_property);
            assert!(BACnetPropertyValue::decode(&value_property, 0).is_err());

            let mut value_index = BytesMut::new();
            primitives::encode_ctx_unsigned(&mut value_index, 0, 1);
            primitives::encode_ctx_unsigned(&mut value_index, 1, overflow);
            append_null_value(&mut value_index);
            assert!(BACnetPropertyValue::decode(&value_index, 0).is_err());
        }
    }

    #[test]
    fn shared_property_values_require_property_context_tag_zero() {
        let mut wrong_tag = BytesMut::new();
        primitives::encode_ctx_unsigned(&mut wrong_tag, 1, 85);
        append_null_value(&mut wrong_tag);

        assert!(PropertyReference::decode(&wrong_tag, 0).is_err());
        assert!(BACnetPropertyValue::decode(&wrong_tag, 0).is_err());
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
