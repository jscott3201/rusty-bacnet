//! Audit notification and query wire models.
//!
//! These codecs follow the formal Clause 21 field and tag productions in
//! ASHRAE 135-2020 within the library's `u64` Unsigned implementation limit.
//! Clause 13.19 conflicts with them by describing an `Unsigned64` start
//! sequence and a three-state success filter. This model intentionally uses
//! Clause 21's `Unsigned32` and strict mandatory Boolean forms.

pub use bacnet_types::constructed::{
    AuditPropertyReference, BACnetAuditLogDatum, BACnetAuditLogQueryParameters,
    BACnetAuditLogRecord, BACnetAuditLogRecordResult, BACnetAuditNotification,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::PropertyReference;

#[path = "audit/notification_codec.rs"]
mod notification_codec;
#[path = "audit/query_ack_codec.rs"]
mod query_ack_codec;
#[path = "audit/query_codec.rs"]
mod query_codec;

/// Parameters shared by confirmed and unconfirmed AuditNotification.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditNotificationRequest {
    /// One or more notifications, encoded under context tag `[0]`.
    pub notifications: Vec<BACnetAuditNotification>,
}

/// AuditLogQuery-ACK service parameters.
///
/// This wire model does not perform storage queries or infer record ordering,
/// continuity, filtering, or the truth of `no_more_items`.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditLogQueryAck {
    pub audit_log: ObjectIdentifier,
    /// Zero or more adjacent record results encoded under context tag `[1]`.
    pub records: Vec<BACnetAuditLogRecordResult>,
    pub no_more_items: bool,
}

/// Audit-local wire-equivalent of `BACnetPropertyReference`.
///
/// The shared service [`PropertyReference`] predates these codecs and narrows
/// the optional array index to `u32`. Clause 21 defines it as unconstrained
/// Unsigned, so Audit preserves every value supported by the primitive layer.
impl From<PropertyReference> for AuditPropertyReference {
    fn from(value: PropertyReference) -> Self {
        Self {
            property_identifier: value.property_identifier,
            property_array_index: value.property_array_index.map(u64::from),
        }
    }
}

impl TryFrom<AuditPropertyReference> for PropertyReference {
    type Error = Error;

    fn try_from(value: AuditPropertyReference) -> Result<Self, Self::Error> {
        Ok(Self {
            property_identifier: value.property_identifier,
            property_array_index: value
                .property_array_index
                .map(u32::try_from)
                .transpose()
                .map_err(|_| {
                    Error::OutOfRange(
                        "Audit property-array-index exceeds shared PropertyReference u32 limit"
                            .into(),
                    )
                })?,
        })
    }
}

/// AuditLogQuery-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditLogQueryRequest {
    pub audit_log: ObjectIdentifier,
    pub query_parameters: BACnetAuditLogQueryParameters,
    /// Clause 21 constrains this field to `Unsigned32`.
    pub start_at_sequence_number: Option<u32>,
    pub requested_count: u16,
}

fn decode_canonical_unsigned(data: &[u8], offset: usize, field: &str) -> Result<u64, Error> {
    if data.len() > 1 && data.first() == Some(&0) {
        return Err(Error::decoding(
            offset,
            format!("{field} must use the shortest Unsigned/Enumerated encoding"),
        ));
    }
    bacnet_encoding::primitives::decode_unsigned(data)
}

impl AuditNotificationRequest {
    /// Encode after validation, leaving `buf` unchanged on failure.
    pub fn try_encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        notification_codec::encode(self, buf)
    }

    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.try_encode(buf)
    }

    /// Decode a complete payload; trailing bytes are rejected.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        notification_codec::decode(data)
    }
}

impl AuditLogQueryRequest {
    /// Encode after validation, leaving `buf` unchanged on failure.
    pub fn try_encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        query_codec::encode(self, buf)
    }

    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.try_encode(buf)
    }

    /// Decode a complete payload; trailing bytes are rejected.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        query_codec::decode(data)
    }
}

impl AuditLogQueryAck {
    /// Encode after validation, leaving `buf` unchanged on failure.
    pub fn try_encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        query_ack_codec::encode(self, buf)
    }

    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.try_encode(buf)
    }

    /// Decode a complete payload; trailing bytes are rejected.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        query_ack_codec::decode(data)
    }
}

#[cfg(test)]
#[path = "audit/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "audit/malformed_tests.rs"]
mod malformed_tests;
