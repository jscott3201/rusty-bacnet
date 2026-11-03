//! [`RemoteError`]: how a peer turned a confirmed request down.

use core::fmt;

use crate::enums::{AbortReason, ErrorClass, ErrorCode, RejectReason};

/// How a peer turned a confirmed request down: an Error PDU's class and
/// code, or a Reject or Abort PDU's reason (Clauses 20.1.5 to 20.1.7).
///
/// One value carries the answer from the wire to the API (#1815). The client
/// TSM completes a transaction with it, [`Error::Remote`](super::Error::Remote)
/// reports it, and a server keeps it as the answer to a request it sent. An
/// Error PDU whose Clause 21 production adds fields beyond the class and code
/// is `Error::Structured` (with `alloc`) instead.
///
/// It displays as the [`Error`](super::Error) that holds it does: an error's
/// class and code, or a reason, by name in lower case with hyphens (the
/// number for a value without a name).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum RemoteError {
    /// An Error PDU (Clause 20.1.7).
    #[error("{}", ProtocolText { class: *class, code: *code })]
    Error {
        /// Error class (Clause 18).
        class: ErrorClass,
        /// Error code (Clause 18).
        code: ErrorCode,
    },
    /// A Reject PDU (Clause 20.1.5).
    #[error("BACnet reject: {}", ReasonText { name: reason.name(), raw: reason.to_raw() })]
    Reject {
        /// Reject reason (Clause 18.9).
        reason: RejectReason,
    },
    /// An Abort PDU (Clause 20.1.6).
    #[error("BACnet abort: {}", ReasonText { name: reason.name(), raw: reason.to_raw() })]
    Abort {
        /// Abort reason (Clause 18.10).
        reason: AbortReason,
    },
}

/// How a protocol error's class and code read: each name in lower case with
/// hyphens, or the raw number for a value without one.
pub(super) struct ProtocolText {
    pub(super) class: ErrorClass,
    pub(super) code: ErrorCode,
}

impl fmt::Display for ProtocolText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BACnet error: ")?;
        write_name(f, self.class.name(), "class", self.class.to_raw())?;
        f.write_str(" / ")?;
        write_name(f, self.code.name(), "code", self.code.to_raw())
    }
}

/// How a Reject or Abort reason reads: its name in lower case with hyphens,
/// or `reason=` and the raw number for a value without one.
struct ReasonText {
    name: Option<&'static str>,
    raw: u8,
}

impl fmt::Display for ReasonText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_name(f, self.name, "reason", u16::from(self.raw))
    }
}

/// Write a constant's `named` name in lower case with hyphens, or
/// `field=value` for a value without a name.
fn write_name(
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
