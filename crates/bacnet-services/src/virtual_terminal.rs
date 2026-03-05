//! Virtual Terminal (VT) services per ASHRAE 135-2020 Clauses 16.3–16.5.
//!
//! Legacy services needed for full spec coverage. All fields use APPLICATION
//! tags (not context-specific) unless noted.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::error::Error;
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;

// ---------------------------------------------------------------------------
// VTOpenRequest / VTOpenAck (Clause 16.3)
// ---------------------------------------------------------------------------

/// VT-Open-Request service parameters.
///
/// `vt_class` is an APPLICATION-tagged ENUMERATED.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VTOpenRequest {
    pub vt_class: u32,
}

impl VTOpenRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_enumerated(buf, self.vt_class);
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (tag, pos) = tags::decode_tag(data, 0)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "VTOpen truncated at vt-class"));
        }
        let vt_class = primitives::decode_unsigned(&data[pos..end])? as u32;
        Ok(Self { vt_class })
    }
}

/// VT-Open-Ack service parameters.
///
/// `remote_vt_session_identifier` is an APPLICATION-tagged Unsigned8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VTOpenAck {
    pub remote_vt_session_identifier: u8,
}

impl VTOpenAck {
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_unsigned(buf, self.remote_vt_session_identifier as u64);
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (tag, pos) = tags::decode_tag(data, 0)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "VTOpenAck truncated at session-identifier",
            ));
        }
        let id = primitives::decode_unsigned(&data[pos..end])? as u8;
        Ok(Self {
            remote_vt_session_identifier: id,
        })
    }
}

// ---------------------------------------------------------------------------
// VTCloseRequest (Clause 16.4)
// ---------------------------------------------------------------------------

/// VT-Close-Request service parameters.
///
/// Contains a SEQUENCE OF Unsigned8 (APPLICATION tagged).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VTCloseRequest {
    pub list_of_remote_vt_session_identifiers: Vec<u8>,
}

impl VTCloseRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        for &id in &self.list_of_remote_vt_session_identifiers {
            primitives::encode_app_unsigned(buf, id as u64);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;
        let mut ids = Vec::new();
        while offset < data.len() {
            if ids.len() >= MAX_DECODED_ITEMS {
                return Err(Error::decoding(offset, "VTClose too many session IDs"));
            }
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "VTClose truncated at session-identifier",
                ));
            }
            ids.push(primitives::decode_unsigned(&data[pos..end])? as u8);
            offset = end;
        }
        Ok(Self {
            list_of_remote_vt_session_identifiers: ids,
        })
    }
}

// ---------------------------------------------------------------------------
// VTDataRequest / VTDataAck (Clause 16.5)
// ---------------------------------------------------------------------------

/// VT-Data-Request service parameters.
///
/// All fields are APPLICATION tagged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VTDataRequest {
    pub vt_session_identifier: u8,
    pub vt_new_data: Vec<u8>,
    pub vt_data_flag: bool,
}

impl VTDataRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_unsigned(buf, self.vt_session_identifier as u64);
        primitives::encode_app_octet_string(buf, &self.vt_new_data);
        primitives::encode_app_boolean(buf, self.vt_data_flag);
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // Unsigned8: vt-session-identifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "VTData truncated at session-identifier",
            ));
        }
        let vt_session_identifier = primitives::decode_unsigned(&data[pos..end])? as u8;
        offset = end;

        // OctetString: vt-new-data
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "VTData truncated at new-data"));
        }
        let vt_new_data = data[pos..end].to_vec();
        offset = end;

        // Boolean: vt-data-flag
        // BACnet application boolean: value is encoded in the tag length field,
        // with no content bytes following.
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let vt_data_flag = tag.length != 0;
        let _ = pos;

        Ok(Self {
            vt_session_identifier,
            vt_new_data,
            vt_data_flag,
        })
    }
}

/// VT-Data-Ack service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VTDataAck {
    /// [0] allNewDataAccepted OPTIONAL
    pub all_new_data_accepted: Option<bool>,
    /// [1] acceptedOctetCount OPTIONAL
    pub accepted_octet_count: Option<u32>,
}

impl VTDataAck {
    pub fn encode(&self, buf: &mut BytesMut) {
        if let Some(v) = self.all_new_data_accepted {
            primitives::encode_ctx_boolean(buf, 0, v);
        }
        if let Some(v) = self.accepted_octet_count {
            primitives::encode_ctx_unsigned(buf, 1, v as u64);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] allNewDataAccepted OPTIONAL
        let mut all_new_data_accepted = None;
        if offset < data.len() {
            let (opt, new_off) = tags::decode_optional_context(data, offset, 0)?;
            if let Some(content) = opt {
                all_new_data_accepted = Some(!content.is_empty() && content[0] != 0);
                offset = new_off;
            }
        }

        // [1] acceptedOctetCount OPTIONAL
        let mut accepted_octet_count = None;
        if offset < data.len() {
            let (opt, new_off) = tags::decode_optional_context(data, offset, 1)?;
            if let Some(content) = opt {
                accepted_octet_count = Some(primitives::decode_unsigned(content)? as u32);
                offset = new_off;
            }
        }
        let _ = offset;

        Ok(Self {
            all_new_data_accepted,
            accepted_octet_count,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vt_open_round_trip() {
        let req = VTOpenRequest { vt_class: 1 };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = VTOpenRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn vt_open_ack_round_trip() {
        let ack = VTOpenAck {
            remote_vt_session_identifier: 42,
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = VTOpenAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    #[test]
    fn vt_close_round_trip() {
        let req = VTCloseRequest {
            list_of_remote_vt_session_identifiers: vec![1, 2, 3],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = VTCloseRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn vt_close_empty() {
        let req = VTCloseRequest {
            list_of_remote_vt_session_identifiers: vec![],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(buf.is_empty());
        let decoded = VTCloseRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn vt_data_round_trip() {
        let req = VTDataRequest {
            vt_session_identifier: 1,
            vt_new_data: vec![0x48, 0x65, 0x6C, 0x6C, 0x6F], // "Hello"
            vt_data_flag: true,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = VTDataRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn vt_data_flag_false() {
        let req = VTDataRequest {
            vt_session_identifier: 5,
            vt_new_data: vec![0x01],
            vt_data_flag: false,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = VTDataRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn vt_data_ack_round_trip() {
        let ack = VTDataAck {
            all_new_data_accepted: Some(true),
            accepted_octet_count: Some(100),
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = VTDataAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    #[test]
    fn vt_data_ack_empty() {
        let ack = VTDataAck {
            all_new_data_accepted: None,
            accepted_octet_count: None,
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        assert!(buf.is_empty());
        let decoded = VTDataAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    #[test]
    fn vt_open_empty_input() {
        assert!(VTOpenRequest::decode(&[]).is_err());
    }

    #[test]
    fn vt_data_empty_input() {
        assert!(VTDataRequest::decode(&[]).is_err());
    }
}
