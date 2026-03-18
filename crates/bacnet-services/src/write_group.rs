//! WriteGroup service per ASHRAE 135-2020 Clause 16.10.8.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;

// ---------------------------------------------------------------------------
// WriteGroupRequest
// ---------------------------------------------------------------------------

/// A single entry in the WriteGroup change list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupChannelValue {
    /// [0] channel OPTIONAL
    pub channel: Option<ObjectIdentifier>,
    /// [1] overridePriority OPTIONAL
    pub override_priority: Option<u8>,
    /// [2] value — raw application-tagged bytes
    pub value: Vec<u8>,
}

/// WriteGroup-Request service parameters.
///
/// ```text
/// WriteGroupRequest ::= SEQUENCE {
///     groupNumber    [0] Unsigned32,
///     writePriority  [1] Unsigned (1-16),
///     changeList     [2] SEQUENCE OF { ... },
///     inhibitDelay   [3] BOOLEAN OPTIONAL
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteGroupRequest {
    pub group_number: u32,
    pub write_priority: u8,
    pub change_list: Vec<GroupChannelValue>,
    pub inhibit_delay: Option<bool>,
}

impl WriteGroupRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        // [0] groupNumber
        primitives::encode_ctx_unsigned(buf, 0, self.group_number as u64);
        // [1] writePriority
        primitives::encode_ctx_unsigned(buf, 1, self.write_priority as u64);
        // [2] changeList
        tags::encode_opening_tag(buf, 2);
        for entry in &self.change_list {
            // [0] channel OPTIONAL
            if let Some(ref ch) = entry.channel {
                primitives::encode_ctx_object_id(buf, 0, ch);
            }
            // [1] overridePriority OPTIONAL
            if let Some(prio) = entry.override_priority {
                primitives::encode_ctx_unsigned(buf, 1, prio as u64);
            }
            // [2] value (opening/closing)
            tags::encode_opening_tag(buf, 2);
            buf.extend_from_slice(&entry.value);
            tags::encode_closing_tag(buf, 2);
        }
        tags::encode_closing_tag(buf, 2);
        // [3] inhibitDelay OPTIONAL
        if let Some(v) = self.inhibit_delay {
            primitives::encode_ctx_boolean(buf, 3, v);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] groupNumber
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "WriteGroup truncated at group-number"));
        }
        let group_number = primitives::decode_unsigned(&data[pos..end])? as u32;
        if group_number == 0 {
            return Err(Error::Encoding(
                "WriteGroup group number 0 is reserved".into(),
            ));
        }
        offset = end;

        // [1] writePriority
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "WriteGroup truncated at write-priority",
            ));
        }
        let write_priority = primitives::decode_unsigned(&data[pos..end])? as u8;
        if !(1..=16).contains(&write_priority) {
            return Err(Error::decoding(
                pos,
                format!("WriteGroup write-priority {write_priority} out of range 1-16"),
            ));
        }
        offset = end;

        // [2] changeList — opening tag 2
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(2) {
            return Err(Error::decoding(offset, "WriteGroup expected opening tag 2"));
        }
        offset = tag_end;

        let mut change_list = Vec::new();
        loop {
            if offset >= data.len() {
                return Err(Error::decoding(offset, "WriteGroup missing closing tag 2"));
            }
            if change_list.len() >= MAX_DECODED_ITEMS {
                return Err(Error::decoding(offset, "WriteGroup change list too large"));
            }
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_closing_tag(2) {
                offset = tag_end;
                break;
            }

            // [0] channel OPTIONAL — peek for context 0
            let mut channel = None;
            if tag.is_context(0) {
                let end = tag_end + tag.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(tag_end, "WriteGroup truncated at channel"));
                }
                channel = Some(ObjectIdentifier::decode(&data[tag_end..end])?);
                offset = end;
            } else {
                offset = tag_end - (tag_end - offset); // stay at current position
            }

            // [1] overridePriority OPTIONAL
            let mut override_priority = None;
            if offset < data.len() {
                let (opt, new_off) = tags::decode_optional_context(data, offset, 1)?;
                if let Some(content) = opt {
                    override_priority = Some(primitives::decode_unsigned(content)? as u8);
                    offset = new_off;
                }
            }

            // [2] value (opening/closing tag 2 — inner)
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(2) {
                return Err(Error::decoding(
                    offset,
                    "WriteGroup expected opening tag 2 for value",
                ));
            }
            let (value_bytes, new_off) = tags::extract_context_value(data, tag_end, 2)?;
            let value = value_bytes.to_vec();
            offset = new_off;

            change_list.push(GroupChannelValue {
                channel,
                override_priority,
                value,
            });
        }

        // [3] inhibitDelay OPTIONAL
        let mut inhibit_delay = None;
        if offset < data.len() {
            let (opt, new_off) = tags::decode_optional_context(data, offset, 3)?;
            if let Some(content) = opt {
                inhibit_delay = Some(!content.is_empty() && content[0] != 0);
                offset = new_off;
            }
        }
        let _ = offset;

        Ok(Self {
            group_number,
            write_priority,
            change_list,
            inhibit_delay,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    #[test]
    fn write_group_round_trip() {
        let req = WriteGroupRequest {
            group_number: 1,
            write_priority: 8,
            change_list: vec![
                GroupChannelValue {
                    channel: Some(ObjectIdentifier::new(ObjectType::CHANNEL, 1).unwrap()),
                    override_priority: Some(10),
                    value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                },
                GroupChannelValue {
                    channel: None,
                    override_priority: None,
                    value: vec![0x91, 0x01],
                },
            ],
            inhibit_delay: Some(true),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = WriteGroupRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn write_group_minimal() {
        let req = WriteGroupRequest {
            group_number: 100,
            write_priority: 16,
            change_list: vec![GroupChannelValue {
                channel: None,
                override_priority: None,
                value: vec![0x10],
            }],
            inhibit_delay: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = WriteGroupRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn write_group_priority_validation() {
        // Encode with valid priority, then corrupt it
        let req = WriteGroupRequest {
            group_number: 1,
            write_priority: 8,
            change_list: vec![GroupChannelValue {
                channel: None,
                override_priority: None,
                value: vec![0x10],
            }],
            inhibit_delay: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let mut data = buf.to_vec();
        // The write_priority byte is after the group number encoding.
        // group_number=1: ctx tag 0 (09 01), then write_priority=8: ctx tag 1 (19 08)
        // Find and change the priority value to 0
        for i in 0..data.len() - 1 {
            if data[i] == 0x19 && data[i + 1] == 0x08 {
                data[i + 1] = 0x00; // set to 0 (invalid)
                break;
            }
        }
        assert!(WriteGroupRequest::decode(&data).is_err());
    }

    #[test]
    fn write_group_empty_input() {
        assert!(WriteGroupRequest::decode(&[]).is_err());
    }
}
