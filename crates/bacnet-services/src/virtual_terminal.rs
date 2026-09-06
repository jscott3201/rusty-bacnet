//! Virtual Terminal (VT) services per ASHRAE 135-2020 Clauses 16.3–16.5.
//!
//! Legacy services needed for full spec coverage. All fields use APPLICATION
//! tags (not context-specific) unless noted.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::error::Error;
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;

fn is_application_tag(tag: &tags::Tag, header: u8, number: u8, max_lvt: u8) -> bool {
    tag.class == tags::TagClass::Application && tag.number == number && header & 0x07 <= max_lvt
}

// ---------------------------------------------------------------------------
// VTOpenRequest / VTOpenAck
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
        if !is_application_tag(&tag, data[0], tags::app_tag::ENUMERATED, 5) {
            return Err(Error::decoding(0, "VTOpen expected application Enumerated"));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "VTOpen truncated at vt-class"));
        }
        let vt_class_raw = primitives::decode_unsigned(&data[pos..end])?;
        let vt_class = u32::try_from(vt_class_raw).map_err(|_| {
            Error::decoding(pos, format!("VTOpen vt-class {vt_class_raw} exceeds u32"))
        })?;
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
        if !is_application_tag(&tag, data[0], tags::app_tag::UNSIGNED, 5) {
            return Err(Error::decoding(
                0,
                "VTOpenAck expected application Unsigned",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "VTOpenAck truncated at session-identifier",
            ));
        }
        let id_raw = primitives::decode_unsigned(&data[pos..end])?;
        let id = u8::try_from(id_raw).map_err(|_| {
            Error::decoding(
                pos,
                format!("VTOpenAck session-identifier {id_raw} exceeds u8"),
            )
        })?;
        Ok(Self {
            remote_vt_session_identifier: id,
        })
    }
}

// ---------------------------------------------------------------------------
// VTCloseRequest
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
            if !is_application_tag(&tag, data[offset], tags::app_tag::UNSIGNED, 5) {
                return Err(Error::decoding(
                    offset,
                    "VTClose expected application Unsigned",
                ));
            }
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "VTClose truncated at session-identifier",
                ));
            }
            let id_raw = primitives::decode_unsigned(&data[pos..end])?;
            ids.push(u8::try_from(id_raw).map_err(|_| {
                Error::decoding(
                    pos,
                    format!("VTClose session-identifier {id_raw} exceeds u8"),
                )
            })?);
            offset = end;
        }
        Ok(Self {
            list_of_remote_vt_session_identifiers: ids,
        })
    }
}

// ---------------------------------------------------------------------------
// VTDataRequest / VTDataAck
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

        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !is_application_tag(&tag, data[offset], tags::app_tag::UNSIGNED, 5) {
            return Err(Error::decoding(
                offset,
                "VTData expected application Unsigned session-identifier",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                pos,
                "VTData truncated at session-identifier",
            ));
        }
        let vt_session_identifier_raw = primitives::decode_unsigned(&data[pos..end])?;
        let vt_session_identifier = u8::try_from(vt_session_identifier_raw).map_err(|_| {
            Error::decoding(
                pos,
                format!("VTData session-identifier {vt_session_identifier_raw} exceeds u8"),
            )
        })?;
        offset = end;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !is_application_tag(&tag, data[offset], tags::app_tag::OCTET_STRING, 5) {
            return Err(Error::decoding(
                offset,
                "VTData expected application OctetString",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "VTData truncated at new-data"));
        }
        let vt_new_data = data[pos..end].to_vec();
        offset = end;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !is_application_tag(&tag, data[offset], tags::app_tag::BOOLEAN, 1) {
            return Err(Error::decoding(
                offset,
                "VTData expected application Boolean",
            ));
        }
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
                let accepted_octet_count_raw = primitives::decode_unsigned(content)?;
                accepted_octet_count = Some(u32::try_from(accepted_octet_count_raw).map_err(
                    |_| {
                        Error::decoding(
                            offset,
                            format!(
                                "VTDataAck accepted-octet-count {accepted_octet_count_raw} exceeds u32"
                            ),
                        )
                    },
                )?);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_application_unsigned(tag_number: u8, value: u64) -> BytesMut {
        let mut buf = BytesMut::new();
        primitives::encode_app_unsigned(&mut buf, value);
        buf[0] = (tag_number << 4) | (buf[0] & 0x0f);
        buf
    }

    fn encode_vt_data(session_identifier: u64) -> BytesMut {
        let mut buf = encode_application_unsigned(tags::app_tag::UNSIGNED, session_identifier);
        primitives::encode_app_octet_string(&mut buf, &[0x01]);
        primitives::encode_app_boolean(&mut buf, false);
        buf
    }

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
    fn vt_open_values_must_fit_field_widths() {
        let maximum = encode_application_unsigned(tags::app_tag::ENUMERATED, u64::from(u32::MAX));
        assert_eq!(VTOpenRequest::decode(&maximum).unwrap().vt_class, u32::MAX);

        let mut leading_zero = BytesMut::new();
        tags::encode_tag(
            &mut leading_zero,
            tags::app_tag::ENUMERATED,
            tags::TagClass::Application,
            5,
        );
        leading_zero.extend_from_slice(&[0, 0xff, 0xff, 0xff, 0xff]);
        assert_eq!(
            VTOpenRequest::decode(&leading_zero).unwrap().vt_class,
            u32::MAX
        );

        for value in [u64::from(u32::MAX) + 1, u64::MAX] {
            let encoded = encode_application_unsigned(tags::app_tag::ENUMERATED, value);
            assert!(VTOpenRequest::decode(&encoded).is_err());
        }
    }

    #[test]
    fn vt_open_ack_identifier_must_fit_u8() {
        let maximum = encode_application_unsigned(tags::app_tag::UNSIGNED, u64::from(u8::MAX));
        assert_eq!(
            VTOpenAck::decode(&maximum)
                .unwrap()
                .remote_vt_session_identifier,
            u8::MAX
        );

        let mut leading_zero = BytesMut::new();
        tags::encode_tag(
            &mut leading_zero,
            tags::app_tag::UNSIGNED,
            tags::TagClass::Application,
            2,
        );
        leading_zero.extend_from_slice(&[0, 0xff]);
        assert_eq!(
            VTOpenAck::decode(&leading_zero)
                .unwrap()
                .remote_vt_session_identifier,
            u8::MAX
        );

        for value in [256, 257, u64::MAX] {
            let encoded = encode_application_unsigned(tags::app_tag::UNSIGNED, value);
            assert!(VTOpenAck::decode(&encoded).is_err());
        }
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
    fn vt_close_identifiers_must_fit_u8() {
        let mut overflow = encode_application_unsigned(tags::app_tag::UNSIGNED, 1);
        overflow.extend_from_slice(&encode_application_unsigned(tags::app_tag::UNSIGNED, 256));
        assert!(VTCloseRequest::decode(&overflow).is_err());

        let mut leading_zero = BytesMut::new();
        tags::encode_tag(
            &mut leading_zero,
            tags::app_tag::UNSIGNED,
            tags::TagClass::Application,
            2,
        );
        leading_zero.extend_from_slice(&[0, 0xff]);
        assert_eq!(
            VTCloseRequest::decode(&leading_zero)
                .unwrap()
                .list_of_remote_vt_session_identifiers,
            [u8::MAX]
        );
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
    fn vt_data_identifier_must_fit_u8() {
        for value in [256, 257, u64::MAX] {
            assert!(VTDataRequest::decode(&encode_vt_data(value)).is_err());
        }

        let mut leading_zero = BytesMut::new();
        tags::encode_tag(
            &mut leading_zero,
            tags::app_tag::UNSIGNED,
            tags::TagClass::Application,
            2,
        );
        leading_zero.extend_from_slice(&[0, 0xff]);
        primitives::encode_app_octet_string(&mut leading_zero, &[0x01]);
        primitives::encode_app_boolean(&mut leading_zero, false);
        assert_eq!(
            VTDataRequest::decode(&leading_zero)
                .unwrap()
                .vt_session_identifier,
            u8::MAX
        );
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
    fn vt_data_ack_count_must_fit_u32() {
        for value in [u64::from(u32::MAX) + 1, u64::MAX] {
            let mut encoded = BytesMut::new();
            primitives::encode_ctx_unsigned(&mut encoded, 1, value);
            assert!(VTDataAck::decode(&encoded).is_err());
        }

        let mut leading_zero = BytesMut::new();
        tags::encode_tag(&mut leading_zero, 1, tags::TagClass::Context, 5);
        leading_zero.extend_from_slice(&[0, 0xff, 0xff, 0xff, 0xff]);
        assert_eq!(
            VTDataAck::decode(&leading_zero)
                .unwrap()
                .accepted_octet_count,
            Some(u32::MAX)
        );
    }

    #[test]
    fn vt_decoders_require_application_field_tags() {
        let open_wrong_tag = encode_application_unsigned(tags::app_tag::UNSIGNED, 1);
        assert!(VTOpenRequest::decode(&open_wrong_tag).is_err());

        let open_ack_wrong_tag = encode_application_unsigned(tags::app_tag::ENUMERATED, 1);
        assert!(VTOpenAck::decode(&open_ack_wrong_tag).is_err());
        assert!(VTCloseRequest::decode(&open_ack_wrong_tag).is_err());

        let mut data_wrong_session = encode_application_unsigned(tags::app_tag::ENUMERATED, 1);
        primitives::encode_app_octet_string(&mut data_wrong_session, &[0x01]);
        primitives::encode_app_boolean(&mut data_wrong_session, false);
        assert!(VTDataRequest::decode(&data_wrong_session).is_err());

        let mut data_wrong_octets = encode_application_unsigned(tags::app_tag::UNSIGNED, 1);
        primitives::encode_app_character_string(&mut data_wrong_octets, "x").unwrap();
        primitives::encode_app_boolean(&mut data_wrong_octets, false);
        assert!(VTDataRequest::decode(&data_wrong_octets).is_err());

        let mut data_wrong_boolean = encode_application_unsigned(tags::app_tag::UNSIGNED, 1);
        primitives::encode_app_octet_string(&mut data_wrong_boolean, &[0x01]);
        primitives::encode_app_unsigned(&mut data_wrong_boolean, 1);
        assert!(VTDataRequest::decode(&data_wrong_boolean).is_err());
    }

    #[test]
    fn vt_decoders_reject_reserved_application_lvt_forms() {
        assert!(VTOpenRequest::decode(&[0x96, 0x01, 0x00]).is_err());
        assert!(VTOpenAck::decode(&[0x26, 0x01, 0x00]).is_err());
        assert!(VTCloseRequest::decode(&[0x26, 0x01, 0x00]).is_err());

        let mut data = BytesMut::from(&[0x26, 0x01, 0x00][..]);
        primitives::encode_app_octet_string(&mut data, &[0x01]);
        primitives::encode_app_boolean(&mut data, false);
        assert!(VTDataRequest::decode(&data).is_err());

        let mut invalid_boolean = encode_vt_data(1);
        *invalid_boolean.last_mut().unwrap() = 0x12;
        assert!(VTDataRequest::decode(&invalid_boolean).is_err());
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
