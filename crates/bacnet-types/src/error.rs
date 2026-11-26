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
//! The local refusals ([`Error::State`], [`Error::InvalidConfig`],
//! [`Error::InvalidArgument`], [`Error::Unsupported`] and
//! [`Error::LimitReached`], #1833) are there too, with a static
//! [`Refusal`] whose owned text needs `std`, and so is persisted state
//! ([`Error::Persistence`]), whose [`PersistenceError`] owns context text
//! only with `std`. A
//! structured error's detail needs `alloc`; the variants for I/O and a
//! client's lookups need `std`. There is no variant for text alone.

#[cfg(all(feature = "alloc", not(feature = "std")))]
use alloc::{boxed::Box, vec::Vec};
use core::fmt;
use core::time::Duration;

#[cfg(feature = "std")]
use crate::constructed::BACnetAddress;
#[cfg(feature = "alloc")]
use crate::constructed::BACnetObjectPropertyReference;
use crate::data_link::DataLink;
#[cfg(feature = "alloc")]
use crate::enums::PropertyIdentifier;
use crate::enums::{AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, RejectReason};

mod decode;
mod encode;
#[cfg(feature = "std")]
mod network_reject;
mod persistence;
mod refusal;
mod remote;
pub use decode::{DecodeError, DecodingKind, RangeError};
pub use encode::{EncodeError, SegmentationError};
#[cfg(feature = "std")]
pub use network_reject::NetworkReject;
pub use persistence::{PersistenceError, PersistenceErrorKind};
pub use refusal::{Lifecycle, Refusal};
#[cfg(feature = "alloc")]
use remote::ProtocolText;
pub use remote::RemoteError;

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

/// [`Error::UnexpectedAck`]'s text.
struct WrongAck {
    service: ConfirmedServiceChoice,
    complex: bool,
}

impl fmt::Display for WrongAck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (came, takes) = if self.complex {
            ("a Complex-ACK", "a Simple-ACK, with no data")
        } else {
            ("a Simple-ACK", "a Complex-ACK carrying its result")
        };
        f.write_str("the device acknowledged ")?;
        remote::write_name(
            f,
            self.service.name(),
            "service",
            u16::from(self.service.to_raw()),
        )?;
        write!(
            f,
            " with {came}, but that service is acknowledged with {takes}; whether it \
             carried out the request is unknown"
        )
    }
}

/// [`Error::InitiationDisabled`]'s text.
struct Initiation {
    sent: bool,
}

impl fmt::Display for Initiation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.sent {
            f.write_str(
                "no retry sent: DeviceCommunicationControl disabled initiation on this device \
                 after the request went out, and no answer came",
            )
        } else {
            f.write_str(
                "not sent: DeviceCommunicationControl has disabled initiation on this device \
                 until it is enabled again",
            )
        }
    }
}

/// [`Error::RequestCutOff`]'s text, which says whether the device may
/// have carried the request out.
struct CutOff {
    sent: bool,
}

impl fmt::Display for CutOff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "the request ended without an answer: the client stopped, or its link stopped or \
             lost its connection, ",
        )?;
        if self.sent {
            f.write_str("after it went out, and the device may have carried it out")
        } else {
            f.write_str("before it was sent: the device did not get it")
        }
    }
}

/// [`Error::AmbiguousDevice`]'s addresses, in the CLI's target grammar where
/// it has a form for one: a routed station as `2:0x05`; on the local
/// network, by the MAC's length, which names its link among this stack's
/// transports: `mstp:5` (MS/TP), `bip6:0x0004D2` (a B/IPv6 VMAC), and
/// six octets as `10.0.0.1:47808`, the same six octets for an SC VMAC or an
/// Ethernet MAC too. Those read back as the same station. A local MAC of any
/// other length, which none of this stack's transports uses, is written as
/// `0x` and hex, which the grammar refuses.
#[cfg(feature = "std")]
struct Candidates<'a>(&'a [BACnetAddress]);

#[cfg(feature = "std")]
impl fmt::Display for Candidates<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hex = |f: &mut fmt::Formatter<'_>, mac: &[u8]| {
            f.write_str("0x")?;
            mac.iter().try_for_each(|octet| write!(f, "{octet:02X}"))
        };
        for (i, address) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            let mac = address.mac_address.as_slice();
            match (address.network_number, mac) {
                (0, [station]) => write!(f, "mstp:{station}")?,
                (0, [_, _, _]) => {
                    f.write_str("bip6:")?;
                    hex(f, mac)?;
                }
                (0, [a, b, c, d, hi, lo]) => {
                    let ip = std::net::Ipv4Addr::new(*a, *b, *c, *d);
                    write!(f, "{ip}:{}", u16::from_be_bytes([*hi, *lo]))?;
                }
                (0, _) => hex(f, mac)?,
                (network, _) => {
                    write!(f, "{network}:")?;
                    hex(f, mac)?;
                }
            }
        }
        Ok(())
    }
}

/// Top-level error type for the BACnet library.
///
/// It is `#[non_exhaustive]`, so a `match` on it outside this crate needs a
/// wildcard arm. Some variants exist only with a feature, and any crate in a
/// build can turn that on: `Error::Transport`, `Error::NetworkRejected`,
/// `Error::RouterBusyTimeout`, `Error::AmbiguousDevice` and
/// `Error::UnknownDevice` need `std`, and `Error::Structured` needs
/// `alloc`. Without the attribute a match written against a smaller build
/// would stop compiling (#1604).
///
/// # Matching
///
/// A caller tells errors apart by variant, never by text (#1833): every
/// error's text is for a person to read and may change, and a number or
/// fact a caller acts on has a typed variant or field of its own, as
/// [`Error::SendTooLong`] carries its length and limit. The variants fall
/// into these groups:
///
/// - A peer's answer: [`Error::Remote`] and `Error::Structured` (an Error,
///   Reject or Abort PDU; [`Error::remote`] gives either as a
///   [`RemoteError`]), a router's refusal on the way
///   ([`Error::RoutedPathTooLong`], `Error::NetworkRejected`), and an
///   acknowledgement of the wrong kind ([`Error::UnexpectedAck`]).
/// - No answer: [`Error::Timeout`] and `Error::RouterBusyTimeout`; a
///   request its client or link stopped under, [`Error::RequestCutOff`];
///   one DeviceCommunicationControl held back, [`Error::InitiationDisabled`].
///   The last two say whether the request may have gone out.
/// - This side's refusal, with nothing sent: [`Error::State`] with its
///   [`Lifecycle`], [`Error::InvalidConfig`], [`Error::InvalidArgument`],
///   [`Error::Unsupported`] and [`Error::LimitReached`], each with a
///   [`Refusal`]; and the ones that carry what a caller acts on:
///   [`Error::SendTooLong`], [`Error::UnsupportedTransport`],
///   [`Error::RoutedPathCapacityExceeded`], `Error::AmbiguousDevice` and
///   `Error::UnknownDevice`.
/// - A codec's refusal: [`Error::Encode`], [`Error::Decoding`] (with its
///   offset and [`DecodingKind`]), [`Error::Range`],
///   [`Error::Segmentation`] and [`Error::BufferTooShort`]; and a
///   well-formed answer that breaks a rule the client checks:
///   [`Error::ReadRangeViolation`], [`Error::LogNotAdvancing`].
/// - I/O the operating system reported, with its error as the source:
///   `Error::Transport`. That is any OS I/O: a socket's, a serial port's,
///   and a storage file's, which the stack's own file backends return
///   this way.
/// - Persisted state: [`Error::Persistence`], for stored octets that are
///   damaged, belong to another object or a version this build doesn't
///   read, or were refused as they were encoded, and for a storage backend
///   that failed with no OS error behind it. Its
///   [`PersistenceError::kind`] and [`PersistenceError::is_damage`] say
///   which.
///
/// What a component is built or started with (a value given to a
/// builder's setter, an option, a table it is configured with, or a value
/// made to configure it) that it refuses is [`Error::InvalidConfig`],
/// wherever the check runs, even when the setter is the call that hands the
/// value over. A value refused at the call that hands it to a component is
/// [`Error::InvalidArgument`], even when the component would keep it (a
/// device added to the client's table, a routed path's limit, a server's new
/// clock zone), and so is text parsed once. There is no variant for text
/// alone: an application's own code that returns this error (a storage
/// backend, a hook, a transport) picks the variant for what failed, such as
/// [`PersistenceError::new`] for its storage or a local refusal with
/// `Refusal::text` (with `std`) where it names a run-time value.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
// Clippy fails if the attribute above goes.
#[deny(clippy::exhaustive_enums)]
pub enum Error {
    /// A peer's Error, Reject or Abort PDU that turned a confirmed request
    /// down (Clauses 20.1.5 to 20.1.7). An Error PDU whose Clause 21
    /// production carries more than the class and code is
    /// `Error::Structured` (with `alloc`) instead.
    #[error("{0}")]
    Remote(RemoteError),

    /// BACnet error response whose Clause 21 production carries more than the
    /// class and code: ChangeList-Error, CreateObject-Error,
    /// WritePropertyMultiple-Error, SubscribeCOVPropertyMultiple-Error,
    /// ConfirmedPrivateTransfer-Error or VTClose-Error. `detail` holds the
    /// extra fields; a body whose optional fields are all absent is reported
    /// as [`Error::Remote`]. Only with `alloc`.
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

    /// A router reported that the active message was too long for a routed path.
    #[error("message is too long for routed path to DNET {dnet}")]
    RoutedPathTooLong {
        /// Destination network rejected by the router.
        dnet: u16,
    },

    /// A router the request went through refused it at the network layer
    /// by sending back a Reject-Message-To-Network (Clause 6.4.4), so no device
    /// answered (#2724). The client ends a routed confirmed request this
    /// way at once for reason 1 (no route to the network), 6 (an address
    /// length the router found invalid), 0 (other), 5 (a security refusal
    /// in earlier revisions) and any reason the standard doesn't define.
    /// A busy router (reason 2) leaves the request to its retries
    /// ([`Error::RouterBusyTimeout`] when they run out), reason 3 answers a
    /// network message and never a request, and reason 4 is
    /// [`Error::RoutedPathTooLong`]. Whether the device can have received
    /// the request is [`NetworkReject::single_frame`]. Only with `std`,
    /// which the client needs; the reject is boxed so the rare refusal
    /// widens no `Result<_, Error>` (on a 32-bit target the error stays 48
    /// octets).
    #[cfg(feature = "std")]
    #[error("{0}")]
    NetworkRejected(Box<NetworkReject>),

    /// A routed confirmed request ran out of retries with no answer after
    /// the router it went through said, with a reject of reason 2, that it
    /// was busy for the station's network (#2724). It is a timeout as the
    /// client's TSM_TIMEOUT abort is, and the low-level crates' and the
    /// bindings' timeout handling treat it as one; the reject names the
    /// router and the network. Whether the device received the request is
    /// unknown, as after any timeout. Only with `std`, boxed as
    /// [`Error::NetworkRejected`] is.
    #[cfg(feature = "std")]
    #[error("BACnet abort: tsm-timeout; on the way, {}", .0.what())]
    RouterBusyTimeout(Box<NetworkReject>),

    /// The device acknowledged a confirmed request with the other kind of
    /// acknowledgement than its service takes (#2721): a Complex-ACK for a
    /// service whose result carries no data, such as WriteProperty, or a
    /// Simple-ACK for one whose result does, such as ReadProperty (Clause
    /// 21 lists the services with a Complex-ACK). The transaction ended
    /// there, as any final acknowledgement ends it, so the client retries
    /// nothing and sends the device nothing more; whether the device
    /// carried out the request is unknown. A service the standard doesn't
    /// define, or one it removed, takes either.
    #[error("{}", WrongAck { service: *.service, complex: *.complex })]
    UnexpectedAck {
        /// The service the request asked for.
        service: ConfirmedServiceChoice,
        /// `true`: a Complex-ACK came for a service acknowledged with a
        /// Simple-ACK; `false`: a Simple-ACK came for one acknowledged with
        /// a Complex-ACK.
        complex: bool,
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

    /// A call its component's lifecycle state turns down (#1833), such as
    /// a second start, a send on a transport that isn't started or has
    /// stopped, or a send on a BACnet/SC link whose hub connection isn't
    /// up. Nothing was done. The [`Lifecycle`] says which state; the
    /// [`Refusal`] is text for a person.
    #[error("{1}")]
    State(Lifecycle, Refusal),

    /// What the component is built or started with, refused (#1833): a
    /// builder setting, an option, a table it is configured with or a value
    /// made to configure it, such as a timeout outside its range or two
    /// modes that can't run together.
    #[error("{0}")]
    InvalidConfig(Refusal),

    /// A value refused at the call that hands it over, or text parsed once,
    /// before anything was sent (#1833), such as an address or MAC of the
    /// wrong form for the link, or a send's options the link can't carry.
    /// It is so even when the component would keep the value, such as a
    /// device added to the client's table.
    #[error("{0}")]
    InvalidArgument(Refusal),

    /// What was asked isn't offered here (#1833): not on this platform, not
    /// in this build, or not by this kind of component. A data link that
    /// can't carry an operation is [`Error::UnsupportedTransport`], which
    /// names both links.
    #[error("{0}")]
    Unsupported(Refusal),

    /// A resource cap was reached (#1833): a table is full, a counter has
    /// run out, a configuration asks for more than the cap allows, or a
    /// resource one holder takes at a time is held by another.
    #[error("{0}")]
    LimitReached(Refusal),

    /// Persisted state that couldn't be loaded or saved (#1833): stored
    /// octets that are damaged or cut short, a sound snapshot saved for
    /// another object or by a build this one can't read, a snapshot refused
    /// as it was encoded, or a storage backend that failed with no
    /// operating-system error behind it (an OS I/O failure, a storage
    /// file's included, is `Error::Transport`). The [`PersistenceError`]
    /// says which by its [`kind`](PersistenceError::kind), and
    /// [`is_damage`](PersistenceError::is_damage) says whether a backend
    /// that keeps two copies should pass over this one for the other. Its
    /// own text is static, so it exists in every build; with `std`,
    /// `Error::with_context` can put a path or other context in front.
    #[error("{0}")]
    Persistence(PersistenceError),

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

    /// Request timed out: its peer gave no answer within the wait. A send
    /// that this side's own link didn't carry before a deadline is an
    /// `Error::Transport` of kind `TimedOut` instead (#1833).
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

    /// A request named a device by its instance, and two or more addresses
    /// in the client's discovery table claim that instance, so which device
    /// is meant is unknown. Nothing was sent. Name one of `candidates` by
    /// its address instead. Only with `std`, which the client needs: a
    /// firmware image built with `alloc` alone links none of it.
    #[cfg(feature = "std")]
    #[error(
        "device {instance} is claimed by {} addresses: {}; name one by its address",
        candidates.len(),
        Candidates(candidates)
    )]
    AmbiguousDevice {
        /// The Device instance the request named.
        instance: u32,
        /// Every address that claims it, ordered by network number and then
        /// MAC: network 0 is the local network. A boxed slice, not a `Vec`,
        /// so the variant lends `Error` no niche to keep its tag in.
        candidates: Box<[BACnetAddress]>,
    },

    /// A request named a device that can't be placed: no address in the
    /// client's discovery table claims the instance, and none answered a
    /// Who-Is for it, or no router is known for a remote network's station,
    /// as when a server's audit notification goes to a recipient whose
    /// router it hasn't learned. Nothing was sent to the device. Only with
    /// `std`, as for [`Error::AmbiguousDevice`].
    #[cfg(feature = "std")]
    #[error("device {device} is not known to the client; discover it first")]
    UnknownDevice {
        /// The device as the request named it: an instance (`1234`,
        /// `2:1234`) or an address (`2:0x05`).
        device: String,
    },

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

    /// The device this request leaves from has had initiation turned off by
    /// DeviceCommunicationControl (DISABLE_INITIATION, Clause 16.1). It ends
    /// when the control's duration runs out, a DeviceCommunicationControl
    /// ENABLE is accepted, or a WARMSTART or COLDSTART ReinitializeDevice
    /// is. A full client on a node with a full server returns it for every
    /// request and unconfirmed send while the node's server holds that
    /// state (#2617).
    #[error("{}", Initiation { sent: *.sent })]
    InitiationDisabled {
        /// `false`: nothing was sent, as the request was refused before it
        /// went out. `true`: the request went out before initiation was
        /// turned off, no answer came, and its retry was not sent, so the
        /// device may have carried it out.
        sent: bool,
    },

    /// A request that ended before an answer came because the client or
    /// its link stopped under it (#2617, #2720, #1833): the client's own
    /// stop, or the stop or drop of the endpoint or node it runs on, while
    /// the request waited for its turn at the destination or for its
    /// answer; or the link stopping, or losing its connection (a BACnet/SC
    /// link's hub), after the request went out, which refuses its retry or
    /// a later segment, or drops the wait of a B/IP BVLC management request
    /// or loses its answer. It is that one request's outcome, not a
    /// lifecycle state: a call made after the stop gets
    /// [`Error::State`], with nothing sent. No answer will come for it.
    #[error("{}", CutOff { sent: *.sent })]
    RequestCutOff {
        /// `false`: nothing of the request left, as it was still waiting
        /// for its turn. `true`: the request, or some of its segments, may
        /// have gone out, and the device may have carried it out, so a
        /// write is not to be sent again blindly.
        sent: bool,
    },
}

impl From<RemoteError> for Error {
    fn from(error: RemoteError) -> Self {
        Self::Remote(error)
    }
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

impl From<PersistenceError> for Error {
    fn from(error: PersistenceError) -> Self {
        Self::Persistence(error)
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
    /// An Error PDU's [`Error::Remote`] with its class and code.
    pub const fn protocol(class: ErrorClass, code: ErrorCode) -> Self {
        Self::Remote(RemoteError::Error { class, code })
    }

    /// A Reject PDU's [`Error::Remote`] with its reason.
    pub const fn reject(reason: RejectReason) -> Self {
        Self::Remote(RemoteError::Reject { reason })
    }

    /// An Abort PDU's [`Error::Remote`] with its reason.
    pub const fn abort(reason: AbortReason) -> Self {
        Self::Remote(RemoteError::Abort { reason })
    }

    /// The error an Error PDU reports: [`Error::Structured`] when its body
    /// carried a detail, [`Error::Remote`] otherwise. Only with `alloc`.
    #[cfg(feature = "alloc")]
    pub fn with_detail(class: ErrorClass, code: ErrorCode, detail: Option<ErrorDetail>) -> Self {
        match detail {
            Some(detail) => Self::Structured {
                class,
                code,
                detail: Box::new(detail),
            },
            None => Self::protocol(class, code),
        }
    }

    /// The peer's answer this error reports, without a structured error's
    /// detail: the [`RemoteError`] of [`Error::Remote`], and an Error with
    /// the class and code of `Error::Structured`. `None` for any other
    /// error, which no peer sent.
    pub fn remote(&self) -> Option<RemoteError> {
        match self {
            Self::Remote(error) => Some(*error),
            #[cfg(feature = "alloc")]
            Self::Structured { class, code, .. } => Some(RemoteError::Error {
                class: *class,
                code: *code,
            }),
            _ => None,
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
    /// 20.1.8): the reason itself for a Reject's [`Error::Remote`], the kind's reason
    /// for a decoding error (the table on
    /// [`DecodingKind::reject_reason`]), and MISSING_REQUIRED_PARAMETER for
    /// [`Error::BufferTooShort`], a member cut short because the data ran out
    /// before it was complete. `None` for any other error, which no decoder
    /// of the request's parameters reports.
    pub fn reject_reason(&self) -> Option<RejectReason> {
        match self {
            Self::Remote(RemoteError::Reject { reason }) => Some(*reason),
            Self::BufferTooShort { .. } => Some(RejectReason::MISSING_REQUIRED_PARAMETER),
            _ => self.decode_fault().map(|(_, kind)| kind.reject_reason()),
        }
    }

    /// This error, met decoding a confirmed request's own parameters, as the
    /// Reject's [`Error::Remote`] a responder answers with: the
    /// [`reject_reason`](Self::reject_reason) when it is a syntax fault, the
    /// error unchanged otherwise.
    ///
    /// A responder converts only the request's decode with this. A decoding
    /// error met later, once the service has started to run, is no reason to
    /// reject the request (Clause 20.1.8), so it stays as it is.
    pub fn into_request_reject(self) -> Self {
        match self.reject_reason() {
            Some(reason) => Self::reject(reason),
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

    /// Create an [`Error::State`] for `state`, described by a static
    /// `label`.
    pub const fn state(state: Lifecycle, label: &'static str) -> Self {
        Self::State(state, Refusal::new(label))
    }

    /// Create an [`Error::InvalidConfig`] described by a static `label`.
    pub const fn invalid_config(label: &'static str) -> Self {
        Self::InvalidConfig(Refusal::new(label))
    }

    /// Create an [`Error::InvalidArgument`] described by a static `label`.
    pub const fn invalid_argument(label: &'static str) -> Self {
        Self::InvalidArgument(Refusal::new(label))
    }

    /// Create an [`Error::Unsupported`] described by a static `label`.
    pub const fn unsupported(label: &'static str) -> Self {
        Self::Unsupported(Refusal::new(label))
    }

    /// Create an [`Error::LimitReached`] described by a static `label`.
    pub const fn limit_reached(label: &'static str) -> Self {
        Self::LimitReached(Refusal::new(label))
    }

    /// This error with `context` and `: ` in front of its text, keeping its
    /// variant, for a caller that says where a local refusal or persisted
    /// state came from, such as a file's path; a persisted-state error keeps
    /// its kind. Any other error is returned as it is. Only with `std`.
    #[cfg(feature = "std")]
    #[must_use]
    pub fn with_context(self, context: &str) -> Self {
        match self {
            Self::State(state, refusal) => Self::State(state, refusal.with_context(context)),
            Self::InvalidConfig(refusal) => Self::InvalidConfig(refusal.with_context(context)),
            Self::InvalidArgument(refusal) => Self::InvalidArgument(refusal.with_context(context)),
            Self::Unsupported(refusal) => Self::Unsupported(refusal.with_context(context)),
            Self::LimitReached(refusal) => Self::LimitReached(refusal.with_context(context)),
            Self::Persistence(refused) => Self::Persistence(refused.with_context(context)),
            other => other,
        }
    }
}

#[cfg(test)]
mod tests;
