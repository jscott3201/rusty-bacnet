//! The connection doctor (#967): which phase of a hub dial failed, and what
//! to try, as a pure function of the typed failure.
//!
//! A hub dial goes through six phases in order: the `wss` URL (Annex AB.7.2),
//! the host lookup, the TCP connection, mutual TLS 1.3 (Annex AB.7.4), the
//! WebSocket upgrade with the hub subprotocol (Annex AB.7.1), and the
//! Connect-Request and Connect-Accept exchange (Annex AB.6.2). Annex AB.7.5.1
//! sorts dial errors into the same groups. Every hint comes from a typed
//! error kind, TLS reason or NAK code; nothing here reads an error's text.

use std::time::Duration;

use bacnet_transport::sc::{
    ScCertificateRejection, ScDialStage, ScTlsAlert, ScTlsFailure, ScWebSocketErrorKind,
};
use bacnet_types::enums::ErrorCode;

use super::Fault;

/// One phase of a hub dial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Parse the `wss` URL.
    Url,
    /// Resolve the host name.
    Dns,
    /// Open the TCP connection.
    Tcp,
    /// Mutual TLS.
    Tls,
    /// The WebSocket upgrade with the hub subprotocol.
    WebSocket,
    /// Connect-Request and Connect-Accept.
    Connect,
}

/// The phases, in the order a dial goes through them.
pub(crate) const PHASES: [Phase; 6] = [
    Phase::Url,
    Phase::Dns,
    Phase::Tcp,
    Phase::Tls,
    Phase::WebSocket,
    Phase::Connect,
];

impl Phase {
    /// Its name in the timeline.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Url => "URL",
            Self::Dns => "DNS",
            Self::Tcp => "TCP",
            Self::Tls => "TLS",
            Self::WebSocket => "WebSocket",
            Self::Connect => "Connect",
        }
    }

    /// What it checks.
    pub(crate) fn what(self) -> &'static str {
        match self {
            Self::Url => "wss scheme and hub host (AB.7.2)",
            Self::Dns => "host name to addresses",
            Self::Tcp => "TCP connection to the hub port",
            Self::Tls => "mutual TLS 1.3, certificates both ways (AB.7.4)",
            Self::WebSocket => "HTTP upgrade with the hub subprotocol (AB.7.1)",
            Self::Connect => "Connect-Request answered by Connect-Accept (AB.6.2)",
        }
    }
}

/// How far a dial got through one phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mark {
    /// Done.
    Passed,
    /// Where it failed.
    Failed,
    /// Never started.
    NotReached,
    /// The failure does not say.
    Unknown,
}

/// A failure, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Diagnosis {
    /// The failing phase, when the failure says which.
    pub(crate) failed: Option<Phase>,
    /// Each phase's mark, in [`PHASES`] order.
    pub(crate) marks: [Mark; 6],
    /// What failed, in a few words.
    pub(crate) summary: String,
    /// What to try.
    pub(crate) hint: String,
}

/// Every phase passed: the marks of a connected link.
pub(crate) const CONNECTED: [Mark; 6] = [Mark::Passed; 6];

/// A peer certificate this close to its validity window more likely means
/// a clock is off than that the certificate is wrong.
pub(crate) const SKEW_WINDOW: Duration = Duration::from_secs(24 * 3_600);

/// Read a failure.
pub(crate) fn diagnose(fault: &Fault) -> Diagnosis {
    let (failed, summary, hint) = read(fault);
    let marks = match failed {
        Some(phase) => {
            let at = PHASES.iter().position(|p| *p == phase).unwrap_or(0);
            std::array::from_fn(|i| match i.cmp(&at) {
                std::cmp::Ordering::Less => Mark::Passed,
                std::cmp::Ordering::Equal => Mark::Failed,
                std::cmp::Ordering::Greater => Mark::NotReached,
            })
        }
        None => [Mark::Unknown; 6],
    };
    Diagnosis {
        failed,
        marks,
        summary,
        hint,
    }
}

fn read(fault: &Fault) -> (Option<Phase>, String, String) {
    use ScWebSocketErrorKind as K;
    let (phase, summary, hint): (Phase, &str, String) = match fault {
        Fault::Tls(failure) => {
            let (summary, hint) = tls(*failure);
            return (Some(Phase::Tls), summary, hint);
        }
        Fault::WebSocket(K::InvalidUrl | K::MissingHubHost) => (
            Phase::Url,
            "the hub URL is not valid",
            "Write it as wss://host[:port][/path], with the hub's host name or address.".into(),
        ),
        Fault::WebSocket(K::UnsupportedScheme) => (
            Phase::Url,
            "the hub URL is not wss",
            "BACnet/SC dials only wss:// URLs; ws:// and https:// are refused before any dial."
                .into(),
        ),
        Fault::WebSocket(K::HostLookup) => (
            Phase::Dns,
            "the hub host name did not resolve",
            "Check the host name's spelling and this computer's DNS; an IP address in the URL \
             skips the lookup."
                .into(),
        ),
        Fault::WebSocket(K::TcpDial) => (
            Phase::Tcp,
            "nothing accepted the TCP connection",
            "Check the port, that the hub is running, and that no firewall blocks it.".into(),
        ),
        Fault::WebSocket(K::TlsHandshake) => (
            Phase::Tls,
            "the TLS handshake failed",
            HANDSHAKE_HINT.into(),
        ),
        Fault::WebSocket(K::WebSocketHandshake) => (
            Phase::WebSocket,
            "the WebSocket upgrade failed",
            "TLS worked, but the HTTP upgrade did not. A proxy or load balancer in the path \
             may drop WebSocket upgrades; let it pass them through, or dial the hub directly. \
             Check the URL path too."
                .into(),
        ),
        Fault::UpgradeRefused(status) => {
            return (
                Some(Phase::WebSocket),
                format!("the WebSocket upgrade was refused with HTTP {status}"),
                format!(
                    "TLS worked, but the server answered the upgrade with HTTP {status} \
                     instead of switching protocols. That is usually a proxy or load \
                     balancer in the path: {}",
                    upgrade_status_hint(*status)
                ),
            )
        }
        Fault::DialTimeout(Some(stage)) => {
            let (phase, hint) = stalled(*stage);
            return (
                Some(phase),
                format!("the dial stalled in the {stage}"),
                hint.into(),
            );
        }
        Fault::ConnectAcceptTimeout => (
            Phase::Connect,
            "the hub never answered the Connect-Request",
            "The WebSocket opened, but no Connect-Accept came within the connect timeout. \
             The server may not be a BACnet/SC hub, or the hub is stalled; check its log."
                .into(),
        ),
        Fault::WebSocket(K::HubSubprotocol | K::DirectSubprotocol) => (
            Phase::WebSocket,
            "the hub subprotocol was not accepted",
            "The server upgraded without the BACnet/SC hub subprotocol. Is the URL a hub's, \
             not a direct-connect port or another web service? A proxy that strips the \
             Sec-WebSocket-Protocol header does this too."
                .into(),
        ),
        Fault::WebSocket(_) | Fault::PeerClose | Fault::BadReply => (
            Phase::Connect,
            "the hub ended the exchange",
            "The WebSocket opened but the hub closed it or answered the Connect-Request with \
             something other than Connect-Accept. Check the hub's log; it may be refusing \
             this node."
                .into(),
        ),
        Fault::Nak { code, reseeded, .. } if *code == ErrorCode::NODE_DUPLICATE_VMAC => (
            Phase::Connect,
            "duplicate VMAC",
            if *reseeded {
                "Another node on this hub already uses this VMAC. The connector drew a new \
                 random VMAC for its next attempt (AB.6.2.2); if this keeps happening, give \
                 this profile a VMAC no other node uses."
                    .into()
            } else {
                "Another node on this hub already uses this VMAC; give this profile a VMAC \
                 no other node uses."
                    .into()
            },
        ),
        Fault::Nak { class, code, .. } => {
            return (
                Some(Phase::Connect),
                "the hub refused the Connect-Request".into(),
                format!(
                    "The hub answered with a NAK ({class:?}, {code:?}); its log says why. \
                     Check that the URL is a hub's and that this node is admitted."
                ),
            )
        }
        Fault::HeartbeatTimeout => (
            Phase::Connect,
            "the hub went silent",
            "Nothing came from the hub within the heartbeat timeout (AB.6.3). The path may \
             drop idle connections; check NAT and firewall idle timers."
                .into(),
        ),
        Fault::LocalClose => {
            return (
                None,
                "closed by this node".into(),
                "The connection was closed here, by a stop or a disconnect.".into(),
            )
        }
        Fault::DialTimeout(None) => {
            return (
                None,
                "the dial got no answer in time".into(),
                "The TCP, TLS or WebSocket step of the dial did not finish within the \
                 connect timeout; the dialler did not say which. Something on the path may \
                 have accepted the connection and then gone quiet."
                    .into(),
            )
        }
        Fault::Other(text) => {
            return (
                None,
                "failed outside the hub dial".into(),
                format!("This failed before or outside the phases below: {text}"),
            )
        }
    };
    (Some(phase), summary.into(), hint)
}

/// The phase and advice for a dial that ran out of time in `stage`.
fn stalled(stage: ScDialStage) -> (Phase, &'static str) {
    match stage {
        ScDialStage::HostLookup => (
            Phase::Dns,
            "The host name lookup did not finish within the connect timeout. Check this \
             computer's DNS servers. An IP address in the URL skips the lookup, but the \
             hub's certificate must then list that IP as a subject alternative name.",
        ),
        ScDialStage::TcpConnect => (
            Phase::Tcp,
            "Nothing answered the TCP connection: a firewall may be dropping the packets \
             silently, or the address is not reachable from here.",
        ),
        ScDialStage::Tls => (
            Phase::Tls,
            "TCP connected, but the TLS handshake never finished. The port may serve \
             something other than TLS, or a middlebox is holding the connection.",
        ),
        ScDialStage::WebSocketUpgrade => (
            Phase::WebSocket,
            "TLS worked, but the HTTP upgrade got no answer: a proxy or load balancer \
             may be holding it. Let it pass WebSocket upgrades, or dial the hub directly.",
        ),
        _ => (
            Phase::Connect,
            "The dial ran out of time in a step this screen has no hint for.",
        ),
    }
}

/// What a proxy's answer to the upgrade most likely means.
fn upgrade_status_hint(status: u16) -> &'static str {
    match status {
        401 | 407 => "it wants credentials this node does not send.",
        403 => "it refused this client; check its access rules.",
        404 => "check the URL path; the hub may serve another path.",
        502..=504 => "it could not reach the hub behind it, or the hub did not answer it.",
        _ => "let it pass WebSocket upgrades, or dial the hub directly.",
    }
}

/// Advice for a TLS failure with no narrower reason.
const HANDSHAKE_HINT: &str = "Check that the URL's port serves TLS (not plain HTTP) and that \
     both sides use TLS 1.3.";

/// Days, hours or minutes, for a skew or an expiry.
fn span(by: Duration) -> String {
    let secs = by.as_secs();
    match secs {
        0..=3_599 => format!("{} min", secs / 60),
        3_600..=86_399 => format!("{} h", secs / 3_600),
        _ => format!("{} days", secs / 86_400),
    }
}

/// The summary and hint for a TLS failure.
pub(crate) fn tls(failure: ScTlsFailure) -> (String, String) {
    use ScCertificateRejection as R;
    let (summary, hint): (String, String) = match failure {
        ScTlsFailure::PeerCertificate(R::UnknownIssuer) => (
            "hub certificate from an untrusted CA".into(),
            "The hub's certificate was not signed by a CA in this profile's CA file. Point \
             the profile at this site's CA, or have the hub serve a certificate from it."
                .into(),
        ),
        ScTlsFailure::PeerCertificate(R::Expired { by }) => match by {
            Some(by) if by <= SKEW_WINDOW => (
                format!("hub certificate expired {} ago", span(by)),
                "It expired so recently that a clock may be off: check this computer's clock \
                 and the hub's (NTP) before renewing anything."
                    .into(),
            ),
            _ => (
                by.map_or("hub certificate expired".into(), |by| {
                    format!("hub certificate expired {} ago", span(by))
                }),
                "The hub's certificate is past its not-after date: its owner needs to renew \
                 it, or this computer's clock is far ahead."
                    .into(),
            ),
        },
        ScTlsFailure::PeerCertificate(R::NotYetValid { by }) => match by {
            Some(by) if by <= SKEW_WINDOW => (
                format!("hub certificate valid only in {}", span(by)),
                "The certificate starts within a day: often clock skew. Check this \
                 computer's clock and NTP."
                    .into(),
            ),
            _ => (
                by.map_or("hub certificate not valid yet".into(), |by| {
                    format!("hub certificate valid only in {}", span(by))
                }),
                "The hub's certificate is dated in the future. Check this computer's clock; \
                 if it is right, the certificate was issued with the wrong start date."
                    .into(),
            ),
        },
        ScTlsFailure::PeerCertificate(R::BadSignature) => (
            "hub certificate signature does not verify".into(),
            "Key mismatch: the hub's certificate names this site's CA but was not signed by \
             that CA's key. The CA may have been re-keyed, or the certificate signed by \
             another CA with the same name."
                .into(),
        ),
        ScTlsFailure::PeerCertificate(R::UnsupportedSignatureAlgorithm) => (
            "hub certificate uses an unsupported signature".into(),
            "This node cannot check the signature algorithm of the hub's certificate (the \
             one it sent is below). Have it reissued with a common algorithm such as ECDSA \
             P-256 with SHA-256, RSA with SHA-256, or Ed25519."
                .into(),
        ),
        ScTlsFailure::PeerCertificate(R::NameMismatch) => (
            "hub certificate is for another host".into(),
            "The certificate does not name the host in the URL. Dial the host name the \
             certificate was issued for. This name check goes past the Annex AB.7.4 default \
             and is kept on purpose (docs/sc-check.md, Hub name verification)."
                .into(),
        ),
        ScTlsFailure::PeerCertificate(R::WrongPurpose) => (
            "hub certificate not for server use".into(),
            "The hub's certificate lacks the TLS server-authentication extended key usage.".into(),
        ),
        ScTlsFailure::PeerCertificate(R::Revoked) => (
            "hub certificate revoked".into(),
            "The hub's certificate is revoked; its owner needs a new one.".into(),
        ),
        ScTlsFailure::PeerCertificate(_) => (
            "hub certificate refused".into(),
            "The hub's certificate could not be accepted; the chain below shows what it sent."
                .into(),
        ),
        ScTlsFailure::AlertReceived(alert) => return refused_by_hub(alert),
        ScTlsFailure::PeerIncompatible => (
            "no TLS version in common".into(),
            "BACnet/SC needs TLS 1.3 (AB.7.4); the hub may offer only TLS 1.2.".into(),
        ),
        // No narrower reason: the same advice as a TLS failure with none.
        _ => ("the TLS handshake failed".into(), HANDSHAKE_HINT.into()),
    };
    (summary, hint)
}

/// The hub's alert, read as its refusal of this node.
fn refused_by_hub(alert: ScTlsAlert) -> (String, String) {
    let (summary, hint) = match alert {
        ScTlsAlert::UnknownCa => (
            "hub does not trust this node's CA",
            "The hub refused this node's certificate: no CA it trusts signed it. Install this \
             node's CA on the hub, or get a certificate from the site CA.",
        ),
        ScTlsAlert::CertificateExpired => (
            "hub refused this node's certificate dates",
            "The hub found this node's certificate expired or not yet valid by its own clock. \
             Check the certificate's dates, then both clocks (NTP) if they look right.",
        ),
        ScTlsAlert::UnsupportedCertificate => (
            "hub refused this node's certificate type",
            "Often: this node's certificate lacks the TLS client-authentication extended \
             key usage. Some hubs also send this for a key type they do not support.",
        ),
        ScTlsAlert::DecryptError => (
            "hub could not verify this node's signature",
            "Often a key mismatch on our side: this node's certificate was not signed by \
             the key of the CA it names, or the private key is not the certificate's.",
        ),
        ScTlsAlert::CertificateRequired => (
            "hub wants a client certificate",
            "The hub requires a client certificate and got none.",
        ),
        ScTlsAlert::CertificateRevoked => (
            "this node's certificate is revoked",
            "The hub refused this node's certificate as revoked; get a new one.",
        ),
        ScTlsAlert::BadCertificate => (
            "hub refused this node's certificate",
            "The hub found this node's certificate unacceptable; check it with `bacnet \
             sc-check`.",
        ),
        ScTlsAlert::ProtocolVersion => (
            "no TLS version in common",
            "BACnet/SC needs TLS 1.3 (AB.7.4); the hub refused the version offered.",
        ),
        _ => (
            "hub ended the TLS handshake",
            "The hub sent a TLS alert this screen has no hint for; see the detail.",
        ),
    };
    (summary.into(), hint.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ErrorClass;

    fn tls_fault(rejection: ScCertificateRejection) -> Fault {
        Fault::Tls(ScTlsFailure::PeerCertificate(rejection))
    }

    /// Each typed failure lands on its phase, with marks before and after.
    #[test]
    fn each_failure_lands_on_its_phase() {
        use ScWebSocketErrorKind as K;
        let cases = [
            (Fault::WebSocket(K::UnsupportedScheme), Some(Phase::Url)),
            (Fault::WebSocket(K::HostLookup), Some(Phase::Dns)),
            (Fault::WebSocket(K::TcpDial), Some(Phase::Tcp)),
            (Fault::WebSocket(K::TlsHandshake), Some(Phase::Tls)),
            (
                Fault::Tls(ScTlsFailure::AlertReceived(ScTlsAlert::UnknownCa)),
                Some(Phase::Tls),
            ),
            (
                Fault::WebSocket(K::WebSocketHandshake),
                Some(Phase::WebSocket),
            ),
            (Fault::WebSocket(K::HubSubprotocol), Some(Phase::WebSocket)),
            (Fault::BadReply, Some(Phase::Connect)),
            (Fault::HeartbeatTimeout, Some(Phase::Connect)),
            (Fault::DialTimeout(None), None),
            (Fault::DialTimeout(Some(ScDialStage::Tls)), Some(Phase::Tls)),
            (
                Fault::DialTimeout(Some(ScDialStage::WebSocketUpgrade)),
                Some(Phase::WebSocket),
            ),
            (Fault::ConnectAcceptTimeout, Some(Phase::Connect)),
            (Fault::UpgradeRefused(502), Some(Phase::WebSocket)),
            (Fault::Other("x".into()), None),
        ];
        for (fault, phase) in cases {
            let diagnosis = diagnose(&fault);
            assert_eq!(diagnosis.failed, phase, "{fault:?}");
            match phase {
                Some(phase) => {
                    let at = PHASES.iter().position(|p| *p == phase).unwrap();
                    assert_eq!(diagnosis.marks[at], Mark::Failed, "{fault:?}");
                    assert!(diagnosis.marks[..at].iter().all(|m| *m == Mark::Passed));
                    assert!(diagnosis.marks[at + 1..]
                        .iter()
                        .all(|m| *m == Mark::NotReached));
                }
                None => assert_eq!(diagnosis.marks, [Mark::Unknown; 6]),
            }
        }
    }

    /// The four certificate cases of the acceptance criteria each get their
    /// own hint on the TLS phase, none of them a timeout.
    #[test]
    fn certificate_cases_each_get_their_own_hint() {
        let far = Some(Duration::from_secs(400 * 86_400));
        let faults = [
            tls_fault(ScCertificateRejection::UnknownIssuer),
            tls_fault(ScCertificateRejection::Expired { by: far }),
            tls_fault(ScCertificateRejection::NotYetValid { by: far }),
            tls_fault(ScCertificateRejection::BadSignature),
        ];
        let read: Vec<Diagnosis> = faults.iter().map(diagnose).collect();
        for diagnosis in &read {
            assert_eq!(diagnosis.failed, Some(Phase::Tls));
            assert!(!diagnosis.summary.contains("no answer"));
        }
        assert!(read[0].hint.contains("CA file"), "{}", read[0].hint);
        assert_eq!(read[1].summary, "hub certificate expired 400 days ago");
        assert!(read[1].hint.contains("renew"));
        assert_eq!(read[2].summary, "hub certificate valid only in 400 days");
        assert!(read[2].hint.contains("future"));
        assert!(read[3].hint.starts_with("Key mismatch"));
        let hints: std::collections::HashSet<_> = read.iter().map(|d| &d.hint).collect();
        assert_eq!(hints.len(), 4, "every case has its own hint");
    }

    #[test]
    fn a_near_miss_on_the_validity_window_reads_as_clock_skew() {
        let near = Some(Duration::from_secs(3 * 3_600));
        let expired = diagnose(&tls_fault(ScCertificateRejection::Expired { by: near }));
        assert_eq!(expired.summary, "hub certificate expired 3 h ago");
        assert!(expired.hint.contains("clock"));
        let early = diagnose(&tls_fault(ScCertificateRejection::NotYetValid { by: near }));
        assert!(early.hint.contains("often clock skew"));
        // Just past the window: no longer called skew.
        let day = Some(SKEW_WINDOW + Duration::from_secs(1));
        let late = diagnose(&tls_fault(ScCertificateRejection::NotYetValid { by: day }));
        assert!(!late.hint.contains("clock skew"));
    }

    #[test]
    fn the_hubs_alerts_name_this_nodes_side() {
        let alert = |a| diagnose(&Fault::Tls(ScTlsFailure::AlertReceived(a)));
        assert!(alert(ScTlsAlert::UnsupportedCertificate)
            .hint
            .contains("client-authentication"));
        assert!(alert(ScTlsAlert::DecryptError)
            .hint
            .starts_with("Often a key mismatch"));
        assert!(alert(ScTlsAlert::CertificateExpired)
            .hint
            .contains("clocks"));
        assert!(alert(ScTlsAlert::UnknownCa)
            .summary
            .contains("this node's CA"));
        let incompatible = diagnose(&Fault::Tls(ScTlsFailure::PeerIncompatible));
        assert!(incompatible.hint.contains("TLS 1.3"));
    }

    /// A TLS failure the library did not narrow down reads like one with no
    /// reason at all; an unsupported signature algorithm is not a key mismatch.
    #[test]
    fn unexplained_tls_and_unsupported_algorithms_get_their_own_hints() {
        let plain = diagnose(&Fault::WebSocket(ScWebSocketErrorKind::TlsHandshake));
        let other = diagnose(&Fault::Tls(ScTlsFailure::Other));
        assert_eq!((other.failed, &other.hint), (Some(Phase::Tls), &plain.hint));
        let algorithm = diagnose(&tls_fault(
            ScCertificateRejection::UnsupportedSignatureAlgorithm,
        ));
        let mismatch = diagnose(&tls_fault(ScCertificateRejection::BadSignature));
        assert_eq!(algorithm.failed, Some(Phase::Tls));
        assert!(algorithm.hint.contains("Ed25519"), "{}", algorithm.hint);
        assert_ne!(algorithm.hint, mismatch.hint);
    }

    #[test]
    fn a_proxy_and_a_duplicate_vmac_get_their_hints() {
        let upgrade = diagnose(&Fault::WebSocket(ScWebSocketErrorKind::WebSocketHandshake));
        assert!(upgrade.hint.contains("proxy"));
        let refused = diagnose(&Fault::UpgradeRefused(502));
        assert!(refused.summary.contains("HTTP 502"), "{}", refused.summary);
        assert!(refused.hint.contains("HTTP 502"), "{}", refused.hint);
        assert!(refused.hint.contains("proxy"), "{}", refused.hint);
        let subprotocol = diagnose(&Fault::WebSocket(ScWebSocketErrorKind::HubSubprotocol));
        assert!(subprotocol.hint.contains("Sec-WebSocket-Protocol"));
        let duplicate = diagnose(&Fault::Nak {
            class: ErrorClass::COMMUNICATION,
            code: ErrorCode::NODE_DUPLICATE_VMAC,
            reseeded: true,
        });
        assert_eq!(duplicate.failed, Some(Phase::Connect));
        assert_eq!(duplicate.summary, "duplicate VMAC");
        assert!(duplicate.hint.contains("new random VMAC"));
        let other = diagnose(&Fault::Nak {
            class: ErrorClass::COMMUNICATION,
            code: ErrorCode::NOT_A_BACNET_SC_HUB,
            reseeded: false,
        });
        assert_eq!(other.summary, "the hub refused the Connect-Request");
    }
}
