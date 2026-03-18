//! AtomicReadFile / AtomicWriteFile services per ASHRAE 135-2020 Clauses 15.1–15.2.

use bacnet_encoding::{primitives, tags};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;

/// Decode a tag and validate the resulting slice bounds.
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

// ---------------------------------------------------------------------------
// AtomicReadFile-Request
// ---------------------------------------------------------------------------

/// AtomicReadFile-Request — stream or record access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomicReadFileRequest {
    pub file_identifier: ObjectIdentifier,
    pub access: FileAccessMethod,
}

/// AtomicWriteFile-Request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomicWriteFileRequest {
    pub file_identifier: ObjectIdentifier,
    pub access: FileWriteAccessMethod,
}

/// File access method for reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAccessMethod {
    /// Stream access: file_start_position, requested_octet_count.
    Stream {
        file_start_position: i32,
        requested_octet_count: u32,
    },
    /// Record access: file_start_record, requested_record_count.
    Record {
        file_start_record: i32,
        requested_record_count: u32,
    },
}

/// File access method for writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileWriteAccessMethod {
    /// Stream access: file_start_position, file_data.
    Stream {
        file_start_position: i32,
        file_data: Vec<u8>,
    },
    /// Record access: file_start_record, record_count, file_record_data.
    Record {
        file_start_record: i32,
        record_count: u32,
        file_record_data: Vec<Vec<u8>>,
    },
}

impl AtomicReadFileRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_object_id(buf, &self.file_identifier);
        match &self.access {
            FileAccessMethod::Stream {
                file_start_position,
                requested_octet_count,
            } => {
                tags::encode_opening_tag(buf, 0);
                primitives::encode_app_signed(buf, *file_start_position);
                primitives::encode_app_unsigned(buf, *requested_octet_count as u64);
                tags::encode_closing_tag(buf, 0);
            }
            FileAccessMethod::Record {
                file_start_record,
                requested_record_count,
            } => {
                tags::encode_opening_tag(buf, 1);
                primitives::encode_app_signed(buf, *file_start_record);
                primitives::encode_app_unsigned(buf, *requested_record_count as u64);
                tags::encode_closing_tag(buf, 1);
            }
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::buffer_too_short(end, data.len()));
        }
        let file_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        let access = if tag.is_opening_tag(0) {
            let (content, _) = tags::extract_context_value(data, tag_end, 0)?;
            let (slice, inner) =
                checked_slice(content, 0, "AtomicReadFile stream file-start-position")?;
            let file_start_position = primitives::decode_signed(slice)?;
            let (slice, _) = checked_slice(
                content,
                inner,
                "AtomicReadFile stream requested-octet-count",
            )?;
            let requested_octet_count = primitives::decode_unsigned(slice)? as u32;
            FileAccessMethod::Stream {
                file_start_position,
                requested_octet_count,
            }
        } else if tag.is_opening_tag(1) {
            let (content, _) = tags::extract_context_value(data, tag_end, 1)?;
            let (slice, inner) =
                checked_slice(content, 0, "AtomicReadFile record file-start-record")?;
            let file_start_record = primitives::decode_signed(slice)?;
            let (slice, _) = checked_slice(
                content,
                inner,
                "AtomicReadFile record requested-record-count",
            )?;
            let requested_record_count = primitives::decode_unsigned(slice)? as u32;
            FileAccessMethod::Record {
                file_start_record,
                requested_record_count,
            }
        } else {
            return Err(Error::decoding(offset, "Unknown file access method"));
        };

        Ok(Self {
            file_identifier,
            access,
        })
    }
}

impl AtomicWriteFileRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_object_id(buf, &self.file_identifier);
        match &self.access {
            FileWriteAccessMethod::Stream {
                file_start_position,
                file_data,
            } => {
                tags::encode_opening_tag(buf, 0);
                primitives::encode_app_signed(buf, *file_start_position);
                primitives::encode_app_octet_string(buf, file_data);
                tags::encode_closing_tag(buf, 0);
            }
            FileWriteAccessMethod::Record {
                file_start_record,
                record_count,
                file_record_data,
            } => {
                tags::encode_opening_tag(buf, 1);
                primitives::encode_app_signed(buf, *file_start_record);
                primitives::encode_app_unsigned(buf, *record_count as u64);
                for record in file_record_data {
                    primitives::encode_app_octet_string(buf, record);
                }
                tags::encode_closing_tag(buf, 1);
            }
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::buffer_too_short(end, data.len()));
        }
        let file_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        let access = if tag.is_opening_tag(0) {
            let (content, _) = tags::extract_context_value(data, tag_end, 0)?;
            let (slice, inner) =
                checked_slice(content, 0, "AtomicWriteFile stream file-start-position")?;
            let file_start_position = primitives::decode_signed(slice)?;
            let (slice, _) = checked_slice(content, inner, "AtomicWriteFile stream file-data")?;
            let file_data = slice.to_vec();
            FileWriteAccessMethod::Stream {
                file_start_position,
                file_data,
            }
        } else if tag.is_opening_tag(1) {
            let (content, _) = tags::extract_context_value(data, tag_end, 1)?;
            let (slice, mut inner) =
                checked_slice(content, 0, "AtomicWriteFile record file-start-record")?;
            let file_start_record = primitives::decode_signed(slice)?;
            let (slice, new_inner) =
                checked_slice(content, inner, "AtomicWriteFile record record-count")?;
            let record_count = primitives::decode_unsigned(slice)? as u32;
            inner = new_inner;
            if record_count as usize > MAX_DECODED_ITEMS {
                return Err(Error::decoding(0, "record count exceeds maximum"));
            }
            let mut file_record_data = Vec::new();
            for i in 0..record_count {
                if inner >= content.len() {
                    break;
                }
                let (slice, new_inner) =
                    checked_slice(content, inner, &format!("AtomicWriteFile record data[{i}]"))?;
                file_record_data.push(slice.to_vec());
                inner = new_inner;
            }
            FileWriteAccessMethod::Record {
                file_start_record,
                record_count,
                file_record_data,
            }
        } else {
            return Err(Error::decoding(offset, "Unknown file write access method"));
        };

        Ok(Self {
            file_identifier,
            access,
        })
    }
}

// ---------------------------------------------------------------------------
// AtomicReadFile-ACK
// ---------------------------------------------------------------------------

/// AtomicReadFile-ACK — response for stream or record access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomicReadFileAck {
    pub end_of_file: bool,
    pub access: FileReadAckMethod,
}

/// Read-ACK access method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileReadAckMethod {
    /// Stream access: file_start_position + returned data.
    Stream {
        file_start_position: i32,
        file_data: Vec<u8>,
    },
    /// Record access: file_start_record + returned records.
    Record {
        file_start_record: i32,
        returned_record_count: u32,
        file_record_data: Vec<Vec<u8>>,
    },
}

impl AtomicReadFileAck {
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_boolean(buf, self.end_of_file);
        match &self.access {
            FileReadAckMethod::Stream {
                file_start_position,
                file_data,
            } => {
                tags::encode_opening_tag(buf, 0);
                primitives::encode_app_signed(buf, *file_start_position);
                primitives::encode_app_octet_string(buf, file_data);
                tags::encode_closing_tag(buf, 0);
            }
            FileReadAckMethod::Record {
                file_start_record,
                returned_record_count,
                file_record_data,
            } => {
                tags::encode_opening_tag(buf, 1);
                primitives::encode_app_signed(buf, *file_start_record);
                primitives::encode_app_unsigned(buf, *returned_record_count as u64);
                for record in file_record_data {
                    primitives::encode_app_octet_string(buf, record);
                }
                tags::encode_closing_tag(buf, 1);
            }
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end_of_file = tag.length != 0;
        offset = pos;

        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        let access = if tag.is_opening_tag(0) {
            let (content, _) = tags::extract_context_value(data, tag_end, 0)?;
            let (slice, inner) =
                checked_slice(content, 0, "AtomicReadFileAck stream file-start-position")?;
            let file_start_position = primitives::decode_signed(slice)?;
            let (slice, _) = checked_slice(content, inner, "AtomicReadFileAck stream file-data")?;
            let file_data = slice.to_vec();
            FileReadAckMethod::Stream {
                file_start_position,
                file_data,
            }
        } else if tag.is_opening_tag(1) {
            let (content, _) = tags::extract_context_value(data, tag_end, 1)?;
            let (slice, mut inner) =
                checked_slice(content, 0, "AtomicReadFileAck record file-start-record")?;
            let file_start_record = primitives::decode_signed(slice)?;
            let (slice, new_inner) = checked_slice(
                content,
                inner,
                "AtomicReadFileAck record returned-record-count",
            )?;
            let returned_record_count = primitives::decode_unsigned(slice)? as u32;
            inner = new_inner;
            if returned_record_count as usize > MAX_DECODED_ITEMS {
                return Err(Error::decoding(0, "record count exceeds maximum"));
            }
            let mut file_record_data = Vec::new();
            for i in 0..returned_record_count {
                if inner >= content.len() {
                    break;
                }
                let (slice, new_inner) = checked_slice(
                    content,
                    inner,
                    &format!("AtomicReadFileAck record data[{i}]"),
                )?;
                file_record_data.push(slice.to_vec());
                inner = new_inner;
            }
            FileReadAckMethod::Record {
                file_start_record,
                returned_record_count,
                file_record_data,
            }
        } else {
            return Err(Error::decoding(
                offset,
                "Unknown read file ACK access method",
            ));
        };

        Ok(Self {
            end_of_file,
            access,
        })
    }
}

// ---------------------------------------------------------------------------
// AtomicWriteFile-ACK
// ---------------------------------------------------------------------------

/// AtomicWriteFile-ACK — response for stream or record access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomicWriteFileAck {
    pub access: FileWriteAckMethod,
}

/// Write-ACK access method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileWriteAckMethod {
    /// Stream: confirmed file_start_position.
    Stream { file_start_position: i32 },
    /// Record: confirmed file_start_record.
    Record { file_start_record: i32 },
}

impl AtomicWriteFileAck {
    pub fn encode(&self, buf: &mut BytesMut) {
        match &self.access {
            FileWriteAckMethod::Stream {
                file_start_position,
            } => {
                primitives::encode_ctx_signed(buf, 0, *file_start_position);
            }
            FileWriteAckMethod::Record { file_start_record } => {
                primitives::encode_ctx_signed(buf, 1, *file_start_record);
            }
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (tag, pos) = tags::decode_tag(data, 0)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::buffer_too_short(end, data.len()));
        }
        let access = if tag.is_context(0) {
            let file_start_position = primitives::decode_signed(&data[pos..end])?;
            FileWriteAckMethod::Stream {
                file_start_position,
            }
        } else if tag.is_context(1) {
            let file_start_record = primitives::decode_signed(&data[pos..end])?;
            FileWriteAckMethod::Record { file_start_record }
        } else {
            return Err(Error::decoding(0, "Unknown write file ACK access method"));
        };

        Ok(Self { access })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    fn file_oid() -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::FILE, 1).unwrap()
    }

    #[test]
    fn atomic_read_stream_round_trip() {
        let req = AtomicReadFileRequest {
            file_identifier: file_oid(),
            access: FileAccessMethod::Stream {
                file_start_position: 0,
                requested_octet_count: 1024,
            },
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = AtomicReadFileRequest::decode(&buf).unwrap();
        assert_eq!(decoded, req);
    }

    #[test]
    fn atomic_read_record_round_trip() {
        let req = AtomicReadFileRequest {
            file_identifier: file_oid(),
            access: FileAccessMethod::Record {
                file_start_record: 5,
                requested_record_count: 10,
            },
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = AtomicReadFileRequest::decode(&buf).unwrap();
        assert_eq!(decoded, req);
    }

    #[test]
    fn atomic_write_stream_round_trip() {
        let req = AtomicWriteFileRequest {
            file_identifier: file_oid(),
            access: FileWriteAccessMethod::Stream {
                file_start_position: 100,
                file_data: vec![0x01, 0x02, 0x03, 0x04],
            },
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = AtomicWriteFileRequest::decode(&buf).unwrap();
        assert_eq!(decoded, req);
    }

    #[test]
    fn atomic_write_record_round_trip() {
        let req = AtomicWriteFileRequest {
            file_identifier: file_oid(),
            access: FileWriteAccessMethod::Record {
                file_start_record: 0,
                record_count: 2,
                file_record_data: vec![vec![0xAA, 0xBB], vec![0xCC, 0xDD]],
            },
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = AtomicWriteFileRequest::decode(&buf).unwrap();
        assert_eq!(decoded, req);
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_atomic_read_file_empty_input() {
        assert!(AtomicReadFileRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_atomic_read_file_truncated_1_byte() {
        let req = AtomicReadFileRequest {
            file_identifier: file_oid(),
            access: FileAccessMethod::Stream {
                file_start_position: 0,
                requested_octet_count: 1024,
            },
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(AtomicReadFileRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_atomic_read_file_truncated_3_bytes() {
        let req = AtomicReadFileRequest {
            file_identifier: file_oid(),
            access: FileAccessMethod::Stream {
                file_start_position: 0,
                requested_octet_count: 1024,
            },
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(AtomicReadFileRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_atomic_read_file_truncated_half() {
        let req = AtomicReadFileRequest {
            file_identifier: file_oid(),
            access: FileAccessMethod::Stream {
                file_start_position: 0,
                requested_octet_count: 1024,
            },
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(AtomicReadFileRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_atomic_read_file_invalid_tag() {
        assert!(AtomicReadFileRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_atomic_write_file_empty_input() {
        assert!(AtomicWriteFileRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_atomic_write_file_truncated_1_byte() {
        let req = AtomicWriteFileRequest {
            file_identifier: file_oid(),
            access: FileWriteAccessMethod::Stream {
                file_start_position: 100,
                file_data: vec![0x01, 0x02, 0x03, 0x04],
            },
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(AtomicWriteFileRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_atomic_write_file_truncated_3_bytes() {
        let req = AtomicWriteFileRequest {
            file_identifier: file_oid(),
            access: FileWriteAccessMethod::Stream {
                file_start_position: 100,
                file_data: vec![0x01, 0x02, 0x03, 0x04],
            },
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(AtomicWriteFileRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_atomic_write_file_truncated_half() {
        let req = AtomicWriteFileRequest {
            file_identifier: file_oid(),
            access: FileWriteAccessMethod::Stream {
                file_start_position: 100,
                file_data: vec![0x01, 0x02, 0x03, 0x04],
            },
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(AtomicWriteFileRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_atomic_write_file_invalid_tag() {
        assert!(AtomicWriteFileRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn atomic_read_file_request_truncated_inner_tag() {
        // Craft a packet where extract_context_value succeeds but inner tag within
        // the content claims more bytes than the content slice contains.
        // Application signed tag (tag 3), lvt=5 (extended length), length=50 → only 2 bytes present.
        let data = [
            0xC4, 0x02, 0x80, 0x00, 0x01, // object identifier (FILE:1)
            // Opening tag [0]
            0x0E,
            // App signed tag (tag 3=0x30), extended len (lvt=5 → 0x05): 0x35, len byte: 50
            0x35, 50, // Only 2 data bytes instead of 50
            0x01, 0x02, // Closing tag [0]
            0x0F,
        ];
        assert!(AtomicReadFileRequest::decode(&data).is_err());
    }

    #[test]
    fn atomic_write_file_request_truncated_inner_tag() {
        // Same technique for AtomicWriteFile: inner tag claims too many bytes
        let data = [
            0xC4, 0x02, 0x80, 0x00, 0x01, // object identifier (FILE:1)
            // Opening tag [0]
            0x0E, // App signed tag, extended len, claims 80 bytes
            0x35, 80,   // Only 1 data byte instead of 80
            0x01, // Closing tag [0]
            0x0F,
        ];
        assert!(AtomicWriteFileRequest::decode(&data).is_err());
    }

    // -----------------------------------------------------------------------
    // AtomicReadFile-ACK round-trip tests
    // -----------------------------------------------------------------------

    #[test]
    fn atomic_read_file_ack_stream_round_trip() {
        let ack = AtomicReadFileAck {
            end_of_file: false,
            access: FileReadAckMethod::Stream {
                file_start_position: 0,
                file_data: vec![0x48, 0x65, 0x6C, 0x6C, 0x6F],
            },
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = AtomicReadFileAck::decode(&buf).unwrap();
        assert_eq!(decoded, ack);
    }

    #[test]
    fn atomic_read_file_ack_stream_eof_true() {
        let ack = AtomicReadFileAck {
            end_of_file: true,
            access: FileReadAckMethod::Stream {
                file_start_position: 512,
                file_data: vec![0xDE, 0xAD],
            },
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = AtomicReadFileAck::decode(&buf).unwrap();
        assert_eq!(decoded, ack);
    }

    #[test]
    fn atomic_read_file_ack_record_round_trip() {
        let ack = AtomicReadFileAck {
            end_of_file: true,
            access: FileReadAckMethod::Record {
                file_start_record: 5,
                returned_record_count: 2,
                file_record_data: vec![vec![0xAA, 0xBB], vec![0xCC, 0xDD, 0xEE]],
            },
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = AtomicReadFileAck::decode(&buf).unwrap();
        assert_eq!(decoded, ack);
    }

    #[test]
    fn atomic_read_file_ack_record_empty() {
        let ack = AtomicReadFileAck {
            end_of_file: true,
            access: FileReadAckMethod::Record {
                file_start_record: 0,
                returned_record_count: 0,
                file_record_data: vec![],
            },
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = AtomicReadFileAck::decode(&buf).unwrap();
        assert_eq!(decoded, ack);
    }

    // -----------------------------------------------------------------------
    // AtomicWriteFile-ACK round-trip tests
    // -----------------------------------------------------------------------

    #[test]
    fn atomic_write_file_ack_stream_round_trip() {
        let ack = AtomicWriteFileAck {
            access: FileWriteAckMethod::Stream {
                file_start_position: 100,
            },
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = AtomicWriteFileAck::decode(&buf).unwrap();
        assert_eq!(decoded, ack);
    }

    #[test]
    fn atomic_write_file_ack_stream_negative_position() {
        let ack = AtomicWriteFileAck {
            access: FileWriteAckMethod::Stream {
                file_start_position: -1,
            },
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = AtomicWriteFileAck::decode(&buf).unwrap();
        assert_eq!(decoded, ack);
    }

    #[test]
    fn atomic_write_file_ack_record_round_trip() {
        let ack = AtomicWriteFileAck {
            access: FileWriteAckMethod::Record {
                file_start_record: 42,
            },
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = AtomicWriteFileAck::decode(&buf).unwrap();
        assert_eq!(decoded, ack);
    }

    // -----------------------------------------------------------------------
    // ACK truncated-input error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_read_file_ack_empty_input() {
        assert!(AtomicReadFileAck::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_read_file_ack_truncated() {
        let ack = AtomicReadFileAck {
            end_of_file: false,
            access: FileReadAckMethod::Stream {
                file_start_position: 0,
                file_data: vec![0x01, 0x02, 0x03],
            },
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        // Only boolean tag — missing access method
        assert!(AtomicReadFileAck::decode(&buf[..1]).is_err());
        // Half the payload
        let half = buf.len() / 2;
        assert!(AtomicReadFileAck::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_write_file_ack_empty_input() {
        assert!(AtomicWriteFileAck::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_write_file_ack_truncated() {
        let ack = AtomicWriteFileAck {
            access: FileWriteAckMethod::Stream {
                file_start_position: 100,
            },
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        // Just the tag byte, no value
        if buf.len() > 1 {
            assert!(AtomicWriteFileAck::decode(&buf[..1]).is_err());
        }
    }
}
