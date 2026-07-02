//! Shared BACnet/SC WebSocket constants and validation helpers.

/// BACnet/SC hub WebSocket subprotocol (Annex AB.7.1).
pub const BACNET_SC_HUB_SUBPROTOCOL: &str = "hub.bsc.bacnet.org";

/// WebSocket close code for unsupported data frames (RFC 6455 status 1003).
pub const WEBSOCKET_DATA_NOT_ACCEPTED_CLOSE_CODE: u16 = 1003;

/// Validate that a configured BACnet/SC WebSocket URI uses the `wss` scheme.
pub fn require_wss_uri(url: &str) -> Result<(), &'static str> {
    let Some((scheme, _rest)) = url.split_once(':') else {
        return Err("BACnet/SC WebSocket URI must use wss scheme");
    };

    if scheme.eq_ignore_ascii_case("wss") {
        Ok(())
    } else {
        Err("BACnet/SC WebSocket URI must use wss scheme")
    }
}

/// Validate that the peer selected the BACnet/SC hub WebSocket subprotocol.
pub fn require_hub_subprotocol(protocol: &str) -> Result<(), &'static str> {
    if protocol == BACNET_SC_HUB_SUBPROTOCOL {
        Ok(())
    } else {
        Err("BACnet/SC hub WebSocket subprotocol was not accepted")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_wss_uri_accepts_secure_websocket_scheme() {
        assert!(require_wss_uri("wss://hub.example.com:443").is_ok());
    }

    #[test]
    fn require_wss_uri_rejects_plain_websocket_scheme() {
        assert!(require_wss_uri("ws://hub.example.com:80").is_err());
    }

    #[test]
    fn require_wss_uri_rejects_missing_scheme() {
        assert!(require_wss_uri("hub.example.com:443").is_err());
    }

    #[test]
    fn require_hub_subprotocol_accepts_hub_protocol() {
        assert!(require_hub_subprotocol(BACNET_SC_HUB_SUBPROTOCOL).is_ok());
    }

    #[test]
    fn require_hub_subprotocol_rejects_missing_or_wrong_protocol() {
        assert!(require_hub_subprotocol("").is_err());
        assert!(require_hub_subprotocol("dc.bsc.bacnet.org").is_err());
    }

    #[test]
    fn websocket_data_not_accepted_close_code_is_unsupported_data() {
        assert_eq!(WEBSOCKET_DATA_NOT_ACCEPTED_CLOSE_CODE, 1003);
    }
}
