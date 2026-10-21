//! WriteGroup service per ASHRAE 135-2020 Clause 15.11.
//!
//! The wire form follows the WriteGroup-Request production in Clause 21.3.2 and the
//! BACnetGroupChannelValue / BACnetChannelValue productions in Clause 21.6.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use core::num::NonZeroU32;

use bacnet_encoding::constructed::channel_value_end;
use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::error::{DecodeError, DecodingKind, EncodeError, Error};
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;
use bacnet_encoding::constructed::tagged::{
    decode_ctx_boolean, decode_ctx_primitive, next_is_context,
};

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// Range of a write priority or overriding priority (Clauses 15.11.1.1.2 and 21.6).
const PRIORITY_RANGE: core::ops::RangeInclusive<u64> = 1..=16;

/// `label` names the priority, with a placeholder for its value.
fn check_priority(label: &'static str, value: u8) -> Result<u8, Error> {
    if PRIORITY_RANGE.contains(&u64::from(value)) {
        Ok(value)
    } else {
        Err(EncodeError::new(label).with_value(value.into()).into())
    }
}

/// `label` names the priority, with a placeholder for its value.
fn decode_priority(label: &'static str, offset: usize, value: u64) -> Result<u8, Error> {
    if PRIORITY_RANGE.contains(&value) {
        Ok(value as u8)
    } else {
        Err(DecodeError::new(DecodingKind::InvalidEncoding, offset, label)
            .with_value(value)
            .into())
    }
}

// ---------------------------------------------------------------------------
// WriteGroupRequest
// ---------------------------------------------------------------------------

/// A single entry in the WriteGroup change list (BACnetGroupChannelValue, Clause 21.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupChannelValue {
    /// \[0\] channel number, matched against a Channel object's Channel_Number.
    pub channel: u16,
    /// \[1\] overriding-priority OPTIONAL: priority 1-16 replacing the request's write priority
    /// for this entry.
    pub override_priority: Option<u8>,
    /// The BACnetChannelValue, already encoded and carried without a wrapper tag: one
    /// well-formed application-tagged primitive, or a constructed context-\[0\] lighting
    /// command, context-\[1\] xy colour or context-\[2\] colour command whose fields are
    /// in order with valid lengths. [`WriteGroupRequest::encode`] rejects anything else.
    pub value: Vec<u8>,
}

/// WriteGroup-Request service parameters (Clause 15.11.1; encoding in Clauses 21.3.2 and 21.6).
///
/// Fields, in order:
/// - group number, context \[0\], mandatory (Unsigned32);
/// - write priority, context \[1\], mandatory (1-16);
/// - change list, context \[2\] constructed, mandatory, holding one or more
///   BACnetGroupChannelValue entries;
/// - inhibit delay, context \[3\], optional (Boolean).
///
/// Each change-list entry is a channel number in context \[0\] (Unsigned16), an optional
/// overriding priority in context \[1\] (1-16), and then the channel value itself with no
/// wrapper tag (an untagged CHOICE).
///
/// For channel 5 holding REAL 72.0 the change-list entry is `09 05 44 42 90 00 00`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteGroupRequest {
    /// Control group to write, matched against each Channel object's Control_Groups; group 0 is
    /// reserved, so it cannot be represented.
    pub group_number: NonZeroU32,
    /// Priority (1-16) used for writes unless an entry overrides it; [`encode`](Self::encode)
    /// rejects values outside that range.
    pub write_priority: u8,
    /// Channel values to apply, each addressed by channel number; must not be empty.
    pub change_list: Vec<GroupChannelValue>,
    /// When true, Channel objects that allow it skip their configured execution delay; `None` or
    /// false leaves delays in force.
    pub inhibit_delay: Option<bool>,
}

impl WriteGroupRequest {
    /// Encode the request parameters into `buf`.
    ///
    /// Fails, leaving `buf` untouched, if the write priority or an overriding priority is outside
    /// 1-16, the change list is empty, or an entry's value is not a single well-formed
    /// BACnetChannelValue.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        check_priority(
            "WriteGroup write-priority {} out of range 1-16",
            self.write_priority,
        )?;
        if self.change_list.is_empty() {
            return Err(Error::encode("WriteGroup change list must contain at least one entry"));
        }
        for entry in &self.change_list {
            if let Some(priority) = entry.override_priority {
                check_priority(
                    "WriteGroup override-priority {} out of range 1-16",
                    priority,
                )?;
            }
            let well_formed =
                channel_value_end(&entry.value, 0).is_ok_and(|end| end == entry.value.len());
            if !well_formed {
                return Err(EncodeError::new(
                    "WriteGroup value for channel {} is not a single BACnetChannelValue",
                )
                .with_value(entry.channel.into())
                .into());
            }
        }

        primitives::encode_ctx_unsigned(buf, 0, u64::from(self.group_number.get()));
        primitives::encode_ctx_unsigned(buf, 1, u64::from(self.write_priority));
        tags::encode_opening_tag(buf, 2);
        for entry in &self.change_list {
            primitives::encode_ctx_unsigned(buf, 0, u64::from(entry.channel));
            if let Some(priority) = entry.override_priority {
                primitives::encode_ctx_unsigned(buf, 1, u64::from(priority));
            }
            buf.extend_from_slice(&entry.value);
        }
        tags::encode_closing_tag(buf, 2);
        if let Some(inhibit) = self.inhibit_delay {
            primitives::encode_ctx_boolean(buf, 3, inhibit);
        }
        Ok(())
    }

    /// Decode the request from service-request octets.
    ///
    /// Fails on malformed, truncated or out-of-range input and on trailing data.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        // [0] group-number
        let (content, mut offset) = decode_ctx_primitive(data, 0, 0, "WriteGroup group-number")?;
        let group_raw = primitives::decode_unsigned(content)?;
        let group_number = u32::try_from(group_raw)
            .ok()
            .and_then(NonZeroU32::new)
            .ok_or_else(|| {
                DecodeError::new(
                    DecodingKind::OutOfRange,
                    0,
                    "WriteGroup group number {} out of range 1-4294967295",
                )
                .with_value(group_raw)
            })?;

        // [1] write-priority
        let priority_pos = offset;
        let (content, end) = decode_ctx_primitive(data, offset, 1, "WriteGroup write-priority")?;
        let write_priority = decode_priority(
            "WriteGroup write-priority {} out of range 1-16",
            priority_pos,
            primitives::decode_unsigned(content)?,
        )?;
        offset = end;

        // [2] change-list
        if offset >= data.len() {
            return Err(Error::decoding(offset, "WriteGroup missing change list"));
        }
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
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_closing_tag(2) {
                offset = tag_end;
                break;
            }
            if change_list.len() >= MAX_DECODED_ITEMS {
                return Err(Error::decoding(offset, "WriteGroup change list too large"));
            }

            // [0] channel
            let (content, end) = decode_ctx_primitive(data, offset, 0, "WriteGroup channel")?;
            let channel_raw = primitives::decode_unsigned(content)?;
            let channel = u16::try_from(channel_raw).map_err(|_| {
                DecodeError::new(
                    DecodingKind::OutOfRange,
                    offset,
                    "WriteGroup channel {} exceeds 65535",
                )
                .with_value(channel_raw)
            })?;
            offset = end;

            // [1] overriding-priority OPTIONAL
            let mut override_priority = None;
            if next_is_context(data, offset, 1)? {
                let (content, end) =
                    decode_ctx_primitive(data, offset, 1, "WriteGroup override-priority")?;
                override_priority = Some(decode_priority(
                    "WriteGroup override-priority {} out of range 1-16",
                    offset,
                    primitives::decode_unsigned(content)?,
                )?);
                offset = end;
            }

            // BACnetChannelValue (untagged CHOICE)
            let end = channel_value_end(data, offset)?;
            change_list.push(GroupChannelValue {
                channel,
                override_priority,
                value: data[offset..end].to_vec(),
            });
            offset = end;
        }
        if change_list.is_empty() {
            return Err(Error::decoding(
                offset,
                "WriteGroup change list must contain at least one entry",
            ));
        }

        // [3] inhibit-delay OPTIONAL
        let mut inhibit_delay = None;
        if next_is_context(data, offset, 3)? {
            let (inhibit, end) = decode_ctx_boolean(data, offset, 3, "WriteGroup inhibit-delay")?;
            inhibit_delay = Some(inhibit);
            offset = end;
        }
        if offset != data.len() {
            return Err(Error::decoding(offset, "WriteGroup has trailing data"));
        }

        Ok(Self {
            group_number,
            write_priority,
            change_list,
            inhibit_delay,
        })
    }
}

#[cfg(test)]
#[path = "write_group_tests.rs"]
mod tests;
