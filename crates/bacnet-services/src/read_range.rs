//! ReadRange service per ASHRAE 135-2020 Clause 15.8.
//!
//! Reads a range of items from a list or log-buffer property.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_encoding::{primitives, tags};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::BytesMut;

use bacnet_encoding::constructed::tagged::{
    decode_app_primitive, decode_app_unsigned, decode_ctx_object_id, decode_ctx_primitive,
    decode_ctx_unsigned, expect_end, misplaced_tag, next_is_context,
};

fn decode_count(data: &[u8], offset: usize, field: &str) -> Result<(i32, usize), Error> {
    let (content, end) = decode_app_primitive(data, offset, tags::app_tag::SIGNED, field)?;
    let value = primitives::decode_signed(content)?;
    let value = i16::try_from(value)
        .map_err(|_| Error::out_of_range(offset, format!("{field} exceeds INTEGER16")))?;
    if value == 0 {
        return Err(Error::out_of_range(
            offset,
            format!("{field} may not be zero"),
        ));
    }
    Ok((i32::from(value), end))
}

fn specific_datetime(date: &Date, time: &Time) -> bool {
    date.year != Date::UNSPECIFIED
        && (1..=12).contains(&date.month)
        && (1..=31).contains(&date.day)
        && (1..=7).contains(&date.day_of_week)
        && time.hour <= 23
        && time.minute <= 59
        && time.second <= 59
        && time.hundredths <= 99
}

/// ReadRange-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRangeRequest {
    /// Object holding the list property to read.
    pub object_identifier: ObjectIdentifier,
    /// List property to read; not ALL, REQUIRED or OPTIONAL.
    pub property_identifier: PropertyIdentifier,
    /// Array index selecting one list within an array-of-lists property; `None` otherwise. Zero is
    /// invalid.
    pub property_array_index: Option<u32>,
    /// Range specification: by-position, by-sequence-number, or by-time.
    pub range: Option<RangeSpec>,
}

/// Range specification for ReadRange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeSpec {
    /// By position: reference_index, count.
    ByPosition {
        /// One-based list index (first item is 1) of the item that anchors the range. Unsigned64
        /// because this stack hosts Audit Log, whose Total_Record_Count is Unsigned64
        /// (Clause 15.8.1.1.4.1.1).
        reference_index: u64,
        /// Signed INTEGER16 item count, never zero. Positive reads forward from the reference
        /// and negative reads backward, ending at the reference.
        count: i32,
    },
    /// By sequence number: reference_seq, count.
    BySequenceNumber {
        /// Sequence number of the item that anchors the range. Unsigned64 for the same reason as
        /// `reference_index` (Clause 15.8.1.1.4.2.1).
        reference_seq: u64,
        /// Signed INTEGER16 item count, never zero. Positive reads forward from the reference
        /// and negative reads backward, ending at the reference.
        count: i32,
    },
    /// By time: reference_time (Date, Time), count.
    ByTime {
        /// Timestamp that anchors the range; must be fully specified.
        reference_time: (Date, Time),
        /// Signed INTEGER16 item count, never zero. The reference time itself is excluded:
        /// positive reads forward from the first item newer than it, negative reads backward
        /// ending at the newest item older than it (Clause 15.8.1.1.4.3).
        count: i32,
    },
}

impl ReadRangeRequest {
    /// Validate typed arguments without output, leases or transport side effects.
    pub fn validate(&self) -> Result<(), Error> {
        if matches!(
            self.property_identifier,
            PropertyIdentifier::ALL | PropertyIdentifier::REQUIRED | PropertyIdentifier::OPTIONAL
        ) {
            return Err(Error::Encoding(
                "ReadRange property may not be ALL, REQUIRED, or OPTIONAL".into(),
            ));
        }
        if self.property_array_index == Some(0) {
            return Err(Error::Encoding(
                "ReadRange array index may not be zero".into(),
            ));
        }
        if let Some(range) = &self.range {
            let count = match range {
                RangeSpec::ByPosition { count, .. } | RangeSpec::BySequenceNumber { count, .. } => {
                    *count
                }
                RangeSpec::ByTime {
                    reference_time: (date, time),
                    count,
                } => {
                    if !specific_datetime(date, time) {
                        return Err(Error::Encoding(
                            "ReadRange byTime requires a specific datetime".into(),
                        ));
                    }
                    *count
                }
            };
            if count == 0 || i16::try_from(count).is_err() {
                return Err(Error::Encoding(
                    "ReadRange count must be a nonzero INTEGER16".into(),
                ));
            }
        }
        Ok(())
    }

    /// Encode only valid requests; on error, existing output remains unchanged.
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
        // Range specification
        if let Some(ref range) = self.range {
            match range {
                RangeSpec::ByPosition {
                    reference_index,
                    count,
                } => {
                    tags::encode_opening_tag(buf, 3);
                    primitives::encode_app_unsigned(buf, *reference_index);
                    primitives::encode_app_signed(buf, *count);
                    tags::encode_closing_tag(buf, 3);
                }
                RangeSpec::BySequenceNumber {
                    reference_seq,
                    count,
                } => {
                    tags::encode_opening_tag(buf, 6);
                    primitives::encode_app_unsigned(buf, *reference_seq);
                    primitives::encode_app_signed(buf, *count);
                    tags::encode_closing_tag(buf, 6);
                }
                RangeSpec::ByTime {
                    reference_time,
                    count,
                } => {
                    tags::encode_opening_tag(buf, 7);
                    primitives::encode_app_date(buf, &reference_time.0);
                    primitives::encode_app_time(buf, &reference_time.1);
                    primitives::encode_app_signed(buf, *count);
                    tags::encode_closing_tag(buf, 7);
                }
            }
        }
        Ok(())
    }

    /// Decode the request from service-request octets; fails on malformed or truncated input.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] objectIdentifier
        let (object_identifier, end) =
            decode_ctx_object_id(data, offset, 0, "ReadRange request object-id")?;
        offset = end;

        // [1] propertyIdentifier
        let (property_identifier, end) =
            decode_ctx_unsigned::<u32>(data, offset, 1, "ReadRange request property-id")?;
        let property_identifier = PropertyIdentifier::from_raw(property_identifier);
        if matches!(
            property_identifier,
            PropertyIdentifier::ALL | PropertyIdentifier::REQUIRED | PropertyIdentifier::OPTIONAL
        ) {
            return Err(Error::decoding(
                offset,
                "ReadRange request property-id may not be ALL, REQUIRED, or OPTIONAL",
            ));
        }
        offset = end;

        // [2] propertyArrayIndex (optional)
        let mut property_array_index = None;
        if next_is_context(data, offset, 2)? {
            let (index, end) =
                decode_ctx_unsigned::<u32>(data, offset, 2, "ReadRange request array-index")?;
            if index == 0 {
                return Err(Error::out_of_range(
                    offset,
                    "ReadRange request array-index may not be zero",
                ));
            }
            property_array_index = Some(index);
            offset = end;
        }

        // Range specification (optional)
        let mut range = None;
        if offset < data.len() {
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_opening_tag(3) {
                // byPosition
                let (content, new_offset) = tags::extract_context_value(data, tag_end, 3)?;
                let (reference_index, inner_offset) =
                    decode_app_unsigned::<u64>(content, 0, "ReadRange byPosition reference-index")?;
                let (count, inner_offset) =
                    decode_count(content, inner_offset, "ReadRange byPosition count")?;
                expect_end(content, inner_offset, inner_offset, "ReadRange byPosition")?;
                range = Some(RangeSpec::ByPosition {
                    reference_index,
                    count,
                });
                offset = new_offset;
            } else if tag.is_opening_tag(6) {
                // bySequenceNumber
                let (content, new_offset) = tags::extract_context_value(data, tag_end, 6)?;
                let (reference_seq, inner_offset) = decode_app_unsigned::<u64>(
                    content,
                    0,
                    "ReadRange bySequenceNumber reference-seq",
                )?;
                let (count, inner_offset) =
                    decode_count(content, inner_offset, "ReadRange bySequenceNumber count")?;
                expect_end(
                    content,
                    inner_offset,
                    inner_offset,
                    "ReadRange bySequenceNumber",
                )?;
                range = Some(RangeSpec::BySequenceNumber {
                    reference_seq,
                    count,
                });
                offset = new_offset;
            } else if tag.is_opening_tag(7) {
                // byTime
                let (content, new_offset) = tags::extract_context_value(data, tag_end, 7)?;
                let (date, inner_offset) =
                    decode_app_primitive(content, 0, tags::app_tag::DATE, "ReadRange byTime date")?;
                let date = Date::decode(date)?;
                let (time, inner_offset) = decode_app_primitive(
                    content,
                    inner_offset,
                    tags::app_tag::TIME,
                    "ReadRange byTime time",
                )?;
                let time = Time::decode(time)?;
                if !specific_datetime(&date, &time) {
                    return Err(Error::decoding(
                        inner_offset,
                        "ReadRange byTime requires a specific datetime",
                    ));
                }
                let (count, inner_offset) =
                    decode_count(content, inner_offset, "ReadRange byTime count")?;
                expect_end(content, inner_offset, inner_offset, "ReadRange byTime")?;
                range = Some(RangeSpec::ByTime {
                    reference_time: (date, time),
                    count,
                });
                offset = new_offset;
            } else {
                return Err(misplaced_tag(
                    data,
                    &tag,
                    None,
                    offset,
                    "ReadRange request has invalid range choice",
                ));
            }
        }
        expect_end(data, offset, offset, "ReadRange request")?;

        Ok(Self {
            object_identifier,
            property_identifier,
            property_array_index,
            range,
        })
    }
}

/// ReadRange-ACK service parameters.
#[derive(Debug, Clone)]
pub struct ReadRangeAck {
    /// Object that was read.
    pub object_identifier: ObjectIdentifier,
    /// Property that was read.
    pub property_identifier: PropertyIdentifier,
    /// Array index that was requested, echoed from the request; `None` when absent.
    pub property_array_index: Option<u32>,
    /// Result flags: first_item, last_item, more_items.
    pub result_flags: (bool, bool, bool),
    /// Number of items encoded in `item_data`.
    pub item_count: u32,
    /// Raw item data (application-layer interprets content).
    pub item_data: Vec<u8>,
    /// Optional first sequence number (context tag \[6\]). Unsigned64, since a device holding
    /// an Audit Log must accept and report its 64-bit sequence numbers (Clause 15.8.1.2.7).
    pub first_sequence_number: Option<u64>,
}

impl ReadRangeAck {
    /// Encode the acknowledgment parameters into `buf`.
    pub fn encode(&self, buf: &mut BytesMut) {
        // [0] objectIdentifier
        primitives::encode_ctx_object_id(buf, 0, &self.object_identifier);
        // [1] propertyIdentifier
        primitives::encode_ctx_enumerated(buf, 1, self.property_identifier.to_raw());
        // [2] propertyArrayIndex (optional)
        if let Some(idx) = self.property_array_index {
            primitives::encode_ctx_unsigned(buf, 2, idx as u64);
        }
        // [3] resultFlags — 3-bit bitstring
        let mut flags: u8 = 0;
        if self.result_flags.0 {
            flags |= 0x80;
        }
        if self.result_flags.1 {
            flags |= 0x40;
        }
        if self.result_flags.2 {
            flags |= 0x20;
        }
        primitives::encode_ctx_bit_string(buf, 3, 5, &[flags]);
        // [4] itemCount
        primitives::encode_ctx_unsigned(buf, 4, self.item_count as u64);
        // [5] itemData
        tags::encode_opening_tag(buf, 5);
        buf.extend_from_slice(&self.item_data);
        tags::encode_closing_tag(buf, 5);
        // [6] firstSequenceNumber (optional)
        if let Some(seq) = self.first_sequence_number {
            primitives::encode_ctx_unsigned(buf, 6, seq);
        }
    }

    /// Decode the acknowledgment from its service-ack octets; fails on malformed or truncated
    /// input.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] objectIdentifier
        let (object_identifier, end) =
            decode_ctx_object_id(data, offset, 0, "ReadRange ACK object-id")?;
        offset = end;

        // [1] propertyIdentifier
        let (property_identifier, end) =
            decode_ctx_unsigned::<u32>(data, offset, 1, "ReadRange ACK property-id")?;
        let property_identifier = PropertyIdentifier::from_raw(property_identifier);
        offset = end;

        // [2] propertyArrayIndex (optional)
        let mut property_array_index = None;
        if next_is_context(data, offset, 2)? {
            let (index, end) =
                decode_ctx_unsigned::<u32>(data, offset, 2, "ReadRange ACK array-index")?;
            property_array_index = Some(index);
            offset = end;
        }

        // [3] resultFlags
        let (content, end) = decode_ctx_primitive(data, offset, 3, "ReadRange ACK result-flags")?;
        let (unused_bits, bits) = primitives::decode_bit_string(content)?;
        if unused_bits != 5 || bits.len() != 1 || bits[0] & 0x1f != 0 {
            return Err(Error::decoding(
                offset,
                "ReadRange ACK result-flags must contain three bits with zero padding",
            ));
        }
        let b = bits[0];
        let result_flags = (b & 0x80 != 0, b & 0x40 != 0, b & 0x20 != 0);
        offset = end;

        // [4] itemCount
        let (item_count, end) =
            decode_ctx_unsigned::<u32>(data, offset, 4, "ReadRange ACK item-count")?;
        offset = end;

        // [5] itemData
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(5) {
            return Err(misplaced_tag(
                data,
                &tag,
                Some(5),
                offset,
                "ReadRange ACK item-data expected opening tag 5",
            ));
        }
        let (content, new_offset) = tags::extract_context_value(data, tag_end, 5)?;
        if (item_count == 0) != content.is_empty() {
            return Err(Error::decoding(
                offset,
                "ReadRange ACK item-count contradicts empty item-data",
            ));
        }
        let item_data = content.to_vec();
        offset = new_offset;

        // [6] firstSequenceNumber (optional)
        let mut first_sequence_number = None;
        if offset < data.len() {
            let (tag, _) = tags::decode_tag(data, offset)?;
            if !tag.is_context(6) {
                return Err(misplaced_tag(
                    data,
                    &tag,
                    Some(6),
                    offset,
                    "ReadRange ACK expected context tag 6 for first-sequence-number",
                ));
            }
            if item_count == 0 {
                return Err(Error::decoding(
                    offset,
                    "ReadRange ACK first-sequence-number requires a nonzero item-count",
                ));
            }
            let (sequence_number, end) =
                decode_ctx_unsigned::<u64>(data, offset, 6, "ReadRange ACK first-sequence-number")?;
            first_sequence_number = Some(sequence_number);
            offset = end;
        }
        expect_end(data, offset, offset, "ReadRange ACK")?;

        Ok(Self {
            object_identifier,
            property_identifier,
            property_array_index,
            result_flags,
            item_count,
            item_data,
            first_sequence_number,
        })
    }
}

#[path = "read_range_records.rs"]
mod records;
pub use records::{LogBufferRecord, LogRecords, LogRecordsError};
#[path = "read_range_checks.rs"]
mod checks;
pub use bacnet_types::error::ReadRangeViolation;
pub use checks::{ReadRangeReply, ReadRangeValidation};

#[cfg(test)]
#[path = "read_range_width_tests.rs"]
mod width_tests;

#[cfg(test)]
#[path = "read_range_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "read_range_validation_tests.rs"]
mod validation_tests;
