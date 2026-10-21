//! A router's local delivery gives the reply capability only to a
//! Confirmed-Request, the one APDU answered on its arrival route (#1713).

use crate::loopback_fixture::{recv, RouterFixture};

/// NPDU bytes for a local APDU: version 1, no addresses, then `apdu`.
fn local(apdu: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x01, 0x00];
    bytes.extend_from_slice(apdu);
    bytes
}

#[tokio::test]
async fn only_a_confirmed_request_carries_the_router_reply_capability() {
    let mut fixture = RouterFixture::start().await;
    // An unconfirmed Who-Is, then a confirmed ReadProperty request header.
    fixture.send_from_a(&local(&[0x10, 0x08])).await;
    fixture.send_from_a(&local(&[0x00, 0x05, 0x01, 0x0C])).await;
    let unconfirmed = recv(&mut fixture.local).await;
    assert!(unconfirmed.router_response.is_none());
    assert!(!unconfirmed.response_route().requires_original_route());
    let confirmed = recv(&mut fixture.local).await;
    assert!(confirmed.router_response.is_some());
    assert!(confirmed.response_route().requires_original_route());
}
