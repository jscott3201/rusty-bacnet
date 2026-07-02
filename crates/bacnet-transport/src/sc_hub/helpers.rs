//! Shared helpers for BACnet/SC hub connection handling.

use std::collections::HashMap;
use std::sync::Arc;

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::Bytes;
use tokio::sync::Mutex;

use crate::sc_frame::{ScFunction, ScMessage, Vmac, BACNET_SC_HUB_SUBPROTOCOL, BROADCAST_VMAC};

use super::{
    Clients, ConnectRequestVmacDisposition, DeviceUuid, HubClient, HubClientRegistrationDecision,
    RelayLimitDecision, WsSink,
};

pub(super) fn offers_websocket_subprotocol(
    request: &tokio_tungstenite::tungstenite::handshake::server::Request,
    expected: &str,
) -> bool {
    request
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').any(|p| p.trim() == expected))
        .unwrap_or(false)
}

pub(super) fn websocket_subprotocol_error_response(
) -> tokio_tungstenite::tungstenite::handshake::server::ErrorResponse {
    tokio_tungstenite::tungstenite::http::Response::builder()
        .status(tokio_tungstenite::tungstenite::http::StatusCode::BAD_REQUEST)
        .body(Some(format!(
            "BACnet/SC hub requires WebSocket subprotocol {BACNET_SC_HUB_SUBPROTOCOL}"
        )))
        .expect("static WebSocket error response is valid")
}

pub(super) fn unexpected_bvlc_function_error_code(function: ScFunction) -> ErrorCode {
    match function {
        ScFunction::Unknown(_) => ErrorCode::BVLC_FUNCTION_UNKNOWN,
        _ => ErrorCode::UNEXPECTED_DATA,
    }
}

pub(super) fn connect_request_vmac_disposition(
    vmac: Vmac,
    hub_vmac: Vmac,
) -> ConnectRequestVmacDisposition {
    if vmac == crate::sc_frame::UNKNOWN_VMAC || vmac == BROADCAST_VMAC {
        ConnectRequestVmacDisposition::CloseReserved
    } else if vmac == hub_vmac {
        ConnectRequestVmacDisposition::Nak(
            ErrorClass::COMMUNICATION,
            ErrorCode::NODE_DUPLICATE_VMAC,
        )
    } else {
        ConnectRequestVmacDisposition::Accept
    }
}

pub(super) fn hub_client_registration_decision<I>(
    requested_vmac: Vmac,
    device_uuid: DeviceUuid,
    existing_clients: I,
    max_clients: usize,
) -> HubClientRegistrationDecision
where
    I: IntoIterator<Item = (Vmac, DeviceUuid)>,
{
    let mut count = 0usize;
    let mut requested_vmac_owner = None;
    let mut existing_uuid_vmac = None;

    for (existing_vmac, existing_uuid) in existing_clients {
        count += 1;
        if existing_vmac == requested_vmac {
            requested_vmac_owner = Some(existing_uuid);
        }
        if existing_uuid == device_uuid {
            existing_uuid_vmac = Some(existing_vmac);
        }
    }

    if requested_vmac_owner.is_some_and(|owner_uuid| owner_uuid != device_uuid) {
        return HubClientRegistrationDecision::NakDuplicateVmac;
    }

    if let Some(old_vmac) = existing_uuid_vmac {
        return HubClientRegistrationDecision::Replace { old_vmac };
    }

    if count >= max_clients {
        HubClientRegistrationDecision::NakMaxClients
    } else {
        HubClientRegistrationDecision::Accept
    }
}

pub(super) async fn registered_client_matches_sink(
    clients: &Clients,
    registered_vmac: Vmac,
    sink: &Arc<Mutex<WsSink>>,
) -> bool {
    let map = clients.lock().await;
    registered_client_matches_sink_in_map(&map, registered_vmac, sink)
}

pub(super) fn registered_client_matches_sink_in_map(
    map: &HashMap<Vmac, HubClient>,
    registered_vmac: Vmac,
    sink: &Arc<Mutex<WsSink>>,
) -> bool {
    map.get(&registered_vmac)
        .is_some_and(|client| Arc::ptr_eq(&client.sink, sink))
}

pub(super) fn relay_limit_decision(
    npdu_len: usize,
    encoded_bvlc_len: usize,
    max_npdu: u16,
    max_bvlc: u16,
) -> RelayLimitDecision {
    if npdu_len > max_npdu as usize {
        RelayLimitDecision::DropMaxNpdu
    } else if encoded_bvlc_len > max_bvlc as usize {
        RelayLimitDecision::DropMaxBvlc
    } else {
        RelayLimitDecision::Send
    }
}

/// Build a BVLC-Result NAK message.
pub(super) fn build_bvlc_result_nak(
    message_id: u16,
    result_for: ScFunction,
    error_class: ErrorClass,
    error_code: ErrorCode,
) -> ScMessage {
    let error_class = error_class.to_raw().to_be_bytes();
    let error_code = error_code.to_raw().to_be_bytes();
    ScMessage {
        function: ScFunction::Result,
        message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(vec![
            result_for.to_raw(),
            0x01, // NAK
            0x00, // error header marker
            error_class[0],
            error_class[1],
            error_code[0],
            error_code[1],
        ]),
    }
}

/// Current time in seconds since UNIX epoch.
pub(super) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
