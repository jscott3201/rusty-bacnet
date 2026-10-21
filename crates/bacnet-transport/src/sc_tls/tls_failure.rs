//! Sort a failed TLS exchange by the TLS library's typed error, never by its
//! text: the reason travels on [`ScConnectError`](crate::sc::ScConnectError)
//! so a caller can explain a refusal without matching on a message.

use std::io;
use std::time::Duration;

use rustls::{AlertDescription, CertificateError};

use crate::sc::{ScCertificateRejection, ScTlsAlert, ScTlsFailure};

/// The reason a TLS stream's I/O error carries, or `None` when the error is
/// not a TLS one (a reset or an end of stream, say).
pub(crate) fn classify(error: &io::Error) -> Option<ScTlsFailure> {
    let tls = error.get_ref()?.downcast_ref::<rustls::Error>()?;
    Some(from_rustls(tls))
}

fn from_rustls(error: &rustls::Error) -> ScTlsFailure {
    match error {
        rustls::Error::InvalidCertificate(problem) => {
            ScTlsFailure::PeerCertificate(rejection(problem))
        }
        rustls::Error::AlertReceived(alert) => ScTlsFailure::AlertReceived(alert_of(*alert)),
        rustls::Error::PeerIncompatible(_) => ScTlsFailure::PeerIncompatible,
        _ => ScTlsFailure::Other,
    }
}

fn gap(later: u64, earlier: u64) -> Option<Duration> {
    later.checked_sub(earlier).map(Duration::from_secs)
}

fn rejection(problem: &CertificateError) -> ScCertificateRejection {
    use CertificateError as C;
    match problem {
        C::UnknownIssuer => ScCertificateRejection::UnknownIssuer,
        C::Expired => ScCertificateRejection::Expired { by: None },
        C::ExpiredContext { time, not_after } => ScCertificateRejection::Expired {
            by: gap(time.as_secs(), not_after.as_secs()),
        },
        C::NotValidYet => ScCertificateRejection::NotYetValid { by: None },
        C::NotValidYetContext { time, not_before } => ScCertificateRejection::NotYetValid {
            by: gap(not_before.as_secs(), time.as_secs()),
        },
        C::BadSignature => ScCertificateRejection::BadSignature,
        #[allow(deprecated)]
        C::UnsupportedSignatureAlgorithm
        | C::UnsupportedSignatureAlgorithmContext { .. }
        | C::UnsupportedSignatureAlgorithmForPublicKeyContext { .. } => {
            ScCertificateRejection::UnsupportedSignatureAlgorithm
        }
        C::NotValidForName | C::NotValidForNameContext { .. } => {
            ScCertificateRejection::NameMismatch
        }
        C::InvalidPurpose | C::InvalidPurposeContext { .. } => ScCertificateRejection::WrongPurpose,
        C::Revoked => ScCertificateRejection::Revoked,
        _ => ScCertificateRejection::Other,
    }
}

fn alert_of(alert: AlertDescription) -> ScTlsAlert {
    use AlertDescription as A;
    match alert {
        A::UnknownCA => ScTlsAlert::UnknownCa,
        A::CertificateExpired => ScTlsAlert::CertificateExpired,
        A::UnsupportedCertificate => ScTlsAlert::UnsupportedCertificate,
        A::DecryptError => ScTlsAlert::DecryptError,
        A::CertificateRequired => ScTlsAlert::CertificateRequired,
        A::CertificateRevoked => ScTlsAlert::CertificateRevoked,
        A::BadCertificate | A::CertificateUnknown => ScTlsAlert::BadCertificate,
        A::ProtocolVersion => ScTlsAlert::ProtocolVersion,
        other => ScTlsAlert::Other(u8::from(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::UnixTime;

    fn io(error: rustls::Error) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, error)
    }

    #[test]
    #[allow(deprecated)]
    fn certificate_problems_keep_their_reason() {
        let at = |secs| UnixTime::since_unix_epoch(Duration::from_secs(secs));
        let cases = [
            (
                CertificateError::UnknownIssuer,
                ScCertificateRejection::UnknownIssuer,
            ),
            (
                CertificateError::ExpiredContext {
                    time: at(1_000),
                    not_after: at(400),
                },
                ScCertificateRejection::Expired {
                    by: Some(Duration::from_secs(600)),
                },
            ),
            (
                CertificateError::NotValidYetContext {
                    time: at(1_000),
                    not_before: at(1_090),
                },
                ScCertificateRejection::NotYetValid {
                    by: Some(Duration::from_secs(90)),
                },
            ),
            (
                CertificateError::BadSignature,
                ScCertificateRejection::BadSignature,
            ),
            // An algorithm this node can't check is not a key mismatch.
            (
                CertificateError::UnsupportedSignatureAlgorithm,
                ScCertificateRejection::UnsupportedSignatureAlgorithm,
            ),
            (
                CertificateError::NotValidForName,
                ScCertificateRejection::NameMismatch,
            ),
            (
                CertificateError::InvalidPurpose,
                ScCertificateRejection::WrongPurpose,
            ),
            (CertificateError::Revoked, ScCertificateRejection::Revoked),
            (CertificateError::BadEncoding, ScCertificateRejection::Other),
        ];
        for (problem, expected) in cases {
            assert_eq!(
                classify(&io(rustls::Error::InvalidCertificate(problem))),
                Some(ScTlsFailure::PeerCertificate(expected))
            );
        }
    }

    #[test]
    fn alerts_and_other_errors_sort_apart() {
        for (alert, expected) in [
            (AlertDescription::UnknownCA, ScTlsAlert::UnknownCa),
            (
                AlertDescription::CertificateExpired,
                ScTlsAlert::CertificateExpired,
            ),
            (
                AlertDescription::UnsupportedCertificate,
                ScTlsAlert::UnsupportedCertificate,
            ),
            (AlertDescription::DecryptError, ScTlsAlert::DecryptError),
            (
                AlertDescription::CertificateRequired,
                ScTlsAlert::CertificateRequired,
            ),
            (
                AlertDescription::CertificateUnknown,
                ScTlsAlert::BadCertificate,
            ),
            (AlertDescription::HandshakeFailure, ScTlsAlert::Other(40)),
        ] {
            assert_eq!(
                classify(&io(rustls::Error::AlertReceived(alert))),
                Some(ScTlsFailure::AlertReceived(expected))
            );
        }
        let incompatible =
            rustls::Error::PeerIncompatible(rustls::PeerIncompatible::Tls13RequiredForQuic);
        assert_eq!(
            classify(&io(incompatible)),
            Some(ScTlsFailure::PeerIncompatible)
        );
        assert_eq!(
            classify(&io(rustls::Error::DecryptError)),
            Some(ScTlsFailure::Other)
        );
        // Not a TLS error at all: a reset, or a plain message.
        assert_eq!(
            classify(&io::Error::from(io::ErrorKind::ConnectionReset)),
            None
        );
        assert_eq!(classify(&io::Error::other("closed")), None);
    }
}
