//! ConfirmedTextMessage / UnconfirmedTextMessage services
//! per ASHRAE 135-2020 Clauses 16.5 and 16.6.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::MessagePriority;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

// ---------------------------------------------------------------------------
// MessageClass
// ---------------------------------------------------------------------------

/// The messageClass CHOICE: numeric or text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageClass {
    Numeric(u32),
    Text(String),
}

// ---------------------------------------------------------------------------
// TextMessageRequest
// ---------------------------------------------------------------------------

/// Request parameters shared by ConfirmedTextMessage and
/// UnconfirmedTextMessage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMessageRequest {
    pub source_device: ObjectIdentifier,
    pub message_class: Option<MessageClass>,
    pub message_priority: MessagePriority,
    pub message: String,
}

impl TextMessageRequest {
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] textMessageSourceDevice
        primitives::encode_ctx_object_id(buf, 0, &self.source_device);
        // messageClass [1] CHOICE { numeric [0], character [1] } OPTIONAL
        if let Some(ref mc) = self.message_class {
            tags::encode_opening_tag(buf, 1);
            match mc {
                MessageClass::Numeric(n) => {
                    primitives::encode_ctx_unsigned(buf, 0, *n as u64);
                }
                MessageClass::Text(s) => {
                    primitives::encode_ctx_character_string(buf, 1, s)?;
                }
            }
            tags::encode_closing_tag(buf, 1);
        }
        // [2] messagePriority (per Clause 16.5/16.6 ASN.1)
        primitives::encode_ctx_enumerated(buf, 2, self.message_priority.to_raw());
        // [3] message
        primitives::encode_ctx_character_string(buf, 3, &self.message)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] textMessageSourceDevice
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(0) {
            return Err(Error::decoding(
                offset,
                "TextMessage expected context tag 0",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "TextMessage truncated at sourceDevice",
            ));
        }
        let source_device = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // messageClass [1] CHOICE { numeric [0], character [1] } OPTIONAL
        let mut message_class = None;
        if offset < data.len() {
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_opening_tag(1) {
                let (content, new_offset) = tags::extract_context_value(data, tag_end, 1)?;
                if content.is_empty() {
                    return Err(Error::decoding(
                        tag_end,
                        "TextMessage messageClass is empty",
                    ));
                }
                let (inner_tag, inner_pos) = tags::decode_tag(content, 0)?;
                let inner_end = inner_pos + inner_tag.length as usize;
                if inner_end > content.len() {
                    return Err(Error::decoding(
                        tag_end + inner_pos,
                        "TextMessage messageClass is truncated",
                    ));
                }
                message_class = if inner_tag.is_context(0) {
                    let value = primitives::decode_unsigned(&content[inner_pos..inner_end])?;
                    Some(MessageClass::Numeric(u32::try_from(value).map_err(
                        |_| {
                            Error::decoding(
                                tag_end + inner_pos,
                                "TextMessage numeric class exceeds u32",
                            )
                        },
                    )?))
                } else if inner_tag.is_context(1) {
                    Some(MessageClass::Text(primitives::decode_character_string(
                        &content[inner_pos..inner_end],
                    )?))
                } else {
                    return Err(Error::decoding(
                        tag_end + inner_pos,
                        "TextMessage messageClass expected context tag 0 or 1",
                    ));
                };
                if inner_end != content.len() {
                    return Err(Error::decoding(
                        tag_end + inner_end,
                        "TextMessage messageClass has trailing data",
                    ));
                }
                offset = new_offset;
            }
        }

        // [2] messagePriority (per Clause 16.5/16.6 ASN.1)
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(2) {
            return Err(Error::decoding(
                offset,
                "TextMessage expected context tag 2",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "TextMessage truncated at messagePriority",
            ));
        }
        let message_priority = primitives::decode_unsigned(&data[pos..end])?;
        let message_priority = u32::try_from(message_priority)
            .map(MessagePriority::from_raw)
            .map_err(|_| Error::decoding(pos, "TextMessage priority exceeds u32"))?;
        offset = end;

        // [3] message
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(3) {
            return Err(Error::decoding(
                offset,
                "TextMessage expected context tag 3",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "TextMessage truncated at message"));
        }
        let message = primitives::decode_character_string(&data[pos..end])?;
        if end != data.len() {
            return Err(Error::decoding(end, "TextMessage has trailing data"));
        }

        Ok(Self {
            source_device,
            message_class,
            message_priority,
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    fn append_required_fields(buf: &mut BytesMut, priority: &[u8]) {
        primitives::encode_ctx_octet_string(buf, 2, priority);
        primitives::encode_ctx_character_string(buf, 3, "message").unwrap();
    }

    fn request_with_numeric_fields(class: Option<&[u8]>, priority: &[u8]) -> BytesMut {
        let mut buf = BytesMut::new();
        let source = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
        primitives::encode_ctx_object_id(&mut buf, 0, &source);
        if let Some(class) = class {
            tags::encode_opening_tag(&mut buf, 1);
            primitives::encode_ctx_octet_string(&mut buf, 0, class);
            tags::encode_closing_tag(&mut buf, 1);
        }
        append_required_fields(&mut buf, priority);
        buf
    }

    fn request_with_class_content(content: &[u8]) -> BytesMut {
        let mut buf = BytesMut::new();
        let source = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
        primitives::encode_ctx_object_id(&mut buf, 0, &source);
        tags::encode_opening_tag(&mut buf, 1);
        buf.extend_from_slice(content);
        tags::encode_closing_tag(&mut buf, 1);
        append_required_fields(&mut buf, &[0]);
        buf
    }

    #[test]
    fn request_numeric_class_round_trip() {
        let req = TextMessageRequest {
            source_device: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
            message_class: Some(MessageClass::Numeric(5)),
            message_priority: MessagePriority::URGENT,
            message: "Fire alarm".into(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = TextMessageRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn request_text_class_round_trip() {
        let req = TextMessageRequest {
            source_device: ObjectIdentifier::new(ObjectType::DEVICE, 200).unwrap(),
            message_class: Some(MessageClass::Text("maintenance".into())),
            message_priority: MessagePriority::NORMAL,
            message: "Scheduled shutdown".into(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = TextMessageRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn request_no_class_round_trip() {
        let req = TextMessageRequest {
            source_device: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
            message_class: None,
            message_priority: MessagePriority::NORMAL,
            message: "Hello".into(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = TextMessageRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn text_message_values_must_fit_u32() {
        let max_with_leading_zero = [0, 0xFF, 0xFF, 0xFF, 0xFF];
        let decoded = TextMessageRequest::decode(&request_with_numeric_fields(
            Some(&max_with_leading_zero),
            &max_with_leading_zero,
        ))
        .unwrap();
        assert_eq!(decoded.message_class, Some(MessageClass::Numeric(u32::MAX)));
        assert_eq!(decoded.message_priority.to_raw(), u32::MAX);

        for overflow in [u32::MAX as u64 + 1, u64::MAX] {
            let overflow = overflow.to_be_bytes();
            assert!(TextMessageRequest::decode(&request_with_numeric_fields(
                Some(&overflow),
                &[0],
            ))
            .is_err());
            assert!(
                TextMessageRequest::decode(&request_with_numeric_fields(None, &overflow,)).is_err()
            );
        }
    }

    #[test]
    fn text_message_rejects_malformed_message_class() {
        assert!(TextMessageRequest::decode(&request_with_class_content(&[])).is_err());
        assert!(TextMessageRequest::decode(&request_with_class_content(&[0x0C, 0])).is_err());
        assert!(
            TextMessageRequest::decode(&request_with_class_content(&[0x09, 1, 0x09, 2,])).is_err()
        );
        assert!(TextMessageRequest::decode(&request_with_class_content(&[0x29, 1])).is_err());
    }

    #[test]
    fn text_message_requires_owned_tags_and_complete_payload() {
        let encoded = request_with_numeric_fields(None, &[0]);
        let (source_tag, source_pos) = tags::decode_tag(&encoded, 0).unwrap();
        let priority_offset = source_pos + source_tag.length as usize;
        let (priority_tag, priority_pos) = tags::decode_tag(&encoded, priority_offset).unwrap();
        let message_offset = priority_pos + priority_tag.length as usize;

        let mut wrong_source = encoded.clone();
        wrong_source[0] = 0x1C;
        assert!(TextMessageRequest::decode(&wrong_source).is_err());

        let mut wrong_priority = encoded.clone();
        wrong_priority[priority_offset] = 0x19;
        assert!(TextMessageRequest::decode(&wrong_priority).is_err());

        let mut wrong_message = encoded.clone();
        wrong_message[message_offset] = (wrong_message[message_offset] & 0x0F) | 0x40;
        assert!(TextMessageRequest::decode(&wrong_message).is_err());

        let mut trailing = encoded;
        primitives::encode_app_null(&mut trailing);
        assert!(TextMessageRequest::decode(&trailing).is_err());
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_empty_input() {
        assert!(TextMessageRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_truncated_1_byte() {
        let req = TextMessageRequest {
            source_device: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
            message_class: None,
            message_priority: MessagePriority::NORMAL,
            message: "Test".into(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(TextMessageRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_truncated_half() {
        let req = TextMessageRequest {
            source_device: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
            message_class: Some(MessageClass::Text("info".into())),
            message_priority: MessagePriority::URGENT,
            message: "Emergency".into(),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let half = buf.len() / 2;
        assert!(TextMessageRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_invalid_tag() {
        assert!(TextMessageRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}
