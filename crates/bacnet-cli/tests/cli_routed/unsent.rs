//! The "nothing was sent" check (#2627, #2680) fails on the command's own
//! datagram and on nothing else.
use super::*;
use std::net::UdpSocket;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// A datagram to the peer from a socket that is not the command's.
fn stray(peer: &Unsent) -> UdpSocket {
    let other = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    assert_ne!(other.local_addr().unwrap().port(), peer.source_port());
    let ack = [0x81, 0x0a, 0x00, 0x0d, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    other
        .send_to(&ack, (Ipv4Addr::LOCALHOST, peer.port()))
        .unwrap();
    other
}

/// A datagram to the peer from the port the command was given.
fn from_the_command(peer: &Unsent) -> UdpSocket {
    let own = UdpSocket::bind((Ipv4Addr::LOCALHOST, peer.source_port())).unwrap();
    let registration = [0x81, 0x05, 0x00, 0x06, 0x00, 0x3c];
    own.send_to(&registration, (Ipv4Addr::LOCALHOST, peer.port()))
        .unwrap();
    own
}

#[test]
fn an_unrelated_datagram_does_not_fail_the_check() {
    let peer = Unsent::new();
    let other = stray(&peer);
    peer.wait_for_from(other.local_addr().unwrap().port(), 1);
    peer.assert_nothing_sent("stray");
    // Set aside, not held: a second look passes too.
    peer.assert_nothing_sent("again");
}

#[test]
fn a_datagram_from_the_commands_port_fails_the_check() {
    let peer = Unsent::new();
    let _own = from_the_command(&peer);
    peer.wait_for_from(peer.source_port(), 1);
    let failed = catch_unwind(AssertUnwindSafe(|| peer.assert_nothing_sent("own")));
    assert!(failed.is_err(), "the command's datagram went unnoticed");
}

#[test]
fn a_stray_ahead_of_the_commands_datagram_does_not_hide_it() {
    let peer = Unsent::new();
    let other = stray(&peer);
    let _own = from_the_command(&peer);
    peer.wait_for_from(other.local_addr().unwrap().port(), 1);
    peer.wait_for_from(peer.source_port(), 1);
    let failed = catch_unwind(AssertUnwindSafe(|| peer.assert_nothing_sent("both")));
    assert!(failed.is_err(), "a stray hid the command's datagram");
}

/// The real CLI, with no refusal in its way (a read needs no flag), registers
/// with the peer as a foreign device from the port it was given, and the
/// check sees it.
#[tokio::test]
async fn a_real_cli_that_sends_fails_the_check() {
    let peer = Unsent::new();
    let mut cmd = command();
    cmd.env("BACNET_CLI_ROUTED_WAIT_MS", "300").args([
        "--interface",
        "127.0.0.1",
        "--port",
        &peer.source_port().to_string(),
        "--json",
        "--foreign-bbmd",
        &format!("127.0.0.1:{}", peer.port()),
        "read",
        "2:1234",
        "analog-input:2",
        "present-value",
    ]);
    let output = Process::spawn(&mut cmd).output().await;
    // Not a refusal: the CLI got as far as the BBMD, which never answers.
    assert_ne!(output.status.code(), Some(4));
    let failed = catch_unwind(AssertUnwindSafe(|| peer.assert_nothing_sent("read")));
    assert!(failed.is_err(), "a CLI that sent went unnoticed");
}
