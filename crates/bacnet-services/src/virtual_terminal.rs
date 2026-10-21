//! Virtual Terminal (VT) services per ASHRAE 135-2020 Clauses 17.2–17.4.
//!
//! Legacy services needed for full spec coverage. The request and VT-Open
//! acknowledgment fields use APPLICATION tags; the VT-Data acknowledgment is
//! the one construct that uses context tags (Clause 21.2.5).

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::VTClass;
use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;
use bacnet_encoding::constructed::tagged::{
    decode_app_enumerated, decode_app_primitive, decode_app_unsigned, decode_ctx_boolean,
    decode_ctx_unsigned, decode_optional_ctx, expect_end, next_is_context,
};

#[path = "virtual_terminal_error.rs"]
mod error;
pub use error::VTCloseError;

// ---------------------------------------------------------------------------
// VTOpenRequest / VTOpenAck
// ---------------------------------------------------------------------------

/// VT-Open-Request service parameters (Clause 17.2.1, Clause 21.2.5).
///
/// Wire form: an application ENUMERATED `vt-class` followed by an application Unsigned8
/// `local-vt-session-identifier`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VTOpenRequest {
    /// Terminal class requested for the session (BACnetVTClass); values beyond the named
    /// constants are carried through unchanged.
    pub vt_class: VTClass,
    /// The requester's own identifier for the new session, in the range 0-255. The responder
    /// quotes it as the session identifier when it sends VT-Data in the other direction.
    pub local_vt_session_identifier: u8,
}

impl VTOpenRequest {
    /// Encode the request parameters into `buf`.
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_enumerated(buf, self.vt_class.to_raw());
        primitives::encode_app_unsigned(buf, u64::from(self.local_vt_session_identifier));
    }

    /// Decode the request from service-request octets; fails on missing, malformed or
    /// truncated fields and on trailing data.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (vt_class, offset) = decode_app_enumerated::<u32>(data, 0, "VTOpen vt-class")?;
        let vt_class = VTClass::from_raw(vt_class);
        let (local_vt_session_identifier, offset) =
            decode_app_unsigned::<u8>(data, offset, "VTOpen local-vt-session-identifier")?;
        expect_end(data, offset, offset, "VTOpen")?;
        Ok(Self {
            vt_class,
            local_vt_session_identifier,
        })
    }
}

/// VT-Open-Ack service parameters.
///
/// `remote_vt_session_identifier` is an APPLICATION-tagged Unsigned8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VTOpenAck {
    /// Identifier the responder assigned to the new session, in the range 0-255; the requester
    /// quotes it in later VT-Data and VT-Close requests.
    pub remote_vt_session_identifier: u8,
}

impl VTOpenAck {
    /// Encode the acknowledgment parameter into `buf`.
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_unsigned(buf, self.remote_vt_session_identifier as u64);
    }

    /// Decode the acknowledgment from its service-ack octets; fails on malformed or truncated
    /// input and on trailing data.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (id, offset) = decode_app_unsigned::<u8>(data, 0, "VTOpenAck session-identifier")?;
        expect_end(data, offset, offset, "VTOpenAck")?;
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
/// Contains a SEQUENCE OF Unsigned8 (APPLICATION tagged). Clause 17.3.1.1.1 requires at least
/// one identifier, so an empty list is rejected on both encode and decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VTCloseRequest {
    /// Session identifiers to terminate, as known to the responding device; must not be empty.
    pub list_of_remote_vt_session_identifiers: Vec<u8>,
}

impl VTCloseRequest {
    /// Encode the request parameters into `buf`; fails if the identifier list is empty.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        if self.list_of_remote_vt_session_identifiers.is_empty() {
            return Err(Error::encode("VTClose requires at least one session identifier"));
        }
        for &id in &self.list_of_remote_vt_session_identifiers {
            primitives::encode_app_unsigned(buf, id as u64);
        }
        Ok(())
    }

    /// Decode the request from service-request octets; fails on malformed or truncated input
    /// and on an empty list.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;
        let mut ids = Vec::new();
        while offset < data.len() {
            if ids.len() >= MAX_DECODED_ITEMS {
                return Err(Error::overflow(offset, "VTClose too many session IDs"));
            }
            let (id, next) = decode_app_unsigned::<u8>(data, offset, "VTClose session-identifier")?;
            ids.push(id);
            offset = next;
        }
        if ids.is_empty() {
            return Err(Error::decoding(
                0,
                "VTClose requires at least one session identifier",
            ));
        }
        Ok(Self {
            list_of_remote_vt_session_identifiers: ids,
        })
    }
}

// ---------------------------------------------------------------------------
// VTDataRequest / VTDataAck
// ---------------------------------------------------------------------------

/// VT-Data-Request service parameters (Clause 17.4.1.1, Clause 21.2.5).
///
/// Wire form: application Unsigned8 session identifier, application OCTET STRING data, and an
/// application Unsigned `vt-data-flag` limited to 0 or 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VTDataRequest {
    /// Session the data belongs to, as known to the responding device.
    pub vt_session_identifier: u8,
    /// Octets of new data for the peer terminal.
    pub vt_new_data: Vec<u8>,
    /// One-bit sequence number, 0 (`false`) or 1 (`true`), that the sender flips on every fresh
    /// VT-Data request within a session so the receiver can spot a repeat. It is sent as an
    /// Unsigned, not as a Boolean.
    pub vt_data_flag: bool,
}

impl VTDataRequest {
    /// Encode the request parameters into `buf`.
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_unsigned(buf, self.vt_session_identifier as u64);
        primitives::encode_app_octet_string(buf, &self.vt_new_data);
        primitives::encode_app_unsigned(buf, u64::from(self.vt_data_flag));
    }

    /// Decode the request from service-request octets; fails on malformed or truncated input,
    /// a flag other than 0 or 1, and trailing data.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (vt_session_identifier, offset) =
            decode_app_unsigned::<u8>(data, 0, "VTData session-identifier")?;
        let (octets, offset) =
            decode_app_primitive(data, offset, tags::app_tag::OCTET_STRING, "VTData new-data")?;
        let vt_new_data = octets.to_vec();
        let flag_offset = offset;
        let (flag, offset) = decode_app_unsigned::<u64>(data, offset, "VTData data-flag")?;
        let vt_data_flag = match flag {
            0 => false,
            1 => true,
            other => {
                return Err(DecodeError::new(
                    DecodingKind::InvalidEncoding,
                    flag_offset,
                    "VTData data-flag {} is outside 0..1",
                )
                .with_value(other)
                .into())
            }
        };
        expect_end(data, offset, offset, "VTData")?;
        Ok(Self {
            vt_session_identifier,
            vt_new_data,
            vt_data_flag,
        })
    }
}

/// VT-Data-ACK service parameters (Clause 17.4.1.2, Clause 21.2.5).
///
/// `all-new-data-accepted` is always present as context tag \[0\]; the `accepted-octet-count`
/// in context tag \[1\] is present exactly when that flag is FALSE. The enum makes the pairing
/// impossible to get wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VTDataAck {
    /// Every octet of the request was accepted (`all-new-data-accepted` TRUE, no count).
    AllAccepted,
    /// Only part of the data was accepted (`all-new-data-accepted` FALSE).
    Partial {
        /// Number of octets of the request's new data that were actually accepted.
        accepted_octet_count: u32,
    },
}

impl VTDataAck {
    /// Whether all of the new data was accepted.
    pub fn all_new_data_accepted(&self) -> bool {
        matches!(self, Self::AllAccepted)
    }

    /// The accepted octet count, present only when the data was partially accepted.
    pub fn accepted_octet_count(&self) -> Option<u32> {
        match self {
            Self::AllAccepted => None,
            Self::Partial {
                accepted_octet_count,
            } => Some(*accepted_octet_count),
        }
    }

    /// Encode the acknowledgment parameters into `buf`.
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_ctx_boolean(buf, 0, self.all_new_data_accepted());
        if let Some(count) = self.accepted_octet_count() {
            primitives::encode_ctx_unsigned(buf, 1, u64::from(count));
        }
    }

    /// Decode the acknowledgment from its service-ack octets; fails when `[0]` is missing or
    /// malformed, when the count is absent for FALSE or present for TRUE, and on trailing data.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        if !next_is_context(data, 0, 0)? {
            return Err(Error::decoding(
                0,
                "VTDataAck missing all-new-data-accepted [0]",
            ));
        }
        let (all_accepted, offset) =
            decode_ctx_boolean(data, 0, 0, "VTDataAck all-new-data-accepted")?;
        let count_offset = offset;
        let (count, offset) = decode_optional_ctx(
            data,
            offset,
            1,
            "VTDataAck accepted-octet-count",
            decode_ctx_unsigned::<u32>,
        )?;
        let ack = match (all_accepted, count) {
            (true, None) => Self::AllAccepted,
            (false, Some(accepted_octet_count)) => Self::Partial {
                accepted_octet_count,
            },
            (true, Some(_)) => {
                return Err(Error::decoding(
                    count_offset,
                    "VTDataAck accepted-octet-count present although all data was accepted",
                ))
            }
            (false, None) => {
                return Err(Error::decoding(
                    count_offset,
                    "VTDataAck accepted-octet-count missing although data was refused",
                ))
            }
        };
        expect_end(data, offset, offset, "VTDataAck")?;
        Ok(ack)
    }
}

#[cfg(test)]
#[path = "virtual_terminal_tests.rs"]
mod tests;
