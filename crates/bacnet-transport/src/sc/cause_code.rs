//! The BACnet error code for a hub connection's failure (#2434).
//!
//! Annex AB.7.5 gives a node error codes (Clause 18.7, values 152-199) to
//! report a WebSocket connection that could not be made or that ended:
//! DNS, IP, TCP, TLS and HTTP failures while connecting, and the WebSocket
//! close status once connected. A code is reported to local layers and
//! management, never sent on the wire. Where the standard has a code for
//! the precise failure and this stack knows it, that code is chosen; where
//! it only knows the layer, the layer's catch-all code (`DNS_ERROR`,
//! `TCP_ERROR`, `TLS_ERROR`, `HTTP_ERROR`, `WEBSOCKET_ERROR`) is.

use bacnet_types::enums::ErrorCode;

use super::{
    ScCertificateRejection, ScDialStage, ScDisconnectCause, ScTlsAlert, ScTlsFailure,
    ScWebSocketErrorKind,
};

impl ScDisconnectCause {
    /// The BACnet error code that names this failure, for display and for
    /// management reporting.
    ///
    /// - A NAK gives the code it carried.
    /// - A DNS, TCP, TLS, HTTP-upgrade or WebSocket failure gives one of
    ///   the BACnet/SC connection codes (Annex AB.7.5). A hub connection is
    ///   one this node dials, so the hub is the TLS server: a certificate
    ///   this node refused is a `TLS_SERVER_*` code, and an alert from the
    ///   hub refusing this node is a `TLS_CLIENT_*` code.
    /// - `None` when no code applies: a local or orderly close, a heartbeat
    ///   or Connect-Accept timeout, a BVLC-level fault, a bad URL, or a
    ///   WebSocket close whose status code was not kept.
    pub fn error_code(&self) -> Option<ErrorCode> {
        match self {
            Self::Nak { error_code, .. } => Some(*error_code),
            Self::Tls(failure) => Some(tls_code(*failure)),
            Self::WebSocket(kind) => websocket_code(*kind),
            Self::UpgradeRefused { http_status } => Some(http_code(*http_status)),
            Self::DialTimeout { stage } => stage.map(|stage| match stage {
                ScDialStage::HostLookup => ErrorCode::DNS_ERROR,
                ScDialStage::TcpConnect => ErrorCode::TCP_CONNECT_TIMEOUT,
                ScDialStage::Tls => ErrorCode::TLS_ERROR,
                ScDialStage::WebSocketUpgrade => ErrorCode::HTTP_RESPONSE_TIMEOUT,
            }),
            Self::LocalClose
            | Self::PeerClose
            | Self::HeartbeatTimeout
            | Self::ConnectAcceptTimeout
            | Self::MalformedResult
            | Self::UnexpectedConnectReply
            | Self::RejectionExpired
            | Self::Other(_) => None,
        }
    }
}

/// TLS failures, with this node as the TLS client.
fn tls_code(failure: ScTlsFailure) -> ErrorCode {
    match failure {
        ScTlsFailure::PeerCertificate(rejection) => match rejection {
            // Either side of the validity window: the code covers both.
            ScCertificateRejection::Expired { .. } | ScCertificateRejection::NotYetValid { .. } => {
                ErrorCode::TLS_SERVER_CERTIFICATE_EXPIRED
            }
            ScCertificateRejection::Revoked => ErrorCode::TLS_SERVER_CERTIFICATE_REVOKED,
            // A sound certificate that no configured CA vouches for.
            ScCertificateRejection::UnknownIssuer | ScCertificateRejection::BadSignature => {
                ErrorCode::TLS_SERVER_AUTHENTICATION_FAILED
            }
            ScCertificateRejection::UnsupportedSignatureAlgorithm
            | ScCertificateRejection::NameMismatch
            | ScCertificateRejection::WrongPurpose
            | ScCertificateRejection::Other => ErrorCode::TLS_SERVER_CERTIFICATE_ERROR,
        },
        ScTlsFailure::AlertReceived(alert) => match alert {
            ScTlsAlert::CertificateExpired => ErrorCode::TLS_CLIENT_CERTIFICATE_EXPIRED,
            ScTlsAlert::CertificateRevoked => ErrorCode::TLS_CLIENT_CERTIFICATE_REVOKED,
            ScTlsAlert::UnknownCa | ScTlsAlert::DecryptError => {
                ErrorCode::TLS_CLIENT_AUTHENTICATION_FAILED
            }
            ScTlsAlert::BadCertificate
            | ScTlsAlert::UnsupportedCertificate
            | ScTlsAlert::CertificateRequired => ErrorCode::TLS_CLIENT_CERTIFICATE_ERROR,
            ScTlsAlert::ProtocolVersion | ScTlsAlert::Other(_) => ErrorCode::TLS_ERROR,
        },
        // No common version or cipher suite. The 2020 edition gives no
        // number for a security-parameter mismatch, so the catch-all.
        ScTlsFailure::PeerIncompatible | ScTlsFailure::Other => ErrorCode::TLS_ERROR,
    }
}

/// Connect-path and WebSocket failures the TLS library did not explain.
fn websocket_code(kind: ScWebSocketErrorKind) -> Option<ErrorCode> {
    Some(match kind {
        ScWebSocketErrorKind::InvalidUrl | ScWebSocketErrorKind::MissingHubHost => return None,
        ScWebSocketErrorKind::UnsupportedScheme => ErrorCode::WEBSOCKET_SCHEME_NOT_SUPPORTED,
        ScWebSocketErrorKind::HostLookup => ErrorCode::DNS_ERROR,
        ScWebSocketErrorKind::TcpDial => ErrorCode::TCP_ERROR,
        ScWebSocketErrorKind::TlsHandshake => ErrorCode::TLS_ERROR,
        ScWebSocketErrorKind::WebSocketHandshake => ErrorCode::HTTP_ERROR,
        // The Sec-WebSocket-Protocol answer lacked the BACnet/SC subprotocol.
        ScWebSocketErrorKind::HubSubprotocol | ScWebSocketErrorKind::DirectSubprotocol => {
            ErrorCode::HTTP_WEBSOCKET_HEADER_ERROR
        }
        // Answered with close status 1003 (Annex AB.7.5.3).
        ScWebSocketErrorKind::NonBinaryFrame => ErrorCode::WEBSOCKET_DATA_NOT_ACCEPTED,
        // The stream ended with no close frame at all.
        ScWebSocketErrorKind::StreamEnded => ErrorCode::WEBSOCKET_CLOSED_ABNORMALLY,
        // A close whose status code was not kept, or a send or receive fault.
        ScWebSocketErrorKind::Closed
        | ScWebSocketErrorKind::Send
        | ScWebSocketErrorKind::Receive => ErrorCode::WEBSOCKET_ERROR,
    })
}

/// A status other than 101 Switching Protocols in answer to the upgrade.
fn http_code(status: u16) -> ErrorCode {
    match status {
        200..=299 => ErrorCode::HTTP_NO_UPGRADE,
        300..=399 => ErrorCode::HTTP_RESOURCE_NOT_LOCAL,
        407 => ErrorCode::HTTP_PROXY_AUTHENTICATION_FAILED,
        503 => ErrorCode::HTTP_TEMPORARY_UNAVAILABLE,
        _ => ErrorCode::HTTP_UNEXPECTED_RESPONSE_CODE,
    }
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::ErrorClass;
    use bacnet_types::error::Error;

    use super::*;
    use crate::sc::ScConnectError;
    use crate::sc_frame::ScFunction;

    fn ws_error(kind: ScWebSocketErrorKind, http_status: Option<u16>) -> Error {
        ScConnectError::WebSocket {
            kind,
            message: "test".into(),
            tls: None,
            #[cfg(feature = "sc-tls")]
            untrusted_peer: None,
            http_status,
        }
        .into_bacnet_error()
    }

    #[test]
    fn a_dial_that_times_out_in_tcp_connect_reports_tcp_connect_timeout() {
        let cause = ScDisconnectCause::from_error(
            &ScConnectError::DialTimeout {
                timeout: std::time::Duration::from_secs(5),
                stage: Some(ScDialStage::TcpConnect),
            }
            .into_bacnet_error(),
        );
        let code = cause.error_code().unwrap();
        assert_eq!(code, ErrorCode::TCP_CONNECT_TIMEOUT);
        assert_eq!(code.to_raw(), 193);
        // Displayed by name, not as a bare number.
        assert_eq!(code.to_string(), "TCP_CONNECT_TIMEOUT");

        let in_upgrade = ScDisconnectCause::DialTimeout {
            stage: Some(ScDialStage::WebSocketUpgrade),
        };
        assert_eq!(
            in_upgrade.error_code(),
            Some(ErrorCode::HTTP_RESPONSE_TIMEOUT)
        );
        assert_eq!(
            ScDisconnectCause::DialTimeout { stage: None }.error_code(),
            None
        );
    }

    #[test]
    fn tls_codes_name_the_side_that_refused() {
        let expired = ScDisconnectCause::Tls(ScTlsFailure::PeerCertificate(
            ScCertificateRejection::Expired { by: None },
        ));
        assert_eq!(
            expired.error_code(),
            Some(ErrorCode::TLS_SERVER_CERTIFICATE_EXPIRED)
        );
        let refused_us = ScDisconnectCause::Tls(ScTlsFailure::AlertReceived(ScTlsAlert::UnknownCa));
        assert_eq!(
            refused_us.error_code(),
            Some(ErrorCode::TLS_CLIENT_AUTHENTICATION_FAILED)
        );
        let revoked_us =
            ScDisconnectCause::Tls(ScTlsFailure::AlertReceived(ScTlsAlert::CertificateRevoked));
        assert_eq!(
            revoked_us.error_code(),
            Some(ErrorCode::TLS_CLIENT_CERTIFICATE_REVOKED)
        );
        assert_eq!(
            ScDisconnectCause::Tls(ScTlsFailure::PeerIncompatible).error_code(),
            Some(ErrorCode::TLS_ERROR)
        );
    }

    #[test]
    fn connect_path_failures_classify_to_their_layer() {
        let code = |error: Error| ScDisconnectCause::from_error(&error).error_code();
        assert_eq!(
            code(ws_error(ScWebSocketErrorKind::UnsupportedScheme, None)),
            Some(ErrorCode::WEBSOCKET_SCHEME_NOT_SUPPORTED)
        );
        assert_eq!(
            code(ws_error(ScWebSocketErrorKind::HostLookup, None)),
            Some(ErrorCode::DNS_ERROR)
        );
        assert_eq!(
            code(ws_error(ScWebSocketErrorKind::TcpDial, None)),
            Some(ErrorCode::TCP_ERROR)
        );
        assert_eq!(
            code(ws_error(ScWebSocketErrorKind::HubSubprotocol, None)),
            Some(ErrorCode::HTTP_WEBSOCKET_HEADER_ERROR)
        );
        assert_eq!(code(ws_error(ScWebSocketErrorKind::InvalidUrl, None)), None);
        // An HTTP status in place of 101 Switching Protocols.
        let upgrade = |status| {
            code(ws_error(
                ScWebSocketErrorKind::WebSocketHandshake,
                Some(status),
            ))
        };
        assert_eq!(upgrade(502), Some(ErrorCode::HTTP_UNEXPECTED_RESPONSE_CODE));
        assert_eq!(upgrade(503), Some(ErrorCode::HTTP_TEMPORARY_UNAVAILABLE));
        assert_eq!(
            upgrade(407),
            Some(ErrorCode::HTTP_PROXY_AUTHENTICATION_FAILED)
        );
        assert_eq!(upgrade(301), Some(ErrorCode::HTTP_RESOURCE_NOT_LOCAL));
        assert_eq!(upgrade(200), Some(ErrorCode::HTTP_NO_UPGRADE));
        // A closed WebSocket reads as a peer close; its status is not kept.
        assert_eq!(code(ws_error(ScWebSocketErrorKind::Closed, None)), None);
    }

    #[test]
    fn a_nak_reports_its_own_code_and_quiet_causes_report_none() {
        let nak = ScDisconnectCause::Nak {
            result_for: ScFunction::ConnectRequest,
            error_class: ErrorClass::COMMUNICATION,
            error_code: ErrorCode::NODE_DUPLICATE_VMAC,
        };
        assert_eq!(nak.error_code(), Some(ErrorCode::NODE_DUPLICATE_VMAC));
        for quiet in [
            ScDisconnectCause::LocalClose,
            ScDisconnectCause::PeerClose,
            ScDisconnectCause::HeartbeatTimeout,
            ScDisconnectCause::ConnectAcceptTimeout,
            ScDisconnectCause::Other("x".into()),
        ] {
            assert_eq!(quiet.error_code(), None, "{quiet:?}");
        }
    }
}
