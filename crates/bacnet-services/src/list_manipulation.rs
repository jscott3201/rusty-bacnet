//! AddListElement / RemoveListElement services per ASHRAE 135-2020 Clause 15.3.

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
    pub fn encode(&self, buf: &mut BytesMut) {
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
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] objectIdentifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::buffer_too_short(end, data.len()));
        }
        let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [1] propertyIdentifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::buffer_too_short(end, data.len()));
        }
        let property_identifier =
            PropertyIdentifier::from_raw(primitives::decode_unsigned(&data[pos..end])? as u32);
        offset = end;

        // [2] propertyArrayIndex (optional)
        let mut property_array_index = None;
        let (opt_data, new_offset) = tags::decode_optional_context(data, offset, 2)?;
        if let Some(content) = opt_data {
            property_array_index = Some(primitives::decode_unsigned(content)? as u32);
            offset = new_offset;
        }

        // [3] listOfElements
        let (_tag, tag_end) = tags::decode_tag(data, offset)?;
        let (content, _) = tags::extract_context_value(data, tag_end, 3)?;
        let list_of_elements = content.to_vec();

        Ok(Self {
            object_identifier,
            property_identifier,
            property_array_index,
            list_of_elements,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

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
        req.encode(&mut buf);
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
        req.encode(&mut buf);
        let decoded = ListElementRequest::decode(&buf).unwrap();
        assert_eq!(decoded.property_array_index, Some(3));
        assert_eq!(decoded.list_of_elements, elements);
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
        req.encode(&mut buf);
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
        req.encode(&mut buf);
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
        req.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(ListElementRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_list_element_invalid_tag() {
        assert!(ListElementRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}
