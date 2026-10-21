//! Peers that keep connecting and ending their connections badly warn
//! through the hub-wide connection throttle (#1950), however many
//! connections they use: once per interval, the rest at debug.

use super::admission::AdmissionRuntime;
use super::heartbeat_test_support::*;
use super::*;
use crate::warn_throttle::WARNING_INTERVAL;

/// A Connect-Request claiming `vmac`.
fn connect_request(vmac: Vmac) -> ScMessage {
    let mut request = frame(ScFunction::ConnectRequest, 1);
    let mut payload = Vec::from(vmac);
    payload.extend_from_slice(&[0x33; 16]);
    payload.extend_from_slice(&1476u16.to_be_bytes());
    payload.extend_from_slice(&1476u16.to_be_bytes());
    request.payload = Bytes::from(payload);
    request
}

#[tokio::test]
async fn connections_a_peer_ends_badly_share_one_warning_throttle() {
    let admission = Arc::new(AdmissionRuntime::default());
    let open = |vmac| LiveClient::open_with(clients(), vmac, Arc::clone(&admission));

    // Three Connect-Requests claiming the hub's own VMAC, each NAKed.
    for _ in 0..3 {
        let mut live = open([0x22; 6]).await;
        live.send(connect_request([0x10; 6])).await;
        assert_eq!(live.recv().await.function, ScFunction::Result);
        live.expect_closed().await;
    }
    // A text frame, which BACnet/SC doesn't carry.
    let mut live = open([0x22; 6]).await;
    live.ws.send(Message::Text("hello".into())).await.unwrap();
    live.expect_closed().await;
    // A second Connect-Request from a client already connected.
    let mut live = open([0x22; 6]).await;
    live.send(connect_request([0x22; 6])).await;
    assert_eq!(live.recv().await.function, ScFunction::ConnectAccept);
    live.send(connect_request([0x22; 6])).await;
    live.expect_closed().await;
    drop(live);

    let throttle = &admission.connection_warning;
    assert_eq!(throttle.note(), None, "the first warned; four held back");
    tokio::time::pause();
    tokio::time::advance(WARNING_INTERVAL).await;
    assert_eq!(
        throttle.note(),
        Some(6),
        "four held back, the check above, and this one"
    );
}
