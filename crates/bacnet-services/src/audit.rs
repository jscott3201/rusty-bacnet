//! Audit notification and query wire models.
//!
//! These codecs follow the corrected 2020 baseline: ANSI/ASHRAE 135-2020 plus
//! the Errata Summary 2024-04-29 (v1), visually verified from the rendered
//! errata page 3 under its page-1 convention (strikeout = removed, italics =
//! added) and recorded by RB-01. Item 7 (Clause 21.6, printed p. 886)
//! corrects the by-target/by-source `successful-actions-only` fields from
//! BOOLEAN to `BACnetSuccessFilter` at unchanged tags \[7\]/\[4\]; item 8
//! (Clause 21.2.3, printed p. 865) corrects `start-at-sequence-number` from
//! Unsigned32 to Unsigned64 at unchanged optional tag \[2\]. Unsigned values
//! use the library's `u64` implementation limit (1-8 octet canonical forms).

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

pub use bacnet_types::constructed::{
    AuditPropertyReference, BACnetAuditLogDatum, BACnetAuditLogQueryParameters,
    BACnetAuditLogRecord, BACnetAuditLogRecordResult, BACnetAuditNotification,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

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
    /// Audit Log object whose records were queried.
    pub audit_log: ObjectIdentifier,
    /// Zero or more adjacent record results encoded under context tag `[1]`.
    pub records: Vec<BACnetAuditLogRecordResult>,
    /// `true` when no records remain beyond those returned; the codec does not verify it.
    pub no_more_items: bool,
}

/// AuditLogQuery-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditLogQueryRequest {
    /// Audit Log object to query.
    pub audit_log: ObjectIdentifier,
    /// Filter selecting which records to return.
    pub query_parameters: BACnetAuditLogQueryParameters,
    /// Corrected-baseline `Unsigned64` cursor (Errata 2024-04-29 item 8).
    pub start_at_sequence_number: Option<u64>,
    /// Maximum number of records the requester wants returned.
    pub requested_count: u16,
}

impl AuditNotificationRequest {
    /// Encode after validation, leaving `buf` unchanged on failure.
    pub fn try_encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        notification_codec::encode(self, buf)
    }

    /// Encode the request into `buf`; same as `try_encode`.
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

    /// Encode the request into `buf`; same as `try_encode`.
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

    /// Encode the ACK into `buf`; same as `try_encode`.
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

#[cfg(test)]
#[path = "audit/empty_value_tests.rs"]
mod empty_value_tests;

#[cfg(test)]
#[path = "audit/address_bound_tests.rs"]
mod address_bound_tests;
