//! How the WebSocket driver reports a failed upgrade and a WebSocket the
//! peer ended (#2438): a subprotocol answer that is missing or wrong, a close
//! frame with its status code, and a connection lost with no close frame.
//! Each becomes a typed [`ScConnectError::WebSocket`], which
//! `ScDisconnectCause` maps to its error code.

use tokio_tungstenite::tungstenite::Error as TungsteniteError;

use bacnet_types::error::Error;

use super::tls_failure;
use crate::sc::{ScConnectError, ScWebSocketErrorKind};
use crate::sc_frame::{BACNET_SC_DIRECT_SUBPROTOCOL, BACNET_SC_HUB_SUBPROTOCOL};

/// Map a failed upgrade that offered `subprotocol`. tungstenite checks the
/// answer's subprotocol itself, so a missing or wrong one fails here.
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
        TungsteniteError::Io(_)
        | TungsteniteError::Tls(_)
        | TungsteniteError::Protocol(_)
        | TungsteniteError::Http(_)
        | TungsteniteError::HttpFormat(_) => ScWebSocketErrorKind::WebSocketHandshake,
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

/// The error for a close frame from the peer, keeping its status code
/// (Annex AB.7.5.5 maps it to an error code).
pub(super) fn closed_by_peer(
    frame: Option<tokio_tungstenite::tungstenite::protocol::CloseFrame>,
) -> Error {
    let close_status = frame.as_ref().map(|frame| u16::from(frame.code));
    let message = match &frame {
        Some(frame) if frame.reason.is_empty() => {
            format!("WebSocket closed by the peer with status {}", frame.code)
        }
        Some(frame) => format!(
            "WebSocket closed by the peer with status {}: {}",
            frame.code, frame.reason
        ),
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
