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
    /// The host name in the URL did not resolve to any address.
    HostLookup,
    /// The TCP dial failed before TLS or WebSocket negotiation began.
    TcpDial,
    /// TLS negotiation failed after the TCP dial succeeded.
    TlsHandshake,
    /// The WebSocket HTTP upgrade failed after the TCP dial succeeded.
    WebSocketHandshake,
    /// The hub did not accept the BACnet/SC hub subprotocol.
    HubSubprotocol,
    /// The direct peer did not accept the BACnet/SC direct subprotocol.
    DirectSubprotocol,
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
            Self::HostLookup => io::ErrorKind::NotFound,
            Self::TcpDial => io::ErrorKind::ConnectionRefused,
            Self::TlsHandshake
            | Self::WebSocketHandshake
            | Self::HubSubprotocol
            | Self::DirectSubprotocol => io::ErrorKind::InvalidData,
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
            Self::HostLookup => "host-lookup",
            Self::TcpDial => "tcp-dial",
            Self::TlsHandshake => "tls-handshake",
            Self::WebSocketHandshake => "websocket-handshake",
            Self::HubSubprotocol => "hub-subprotocol",
            Self::DirectSubprotocol => "direct-subprotocol",
            Self::Send => "send",
            Self::Receive => "receive",
            Self::Closed => "closed",
            Self::NonBinaryFrame => "non-binary-frame",
            Self::StreamEnded => "stream-ended",
        };
        f.write_str(name)
    }
}

/// The step a hub dial had reached when it ran out of time (#1839).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScDialStage {
    /// Resolving the host name.
    HostLookup,
    /// Opening the TCP connection.
    TcpConnect,
    /// The TLS handshake, after TCP connected.
    Tls,
    /// The WebSocket HTTP upgrade, after TLS completed.
    WebSocketUpgrade,
}

impl ScDialStage {
    /// A nonzero code for the dial tracker; 0 means no stage yet.
    #[cfg(feature = "sc-tls")]
    pub(crate) fn code(self) -> u8 {
        match self {
            Self::HostLookup => 1,
            Self::TcpConnect => 2,
            Self::Tls => 3,
            Self::WebSocketUpgrade => 4,
        }
    }

    pub(crate) fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::HostLookup),
            2 => Some(Self::TcpConnect),
            3 => Some(Self::Tls),
            4 => Some(Self::WebSocketUpgrade),
            _ => None,
        }
    }
}

impl fmt::Display for ScDialStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::HostLookup => "host lookup",
            Self::TcpConnect => "TCP connect",
            Self::Tls => "TLS handshake",
            Self::WebSocketUpgrade => "WebSocket upgrade",
        })
    }
}

/// Why a TLS handshake for BACnet/SC failed, from the TLS library's own
/// error rather than its text. Under Annex AB.7.4 both peers vet each
/// other's certificate (its encoding, its dates, revocation where known,
/// and a signature from a configured CA); this says which check failed,
/// and on which side.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScTlsFailure {
    /// This node refused the certificate the peer presented.
    PeerCertificate(ScCertificateRejection),
    /// The peer ended the handshake with this alert, most often refusing
    /// this node's certificate. With TLS 1.3 the alert can arrive after the
    /// client's side of the handshake is done, during the WebSocket upgrade.
    AlertReceived(ScTlsAlert),
    /// The peer offered no TLS version or cipher suite in common, such as a
    /// hub that speaks only TLS 1.2 (BACnet/SC needs TLS 1.3).
    PeerIncompatible,
    /// Any other TLS failure.
    Other,
}

/// Why this node refused a peer's certificate.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScCertificateRejection {
    /// Not issued by any configured CA.
    UnknownIssuer,
    /// Past its validity window.
    Expired {
        /// How long ago the not-after date passed, when known.
        by: Option<std::time::Duration>,
    },
    /// Before its validity window.
    NotYetValid {
        /// How long until the not-before date, when known.
        by: Option<std::time::Duration>,
    },
    /// The signature does not verify: signed by a different key than the CA
    /// it names.
    BadSignature,
    /// Signed with an algorithm this node does not support, so the
    /// signature could not be checked at all.
    UnsupportedSignatureAlgorithm,
    /// The certificate does not name the host the URL dials.
    NameMismatch,
    /// The certificate's extended key usage does not allow its role.
    WrongPurpose,
    /// Revoked.
    Revoked,
    /// Any other certificate problem, such as a malformed one.
    Other,
}

/// A TLS alert the peer sent, for the alerts that explain a refusal.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScTlsAlert {
    /// The peer does not trust the CA that issued this node's certificate.
    UnknownCa,
    /// This node's certificate is outside its validity window by the peer's
    /// clock: expired or not yet valid.
    CertificateExpired,
    /// The peer refused this node's certificate for its key usage.
    UnsupportedCertificate,
    /// A signature did not verify: often a certificate signed by a different
    /// key than its issuer's, or a certificate and private key that differ.
    DecryptError,
    /// The peer requires a client certificate and got none.
    CertificateRequired,
    /// This node's certificate is revoked.
    CertificateRevoked,
    /// The peer found this node's certificate malformed or unacceptable.
    BadCertificate,
    /// The peer allows no TLS version this node offered.
    ProtocolVersion,
    /// Another alert, by its TLS alert number.
    Other(u8),
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
/// A hub dial (host lookup, TCP, TLS and WebSocket upgrade) that runs out
/// of time is
/// [`ScConnectError::DialTimeout`]. A Connect-Request that gets no
/// Connect-Accept in time uses the top-level [`Error::Timeout`] variant.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScConnectError {
    /// The hub rejected the Connect-Request with a BVLC-Result NAK.
    #[non_exhaustive]
    HandshakeNak {
        /// Function code of the BVLC message this Result answers.
        result_for: ScFunction,
        /// Marker of the header option at fault, or 0 when no header option was involved.
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
        /// Function code of the BVLC message this Result answers.
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
        /// The TLS library's reason when TLS failed: in the handshake, or as
        /// an alert read during the WebSocket upgrade.
        tls: Option<ScTlsFailure>,
        /// Untrusted certificate evidence from this failed outbound TLS attempt.
        /// Absent when no verifier callback was observed, including failures
        /// before that point or resumed handshakes. Requires `sc-tls`.
        #[cfg(feature = "sc-tls")]
        untrusted_peer: Option<std::sync::Arc<crate::sc_tls::ScUntrustedPeerInfo>>,
        /// The HTTP status a server answered the WebSocket upgrade with
        /// instead of switching protocols, such as a proxy's 403 or 502.
        /// `None` when no HTTP response came back.
        http_status: Option<u16>,
    },
    /// The hub dial (host lookup, TCP, TLS and WebSocket upgrade) did not
    /// finish within the connect timeout.
    #[non_exhaustive]
    DialTimeout {
        /// The connect timeout that ran out.
        timeout: std::time::Duration,
        /// The step the dial was in, when the dialler reports it (the
        /// `sc-tls` hub dialler does); `None` for a custom connector.
        stage: Option<ScDialStage>,
    },
}

impl ScConnectError {
    /// Untrusted certificates from the actual failed outbound TLS attempt.
    ///
    /// This never performs I/O or establishes an authenticated identity. `None`
    /// means unavailable at the observation point, not proof no certificate
    /// was sent. Background reconnect history and inbound failures are not
    /// retained by this API. Requires `sc-tls`.
    #[cfg(feature = "sc-tls")]
    pub fn untrusted_peer_info(&self) -> Option<&crate::sc_tls::ScUntrustedPeerInfo> {
        match self {
            Self::WebSocket { untrusted_peer, .. } => untrusted_peer.as_deref(),
            _ => None,
        }
    }

    /// Why TLS failed, when this is a TLS failure the TLS library explained;
    /// `None` for every other error.
    pub fn tls_failure(&self) -> Option<ScTlsFailure> {
        match self {
            Self::WebSocket { tls, .. } => *tls,
            _ => None,
        }
    }

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
            Self::DialTimeout { .. } => io::ErrorKind::TimedOut,
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
            Self::WebSocket { kind, message, .. } => {
                write!(f, "BACnet/SC WebSocket {kind} failure: {message}")
            }
            Self::DialTimeout { timeout, stage } => {
                write!(f, "BACnet/SC hub dial timed out after {timeout:?}")?;
                match stage {
                    Some(stage) => write!(f, " during the {stage}"),
                    None => Ok(()),
                }
            }
        }
    }
}

impl std::error::Error for ScConnectError {}
