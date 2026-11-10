//! `ErrorCode`: the BACnet/SC connection codes 152-199 (#2434) and the
//! table's extent.

use super::super::*;

/// The HTTP, WebSocket, TLS, DNS, TCP and IP codes, in value order, as the
/// 2020 edition numbers them (Clause 21, `ErrorCode`).
const SC_CONNECTION_CODES: [(&str, u16); 48] = [
    ("HTTP_UNEXPECTED_RESPONSE_CODE", 152),
    ("HTTP_NO_UPGRADE", 153),
    ("HTTP_RESOURCE_NOT_LOCAL", 154),
    ("HTTP_PROXY_AUTHENTICATION_FAILED", 155),
    ("HTTP_RESPONSE_TIMEOUT", 156),
    ("HTTP_RESPONSE_SYNTAX_ERROR", 157),
    ("HTTP_RESPONSE_VALUE_ERROR", 158),
    ("HTTP_RESPONSE_MISSING_HEADER", 159),
    ("HTTP_WEBSOCKET_HEADER_ERROR", 160),
    ("HTTP_UPGRADE_REQUIRED", 161),
    ("HTTP_UPGRADE_ERROR", 162),
    ("HTTP_TEMPORARY_UNAVAILABLE", 163),
    ("HTTP_NOT_A_SERVER", 164),
    ("HTTP_ERROR", 165),
    ("WEBSOCKET_SCHEME_NOT_SUPPORTED", 166),
    ("WEBSOCKET_UNKNOWN_CONTROL_MESSAGE", 167),
    ("WEBSOCKET_CLOSE_ERROR", 168),
    ("WEBSOCKET_CLOSED_BY_PEER", 169),
    ("WEBSOCKET_ENDPOINT_LEAVES", 170),
    ("WEBSOCKET_PROTOCOL_ERROR", 171),
    ("WEBSOCKET_DATA_NOT_ACCEPTED", 172),
    ("WEBSOCKET_CLOSED_ABNORMALLY", 173),
    ("WEBSOCKET_DATA_INCONSISTENT", 174),
    ("WEBSOCKET_DATA_AGAINST_POLICY", 175),
    ("WEBSOCKET_FRAME_TOO_LONG", 176),
    ("WEBSOCKET_EXTENSION_MISSING", 177),
    ("WEBSOCKET_REQUEST_UNAVAILABLE", 178),
    ("WEBSOCKET_ERROR", 179),
    ("TLS_CLIENT_CERTIFICATE_ERROR", 180),
    ("TLS_SERVER_CERTIFICATE_ERROR", 181),
    ("TLS_CLIENT_AUTHENTICATION_FAILED", 182),
    ("TLS_SERVER_AUTHENTICATION_FAILED", 183),
    ("TLS_CLIENT_CERTIFICATE_EXPIRED", 184),
    ("TLS_SERVER_CERTIFICATE_EXPIRED", 185),
    ("TLS_CLIENT_CERTIFICATE_REVOKED", 186),
    ("TLS_SERVER_CERTIFICATE_REVOKED", 187),
    ("TLS_ERROR", 188),
    ("DNS_UNAVAILABLE", 189),
    ("DNS_NAME_RESOLUTION_FAILED", 190),
    ("DNS_RESOLVER_FAILURE", 191),
    ("DNS_ERROR", 192),
    ("TCP_CONNECT_TIMEOUT", 193),
    ("TCP_CONNECTION_REFUSED", 194),
    ("TCP_CLOSED_BY_LOCAL", 195),
    ("TCP_CLOSED_OTHER", 196),
    ("TCP_ERROR", 197),
    ("IP_ADDRESS_NOT_REACHABLE", 198),
    ("IP_ERROR", 199),
];

#[test]
fn sc_connection_codes_round_trip_by_value_and_name() {
    // They are the table's tail, in order, right after NODE_DUPLICATE_VMAC.
    let tail = &ErrorCode::ALL_NAMED[ErrorCode::ALL_NAMED.len() - SC_CONNECTION_CODES.len()..];
    for (&(name, raw), &(named, value)) in SC_CONNECTION_CODES.iter().zip(tail) {
        assert_eq!(named, name);
        assert_eq!(value.to_raw(), raw, "{name}");
        assert_eq!(ErrorCode::from_raw(raw), value, "{name}");
        assert_eq!(value.name(), Some(name));
        assert_eq!(format!("{value}"), name);
        assert_eq!(format!("{value:?}"), format!("ErrorCode::{name}"));
        // Every spelling parses, kebab-case as Clause 21 writes it included.
        let kebab = name.to_ascii_lowercase().replace('_', "-");
        assert_eq!(kebab.parse::<ErrorCode>(), Ok(value), "{kebab}");
    }
    assert_eq!(ErrorCode::WEBSOCKET_CLOSED_BY_PEER.to_raw(), 169);
    assert_eq!(ErrorCode::IP_ERROR.to_raw(), 199);
}

#[test]
fn error_code_table_ends_at_199_with_every_value_but_33_named() {
    let highest = ErrorCode::ALL_NAMED
        .iter()
        .map(|&(_, code)| code.to_raw())
        .max();
    assert_eq!(highest, Some(199));
    assert_eq!(
        ErrorCode::ALL_NAMED.last().map(|&(n, _)| n),
        Some("IP_ERROR")
    );
    // 0..=199 with value 33 (removed from the standard) left unnamed.
    assert_eq!(ErrorCode::ALL_NAMED.len(), 199);
    assert_eq!(ErrorCode::from_raw(33).name(), None);
    // Values past the table still display as bare numbers.
    assert_eq!(ErrorCode::from_raw(200).name(), None);
    assert_eq!(format!("{}", ErrorCode::from_raw(200)), "200");
    // Raw values are strictly increasing, so no value is named twice.
    assert!(
        ErrorCode::ALL_NAMED
            .windows(2)
            .all(|pair| pair[0].1.to_raw() < pair[1].1.to_raw())
    );
}
