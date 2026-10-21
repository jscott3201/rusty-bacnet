//! BACnet error types.
//!
//! Provides the top-level [`Error`] type used throughout the library,
//! covering protocol errors, encoding/decoding failures, transport issues,
//! and timeouts.
//!
//! Without `alloc` it keeps only the variants that own nothing: a decoder's
//! [`DecodeError`], an encoder's [`EncodeError`], a constructor's
//! [`RangeError`] and a segmenter's [`SegmentationError`] carry static
//! labels or numbers and build their text when displayed (#1646, #1802).
//! The variants holding a `String` or a `Box` need `alloc`.

#[cfg(all(feature = "alloc", not(feature = "std")))]
use alloc::{boxed::Box, string::String, vec::Vec};
use core::fmt;
use core::time::Duration;

#[cfg(feature = "alloc")]
use crate::constructed::BACnetObjectPropertyReference;
use crate::data_link::DataLink;
#[cfg(feature = "alloc")]
use crate::enums::PropertyIdentifier;
use crate::enums::{AbortReason, ErrorClass, ErrorCode, RejectReason};

mod decode;
mod encode;
pub use decode::{DecodeError, DecodingKind, RangeError};
pub use encode::{EncodeError, SegmentationError};

/// How a protocol error's class and code read: each name in lower case with
/// hyphens, or the raw number for a value without one.
struct ProtocolText {
    class: ErrorClass,
    code: ErrorCode,
}

impl fmt::Display for ProtocolText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn name(
            f: &mut fmt::Formatter<'_>,
            named: Option<&str>,
            field: &str,
            value: u16,
        ) -> fmt::Result {
            let Some(named) = named else {
                return write!(f, "{field}={value}");
            };
            for c in named.chars() {
                let c = if c == '_' {
                    '-'
                } else {
                    c.to_ascii_lowercase()
                };
                fmt::Write::write_char(f, c)?;
            }
            Ok(())
        }
        f.write_str("BACnet error: ")?;
        name(f, self.class.name(), "class", self.class.to_raw())?;
        f.write_str(" / ")?;
        name(f, self.code.name(), "code", self.code.to_raw())
    }
}

/// [`Error::LogNotAdvancing`]'s text.
struct NotAdvancing {
    requested: u64,
    returned: Option<u64>,
}

impl fmt::Display for NotAdvancing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let requested = self.requested;
        match self.returned {
            Some(returned) => write!(
                f,
                "asked for sequence number {requested}, the device answered from {returned}"
            ),
            None => write!(
                f,
                "asked for sequence number {requested}, which the log's counts say it holds, \
                 the device answered with no records"
            ),
        }
    }
}

/// Top-level error type for the BACnet library.
///
/// It is `#[non_exhaustive]`, so a `match` on it outside this crate needs a
/// wildcard arm. Some variants exist only with a feature, and any crate in a
/// build can turn that on: `Error::Transport` needs `std`, and the variants
/// that hold a `String` or a `Box` need `alloc`. Without the attribute a
/// match written against a smaller build would stop compiling (#1604).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
// Clippy fails if the attribute above goes.
#[deny(clippy::exhaustive_enums)]
pub enum Error {
    /// BACnet protocol error response (Clause 20.1.7).
    #[error("{}", ProtocolText { class: *.class, code: *.code })]
    Protocol {
        /// Error class (Clause 18).
        class: ErrorClass,
        /// Error code (Clause 18).
        code: ErrorCode,
    },

    /// BACnet error response whose Clause 21 production carries more than the
    /// class and code: ChangeList-Error, CreateObject-Error,
    /// WritePropertyMultiple-Error, SubscribeCOVPropertyMultiple-Error,
    /// ConfirmedPrivateTransfer-Error or VTClose-Error. `detail` holds the
    /// extra fields; a body whose optional fields are all absent is reported
    /// as [`Error::Protocol`]. Only with `alloc`.
    #[cfg(feature = "alloc")]
    #[error("{}, {detail}", ProtocolText { class: *.class, code: *.code })]
    Structured {
        /// Error class (Clause 18).
        class: ErrorClass,
        /// Error code (Clause 18).
        code: ErrorCode,
        /// The production's fields after the class and code. Boxed so the
        /// rare structured error does not widen every `Result<_, Error>`.
        detail: Box<ErrorDetail>,
    },

    /// BACnet reject PDU (Clause 20.1.5).
    #[error("BACnet reject: reason={}", reason.to_raw())]
    Reject {
        /// Reject reason (Clause 18.9).
        reason: RejectReason,
    },

    /// BACnet abort PDU (Clause 20.1.6).
    #[error("BACnet abort: reason={}", reason.to_raw())]
    Abort {
        /// Abort reason (Clause 18.10).
        reason: AbortReason,
    },

    /// A router reported that the active message was too long for a routed path.
    #[error("message is too long for routed path to DNET {dnet}")]
    RoutedPathTooLong {
        /// Destination network rejected by the router.
        dnet: u16,
    },

    /// The client cannot safely allocate state for another routed path.
    #[error("routed path state capacity of {capacity} entries is exhausted")]
    RoutedPathCapacityExceeded {
        /// Maximum number of immediate-router/DNET path entries retained.
        capacity: usize,
    },

    /// The endpoint's transport cannot carry the requested operation, such as
    /// a BBMD management request through a data link other than BACnet/IP.
    /// Nothing was sent.
    #[error("operation requires {required}; this transport is {actual}")]
    UnsupportedTransport {
        /// Data link the operation needs.
        required: DataLink,
        /// Data link the endpoint's transport carries.
        actual: DataLink,
    },

    /// A definite length refusal before any bytes of this send were written
    /// or admitted to a transport's write queue. The limit may apply to the
    /// NPDU or its full link envelope, including data attributes. An I/O
    /// failure, including an uncertain partial write, must not use this.
    #[error("send refused before transmission: {length} octets exceed the {limit}-octet limit")]
    SendTooLong {
        /// Length of the NPDU or encoded link frame that was refused.
        length: usize,
        /// The corresponding bound when this send was refused.
        limit: usize,
    },

    /// Error encoding a PDU, with a formatted message. Only with `alloc`;
    /// [`Error::Encode`] is the static form, which the codec crates return.
    #[cfg(feature = "alloc")]
    #[error("encoding error: {0}")]
    Encoding(String),

    /// A value an encoder or validator refused, with a static label
    /// (#1802). The encoders and validators of `bacnet-encoding` and
    /// `bacnet-services` report this form.
    #[error("{0}")]
    Encode(EncodeError),

    /// Error decoding received data, with a static label and its numbers
    /// (#1646, #1776). Every decoder reports this form.
    #[error("{0}")]
    Decoding(DecodeError),

    /// Transport-level I/O error. Only with the `std` feature, which has
    /// `std::io`.
    #[cfg(feature = "std")]
    #[error("transport error: {0}")]
    Transport(#[from] std::io::Error),

    /// Request timed out.
    #[error("request timed out after {0:?}")]
    Timeout(Duration),

    /// A payload that can't be segmented, segments that can't be
    /// reassembled, or a segmented request a peer can't take (#1802).
    #[error("segmentation error: {0}")]
    Segmentation(SegmentationError),

    /// Buffer too short for the expected data.
    #[error("buffer too short: need {need} bytes, have {have}")]
    BufferTooShort {
        /// Minimum bytes needed.
        need: usize,
        /// Bytes actually available.
        have: usize,
    },

    /// Value out of valid range, with a formatted message. Only with
    /// `alloc`; [`Error::Range`] is the static form.
    #[cfg(feature = "alloc")]
    #[error("value out of range: {0}")]
    OutOfRange(String),

    /// A value a checked constructor refused, with a static label (#1646),
    /// such as an object type or instance
    /// [`ObjectIdentifier::new`](crate::primitives::ObjectIdentifier::new)
    /// can't encode.
    #[error("{0}")]
    Range(RangeError),

    /// A well-formed ReadRange acknowledgement that breaks a rule the client
    /// checks against the request it answers, so the client refused it. The
    /// acknowledgement decoded; a malformed one is [`Error::Decoding`]
    /// instead. Read again leniently to keep its items.
    #[error("ReadRange acknowledgement refused: {0}")]
    ReadRangeViolation(ReadRangeViolation),

    /// A paged log read asked for the records from one sequence number and
    /// the device answered with a page starting before it, so reading on
    /// would repeat records instead of advancing; or answered with none
    /// although its record counts say it holds that record. Some devices
    /// clamp a reference past their sequence wrap back to the oldest record,
    /// or number the record after the wrap 0; read such a log by position
    /// instead.
    #[error(
        "log read is not advancing: {}",
        NotAdvancing { requested: *.requested, returned: *.returned }
    )]
    LogNotAdvancing {
        /// The sequence number the read asked for.
        requested: u64,
        /// What the device numbered the first record it sent;
        /// `None` when it returned no records.
        returned: Option<u64>,
    },
}

impl From<DecodeError> for Error {
    fn from(error: DecodeError) -> Self {
        Self::Decoding(error)
    }
}

impl From<EncodeError> for Error {
    fn from(error: EncodeError) -> Self {
        Self::Encode(error)
    }
}

impl From<SegmentationError> for Error {
    fn from(error: SegmentationError) -> Self {
        Self::Segmentation(error)
    }
}

impl From<RangeError> for Error {
    fn from(error: RangeError) -> Self {
        Self::Range(error)
    }
}

/// A rule a ReadRange acknowledgement can break against the request it
/// answers (Clause 15.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadRangeViolation {
    /// The object identifier isn't the one the request named.
    ObjectMismatch,
    /// The property identifier isn't the one the request named.
    PropertyMismatch,
    /// The array index isn't the one the request named, or is present when
    /// the request had none, or the reverse.
    ArrayIndexMismatch,
    /// A nonempty answer to a By-Sequence or By-Time read without the
    /// sequence number of its first item (Clause 15.8.1.2.7).
    MissingFirstSequenceNumber,
    /// A nonempty answer to a By-Sequence or By-Time read that numbers its
    /// first item 0, a number logs never assign: they count from 1 and skip
    /// 0 when they wrap. Some devices do number the record after the wrap 0.
    ZeroFirstSequenceNumber,
    /// A first sequence number in the answer to a read that was by position
    /// or had no range, which carries none (Clause 15.8.1.2.7).
    UnexpectedFirstSequenceNumber,
    /// MORE_ITEMS together with the flag that says the page reached the end
    /// a ranged read moves toward: LAST_ITEM for a forward read, FIRST_ITEM
    /// for a backward one. Items left out of a full page lie beyond that
    /// end, so the two contradict each other (Clause 15.8.2). A read with no
    /// range may keep either end, so for it only all three flags together
    /// break the rule.
    MoreItemsPastEnd,
    /// More items than the request's count asked for.
    ItemCountExceedsRequest,
}

impl ReadRangeViolation {
    /// The rule's name in snake case, as the Python bindings report it.
    pub fn name(self) -> &'static str {
        match self {
            Self::ObjectMismatch => "object_mismatch",
            Self::PropertyMismatch => "property_mismatch",
            Self::ArrayIndexMismatch => "array_index_mismatch",
            Self::MissingFirstSequenceNumber => "missing_first_sequence_number",
            Self::ZeroFirstSequenceNumber => "zero_first_sequence_number",
            Self::UnexpectedFirstSequenceNumber => "unexpected_first_sequence_number",
            Self::MoreItemsPastEnd => "more_items_past_end",
            Self::ItemCountExceedsRequest => "item_count_exceeds_request",
        }
    }
}

impl core::fmt::Display for ReadRangeViolation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::ObjectMismatch => "object identifier does not match the request",
            Self::PropertyMismatch => "property identifier does not match the request",
            Self::ArrayIndexMismatch => "array index does not match the request",
            Self::MissingFirstSequenceNumber => {
                "nonempty By Sequence/Time answer has no first sequence number"
            }
            Self::ZeroFirstSequenceNumber => {
                "nonempty By Sequence/Time answer has first sequence number 0"
            }
            Self::UnexpectedFirstSequenceNumber => {
                "answer to a read by position or without a range has a first sequence number"
            }
            Self::MoreItemsPastEnd => {
                "MORE_ITEMS is set with the flag for the end a ranged read moves toward, \
                 or with both FIRST_ITEM and LAST_ITEM on a read without a range"
            }
            Self::ItemCountExceedsRequest => "more items than the request's count",
        })
    }
}

/// The fields a Clause 21 error production adds to the error class and code,
/// one variant per shape. The service the error answers names the production.
/// Only with `alloc`.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorDetail {
    /// ChangeList-Error (AddListElement, RemoveListElement; Clauses 15.1.1.3
    /// and 15.2.1.3) and CreateObject-Error (Clause 15.3.1.3): the 1-based
    /// position of the refused element in the request's List of Elements or
    /// List of Initial Values, or 0 when the failure is not about one element.
    /// An object refusing one element of a list-valued write uses it too, for
    /// that element's position in the list it was given; the server's list
    /// handler turns that into the request's position.
    FirstFailedElementNumber(u32),
    /// WritePropertyMultiple-Error (Clause 15.10.1.3): the object, property
    /// and array index of the first write that failed.
    FirstFailedWriteAttempt(BACnetObjectPropertyReference),
    /// SubscribeCOVPropertyMultiple-Error (Clause 13.16.1.3) in its
    /// first-failed-subscription form: the monitored object and the property
    /// reference of the COV reference that failed. The other form, a general
    /// error, has no detail.
    FirstFailedSubscription(BACnetObjectPropertyReference),
    /// ConfirmedPrivateTransfer-Error (Clause 16.2.1.3): the vendor and
    /// service the error answers, and any vendor-defined error parameters.
    PrivateTransfer {
        /// Vendor identifier of the private service.
        vendor_id: u32,
        /// Vendor-defined service number.
        service_number: u32,
        /// Error parameters as encoded inside their context tag 3 frame,
        /// opaque to the stack; `None` when absent.
        error_parameters: Option<Vec<u8>>,
    },
    /// VTClose-Error (Clause 17.3.1.3) with its list present: the
    /// requester's local identifiers of the VT sessions that could not be
    /// closed. Without the list the error has no detail.
    VtSessionIdentifiers(Vec<u8>),
}

#[cfg(feature = "alloc")]
impl core::fmt::Display for ErrorDetail {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        fn reference(
            f: &mut core::fmt::Formatter<'_>,
            what: &str,
            reference: &BACnetObjectPropertyReference,
        ) -> core::fmt::Result {
            write!(
                f,
                "{what} {} {}",
                reference.object_identifier,
                PropertyIdentifier::from_raw(reference.property_identifier)
            )?;
            match reference.property_array_index {
                Some(index) => write!(f, "[{index}]"),
                None => Ok(()),
            }
        }
        match self {
            Self::FirstFailedElementNumber(number) => write!(f, "first failed element {number}"),
            Self::FirstFailedWriteAttempt(attempt) => {
                reference(f, "first failed write attempt", attempt)
            }
            Self::FirstFailedSubscription(subscription) => {
                reference(f, "first failed subscription", subscription)
            }
            Self::PrivateTransfer {
                vendor_id,
                service_number,
                error_parameters,
            } => {
                write!(f, "vendor {vendor_id} service {service_number}")?;
                match error_parameters {
                    Some(parameters) => {
                        write!(f, ", {} octets of error parameters", parameters.len())
                    }
                    None => Ok(()),
                }
            }
            Self::VtSessionIdentifiers(sessions) => {
                f.write_str("VT sessions not closed:")?;
                for session in sessions {
                    write!(f, " {session}")?;
                }
                Ok(())
            }
        }
    }
}

/// Convenience alias for `Result<T, Error>`.
pub type Result<T> = core::result::Result<T, Error>;

impl Error {
    /// The error an Error PDU reports: [`Error::Structured`] when its body
    /// carried a detail, [`Error::Protocol`] otherwise. Only with `alloc`.
    #[cfg(feature = "alloc")]
    pub fn protocol(class: ErrorClass, code: ErrorCode, detail: Option<ErrorDetail>) -> Self {
        match detail {
            Some(detail) => Self::Structured {
                class,
                code,
                detail: Box::new(detail),
            },
            None => Self::Protocol { class, code },
        }
    }

    /// Create a [`DecodingKind::InvalidEncoding`] decoding error at the given
    /// byte offset: contents whose encoding isn't valid for their datatype.
    /// Build a [`DecodeError`] directly for a label that names a context or
    /// a number.
    pub fn decoding(offset: usize, label: &'static str) -> Self {
        Self::decoding_kind(DecodingKind::InvalidEncoding, offset, label)
    }

    /// Create an [`Error::Decoding`] of `kind` at the given byte offset,
    /// described by a static `label`.
    pub fn decoding_kind(kind: DecodingKind, offset: usize, label: &'static str) -> Self {
        Self::Decoding(DecodeError::new(kind, offset, label))
    }

    /// Create a [`DecodingKind::InvalidTag`] decoding error: a tag at
    /// `offset` that doesn't fit there.
    pub fn invalid_tag(offset: usize, label: &'static str) -> Self {
        Self::decoding_kind(DecodingKind::InvalidTag, offset, label)
    }

    /// Create a [`DecodingKind::Missing`] decoding error: a required member
    /// due at `offset` isn't there.
    pub fn missing(offset: usize, label: &'static str) -> Self {
        Self::decoding_kind(DecodingKind::Missing, offset, label)
    }

    /// Create a [`DecodingKind::Trailing`] decoding error: octets from
    /// `offset` on that the value leaves unread.
    pub fn trailing(offset: usize, label: &'static str) -> Self {
        Self::decoding_kind(DecodingKind::Trailing, offset, label)
    }

    /// Create a [`DecodingKind::OutOfRange`] decoding error: a value at
    /// `offset` too large for its field, or outside its range.
    pub fn out_of_range(offset: usize, label: &'static str) -> Self {
        Self::decoding_kind(DecodingKind::OutOfRange, offset, label)
    }

    /// Create a [`DecodingKind::Overflow`] decoding error: more items, or a
    /// longer tag, than the decoder takes.
    pub fn overflow(offset: usize, label: &'static str) -> Self {
        Self::decoding_kind(DecodingKind::Overflow, offset, label)
    }

    /// This error, an inner reader's refusal, as a static [`DecodeError`] at
    /// `offset`, for a decoder that reports it as its own fault there and
    /// names its own context or item (#1776). A decoding error keeps its
    /// kind, context, label, values and index; a range error is
    /// [`DecodingKind::OutOfRange`] with its label and values; a short buffer
    /// is [`DecodingKind::Missing`] with its sizes. Any other error, which a
    /// decoder doesn't return, is an [`DecodingKind::InvalidEncoding`]
    /// labelled `malformed`.
    pub fn to_decode_error(&self, offset: usize) -> DecodeError {
        let mut refused = match self {
            Self::Decoding(refused) => *refused,
            Self::Range(refused) => {
                let mut range = DecodeError::new(DecodingKind::OutOfRange, offset, refused.label);
                range.values = refused.values;
                range
            }
            Self::BufferTooShort { need, have } => DecodeError::new(
                DecodingKind::Missing,
                offset,
                "buffer too short: need {} bytes, have {}",
            )
            .with_values(*need as u64, *have as u64),
            _ => DecodeError::new(DecodingKind::InvalidEncoding, offset, "malformed"),
        };
        refused.offset = offset;
        refused
    }

    /// The offset and kind of an [`Error::Decoding`]; `None` for any other
    /// error.
    pub fn decode_fault(&self) -> Option<(usize, DecodingKind)> {
        match self {
            Self::Decoding(error) => Some((error.offset, error.kind)),
            _ => None,
        }
    }

    /// The Reject reason a responder answers a confirmed request with when
    /// this error refused it while decoding its parameters (Clauses 18.9 and
    /// 20.1.8): the reason itself for [`Error::Reject`], the kind's reason
    /// for a decoding error (the table on
    /// [`DecodingKind::reject_reason`]), and MISSING_REQUIRED_PARAMETER for
    /// [`Error::BufferTooShort`], a member cut short because the data ran out
    /// before it was complete. `None` for any other error, which no decoder
    /// of the request's parameters reports.
    pub fn reject_reason(&self) -> Option<RejectReason> {
        match self {
            Self::Reject { reason } => Some(*reason),
            Self::BufferTooShort { .. } => Some(RejectReason::MISSING_REQUIRED_PARAMETER),
            _ => self.decode_fault().map(|(_, kind)| kind.reject_reason()),
        }
    }

    /// This error, met decoding a confirmed request's own parameters, as the
    /// [`Error::Reject`] a responder answers with: the
    /// [`reject_reason`](Self::reject_reason) when it is a syntax fault, the
    /// error unchanged otherwise.
    ///
    /// A responder converts only the request's decode with this. A decoding
    /// error met later, once the service has started to run, is no reason to
    /// reject the request (Clause 20.1.8), so it stays as it is.
    pub fn into_request_reject(self) -> Self {
        match self.reject_reason() {
            Some(reason) => Self::Reject { reason },
            None => self,
        }
    }

    /// Create an [`Error::Encode`] described by a static `label`. Build an
    /// [`EncodeError`] directly for a label that names a context or a
    /// number.
    pub fn encode(label: &'static str) -> Self {
        Self::Encode(EncodeError::new(label))
    }

    /// Create an [`Error::Range`] described by a static `label`. Build a
    /// [`RangeError`] directly for a label with numbers.
    pub fn range(label: &'static str) -> Self {
        Self::Range(RangeError::new(label))
    }

    /// This error, met checking octets an encoder was handed to copy onto
    /// the wire, such as a captured raw value, as the encoder's own
    /// [`Error::Encode`] (#1802). A decoder's refusal keeps its offset (see
    /// [`EncodeError::from_decode`]); a short buffer or another refusal keeps
    /// the label and numbers [`to_decode_error`](Self::to_decode_error) gives
    /// it, without an offset; an [`Error::Encode`] stays as it is.
    pub fn into_encode_refusal(self) -> Self {
        match self {
            Self::Encode(_) => self,
            Self::Decoding(refused) => Self::Encode(EncodeError::from_decode(&refused)),
            other => Self::Encode(EncodeError::restating(&other.to_decode_error(0))),
        }
    }

    /// Create a buffer-too-short error.
    pub fn buffer_too_short(need: usize, have: usize) -> Self {
        Self::BufferTooShort { need, have }
    }
}

#[cfg(test)]
mod tests;
