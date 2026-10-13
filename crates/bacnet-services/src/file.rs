//! AtomicReadFile / AtomicWriteFile services per ASHRAE 135-2020 Clauses 14.1–14.2.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_encoding::constructed::tagged::{
    decode_app_object_id, decode_app_primitive, decode_app_unsigned, decode_ctx_constructed,
    decode_ctx_primitive, expect_end, next_is_context, next_is_opening,
};
use bacnet_encoding::{primitives, tags};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;

/// The application-tagged INTEGER at `offset` (a file-start-position or
/// file-start-record) and the offset past it.
fn decode_start(data: &[u8], offset: usize, what: &str) -> Result<(i32, usize), Error> {
    let (octets, end) = decode_app_primitive(data, offset, tags::app_tag::SIGNED, what)?;
    Ok((primitives::decode_signed(octets)?, end))
}

/// The fault where `what`'s access-method choice, `[0]` or `[1]`, is due at
/// `offset` and neither opens there: a missing member when the data ends
/// after the file identifier, any other tag a wrong one.
fn unknown_access_method(data: &[u8], offset: usize, what: &str) -> Error {
    if offset >= data.len() {
        Error::missing(offset, format!("{what}: access method missing"))
    } else {
        Error::invalid_tag(offset, format!("{what}: unknown access method"))
    }
}

// ---------------------------------------------------------------------------
// AtomicReadFile-Request
// ---------------------------------------------------------------------------

/// AtomicReadFile-Request — stream or record access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomicReadFileRequest {
    /// File object to read from; should be of object type File (not checked here).
    pub file_identifier: ObjectIdentifier,
    /// Stream or record access, with the position and amount requested.
    pub access: FileAccessMethod,
}

/// AtomicWriteFile-Request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomicWriteFileRequest {
    /// File object to write to; should be of object type File (not checked here).
    pub file_identifier: ObjectIdentifier,
    /// Stream or record access, with the data to write.
    pub access: FileWriteAccessMethod,
}

/// File access method for reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAccessMethod {
    /// Stream access: file_start_position, requested_octet_count.
    Stream {
        /// Octet offset at which reading starts (0 is the first octet).
        file_start_position: i32,
        /// Maximum number of octets the responder should return.
        requested_octet_count: u32,
    },
    /// Record access: file_start_record, requested_record_count.
    Record {
        /// Record number at which reading starts (0 is the first record).
        file_start_record: i32,
        /// Maximum number of records the responder should return.
        requested_record_count: u32,
    },
}

/// File access method for writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileWriteAccessMethod {
    /// Stream access: `file_data` is written beginning at octet offset
    /// `file_start_position`, or appended when that offset is -1.
    Stream {
        /// Octet offset at which writing starts, or -1 to append to the end of the file.
        file_start_position: i32,
        /// Octets to write.
        file_data: Vec<u8>,
    },
    /// Record access: `record_count` records taken from `file_record_data` are
    /// written beginning at record number `file_start_record`, or appended when
    /// that record number is -1.
    Record {
        /// Record number at which writing starts, or -1 to append after the last record.
        file_start_record: i32,
        /// Number of records supplied in `file_record_data`. Encode writes it as given; decode
        /// rejects a count that doesn't match the records.
        record_count: u32,
        /// Records to write, each one an opaque octet string.
        file_record_data: Vec<Vec<u8>>,
    },
}

impl AtomicReadFileRequest {
    /// Encode the request parameters (Clause 14.1.2) into `buf`.
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

    /// Decode the request from service-request octets; fails on malformed or truncated input,
    /// on a member under any tag but its application tag, and on octets after the access
    /// frame's members or after the frame.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (file_identifier, offset) =
            decode_app_object_id(data, 0, "AtomicReadFile file-identifier")?;

        let (access, end) = if next_is_opening(data, offset, 0)? {
            const WHAT: &str = "AtomicReadFile stream";
            let (content, end) = decode_ctx_constructed(data, offset, 0, WHAT)?;
            let (file_start_position, inner) =
                decode_start(content, 0, "AtomicReadFile stream file-start-position")?;
            let (requested_octet_count, inner) = decode_app_unsigned::<u32>(
                content,
                inner,
                "AtomicReadFile stream requested-octet-count",
            )?;
            expect_end(content, inner, offset, WHAT)?;
            let access = FileAccessMethod::Stream {
                file_start_position,
                requested_octet_count,
            };
            (access, end)
        } else if next_is_opening(data, offset, 1)? {
            const WHAT: &str = "AtomicReadFile record";
            let (content, end) = decode_ctx_constructed(data, offset, 1, WHAT)?;
            let (file_start_record, inner) =
                decode_start(content, 0, "AtomicReadFile record file-start-record")?;
            let (requested_record_count, inner) = decode_app_unsigned::<u32>(
                content,
                inner,
                "AtomicReadFile record requested-record-count",
            )?;
            expect_end(content, inner, offset, WHAT)?;
            let access = FileAccessMethod::Record {
                file_start_record,
                requested_record_count,
            };
            (access, end)
        } else {
            return Err(unknown_access_method(data, offset, "AtomicReadFile"));
        };
        expect_end(data, end, end, "AtomicReadFile")?;

        Ok(Self {
            file_identifier,
            access,
        })
    }
}

impl AtomicWriteFileRequest {
    /// Encode the request parameters (Clause 14.2.2) into `buf`.
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

    /// Decode the request from service-request octets; fails on malformed or truncated input,
    /// on a member under any tag but its application tag, and on octets after the stream
    /// frame's file data or after the frame. In record access, a record list that doesn't
    /// match its count is a Reject: MISSING_REQUIRED_PARAMETER when records are missing,
    /// TOO_MANY_ARGUMENTS when anything follows the counted ones.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (file_identifier, offset) =
            decode_app_object_id(data, 0, "AtomicWriteFile file-identifier")?;

        let (access, end) = if next_is_opening(data, offset, 0)? {
            const WHAT: &str = "AtomicWriteFile stream";
            let (content, end) = decode_ctx_constructed(data, offset, 0, WHAT)?;
            let (file_start_position, inner) =
                decode_start(content, 0, "AtomicWriteFile stream file-start-position")?;
            let (file_data, inner) = decode_app_primitive(
                content,
                inner,
                tags::app_tag::OCTET_STRING,
                "AtomicWriteFile stream file-data",
            )?;
            expect_end(content, inner, offset, WHAT)?;
            let access = FileWriteAccessMethod::Stream {
                file_start_position,
                file_data: file_data.to_vec(),
            };
            (access, end)
        } else if next_is_opening(data, offset, 1)? {
            let (content, end) = decode_ctx_constructed(data, offset, 1, "AtomicWriteFile record")?;
            let (file_start_record, mut inner) =
                decode_start(content, 0, "AtomicWriteFile record file-start-record")?;
            let (record_count, new_inner) =
                decode_app_unsigned::<u32>(content, inner, "AtomicWriteFile record record-count")?;
            inner = new_inner;
            if record_count as usize > MAX_DECODED_ITEMS {
                return Err(Error::overflow(0, "record count exceeds maximum"));
            }
            let mut file_record_data = Vec::new();
            for i in 0..record_count {
                if inner >= content.len() {
                    return Err(Error::missing(
                        inner,
                        format!("AtomicWriteFile record: record {i} of the count is missing"),
                    ));
                }
                let (record, new_inner) = decode_app_primitive(
                    content,
                    inner,
                    tags::app_tag::OCTET_STRING,
                    // Formatted only if the record fails to decode (#1645).
                    format_args!("AtomicWriteFile record data[{i}]"),
                )?;
                file_record_data.push(record.to_vec());
                inner = new_inner;
            }
            expect_end(content, inner, inner, "AtomicWriteFile record")?;
            let access = FileWriteAccessMethod::Record {
                file_start_record,
                record_count,
                file_record_data,
            };
            (access, end)
        } else {
            return Err(unknown_access_method(data, offset, "AtomicWriteFile"));
        };
        expect_end(data, end, end, "AtomicWriteFile")?;

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
    /// True when the returned data reaches the end of the file.
    pub end_of_file: bool,
    /// Stream or record data returned, with its starting position.
    pub access: FileReadAckMethod,
}

/// Read-ACK access method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileReadAckMethod {
    /// Stream access: file_start_position + returned data.
    Stream {
        /// Octet offset within the file of the first returned octet.
        file_start_position: i32,
        /// Octets read from the file.
        file_data: Vec<u8>,
    },
    /// Record access: file_start_record + returned records.
    Record {
        /// Record number of the first returned record.
        file_start_record: i32,
        /// Number of records returned in `file_record_data`.
        returned_record_count: u32,
        /// Records read from the file, each one an opaque octet string.
        file_record_data: Vec<Vec<u8>>,
    },
}

impl AtomicReadFileAck {
    /// Encode the acknowledgment parameters into `buf`.
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

    /// Decode the acknowledgment from its service-ack octets; fails on malformed or truncated
    /// input.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        if tag.class != tags::TagClass::Application
            || tag.number != tags::app_tag::BOOLEAN
            || tag.is_opening
            || tag.is_closing
            || data[offset] & 0x07 > 1
        {
            return Err(Error::decoding(
                offset,
                "AtomicReadFileAck expected application Boolean end-of-file",
            ));
        }
        let end_of_file = tag.length != 0;
        offset = pos;

        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        let (access, access_end) = if tag.is_opening_tag(0) {
            let (content, access_end) = tags::extract_context_value(data, tag_end, 0)?;
            let (file_start_position, inner) =
                decode_start(content, 0, "AtomicReadFileAck stream file-start-position")?;
            let (slice, inner) = decode_app_primitive(
                content,
                inner,
                tags::app_tag::OCTET_STRING,
                "AtomicReadFileAck stream file-data",
            )?;
            expect_end(content, inner, inner, "AtomicReadFileAck stream")?;
            let file_data = slice.to_vec();
            (
                FileReadAckMethod::Stream {
                    file_start_position,
                    file_data,
                },
                access_end,
            )
        } else if tag.is_opening_tag(1) {
            let (content, access_end) = tags::extract_context_value(data, tag_end, 1)?;
            let (file_start_record, mut inner) =
                decode_start(content, 0, "AtomicReadFileAck record file-start-record")?;
            let (returned_record_count, new_inner) = decode_app_unsigned::<u32>(
                content,
                inner,
                "AtomicReadFileAck record returned-record-count",
            )?;
            inner = new_inner;
            if returned_record_count as usize > MAX_DECODED_ITEMS {
                return Err(Error::overflow(0, "record count exceeds maximum"));
            }
            let mut file_record_data = Vec::new();
            for i in 0..returned_record_count {
                if inner >= content.len() {
                    return Err(Error::missing(
                        inner,
                        format!("AtomicReadFileAck record: record {i} of the count is missing"),
                    ));
                }
                let (slice, new_inner) = decode_app_primitive(
                    content,
                    inner,
                    tags::app_tag::OCTET_STRING,
                    // Formatted only if the record fails to decode (#1645).
                    format_args!("AtomicReadFileAck record data[{i}]"),
                )?;
                file_record_data.push(slice.to_vec());
                inner = new_inner;
            }
            expect_end(content, inner, inner, "AtomicReadFileAck record")?;
            (
                FileReadAckMethod::Record {
                    file_start_record,
                    returned_record_count,
                    file_record_data,
                },
                access_end,
            )
        } else {
            return Err(Error::decoding(
                offset,
                "Unknown read file ACK access method",
            ));
        };
        expect_end(data, access_end, access_end, "AtomicReadFileAck")?;

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
    /// Stream or record form of the acknowledgment, echoing where the write took place.
    pub access: FileWriteAckMethod,
}

/// Write-ACK access method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileWriteAckMethod {
    /// Stream: confirmed file_start_position.
    Stream {
        /// Octet offset at which the data was actually written.
        file_start_position: i32,
    },
    /// Record: confirmed file_start_record.
    Record {
        /// Record number at which the records were actually written.
        file_start_record: i32,
    },
}

impl AtomicWriteFileAck {
    /// Encode the acknowledgment parameters into `buf`.
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

    /// Decode the acknowledgment from its service-ack octets; fails on malformed or truncated
    /// input and on octets after the choice.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (access, end) = if next_is_context(data, 0, 0)? {
            let (octets, end) =
                decode_ctx_primitive(data, 0, 0, "AtomicWriteFileAck file-start-position")?;
            let access = FileWriteAckMethod::Stream {
                file_start_position: primitives::decode_signed(octets)?,
            };
            (access, end)
        } else if next_is_context(data, 0, 1)? {
            let (octets, end) =
                decode_ctx_primitive(data, 0, 1, "AtomicWriteFileAck file-start-record")?;
            let access = FileWriteAckMethod::Record {
                file_start_record: primitives::decode_signed(octets)?,
            };
            (access, end)
        } else {
            return Err(Error::decoding(0, "Unknown write file ACK access method"));
        };
        expect_end(data, end, end, "AtomicWriteFileAck")?;

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
}

#[cfg(test)]
#[path = "file_width_tests.rs"]
mod width_tests;

#[cfg(test)]
#[path = "file_ack_strict_tests.rs"]
mod ack_strict_tests;

#[cfg(test)]
#[path = "file_ack_roundtrip_tests.rs"]
mod ack_roundtrip_tests;
#[path = "file_ack_size.rs"]
mod ack_size;
