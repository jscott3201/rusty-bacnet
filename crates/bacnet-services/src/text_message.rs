//! ConfirmedTextMessage / UnconfirmedTextMessage services
//! per ASHRAE 135-2020 Clauses 15.20 and 16.10.7.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::MessagePriority;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

// ---------------------------------------------------------------------------
// MessageClass choice
// ---------------------------------------------------------------------------

/// The messageClass CHOICE: numeric ([1] Unsigned) or text ([2] CharacterString).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageClass {
    Numeric(u32),
    Text(String),
}

// ---------------------------------------------------------------------------
// TextMessageRequest (Clause 15.20.1 / 16.10.7)
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
        // messageClass CHOICE (optional): [1] Unsigned or [2] CharacterString
        if let Some(ref mc) = self.message_class {
            match mc {
                MessageClass::Numeric(n) => {
                    primitives::encode_ctx_unsigned(buf, 1, *n as u64);
                }
                MessageClass::Text(s) => {
                    primitives::encode_ctx_character_string(buf, 2, s)?;
                }
            }
        }
        // [3] messagePriority
        primitives::encode_ctx_enumerated(buf, 3, self.message_priority.to_raw());
        // [4] message
        primitives::encode_ctx_character_string(buf, 4, &self.message)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] textMessageSourceDevice
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "TextMessage truncated at sourceDevice",
            ));
        }
        let source_device = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // messageClass CHOICE (optional): [1] Unsigned or [2] CharacterString
        let mut message_class = None;
        if offset < data.len() {
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if tag.is_context(1) {
                let end = pos + tag.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        pos,
                        "TextMessage truncated at messageClass numeric",
                    ));
                }
                message_class = Some(MessageClass::Numeric(primitives::decode_unsigned(
                    &data[pos..end],
                )? as u32));
                offset = end;
            } else if tag.is_context(2) {
                let end = pos + tag.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        pos,
                        "TextMessage truncated at messageClass text",
                    ));
                }
                let s = primitives::decode_character_string(&data[pos..end])?;
                message_class = Some(MessageClass::Text(s));
                offset = end;
            }
            // else: not tag 1 or 2 — no messageClass, don't advance offset
        }

        // [3] messagePriority
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "TextMessage truncated at messagePriority",
            ));
        }
        let message_priority =
            MessagePriority::from_raw(primitives::decode_unsigned(&data[pos..end])? as u32);
        offset = end;

        // [4] message
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "TextMessage truncated at message"));
        }
        let message = primitives::decode_character_string(&data[pos..end])?;

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
