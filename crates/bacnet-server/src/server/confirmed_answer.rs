//! The answer a device gives a confirmed request this server sent: a
//! confirmed notification, a WriteProperty a run makes in another device, or
//! the ReadProperty a Channel sends there to learn a member's datatype
//! (#1342).
//!
//! An answer that turns the request down keeps what it said (#1323) as a
//! [`RemoteError`]: the Error PDU's class and code, or the Reject or Abort
//! reason. Notification senders only ask whether the request was taken; a
//! Channel writing a member in another device needs the code too, since a
//! NULL refused as an invalid datatype still counts as written (Clause
//! 12.53.7).

use bacnet_types::error::RemoteError;
use bytes::Bytes;

/// Result of a confirmed request from the receiving device's side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CovAckResult {
    /// SimpleAck received: the device took the request.
    Ack,
    /// An unsegmented ComplexAck received, with its service data: the device
    /// answered a read.
    Data(Bytes),
    /// Error, Reject or Abort received: the device turned the request down.
    /// It converts to the [`Error::Remote`](bacnet_types::error::Error::Remote)
    /// a client's request answered this way reports. It keeps only an
    /// Error's class and code, so it never becomes
    /// [`Error::Structured`](bacnet_types::error::Error::Structured).
    Error(RemoteError),
}
