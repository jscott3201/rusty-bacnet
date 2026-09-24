//! AddListElement / RemoveListElement services per ASHRAE 135-2020 Clauses 15.1 and 15.2.

use bacnet_encoding::{primitives, tags};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

/// AddListElement-Request / RemoveListElement-Request service parameters.
///
/// Both services share the same PDU structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListElementRequest {
    pub object_identifier: ObjectIdentifier,
    pub property_identifier: PropertyIdentifier,
    pub property_array_index: Option<u32>,
    /// Raw encoded list of elements to add/remove.
    pub list_of_elements: Vec<u8>,
}

impl ListElementRequest {
    /// Validate local outbound constraints without interpreting element values.
    ///
    /// Requires elements and a nonzero optional index. Tag headers, lengths and
    /// balanced context nesting use the shared parser limits, counting the outer
    /// service [3] wrapper. Application Boolean has no payload octets. Primitive
    /// value forms, vendor semantics and remote property types are not checked.
    pub fn validate(&self) -> Result<(), Error> {
        if self.property_array_index == Some(0) {
            return Err(Error::Encoding(
                "ListElement array index must be nonzero".into(),
            ));
        }
        if self.list_of_elements.is_empty() {
            return Err(Error::Encoding(
                "ListElement requires at least one encoded element".into(),
            ));
        }
        validate_elements_framing(&self.list_of_elements)
            .map_err(|error| Error::Encoding(format!("ListElement framing: {error}")))
    }

    /// Encode transactionally; invalid input leaves the caller's buffer unchanged.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.validate()?;
        // [0] objectIdentifier
        primitives::encode_ctx_object_id(buf, 0, &self.object_identifier);
        // [1] propertyIdentifier
        primitives::encode_ctx_enumerated(buf, 1, self.property_identifier.to_raw());
        // [2] propertyArrayIndex (optional)
        if let Some(idx) = self.property_array_index {
            primitives::encode_ctx_unsigned(buf, 2, idx as u64);
        }
        // [3] listOfElements
        tags::encode_opening_tag(buf, 3);
        buf.extend_from_slice(&self.list_of_elements);
        tags::encode_closing_tag(buf, 3);
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] objectIdentifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(0) {
            return Err(Error::decoding(
                offset,
                "ListElement request expected context tag 0",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::buffer_too_short(end, data.len()));
        }
        let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [1] propertyIdentifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(1) {
            return Err(Error::decoding(
                offset,
                "ListElement request expected context tag 1",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::buffer_too_short(end, data.len()));
        }
        let property_identifier = primitives::decode_unsigned(&data[pos..end])?;
        let property_identifier = u32::try_from(property_identifier)
            .map(PropertyIdentifier::from_raw)
            .map_err(|_| Error::decoding(pos, "ListElement property-id exceeds u32"))?;
        offset = end;

        // [2] propertyArrayIndex (optional)
        let mut property_array_index = None;
        let (opt_data, new_offset) = tags::decode_optional_context(data, offset, 2)?;
        if let Some(content) = opt_data {
            let value = primitives::decode_unsigned(content)?;
            property_array_index = Some(
                u32::try_from(value)
                    .map_err(|_| Error::decoding(offset, "ListElement array-index exceeds u32"))?,
            );
            offset = new_offset;
        }

        // [3] listOfElements
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(3) {
            return Err(Error::decoding(
                offset,
                "ListElement request expected opening tag 3",
            ));
        }
        let (content, offset) = tags::extract_context_value(data, tag_end, 3)?;
        if offset != data.len() {
            return Err(Error::decoding(
                offset,
                "ListElement request has trailing data",
            ));
        }
        let list_of_elements = content.to_vec();

        Ok(Self {
            object_identifier,
            property_identifier,
            property_array_index,
            list_of_elements,
        })
    }
}

// The surrounding service [3] is already open: callers cannot close it from
// within the element bytes, and it consumes one slot of the shared depth limit.
// This framing-only walk deliberately does not decode application values.
fn validate_elements_framing(data: &[u8]) -> Result<(), Error> {
    let mut open_tags = [0; tags::MAX_CONTEXT_NESTING_DEPTH];
    open_tags[0] = 3;
    let mut depth = 1;
    let mut offset = 0;
    while offset < data.len() {
        let (tag, next) = tags::decode_tag(data, offset)?;
        if tag.is_opening {
            if depth == open_tags.len() {
                return Err(Error::decoding(
                    offset,
                    "context nesting exceeds shared limit",
                ));
            }
            open_tags[depth] = tag.number;
            depth += 1;
            offset = next;
        } else if tag.is_closing {
            if depth == 1 || open_tags[depth - 1] != tag.number {
                return Err(Error::decoding(
                    offset,
                    "unmatched or mismatched closing tag",
                ));
            }
            depth -= 1;
            offset = next;
        } else if tag.class == tags::TagClass::Application && tag.number == tags::app_tag::BOOLEAN {
            offset = next;
        } else {
            offset = next
                .checked_add(tag.length as usize)
                .filter(|&end| end <= data.len())
                .ok_or_else(|| Error::decoding(next, "tag contents exceed element bytes"))?;
        }
    }
    if depth != 1 {
        return Err(Error::decoding(offset, "unclosed context tag"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    fn request_with_property_fields(property: &[u8], array_index: Option<&[u8]>) -> BytesMut {
        let mut buf = BytesMut::new();
        let object = ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap();
        primitives::encode_ctx_object_id(&mut buf, 0, &object);
        primitives::encode_ctx_octet_string(&mut buf, 1, property);
        if let Some(array_index) = array_index {
            primitives::encode_ctx_octet_string(&mut buf, 2, array_index);
        }
        tags::encode_opening_tag(&mut buf, 3);
        primitives::encode_app_null(&mut buf);
        tags::encode_closing_tag(&mut buf, 3);
        buf
    }

    #[test]
    fn add_list_element_round_trip() {
        // list_of_elements must be valid tagged data (app-tagged unsigned 42 = [0x21, 0x2A])
        let elements = vec![0x21, 0x2A];
        let req = ListElementRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap(),
            property_identifier: PropertyIdentifier::RECIPIENT_LIST,
            property_array_index: None,
            list_of_elements: elements.clone(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = ListElementRequest::decode(&buf).unwrap();
        assert_eq!(decoded.object_identifier, req.object_identifier);
        assert_eq!(decoded.property_identifier, req.property_identifier);
        assert_eq!(decoded.list_of_elements, elements);
    }

    #[test]
    fn with_array_index_round_trip() {
        // Two app-tagged unsigned values: 10 and 20
        let elements = vec![0x21, 0x0A, 0x21, 0x14];
        let req = ListElementRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::SCHEDULE, 1).unwrap(),
            property_identifier: PropertyIdentifier::WEEKLY_SCHEDULE,
            property_array_index: Some(3),
            list_of_elements: elements.clone(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = ListElementRequest::decode(&buf).unwrap();
        assert_eq!(decoded.property_array_index, Some(3));
        assert_eq!(decoded.list_of_elements, elements);
    }

    #[test]
    fn list_element_values_must_fit_u32() {
        let max_with_leading_zero = [0, 0xFF, 0xFF, 0xFF, 0xFF];
        let encoded =
            request_with_property_fields(&max_with_leading_zero, Some(&max_with_leading_zero));
        let decoded = ListElementRequest::decode(&encoded).unwrap();
        assert_eq!(decoded.property_identifier.to_raw(), u32::MAX);
        assert_eq!(decoded.property_array_index, Some(u32::MAX));

        for overflow in [u32::MAX as u64 + 1, u64::MAX] {
            let overflow = overflow.to_be_bytes();
            assert!(
                ListElementRequest::decode(&request_with_property_fields(&overflow, None)).is_err()
            );
            assert!(ListElementRequest::decode(&request_with_property_fields(
                &[1],
                Some(&overflow),
            ))
            .is_err());
        }
    }

    #[test]
    fn list_element_requires_owned_tags_and_complete_payload() {
        let encoded = request_with_property_fields(&[85], None);
        let (object_tag, object_pos) = tags::decode_tag(&encoded, 0).unwrap();
        let property_offset = object_pos + object_tag.length as usize;
        let (property_tag, property_pos) = tags::decode_tag(&encoded, property_offset).unwrap();
        let list_offset = property_pos + property_tag.length as usize;

        let mut wrong_object = encoded.clone();
        wrong_object[0] = 0x1C;
        assert!(ListElementRequest::decode(&wrong_object).is_err());

        let mut wrong_property = encoded.clone();
        wrong_property[property_offset] = 0x29;
        assert!(ListElementRequest::decode(&wrong_property).is_err());

        let mut wrong_list = encoded.clone();
        wrong_list[list_offset] = 0x4E;
        assert!(ListElementRequest::decode(&wrong_list).is_err());

        let mut trailing = encoded;
        primitives::encode_app_null(&mut trailing);
        assert!(ListElementRequest::decode(&trailing).is_err());
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_list_element_empty_input() {
        assert!(ListElementRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_list_element_truncated_1_byte() {
        let req = ListElementRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap(),
            property_identifier: PropertyIdentifier::RECIPIENT_LIST,
            property_array_index: None,
            list_of_elements: vec![0x21, 0x2A],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(ListElementRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_list_element_truncated_3_bytes() {
        let req = ListElementRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap(),
            property_identifier: PropertyIdentifier::RECIPIENT_LIST,
            property_array_index: None,
            list_of_elements: vec![0x21, 0x2A],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(ListElementRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_list_element_truncated_half() {
        let req = ListElementRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap(),
            property_identifier: PropertyIdentifier::RECIPIENT_LIST,
            property_array_index: None,
            list_of_elements: vec![0x21, 0x2A],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let half = buf.len() / 2;
        assert!(ListElementRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_list_element_invalid_tag() {
        assert!(ListElementRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}

#[cfg(test)]
#[path = "list_validation_tests.rs"]
mod validation_tests;
