//! Original accepted-direct replies from public client and endpoint consumers.
#![cfg(feature = "sc-tls")]
#[path = "direct_replies/support.rs"]
mod support;
use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_encoding::apdu::Apdu;
use bacnet_endpoint::session::{EndpointSession, SessionConfig, SessionRole};
use support::*;

#[tokio::test]
async fn standalone_client_replies_on_original_tls_socket() {
    let ca = TestCa::new();
    let (mut f, port) = Fixture::new(&ca).await;
    let mut client = BACnetClient::start(ClientConfig::default(), port)
        .await
        .unwrap();
    let mut peer = f.peer(ca.tls("A"), None).await;
    let admitted = f.capture(&mut peer, &unsupported(7), None).await;
    f.feed(admitted).await;
    assert!(matches!(f.response(&peer).await.1, Apdu::Reject(r) if r.invoke_id == 7));
    client.stop().await.unwrap();
    f.listener.stop().await;
}

#[tokio::test]
async fn shared_endpoint_replies_on_original_tls_socket() {
    let ca = TestCa::new();
    let (mut f, port) = Fixture::new(&ca).await;
    let mut session = EndpointSession::new(port, SessionRole::Both, SessionConfig::default())
        .unwrap()
        .device(node())
        .with_database(database("original"));
    session.start().await.unwrap();
    let mut peer = f.peer(ca.tls("A"), None).await;
    let admitted = f.capture(&mut peer, &read_name(8), None).await;
    f.feed(admitted).await;
    assert!(matches!(f.response(&peer).await.1, Apdu::ComplexAck(a) if a.invoke_id == 8));
    session.stop().await.unwrap();
    f.listener.stop().await;
}

#[path = "direct_replies/authority.rs"]
mod authority;
#[path = "direct_replies/budget.rs"]
mod budget;
#[path = "direct_replies/execution.rs"]
mod execution;
#[path = "direct_replies/lifecycle.rs"]
mod lifecycle;
#[path = "direct_replies/notifications.rs"]
mod notifications;

#[path = "direct_replies/bidirectional.rs"]
mod bidirectional;
