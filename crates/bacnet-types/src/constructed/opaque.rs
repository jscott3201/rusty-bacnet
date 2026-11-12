//! An alternative of a CHOICE under a context tag the stack has no variant
//! for, kept as received (#2474, #2483).

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

/// An alternative of a CHOICE under a context tag that CHOICE doesn't
/// assign (a later revision's, or one Clause 21 deprecates or reserves),
/// kept as received so that it re-encodes unchanged: the `Opaque` variant
/// of [`NotificationParameters`](super::NotificationParameters),
/// [`LogDatum`](super::LogDatum), [`EventLogDatum`](super::EventLogDatum),
/// [`LogData`](super::LogData), [`LogValue`](super::LogValue) and
/// [`BACnetAuditLogDatum`](super::BACnetAuditLogDatum).
///
/// The encoder writes it back under its own tag, primitive or between an
/// opening and closing tag pair. It refuses a tag its CHOICE assigns (or,
/// for event values, leaves out), 255, a primitive longer than the tag
/// decoder takes, and a constructed body that isn't complete tagged values
/// with every context tag balanced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpaqueChoice {
    /// The unassigned context tag.
    pub tag: u8,
    /// Whether the alternative is constructed (an opening and closing tag
    /// pair around `data`) rather than primitive.
    pub constructed: bool,
    /// A primitive alternative's contents, or a constructed one's body
    /// without its enclosing tag pair.
    pub data: Vec<u8>,
}
