//! ConfirmedTextMessage / UnconfirmedTextMessage services
//! per ASHRAE 135-2020 Clauses 16.5 and 16.6.

#[cfg(not(feature = "std"))]
use alloc::string::String;

use bacnet_encoding::constructed::tagged::{
    decode_ctx_character_string, decode_ctx_constructed, decode_ctx_object_id, decode_ctx_unsigned,
    expect_end, misplaced_kind, next_is_context, next_is_opening,
};
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
    /// Numeric class code chosen by the sender.
    Numeric(u32),
    /// Free-form class name chosen by the sender.
    Text(String),
}

// ---------------------------------------------------------------------------
// TextMessageRequest
// ---------------------------------------------------------------------------

/// Request parameters shared by ConfirmedTextMessage and
/// UnconfirmedTextMessage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMessageRequest {
    /// Device object of the sending device.
    pub source_device: ObjectIdentifier,
    /// Optional classification of the message; `None` when the sender did not supply one.
    pub message_class: Option<MessageClass>,
    /// Urgency of the message (normal or urgent).
    pub message_priority: MessagePriority,
    /// Message text shown to the recipient.
    pub message: String,
}

impl TextMessageRequest {
    /// Encode the request parameters into `buf`; fails if a character string cannot be encoded.
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

    /// Decode the request from service-request octets; fails on malformed or truncated input.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        // [0] textMessageSourceDevice
        let (source_device, mut offset) =
            decode_ctx_object_id(data, 0, 0, "TextMessage sourceDevice")?;

        // messageClass [1] CHOICE { numeric [0], character [1] } OPTIONAL
        let mut message_class = None;
        if next_is_opening(data, offset, 1)? {
            let what = "TextMessage messageClass";
            let (content, new_offset) = decode_ctx_constructed(data, offset, 1, what)?;
            let (class, end) = if next_is_context(content, 0, 0)? {
                let (n, end) = decode_ctx_unsigned::<u32>(content, 0, 0, what)?;
                (MessageClass::Numeric(n), end)
            } else if next_is_context(content, 0, 1)? {
                let (text, end) = decode_ctx_character_string(content, 0, 1, what)?;
                (MessageClass::Text(text), end)
            } else {
                // Neither alternative: the frame is empty, or another tag
                // stands there.
                let (found, _) = tags::decode_tag(content, 0)?;
                return Err(Error::decoding_kind(
                    misplaced_kind(content, 0, &found, None),
                    offset,
                    "TextMessage messageClass expected context tag 0 or 1",
                ));
            };
            expect_end(content, end, offset, what)?;
            message_class = Some(class);
            offset = new_offset;
        }

        // [2] messagePriority (per Clause 16.5/16.6 ASN.1)
        let (priority, offset) =
            decode_ctx_unsigned::<u32>(data, offset, 2, "TextMessage messagePriority")?;
        let message_priority = MessagePriority::from_raw(priority);

        // [3] message, and nothing after it
        let (message, end) = decode_ctx_character_string(data, offset, 3, "TextMessage message")?;
        expect_end(data, end, end, "TextMessage")?;

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
