//! ReadRange service per ASHRAE 135-2020 Clause 15.8.
//!
//! Reads a range of items from a list or log-buffer property.

use bacnet_encoding::{primitives, tags};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::BytesMut;

/// Decode a tag from content and validate the slice bounds.
fn checked_slice<'a>(
    content: &'a [u8],
    offset: usize,
    context: &str,
) -> Result<(&'a [u8], usize), Error> {
    let (t, p) = tags::decode_tag(content, offset)?;
    let end = p + t.length as usize;
    if end > content.len() {
        return Err(Error::decoding(p, format!("{context} truncated")));
    }
    Ok((&content[p..end], end))
}

/// ReadRange-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRangeRequest {
    pub object_identifier: ObjectIdentifier,
    pub property_identifier: PropertyIdentifier,
    pub property_array_index: Option<u32>,
    /// Range specification: by-position, by-sequence-number, or by-time.
    pub range: Option<RangeSpec>,
}

/// Range specification for ReadRange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeSpec {
    /// By position: reference_index, count.
    ByPosition { reference_index: u32, count: i32 },
    /// By sequence number: reference_seq, count.
    BySequenceNumber { reference_seq: u32, count: i32 },
    /// By time: reference_time (Date, Time), count.
    ByTime {
        reference_time: (Date, Time),
        count: i32,
    },
}

impl ReadRangeRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
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
                    primitives::encode_app_unsigned(buf, *reference_index as u64);
                    primitives::encode_app_signed(buf, *count);
                    tags::encode_closing_tag(buf, 3);
                }
                RangeSpec::BySequenceNumber {
                    reference_seq,
                    count,
                } => {
                    tags::encode_opening_tag(buf, 6);
                    primitives::encode_app_unsigned(buf, *reference_seq as u64);
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
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] objectIdentifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "ReadRange request truncated at object-id",
            ));
        }
        let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [1] propertyIdentifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "ReadRange request truncated at property-id",
            ));
        }
        let property_identifier =
            PropertyIdentifier::from_raw(primitives::decode_unsigned(&data[pos..end])? as u32);
        offset = end;

        // [2] propertyArrayIndex (optional)
        let mut property_array_index = None;
        if offset < data.len() {
            let (opt_data, new_offset) = tags::decode_optional_context(data, offset, 2)?;
            if let Some(content) = opt_data {
                property_array_index = Some(primitives::decode_unsigned(content)? as u32);
                offset = new_offset;
            }
        }

        // Range specification (optional)
        let mut range = None;
        if offset < data.len() {
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if tag.is_opening_tag(3) {
                // byPosition
                let (content, new_offset) = tags::extract_context_value(data, tag_end, 3)?;
                let (slice, inner_offset) =
                    checked_slice(content, 0, "ReadRange byPosition reference-index")?;
                let reference_index = primitives::decode_unsigned(slice)? as u32;
                let (slice, _) =
                    checked_slice(content, inner_offset, "ReadRange byPosition count")?;
                let count = primitives::decode_signed(slice)?;
                range = Some(RangeSpec::ByPosition {
                    reference_index,
                    count,
                });
                offset = new_offset;
            } else if tag.is_opening_tag(6) {
                // bySequenceNumber
                let (content, new_offset) = tags::extract_context_value(data, tag_end, 6)?;
                let (slice, inner_offset) =
                    checked_slice(content, 0, "ReadRange bySequenceNumber reference-seq")?;
                let reference_seq = primitives::decode_unsigned(slice)? as u32;
                let (slice, _) =
                    checked_slice(content, inner_offset, "ReadRange bySequenceNumber count")?;
                let count = primitives::decode_signed(slice)?;
                range = Some(RangeSpec::BySequenceNumber {
                    reference_seq,
                    count,
                });
                offset = new_offset;
            } else if tag.is_opening_tag(7) {
                // byTime
                let (content, new_offset) = tags::extract_context_value(data, tag_end, 7)?;
                let (slice, inner_offset) = checked_slice(content, 0, "ReadRange byTime date")?;
                let date = Date::decode(slice)?;
                let (slice, inner_offset) =
                    checked_slice(content, inner_offset, "ReadRange byTime time")?;
                let time = Time::decode(slice)?;
                let (slice, _) = checked_slice(content, inner_offset, "ReadRange byTime count")?;
                let count = primitives::decode_signed(slice)?;
                range = Some(RangeSpec::ByTime {
                    reference_time: (date, time),
                    count,
                });
                offset = new_offset;
            }
        }
        let _ = offset;

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
    pub object_identifier: ObjectIdentifier,
    pub property_identifier: PropertyIdentifier,
    pub property_array_index: Option<u32>,
    /// Result flags: first_item, last_item, more_items.
    pub result_flags: (bool, bool, bool),
    pub item_count: u32,
    /// Raw item data (application-layer interprets content).
    pub item_data: Vec<u8>,
    /// Optional first sequence number (context tag [6]).
    pub first_sequence_number: Option<u32>,
}

impl ReadRangeAck {
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
            primitives::encode_ctx_unsigned(buf, 6, seq as u64);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] objectIdentifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "ReadRange ACK truncated at object-id"));
        }
        let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [1] propertyIdentifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "ReadRange ACK truncated at property-id",
            ));
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

        // [3] resultFlags
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "ReadRange ACK truncated at result-flags",
            ));
        }
        let (_, bits) = primitives::decode_bit_string(&data[pos..end])?;
        let b = bits.first().copied().unwrap_or(0);
        let result_flags = (b & 0x80 != 0, b & 0x40 != 0, b & 0x20 != 0);
        offset = end;

        // [4] itemCount
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "ReadRange ACK truncated at item-count",
            ));
        }
        let item_count = primitives::decode_unsigned(&data[pos..end])? as u32;
        offset = end;

        // [5] itemData
        let (_tag, tag_end) = tags::decode_tag(data, offset)?;
        let (content, _new_offset) = tags::extract_context_value(data, tag_end, 5)?;
        let item_data = content.to_vec();
        let mut new_offset = _new_offset;

        // [6] firstSequenceNumber (optional)
        let mut first_sequence_number = None;
        if new_offset < data.len() {
            let (opt_data, after) = tags::decode_optional_context(data, new_offset, 6)?;
            if let Some(content) = opt_data {
                first_sequence_number = Some(primitives::decode_unsigned(content)? as u32);
                new_offset = after;
            }
        }
        let _ = new_offset;

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

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::{Date, Time};

    fn make_oid() -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap()
    }

    #[test]
    fn request_round_trip() {
        let req = ReadRangeRequest {
            object_identifier: make_oid(),
            property_identifier: PropertyIdentifier::LOG_BUFFER,
            property_array_index: None,
            range: Some(RangeSpec::ByPosition {
                reference_index: 1,
                count: 10,
            }),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = ReadRangeRequest::decode(&buf).unwrap();
        assert_eq!(decoded.object_identifier, req.object_identifier);
        assert_eq!(decoded.property_identifier, req.property_identifier);
        assert_eq!(decoded.range, req.range);
    }

    #[test]
    fn request_no_range() {
        let req = ReadRangeRequest {
            object_identifier: make_oid(),
            property_identifier: PropertyIdentifier::LOG_BUFFER,
            property_array_index: None,
            range: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = ReadRangeRequest::decode(&buf).unwrap();
        assert!(decoded.range.is_none());
    }

    #[test]
    fn request_by_sequence_number() {
        let req = ReadRangeRequest {
            object_identifier: make_oid(),
            property_identifier: PropertyIdentifier::LOG_BUFFER,
            property_array_index: None,
            range: Some(RangeSpec::BySequenceNumber {
                reference_seq: 100,
                count: -5,
            }),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = ReadRangeRequest::decode(&buf).unwrap();
        assert_eq!(decoded.range, req.range);
    }

    #[test]
    fn ack_round_trip() {
        let ack = ReadRangeAck {
            object_identifier: make_oid(),
            property_identifier: PropertyIdentifier::LOG_BUFFER,
            property_array_index: None,
            result_flags: (true, false, true),
            item_count: 2,
            item_data: vec![0xAA, 0xBB, 0xCC],
            first_sequence_number: None,
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = ReadRangeAck::decode(&buf).unwrap();
        assert_eq!(decoded.object_identifier, ack.object_identifier);
        assert_eq!(decoded.result_flags, (true, false, true));
        assert_eq!(decoded.item_count, 2);
        assert_eq!(decoded.item_data, vec![0xAA, 0xBB, 0xCC]);
        assert_eq!(decoded.first_sequence_number, None);
    }

    #[test]
    fn ack_round_trip_with_first_sequence_number() {
        let ack = ReadRangeAck {
            object_identifier: make_oid(),
            property_identifier: PropertyIdentifier::LOG_BUFFER,
            property_array_index: None,
            result_flags: (true, true, false),
            item_count: 5,
            item_data: vec![0x01, 0x02],
            first_sequence_number: Some(42),
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = ReadRangeAck::decode(&buf).unwrap();
        assert_eq!(decoded.object_identifier, ack.object_identifier);
        assert_eq!(decoded.result_flags, (true, true, false));
        assert_eq!(decoded.item_count, 5);
        assert_eq!(decoded.item_data, vec![0x01, 0x02]);
        assert_eq!(decoded.first_sequence_number, Some(42));
    }

    #[test]
    fn request_by_time() {
        let req = ReadRangeRequest {
            object_identifier: make_oid(),
            property_identifier: PropertyIdentifier::LOG_BUFFER,
            property_array_index: None,
            range: Some(RangeSpec::ByTime {
                reference_time: (
                    Date {
                        year: 126, // 2026
                        month: 3,
                        day: 1,
                        day_of_week: 7, // Sunday
                    },
                    Time {
                        hour: 14,
                        minute: 30,
                        second: 0,
                        hundredths: 0,
                    },
                ),
                count: -10,
            }),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = ReadRangeRequest::decode(&buf).unwrap();
        assert_eq!(decoded.range, req.range);
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_read_range_request_empty_input() {
        assert!(ReadRangeRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_read_range_request_truncated_1_byte() {
        let req = ReadRangeRequest {
            object_identifier: make_oid(),
            property_identifier: PropertyIdentifier::LOG_BUFFER,
            property_array_index: None,
            range: Some(RangeSpec::ByPosition {
                reference_index: 1,
                count: 10,
            }),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(ReadRangeRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_read_range_request_truncated_3_bytes() {
        let req = ReadRangeRequest {
            object_identifier: make_oid(),
            property_identifier: PropertyIdentifier::LOG_BUFFER,
            property_array_index: None,
            range: Some(RangeSpec::ByPosition {
                reference_index: 1,
                count: 10,
            }),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(ReadRangeRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_read_range_request_invalid_tag() {
        assert!(ReadRangeRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_read_range_ack_empty_input() {
        assert!(ReadRangeAck::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_read_range_ack_truncated_1_byte() {
        let ack = ReadRangeAck {
            object_identifier: make_oid(),
            property_identifier: PropertyIdentifier::LOG_BUFFER,
            property_array_index: None,
            result_flags: (true, false, true),
            item_count: 2,
            item_data: vec![0xAA, 0xBB, 0xCC],
            first_sequence_number: None,
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        assert!(ReadRangeAck::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_read_range_ack_truncated_3_bytes() {
        let ack = ReadRangeAck {
            object_identifier: make_oid(),
            property_identifier: PropertyIdentifier::LOG_BUFFER,
            property_array_index: None,
            result_flags: (true, false, true),
            item_count: 2,
            item_data: vec![0xAA, 0xBB, 0xCC],
            first_sequence_number: None,
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        assert!(ReadRangeAck::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_read_range_ack_truncated_half() {
        let ack = ReadRangeAck {
            object_identifier: make_oid(),
            property_identifier: PropertyIdentifier::LOG_BUFFER,
            property_array_index: None,
            result_flags: (true, false, true),
            item_count: 2,
            item_data: vec![0xAA, 0xBB, 0xCC],
            first_sequence_number: None,
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(ReadRangeAck::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_read_range_ack_invalid_tag() {
        assert!(ReadRangeAck::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn read_range_request_truncated_inner_tag() {
        // Craft a ReadRangeRequest with truncated inner content in byPosition
        let data = [
            0x0C, 0x05, 0x00, 0x00, 0x01, // [0] object id (TrendLog:1)
            0x19, 0x83, // [1] property id (LOG_BUFFER=131)
            // Opening tag [3] byPosition
            0x3E, // Inner tag claiming 50 bytes but only 1 byte present
            0x21, 50, 0x01, // Closing tag [3]
            0x3F,
        ];
        assert!(ReadRangeRequest::decode(&data).is_err());
    }
}
