//! A B/IP stop through a shared reference (#2617): with another holder of
//! the transport alive, every later send fails with the stopped error, the
//! port can be bound again as soon as the stop returns, stops beside each
//! other both wait for the port, sends racing a stop end cleanly, and a
//! send left waiting holds no socket.

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket as StdUdpSocket};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::task::Poll;
use std::time::Duration;

use super::*;

/// The error kind every send after a stop fails with.
fn stopped(sent: &Result<(), Error>) -> bool {
    matches!(sent, Err(Error::Transport(io)) if io.kind() == std::io::ErrorKind::NotConnected)
}

/// A started transport on loopback, shared, the address it bound and its
/// receiver.
async fn shared() -> (
    Arc<BipTransport>,
    SocketAddrV4,
    mpsc::Receiver<ReceivedNpdu>,
) {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let rx = transport.start().await.unwrap();
    // The socket binds the wildcard address at the transport's port.
    let bound = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, transport.port);
    (Arc::new(transport), bound, rx)
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

/// Whether `peer` has received nothing.
fn received_nothing(peer: &StdUdpSocket) -> bool {
    peer.set_nonblocking(true).unwrap();
    peer.recv(&mut [0; 64]).is_err()
}

/// Make every try of the transport's socket report that it would block.
fn block_every_try(transport: &BipTransport) {
    let socket = transport.socket().unwrap();
    socket.block_tries.store(true, Ordering::Relaxed);
}

#[tokio::test]
async fn a_stop_with_another_holder_alive_fails_every_later_send_and_frees_the_port() {
    let (transport, bound, mut rx) = shared().await;
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
    assert!(rx.recv().await.is_none(), "the receiver ends");
    // Idempotent: a second stop finds nothing to do.
    other.stop().await.unwrap();
}

/// Two stops at once both succeed, and the port is free when the second
/// returns. That a stop waits for one already running is the stop gate's,
/// pinned in `stopping_tests`.
#[tokio::test]
async fn stops_beside_each_other_both_return_once_the_port_is_free() {
    for _ in 0..20 {
        let (transport, bound, _rx) = shared().await;
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
    let (transport, bound, _rx) = shared().await;
    let (_peer, mac) = peer();
    let (going, mut all_going) = mpsc::channel(4);
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

/// A send the task's cooperative budget holds back at its first poll holds
/// nothing: once the stop returns the port binds again, and the send,
/// polled then, fails with the stopped error and sends nothing.
#[tokio::test]
async fn a_send_held_back_by_the_budget_holds_no_socket() {
    let (transport, bound, _rx) = shared().await;
    let (peer, mac) = peer();
    let mut send = Box::pin(transport.send_unicast(&[0x01, 0x00], &mac));
    let first = std::future::poll_fn(|cx| {
        // No budget left when the send is polled.
        while let Poll::Ready(coop) = tokio::task::coop::poll_proceed(cx) {
            coop.made_progress();
        }
        Poll::Ready(send.as_mut().poll(cx))
    })
    .await;
    assert!(first.is_pending());
    transport.stop().await.unwrap();
    drop(StdUdpSocket::bind(bound).expect("the held-back send held no socket"));
    assert!(stopped(&send.await));
    assert!(received_nothing(&peer));
}

/// A send the socket makes wait holds no socket either: a stop frees the
/// port while the send future lives, not polled again, and the send then
/// fails with the stopped error, sending nothing.
#[tokio::test]
async fn a_send_waiting_for_the_socket_holds_none_and_fails_after_the_stop() {
    let (transport, bound, _rx) = shared().await;
    let (peer, mac) = peer();
    block_every_try(&transport);
    let mut send = Box::pin(transport.send_unicast(&[0x01, 0x00], &mac));
    let first = std::future::poll_fn(|cx| Poll::Ready(send.as_mut().poll(cx))).await;
    assert!(first.is_pending(), "it waits");
    transport.stop().await.unwrap();
    drop(StdUdpSocket::bind(bound).expect("the waiting send held no socket"));
    assert!(stopped(&send.await));
    assert!(received_nothing(&peer));
}

/// A waiting send that the runtime goes on polling wakes at the stop and
/// fails with the stopped error.
#[tokio::test]
async fn a_waiting_send_wakes_at_the_stop() {
    let (transport, bound, _rx) = shared().await;
    let (peer, mac) = peer();
    block_every_try(&transport);
    let send = tokio::spawn({
        let transport = Arc::clone(&transport);
        async move { transport.send_unicast(&[0x01, 0x00], &mac).await }
    });
    tokio::task::yield_now().await;
    assert!(!send.is_finished(), "it waits");
    transport.stop().await.unwrap();
    let sent = tokio::time::timeout(Duration::from_secs(1), send)
        .await
        .expect("the stop woke the send")
        .unwrap();
    assert!(stopped(&sent));
    drop(StdUdpSocket::bind(bound).expect("the port is free"));
    assert!(received_nothing(&peer));
}
