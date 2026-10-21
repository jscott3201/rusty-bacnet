//! [`RejectMessageToNetwork`]: the data a Reject-Message-To-Network carries.

use core::fmt;

use crate::RejectMessageReason;

/// What a Reject-Message-To-Network carries after its type (Clause 6.4.4):
/// one reason octet, then the two-octet network the refused message was for.
///
/// It is the one reader and writer of that data in the stack: the router
/// core, the hosted network layer and the client all go through it. It
/// checks layout only; whether the enclosing NPDU is a reject is the
/// caller's question, and so is what the reason means for a route.
///
/// ```
/// use bacnet_npdu::{RejectMessageReason, RejectMessageToNetwork};
///
/// let reject = RejectMessageToNetwork::decode(&[4, 0x0B, 0xB8])?;
/// assert_eq!(reject.reason, RejectMessageReason::MESSAGE_TOO_LONG);
/// assert_eq!(reject.network, 3000);
/// assert_eq!(reject.to_bytes(), [4, 0x0B, 0xB8]);
/// # Ok::<(), bacnet_npdu::RejectLengthError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RejectMessageToNetwork {
    /// Why the message was refused. The set is open: an octet the standard
    /// doesn't name is kept as received.
    pub reason: RejectMessageReason,
    /// The network the refused message was for (its DNET).
    pub network: u16,
}

impl RejectMessageToNetwork {
    /// The data's length, in octets.
    pub const LEN: usize = 3;

    /// Read exactly a reason octet and a network number. Any other length
    /// is refused, so a router never acts on a cut-short or padded reject.
    pub const fn decode(payload: &[u8]) -> Result<Self, RejectLengthError> {
        match *payload {
            [reason, high, low] => Ok(Self {
                reason: RejectMessageReason::from_raw(reason),
                network: u16::from_be_bytes([high, low]),
            }),
            _ => Err(RejectLengthError {
                length: payload.len(),
            }),
        }
    }

    /// The data, as sent.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; Self::LEN] {
        let [high, low] = self.network.to_be_bytes();
        [self.reason.to_raw(), high, low]
    }
}

/// A Reject-Message-To-Network's data isn't
/// [`RejectMessageToNetwork::LEN`] octets: it ends inside the network, or
/// goes on past it.
///
/// It is `#[non_exhaustive]`: only the crate builds one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RejectLengthError {
    /// The data's length, in octets.
    pub length: usize,
}

impl fmt::Display for RejectLengthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a reject's data is {} octets, not {}",
            self.length,
            RejectMessageToNetwork::LEN
        )
    }
}

impl core::error::Error for RejectLengthError {}
