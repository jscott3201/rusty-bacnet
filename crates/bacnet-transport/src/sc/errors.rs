//! BACnet/SC typed connect-path errors.

use std::fmt;
use std::io;

use bacnet_types::error::Error;

use crate::sc_frame::ScFunction;

/// Specific WebSocket failure kind observed by the BACnet/SC connect path.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScWebSocketErrorKind {
    /// The configured WebSocket URL could not be parsed.
    InvalidUrl,
    /// The configured WebSocket URL did not use the required `wss` scheme.
    UnsupportedScheme,
    /// The configured WebSocket URL did not include a hub host.
    MissingHubHost,
    /// The TCP dial failed before TLS or WebSocket negotiation began.
    TcpDial,
    /// TLS negotiation failed after the TCP dial succeeded.
    TlsHandshake,
    /// The WebSocket HTTP upgrade failed after the TCP dial succeeded.
    WebSocketHandshake,
    /// The hub did not accept the BACnet/SC hub subprotocol.
    HubSubprotocol,
    /// Sending a WebSocket frame failed.
    Send,
    /// Receiving a WebSocket frame failed.
    Receive,
    /// The WebSocket closed.
    Closed,
    /// A non-binary WebSocket frame was received.
    NonBinaryFrame,
    /// The WebSocket stream ended without a frame.
    StreamEnded,
}

impl ScWebSocketErrorKind {
    fn default_io_kind(self) -> io::ErrorKind {
        match self {
            Self::InvalidUrl | Self::UnsupportedScheme | Self::MissingHubHost => {
                io::ErrorKind::InvalidInput
            }
            Self::TcpDial => io::ErrorKind::ConnectionRefused,
            Self::TlsHandshake | Self::WebSocketHandshake | Self::HubSubprotocol => {
                io::ErrorKind::InvalidData
            }
            Self::Send
            | Self::Receive
            | Self::Closed
            | Self::NonBinaryFrame
            | Self::StreamEnded => io::ErrorKind::ConnectionAborted,
        }
    }
}

impl fmt::Display for ScWebSocketErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::InvalidUrl => "invalid-url",
            Self::UnsupportedScheme => "unsupported-scheme",
            Self::MissingHubHost => "missing-hub-host",
            Self::TcpDial => "tcp-dial",
            Self::TlsHandshake => "tls-handshake",
            Self::WebSocketHandshake => "websocket-handshake",
            Self::HubSubprotocol => "hub-subprotocol",
            Self::Send => "send",
            Self::Receive => "receive",
            Self::Closed => "closed",
            Self::NonBinaryFrame => "non-binary-frame",
            Self::StreamEnded => "stream-ended",
        };
        f.write_str(name)
    }
}

/// Typed BACnet/SC connect-path error carried inside [`Error::Transport`].
///
/// The top-level [`Error`] enum is shared by multiple crates and is not
/// `non_exhaustive`; carrying this type inside the existing transport variant
/// gives callers a non-breaking way to recover SC-specific details:
///
/// ```ignore
/// if let Some(sc_error) = ScConnectError::from_error(&err) {
///     match sc_error {
///         ScConnectError::HandshakeNak { error_code, .. } => { /* ... */ }
///         ScConnectError::WebSocket { kind, .. } => { /* ... */ }
///         _ => {}
///     }
/// }
/// ```
///
/// BACnet/SC connect timeouts use the existing top-level [`Error::Timeout`]
/// variant, so callers should match that variant directly.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScConnectError {
    /// The hub rejected the Connect-Request with a BVLC-Result NAK.
    #[non_exhaustive]
    HandshakeNak {
        /// BVLC function for which this Result was sent.
        result_for: ScFunction,
        /// Header marker that caused the error, or 0 when unrelated to a header option.
        error_header_marker: u8,
        /// BACnet Error Class value carried by the NAK.
        error_class: u16,
        /// BACnet Error Code value carried by the NAK.
        error_code: u16,
        /// Optional UTF-8 details carried by the NAK.
        error_details: String,
        /// Whether a duplicate-VMAC NAK caused the transport to reseed its local VMAC.
        duplicate_vmac_reseeded: bool,
    },
    /// The connect path received a malformed BVLC-Result.
    #[non_exhaustive]
    MalformedBvlcResult {
        /// Decode offset reported by the frame/result decoder.
        offset: usize,
        /// Decode failure details.
        message: String,
    },
    /// A BVLC-Result ACK was received where Connect-Accept was required.
    #[non_exhaustive]
    UnexpectedResultAck {
        /// BVLC function for which this Result was sent.
        result_for: ScFunction,
    },
    /// Connect-Accept did not match the pending Connect-Request.
    ConnectAcceptMismatch,
    /// A WebSocket/TCP/TLS failure happened while connecting or using BACnet/SC.
    #[non_exhaustive]
    WebSocket {
        /// Machine-readable failure category.
        kind: ScWebSocketErrorKind,
        /// Human-readable detail from the underlying driver.
        message: String,
    },
}

impl ScConnectError {
    pub(crate) fn into_bacnet_error(self) -> Error {
        let kind = self.default_io_kind();
        self.into_bacnet_error_with_io_kind(kind)
    }

    pub(crate) fn into_bacnet_error_with_io_kind(self, kind: io::ErrorKind) -> Error {
        Error::Transport(io::Error::new(kind, self))
    }

    fn default_io_kind(&self) -> io::ErrorKind {
        match self {
            Self::HandshakeNak { .. }
            | Self::MalformedBvlcResult { .. }
            | Self::UnexpectedResultAck { .. }
            | Self::ConnectAcceptMismatch => io::ErrorKind::InvalidData,
            Self::WebSocket { kind, .. } => kind.default_io_kind(),
        }
    }

    /// Return a typed BACnet/SC connect error from a top-level BACnet error.
    pub fn from_error(error: &Error) -> Option<&Self> {
        match error {
            Error::Transport(io_error) => Self::from_io_error(io_error),
            _ => None,
        }
    }

    /// Return a typed BACnet/SC connect error from an I/O error wrapper.
    pub fn from_io_error(error: &io::Error) -> Option<&Self> {
        error.get_ref()?.downcast_ref::<Self>()
    }
}

impl fmt::Display for ScConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HandshakeNak {
                result_for,
                error_class,
                error_code,
                error_details,
                duplicate_vmac_reseeded,
                ..
            } => {
                write!(
                    f,
                    "BACnet/SC BVLC-Result NAK during connect: function={:#x} \
                     error_class={} error_code={} details={}",
                    result_for.to_raw(),
                    error_class,
                    error_code,
                    error_details
                )?;
                if *duplicate_vmac_reseeded {
                    f.write_str("; selected new Random-48 local VMAC")?;
                }
                Ok(())
            }
            Self::MalformedBvlcResult { offset, message } => write!(
                f,
                "malformed BACnet/SC BVLC-Result during connect at offset {offset}: {message}"
            ),
            Self::UnexpectedResultAck { result_for } => write!(
                f,
                "unexpected BACnet/SC BVLC-Result ACK during connect: function={:#x}",
                result_for.to_raw()
            ),
            Self::ConnectAcceptMismatch => {
                f.write_str("BACnet/SC Connect-Accept did not match pending Connect-Request")
            }
            Self::WebSocket { kind, message } => {
                write!(f, "BACnet/SC WebSocket {kind} failure: {message}")
            }
        }
    }
}

impl std::error::Error for ScConnectError {}
