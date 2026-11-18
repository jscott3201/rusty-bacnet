//! A session built over a wrapped transport (#2617): the builders'
//! `*_wrapped` forms hand the one transport they build to the caller's
//! wrapper first, as a binding does to give the full client the type its
//! other clients have, and the session's link calls keep working.

use std::net::Ipv4Addr;

use bacnet_transport::any::AnyTransport;
use bacnet_transport::mstp::LoopbackSerial;
use bacnet_transport::port::TransportPort;

use crate::bip::BipEndpointBuilder;
use crate::mstp::MstpEndpointBuilder;
use crate::session::SessionRole;

/// A B/IP session over `AnyTransport` keeps its BBMD table reader, its
/// frame tap and its local address, and its full client sends from the
/// session's one socket. Real UDP on loopback, so real time.
#[tokio::test]
async fn a_wrapped_bip_session_keeps_its_link_calls_and_one_socket() {
    let mut session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST)
        .role(SessionRole::Both)
        .device(crate::DeviceIdentity::new(123, 42).unwrap())
        .build_session_wrapped(AnyTransport::from)
        .unwrap();
    assert!(session.bvlc_tables().is_some(), "taken before the wrap");
    session.start().await.unwrap();
    let local = session.bip_local_address().expect("running");
    assert!(
        session.frames(4).is_some(),
        "the tap, taken before the wrap"
    );
    let client = session.full_client().unwrap();
    let (ip, port) =
        bacnet_transport::bvll::decode_bip_mac(client.transport().local_mac()).unwrap();
    assert_eq!((Ipv4Addr::from(ip), port), (*local.ip(), local.port()));
    assert!(client.transport().bip_transport().is_some());
    drop(client);
    session.stop().await.unwrap();
}

/// An MS/TP session over `AnyTransport`, its serial port boxed, keeps its
/// diagnostics: the handle taken before the wrap still counts what the
/// wrapped transport does, here the read error a closed port gives.
#[tokio::test]
async fn a_wrapped_mstp_session_keeps_its_diagnostics() {
    let (serial, peer) = LoopbackSerial::pair();
    let mut session = MstpEndpointBuilder::new(serial, 3)
        .role(SessionRole::ClientOnly)
        .build_session_wrapped(AnyTransport::from)
        .unwrap();
    let diagnostics = session.mstp_diagnostics().expect("taken before the wrap");
    session.start().await.unwrap();
    assert_eq!(diagnostics.snapshot().serial_read_errors, 0);
    drop(peer);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while diagnostics.snapshot().serial_read_errors == 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the closed port's read error is counted"
        );
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    let _ = session.stop().await;
}
