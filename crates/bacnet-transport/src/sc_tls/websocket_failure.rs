//! How the WebSocket driver reports a failed upgrade and a WebSocket the
//! peer ended (#2438): a subprotocol answer that is missing or wrong, a close
//! frame with its status code, and a connection lost with no close frame.
//! Each becomes a typed [`ScConnectError::WebSocket`], which
//! `ScDisconnectCause` maps to its error code.

use tokio_tungstenite::tungstenite::Error as TungsteniteError;

use bacnet_types::error::Error;

use super::peer_info::escaped_bounded;
use super::tls_failure;
use crate::sc::{ScConnectError, ScWebSocketErrorKind};
use crate::sc_frame::{BACNET_SC_DIRECT_SUBPROTOCOL, BACNET_SC_HUB_SUBPROTOCOL};

/// The kind for a failed TCP dial, from the OS error's kind: a refusal and
/// no route each have their own code (Annex AB.7.5.1's TCP and IP tables).
/// The standard library gives these kinds for the Unix and Winsock errors
/// alike (`ECONNREFUSED`, `EHOSTUNREACH`, `ENETUNREACH` and their `WSAE`
/// twins), so the test is the same on every OS (#2463).
pub(super) fn tcp_dial_kind(kind: std::io::ErrorKind) -> ScWebSocketErrorKind {
    use std::io::ErrorKind;
    match kind {
        ErrorKind::ConnectionRefused => ScWebSocketErrorKind::TcpRefused,
        ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable => {
            ScWebSocketErrorKind::AddressUnreachable
        }
        _ => ScWebSocketErrorKind::TcpDial,
    }
}

/// Map a failed upgrade that offered `subprotocol`. tungstenite checks the
/// answer's subprotocol itself, so a missing or wrong one fails here.
///
/// tungstenite also tells apart (#2463): a fault in the answer's `Upgrade`,
/// `Connection` or `Sec-WebSocket-Accept` header (without saying whether
/// the header was absent or wrong), and an answer that is not HTTP/1.1 it
/// can parse. Each gets its own kind. What it does not tell apart stays
/// `WebSocketHandshake`: the connection ending before the answer was
/// complete, an I/O error, more than its header limit, an answer it reads
/// as an attack (too large, or in too many small pieces), and a
/// subprotocol value that is not text.
pub(super) fn map_websocket_upgrade_error(error: TungsteniteError, subprotocol: &str) -> Error {
    use tokio_tungstenite::tungstenite::error::{ProtocolError, SubProtocolError};
    let kind = match &error {
        TungsteniteError::Protocol(ProtocolError::SecWebSocketSubProtocolError(fault)) => {
            let (wrong, missing) = subprotocol_faults(subprotocol);
            match fault {
                SubProtocolError::NoSubProtocol => missing,
                _ => wrong,
            }
        }
        TungsteniteError::Protocol(
            ProtocolError::MissingUpgradeWebSocketHeader
            | ProtocolError::MissingConnectionUpgradeHeader
            | ProtocolError::SecWebSocketAcceptKeyMismatch,
        ) => ScWebSocketErrorKind::WebSocketHeader,
        TungsteniteError::Protocol(
            ProtocolError::HttparseError(_) | ProtocolError::WrongHttpVersion,
        )
        | TungsteniteError::HttpFormat(_) => ScWebSocketErrorKind::UpgradeResponseSyntax,
        TungsteniteError::Io(_)
        | TungsteniteError::Tls(_)
        | TungsteniteError::Protocol(_)
        | TungsteniteError::Http(_) => ScWebSocketErrorKind::WebSocketHandshake,
        TungsteniteError::Url(_) => ScWebSocketErrorKind::InvalidUrl,
        _ => ScWebSocketErrorKind::WebSocketHandshake,
    };
    ScConnectError::WebSocket {
        // A server that answers the upgrade with another status, such as a
        // proxy's 502, keeps that status for diagnosis (#1839).
        http_status: match &error {
            TungsteniteError::Http(response) => Some(response.status().as_u16()),
            _ => None,
        },
        close_status: None,
        untrusted_peer: None,
        // A TLS 1.3 server refuses a client certificate after the client has
        // finished its side of the handshake, so the alert surfaces here.
        tls: match &error {
            TungsteniteError::Io(io) => tls_failure::classify(io),
            _ => None,
        },
        kind,
        message: format!("WebSocket upgrade failed: {error}"),
    }
    .into_bacnet_error()
}

/// The failure kinds for an upgrade answer that names another subprotocol
/// than `subprotocol`, and for one that names none.
fn subprotocol_faults(subprotocol: &str) -> (ScWebSocketErrorKind, ScWebSocketErrorKind) {
    if subprotocol == BACNET_SC_DIRECT_SUBPROTOCOL {
        (
            ScWebSocketErrorKind::DirectSubprotocol,
            ScWebSocketErrorKind::DirectSubprotocolMissing,
        )
    } else {
        (
            ScWebSocketErrorKind::HubSubprotocol,
            ScWebSocketErrorKind::HubSubprotocolMissing,
        )
    }
}

pub(super) fn verify_hub_subprotocol(
    response: &tokio_tungstenite::tungstenite::handshake::client::Response,
) -> Result<(), Error> {
    verify_selected_subprotocol(response, BACNET_SC_HUB_SUBPROTOCOL, "hub")
}

pub(super) fn verify_direct_subprotocol(
    response: &tokio_tungstenite::tungstenite::handshake::client::Response,
) -> Result<(), Error> {
    verify_selected_subprotocol(response, BACNET_SC_DIRECT_SUBPROTOCOL, "direct")
}

/// tungstenite has already checked the answer; this keeps the check here
/// should it stop doing so.
fn verify_selected_subprotocol(
    response: &tokio_tungstenite::tungstenite::handshake::client::Response,
    expected: &str,
    role: &str,
) -> Result<(), Error> {
    let (wrong, missing) = subprotocol_faults(expected);
    match response.headers().get("Sec-WebSocket-Protocol") {
        Some(selected) if selected.to_str().ok() == Some(expected) => Ok(()),
        Some(_) => Err(ScConnectError::websocket(
            wrong,
            format!("BACnet/SC {role} WebSocket subprotocol {expected} was not accepted"),
        )
        .into_bacnet_error()),
        None => Err(ScConnectError::websocket(
            missing,
            format!("BACnet/SC {role} WebSocket upgrade answer named no subprotocol"),
        )
        .into_bacnet_error()),
    }
}

/// The most of a peer's close reason an error message keeps, after
/// escaping. RFC 6455 allows 123 bytes; escaping can lengthen them.
const CLOSE_REASON_LIMIT: usize = 256;

/// The error for a close frame from the peer, keeping its status code
/// (Annex AB.7.5.5 maps it to an error code). The reason is text the peer
/// chose and reaches terminals and logs, so its controls and non-ASCII are
/// escaped and its length bounded, as for peer certificate text.
pub(super) fn closed_by_peer(
    frame: Option<tokio_tungstenite::tungstenite::protocol::CloseFrame>,
) -> Error {
    let close_status = frame.as_ref().map(|frame| u16::from(frame.code));
    let message = match &frame {
        Some(frame) if frame.reason.is_empty() => {
            format!("WebSocket closed by the peer with status {}", frame.code)
        }
        Some(frame) => {
            let mut truncated = false;
            let mut reason =
                escaped_bounded(&frame.reason.as_str(), &mut truncated, CLOSE_REASON_LIMIT);
            if truncated {
                reason.push_str("...");
            }
            format!(
                "WebSocket closed by the peer with status {}: {reason}",
                frame.code
            )
        }
        None => "WebSocket closed by the peer without a status".to_owned(),
    };
    ScConnectError::WebSocket {
        kind: ScWebSocketErrorKind::Closed,
        message,
        tls: None,
        untrusted_peer: None,
        http_status: None,
        close_status,
    }
    .into_bacnet_error()
}

/// The connection under the WebSocket was lost with no close frame:
/// tungstenite's own report of that, or the TCP or TLS stream failing in a
/// way that means the peer is gone. RFC 6455 reports such an end as 1006.
pub(super) fn ended_without_close(error: &TungsteniteError) -> bool {
    use std::io::ErrorKind;
    use tokio_tungstenite::tungstenite::error::ProtocolError;
    match error {
        TungsteniteError::Protocol(ProtocolError::ResetWithoutClosingHandshake) => true,
        TungsteniteError::Io(io) => matches!(
            io.kind(),
            ErrorKind::UnexpectedEof
                | ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted
                | ErrorKind::BrokenPipe
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

    use super::*;

    fn message(reason: &str) -> String {
        let error = closed_by_peer(Some(CloseFrame {
            code: CloseCode::Away,
            reason: reason.into(),
        }));
        match ScConnectError::from_error(&error) {
            Some(ScConnectError::WebSocket {
                message,
                close_status,
                ..
            }) => {
                assert_eq!(*close_status, Some(1001));
                message.clone()
            }
            other => panic!("expected a WebSocket close, got {other:?}"),
        }
    }

    /// A hostile close reason can't drive a terminal: its controls, an ANSI
    /// escape sequence and bidi text arrive escaped (#2438).
    #[test]
    fn a_close_reason_is_escaped() {
        let shown = message("bye\x1b[2J\x1b[31mred\r\nfake: line\u{202e}txt\x07");
        assert!(!shown.chars().any(char::is_control), "{shown:?}");
        assert!(shown.is_ascii(), "{shown:?}");
        assert!(
            shown
                .ends_with(r"status 1001: bye\u{1b}[2J\u{1b}[31mred\r\nfake: line\u{202e}txt\u{7}"),
            "{shown}"
        );
    }

    /// The OS errors for a refused dial and for no route reach the same
    /// kinds on every OS: the standard library decodes the Unix and Winsock
    /// numbers alike, and the classifier reads only the kind (#2463).
    #[cfg(any(unix, windows))]
    #[test]
    fn os_dial_errors_classify_the_same_on_every_os() {
        use std::io::{Error as IoError, ErrorKind};
        #[cfg(unix)]
        let (refused, host, network) = (libc::ECONNREFUSED, libc::EHOSTUNREACH, libc::ENETUNREACH);
        #[cfg(windows)]
        let (refused, host, network) = {
            use windows_sys::Win32::Networking::WinSock as ws;
            (ws::WSAECONNREFUSED, ws::WSAEHOSTUNREACH, ws::WSAENETUNREACH)
        };
        for (code, kind) in [
            (refused, ScWebSocketErrorKind::TcpRefused),
            (host, ScWebSocketErrorKind::AddressUnreachable),
            (network, ScWebSocketErrorKind::AddressUnreachable),
        ] {
            assert_eq!(
                tcp_dial_kind(IoError::from_raw_os_error(code).kind()),
                kind,
                "OS error {code}"
            );
        }
        for other in [
            ErrorKind::TimedOut,
            ErrorKind::NetworkDown,
            ErrorKind::Other,
        ] {
            assert_eq!(tcp_dial_kind(other), ScWebSocketErrorKind::TcpDial);
        }
    }

    /// A long reason is cut at the bound, and says so.
    #[test]
    fn a_long_close_reason_is_bounded() {
        let shown = message(&"\u{1b}".repeat(123));
        let reason = shown.split_once(": ").unwrap().1;
        assert!(reason.len() <= CLOSE_REASON_LIMIT + 3, "{}", reason.len());
        assert!(reason.ends_with("..."), "{reason}");
        assert!(!shown.chars().any(char::is_control));
    }
}
