//! The SC screen's messages (#967): what the SC worker reports about the hub
//! connector, as plain data the app can hold and tests can build.
//!
//! The transport's own types (`ScEvent`, `ScLinkStatus`, `ScDisconnectCause`)
//! are mostly non-exhaustive, so they are converted here once, in the worker,
//! and the app, the views and the doctor see only these.
//!
//! - [`doctor`]: which phase of a hub dial failed, and a plain-language hint.
//! - [`credentials`]: this node's own certificates and key, as the pre-dial
//!   lint read them (#968).

// Only an `sc-tls` build inspects credentials; without it the view says so.
#[cfg_attr(not(feature = "sc-tls"), allow(dead_code))]
pub(crate) mod credentials;
pub(crate) mod doctor;

use std::time::Duration;

use bacnet_transport::sc::{
    ScActiveHub, ScConnectError, ScConnectionState, ScDialStage, ScDisconnectCause, ScTlsFailure,
    ScWebSocketErrorKind,
};
use bacnet_transport::sc_frame::ScFunction;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;

/// What a session dials, from its connection settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScIdentity {
    /// The primary hub's URL.
    pub(crate) primary: String,
    /// The failover hub's URL, when one is set.
    pub(crate) failover: Option<String>,
    /// The VMAC this node asks for.
    pub(crate) vmac: [u8; 6],
    /// The device UUID, persisted in the profile (Annex AB.1.5.3).
    pub(crate) uuid: [u8; 16],
}

impl ScIdentity {
    /// The URL of `hub`.
    pub(crate) fn url(&self, hub: ScActiveHub) -> Option<&str> {
        match hub {
            ScActiveHub::Primary => Some(&self.primary),
            ScActiveHub::Failover => self.failover.as_deref(),
        }
    }
}

/// The hub link as one status poll saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LinkSnapshot {
    /// The connection state.
    pub(crate) state: ScConnectionState,
    /// The hub in use, or being dialled.
    pub(crate) hub: Option<ScActiveHub>,
    /// The connected hub's VMAC.
    pub(crate) hub_vmac: Option<[u8; 6]>,
    /// Time since the hub last sent anything.
    pub(crate) heartbeat_age: Option<Duration>,
    /// Round trip of the latest answered heartbeat.
    pub(crate) heartbeat_rtt: Option<Duration>,
}

impl From<bacnet_transport::sc::ScLinkStatus> for LinkSnapshot {
    fn from(status: bacnet_transport::sc::ScLinkStatus) -> Self {
        Self {
            state: status.state,
            hub: status.active_hub,
            hub_vmac: status.hub_vmac,
            heartbeat_age: status.heartbeat_age,
            heartbeat_rtt: status.heartbeat_rtt,
        }
    }
}

/// Why a connection failed or ended, sorted for the doctor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fault {
    /// A URL, lookup, TCP, TLS or WebSocket failure of this kind.
    WebSocket(ScWebSocketErrorKind),
    /// A TLS failure with the TLS library's reason.
    Tls(ScTlsFailure),
    /// The hub answered the Connect-Request with a NAK.
    Nak {
        /// Error class.
        class: ErrorClass,
        /// Error code.
        code: ErrorCode,
        /// The transport drew a new random VMAC for the next attempt.
        reseeded: bool,
    },
    /// The dial (TCP, TLS, WebSocket upgrade) ran past the connect
    /// timeout, in this step when the transport says.
    DialTimeout(Option<ScDialStage>),
    /// The WebSocket opened but no Connect-Accept came within the connect
    /// timeout.
    ConnectAcceptTimeout,
    /// The WebSocket upgrade was answered with this HTTP status.
    UpgradeRefused(u16),
    /// The hub went silent past the heartbeat timeout.
    HeartbeatTimeout,
    /// The hub closed the connection.
    PeerClose,
    /// This node closed it.
    LocalClose,
    /// A reply to the Connect-Request that was malformed or not an accept.
    BadReply,
    /// Anything else, with its text.
    Other(String),
}

impl Fault {
    /// Sort a connect error.
    pub(crate) fn from_error(error: &Error) -> Self {
        let reseeded = matches!(
            ScConnectError::from_error(error),
            Some(ScConnectError::HandshakeNak {
                duplicate_vmac_reseeded: true,
                ..
            })
        );
        match Self::from(ScDisconnectCause::from_error(error)) {
            Self::Nak { class, code, .. } => Self::Nak {
                class,
                code,
                reseeded,
            },
            other => other,
        }
    }

    /// A duplicate-VMAC NAK (Annex AB.6.2.2).
    pub(crate) fn is_duplicate_vmac(&self) -> bool {
        matches!(self, Self::Nak { code, .. } if *code == ErrorCode::NODE_DUPLICATE_VMAC)
    }
}

impl From<ScDisconnectCause> for Fault {
    fn from(cause: ScDisconnectCause) -> Self {
        match cause {
            ScDisconnectCause::LocalClose => Self::LocalClose,
            ScDisconnectCause::PeerClose => Self::PeerClose,
            ScDisconnectCause::HeartbeatTimeout | ScDisconnectCause::RejectionExpired => {
                Self::HeartbeatTimeout
            }
            ScDisconnectCause::WebSocket(kind) => Self::WebSocket(kind),
            ScDisconnectCause::Tls(failure) => Self::Tls(failure),
            ScDisconnectCause::Nak {
                result_for,
                error_class,
                error_code,
                ..
            } => Self::Nak {
                class: error_class,
                code: error_code,
                // The transport draws a new VMAC on a duplicate-VMAC NAK to
                // its Connect-Request (Annex AB.6.2.2), and only then.
                reseeded: error_code == ErrorCode::NODE_DUPLICATE_VMAC
                    && result_for == ScFunction::ConnectRequest,
            },
            ScDisconnectCause::DialTimeout { stage, .. } => Self::DialTimeout(stage),
            ScDisconnectCause::ConnectAcceptTimeout => Self::ConnectAcceptTimeout,
            ScDisconnectCause::UpgradeRefused { http_status, .. } => {
                Self::UpgradeRefused(http_status)
            }
            ScDisconnectCause::MalformedResult | ScDisconnectCause::UnexpectedConnectReply => {
                Self::BadReply
            }
            ScDisconnectCause::Other(text) => Self::Other(text),
            other => Self::Other(format!("{other:?}")),
        }
    }
}

/// One certificate of a chain, for display only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CertSummary {
    /// Subject distinguished name, escaped.
    pub(crate) subject: String,
    /// Issuer distinguished name, escaped.
    pub(crate) issuer: String,
    /// Not-after, in Unix seconds.
    pub(crate) not_after_unix: i64,
    /// Not-after as a UTC date.
    pub(crate) not_after: String,
    /// The signature algorithm, as an OID.
    pub(crate) signature_algorithm: String,
}

/// A failed client build, for the doctor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScFailure {
    /// Why.
    pub(crate) fault: Fault,
    /// The error's text, shown as is.
    pub(crate) detail: String,
    /// The certificates a refused hub presented, leaf first; untrusted.
    pub(crate) presented: Vec<CertSummary>,
}

impl ScFailure {
    /// Sort a failed build's error.
    pub(crate) fn from_error(error: &Error) -> Self {
        Self {
            fault: Fault::from_error(error),
            detail: error.to_string(),
            presented: presented(error),
        }
    }
}

#[cfg(feature = "sc-tls")]
fn presented(error: &Error) -> Vec<CertSummary> {
    ScConnectError::from_error(error)
        .and_then(ScConnectError::untrusted_peer_info)
        .map(|info| summarize(info.certificates()))
        .unwrap_or_default()
}

#[cfg(not(feature = "sc-tls"))]
fn presented(_: &Error) -> Vec<CertSummary> {
    Vec::new()
}

/// Display summaries of a chain; a certificate the parser could not read
/// shows as such.
#[cfg(feature = "sc-tls")]
pub(crate) fn summarize(
    certificates: &[bacnet_transport::sc_tls::ScPeerCertificate],
) -> Vec<CertSummary> {
    certificates
        .iter()
        .map(|certificate| match certificate.metadata() {
            Some(meta) => CertSummary {
                subject: meta.subject().to_string(),
                issuer: meta.issuer().to_string(),
                not_after_unix: meta.not_after_unix(),
                not_after: crate::sc_check::date(meta.not_after_unix()),
                signature_algorithm: meta.signature_algorithm_oid().to_string(),
            },
            None => CertSummary {
                subject: "(unparsed certificate)".into(),
                issuer: String::new(),
                not_after_unix: 0,
                not_after: String::new(),
                signature_algorithm: String::new(),
            },
        })
        .collect()
}

/// A hub-connector event, from `ScEvent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinkEvent {
    /// Connected to `hub`, whose VMAC is `hub_vmac`.
    Connected {
        /// The hub.
        hub: ScActiveHub,
        /// Its VMAC from Connect-Accept.
        hub_vmac: [u8; 6],
    },
    /// The connection to `hub` ended.
    Disconnected {
        /// The hub.
        hub: ScActiveHub,
        /// Why.
        fault: Fault,
    },
    /// A dial to `hub` failed.
    ConnectFailed {
        /// The hub.
        hub: ScActiveHub,
        /// Why.
        fault: Fault,
    },
    /// Dialling `hub` again after `backoff`.
    Reconnecting {
        /// The hub.
        hub: ScActiveHub,
        /// Attempt number, from 1.
        attempt: u32,
        /// The delay waited before it.
        backoff: Duration,
    },
    /// The primary could not be reached; turning to the failover hub.
    FailoverEngaged,
    /// The primary is back and took over.
    PrimaryRestored,
    /// A probe of the primary, made from the failover hub, failed.
    PrimaryRestoreFailed {
        /// Why.
        fault: Fault,
    },
    /// Reconnection stopped.
    GaveUp {
        /// Attempts made.
        attempts: u32,
    },
}

impl LinkEvent {
    /// Convert a transport event; `None` for a kind this screen doesn't know.
    pub(crate) fn from_event(event: bacnet_transport::sc::ScEvent) -> Option<Self> {
        use bacnet_transport::sc::ScEvent as E;
        Some(match event {
            E::Connected { hub, hub_vmac, .. } => Self::Connected { hub, hub_vmac },
            E::Disconnected { hub, cause, .. } => Self::Disconnected {
                hub,
                fault: cause.into(),
            },
            E::ConnectFailed { hub, cause, .. } => Self::ConnectFailed {
                hub,
                fault: cause.into(),
            },
            E::ReconnectAttempt {
                hub,
                attempt,
                backoff,
                ..
            } => Self::Reconnecting {
                hub,
                attempt,
                backoff,
            },
            E::FailoverEngaged => Self::FailoverEngaged,
            E::PrimaryRestored => Self::PrimaryRestored,
            E::PrimaryRestoreFailed { cause, .. } => Self::PrimaryRestoreFailed {
                fault: cause.into(),
            },
            E::ReconnectGaveUp { attempts, .. } => Self::GaveUp { attempts },
            _ => return None,
        })
    }
}

/// SC worker to UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScReport {
    /// A session is about to dial with these settings.
    Dialing(ScIdentity),
    /// The client build failed.
    Failed(Box<ScFailure>),
    /// The link, polled once a second and after a resync.
    Status(LinkSnapshot),
    /// A hub-connector event.
    Event(LinkEvent),
    /// The worker fell behind the event stream and lost this many events;
    /// a [`ScReport::Status`] follows to resync.
    Gap {
        /// Events lost.
        missed: u64,
    },
    /// The certificates the connected hub presented, leaf first.
    Chain(Vec<CertSummary>),
    /// This node's credentials as the pre-dial lint read them, or why the
    /// lint could not read them. Sent before the dial.
    #[cfg_attr(not(feature = "sc-tls"), allow(dead_code))]
    Credentials(Box<Result<credentials::Credentials, String>>),
}

/// `hub` in words.
pub(crate) fn hub_name(hub: ScActiveHub) -> &'static str {
    match hub {
        ScActiveHub::Primary => "primary",
        ScActiveHub::Failover => "failover",
    }
}

/// A VMAC as `02:00:00:00:00:01`.
pub(crate) fn vmac_text(vmac: &[u8]) -> String {
    vmac.iter()
        .map(|octet| format!("{octet:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// A UUID in its 8-4-4-4-12 form.
pub(crate) fn uuid_text(uuid: &[u8; 16]) -> String {
    let hex: String = uuid.iter().map(|octet| format!("{octet:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_transport::sc::{ScCertificateRejection, ScTlsAlert};

    #[test]
    fn faults_sort_from_connect_errors() {
        let timeout = Error::Timeout(Duration::from_secs(10));
        assert_eq!(Fault::from_error(&timeout), Fault::ConnectAcceptTimeout);
        let other = Error::Encoding("TLS config error".into());
        assert!(matches!(Fault::from_error(&other), Fault::Other(_)));
        let tls = ScDisconnectCause::Tls(ScTlsFailure::PeerCertificate(
            ScCertificateRejection::UnknownIssuer,
        ));
        assert_eq!(
            Fault::from(tls),
            Fault::Tls(ScTlsFailure::PeerCertificate(
                ScCertificateRejection::UnknownIssuer
            ))
        );
        let alert = ScDisconnectCause::Tls(ScTlsFailure::AlertReceived(ScTlsAlert::UnknownCa));
        assert!(!Fault::from(alert).is_duplicate_vmac());
        let duplicate = Fault::Nak {
            class: ErrorClass::COMMUNICATION,
            code: ErrorCode::NODE_DUPLICATE_VMAC,
            reseeded: true,
        };
        assert!(duplicate.is_duplicate_vmac());
    }

    #[test]
    fn identities_print_as_operators_write_them() {
        assert_eq!(vmac_text(&[2, 0, 0, 0, 0, 0x1a]), "02:00:00:00:00:1a");
        let uuid = [
            0x6f, 0x1c, 0x2a, 0x9e, 0x44, 0x07, 0x4b, 0x3d, 0x9a, 0x51, 0x0e, 0x7c, 0x88, 0x21,
            0xd4, 0x0b,
        ];
        assert_eq!(uuid_text(&uuid), "6f1c2a9e-4407-4b3d-9a51-0e7c8821d40b");
    }
}
