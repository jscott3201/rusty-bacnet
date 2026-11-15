//! A B/IP stop through a shared reference (#2617): with another holder of
//! the transport alive, every later send fails with the stopped error, the
//! port can be bound again as soon as the stop returns, stops beside each
//! other both wait for the port, and sends racing a stop end cleanly.

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket as StdUdpSocket};
use std::sync::Arc;
use std::time::Duration;

use super::*;

/// The error kind every send after a stop fails with.
fn stopped(sent: &Result<(), Error>) -> bool {
    matches!(sent, Err(Error::Transport(io)) if io.kind() == std::io::ErrorKind::NotConnected)
}

/// A started transport on loopback, shared, and the address it bound.
async fn shared() -> (Arc<BipTransport>, SocketAddrV4) {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let _rx = transport.start().await.unwrap();
    // The socket binds the wildcard address at the transport's port.
    let bound = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, transport.port);
    (Arc::new(transport), bound)
}

/// A peer on loopback and its B/IP MAC.
fn peer() -> (StdUdpSocket, Vec<u8>) {
    let peer = StdUdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = peer.local_addr().unwrap().port();
    (
        peer,
        encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), port).to_vec(),
    )
}

#[tokio::test]
async fn a_stop_with_another_holder_alive_fails_every_later_send_and_frees_the_port() {
    let (transport, bound) = shared().await;
    let other = Arc::clone(&transport);
    let (peer, mac) = peer();
    other.send_unicast(&[0x01, 0x00], &mac).await.unwrap();
    let mut buf = [0; 16];
    peer.recv(&mut buf).unwrap();

    transport.stop().await.unwrap();
    // The port is free at once, while `other` still holds the transport.
    drop(StdUdpSocket::bind(bound).expect("the stop let go of the port"));
    for sent in [
        other.send_unicast(&[0x01, 0x00], &mac).await,
        other.send_broadcast(&[0x01, 0x00]).await,
        other
            .send_unicast_with_data_attributes(&[0x01, 0x00], &mac, &[])
            .await,
        other
            .send_broadcast_with_data_attributes(&[0x01, 0x00], &[])
            .await,
    ] {
        assert!(stopped(&sent), "{sent:?}");
        assert!(sent.unwrap_err().to_string().contains("transport stopped"));
    }
    assert!(!other.is_running());
    // Idempotent: a second stop finds nothing to do.
    other.stop().await.unwrap();
}

/// Two stops at once both succeed, and the port is free when the second
/// returns. That a stop waits for one already running is the stop gate's,
/// pinned in `stopping_tests`.
#[tokio::test]
async fn stops_beside_each_other_both_return_once_the_port_is_free() {
    for _ in 0..20 {
        let (transport, bound) = shared().await;
        let other = Arc::clone(&transport);
        let first = tokio::spawn(async move { transport.stop().await });
        tokio::task::yield_now().await;
        other.stop().await.unwrap();
        drop(StdUdpSocket::bind(bound).expect("the second stop returned with the port free"));
        first.await.unwrap().unwrap();
    }
}

/// Sends from several tasks race one stop. Each send either went out or
/// failed with the stopped error; none hangs or panics, and once the last
/// of them has returned the port is free. Real UDP, so real time: each
/// sender ends at its first failure, which the stop brings at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sends_racing_a_stop_complete_or_fail_cleanly() {
    let (transport, bound) = shared().await;
    let (_peer, mac) = peer();
    let (going, mut all_going) = tokio::sync::mpsc::channel(4);
    let senders: Vec<_> = (0..4)
        .map(|n| {
            let transport = Arc::clone(&transport);
            let mac = mac.clone();
            let going = going.clone();
            tokio::spawn(async move {
                let mut sent = 0u64;
                loop {
                    let result = if n % 2 == 0 {
                        transport.send_unicast(&[0x01, 0x00], &mac).await
                    } else {
                        transport.send_broadcast(&[0x01, 0x00]).await
                    };
                    match result {
                        Ok(()) => sent += 1,
                        Err(error) => {
                            assert!(stopped(&Err(error)), "a send failed other than stopped");
                            return sent;
                        }
                    }
                    if sent == 1 {
                        let _ = going.send(()).await;
                    }
                    tokio::task::yield_now().await;
                }
            })
        })
        .collect();
    for _ in 0..4 {
        all_going.recv().await.unwrap();
    }
    tokio::time::timeout(Duration::from_secs(5), transport.stop())
        .await
        .expect("the stop doesn't wait on the senders")
        .unwrap();
    for sender in senders {
        tokio::time::timeout(Duration::from_secs(5), sender)
            .await
            .expect("every sender ends")
            .expect("no sender panicked");
    }
    drop(StdUdpSocket::bind(bound).expect("the port is free"));
}
