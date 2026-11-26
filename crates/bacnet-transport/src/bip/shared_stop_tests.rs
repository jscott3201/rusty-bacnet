//! A B/IP stop through a shared reference (#2617): with another holder of
//! the transport alive, every later send fails with the stopped error, the
//! socket is closed, and so its port free, as soon as the stop returns,
//! stops beside each other both wait for it, sends racing a stop end
//! cleanly, and a send left waiting holds no socket.
//!
//! That the socket is closed is read from the transport's own references
//! ([`closed`]), not by binding its port again: the port is an ephemeral
//! one, and another test process can be given it between the close and
//! that bind, which failed a run on Windows (#2657).

use std::net::{Ipv4Addr, UdpSocket as StdUdpSocket};
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::task::Poll;
use std::time::Duration;

use super::*;

/// The error every send after a stop fails with: the transport starts again,
/// so it is not running (#1833).
fn stopped(sent: &Result<(), Error>) -> bool {
    matches!(sent, Err(Error::State(Lifecycle::NotRunning, _)))
}

/// A started transport on loopback, shared, its socket, watched but not
/// held, and its receiver.
async fn shared() -> (
    Arc<BipTransport>,
    Weak<BipSocket>,
    mpsc::Receiver<ReceivedNpdu>,
) {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let rx = transport.start().await.unwrap();
    // The one socket a loopback transport binds: the wildcard address at
    // its port, owned by the node, the receive loop, the fanout worker and
    // the relay.
    let socket = Arc::downgrade(&transport.socket().unwrap());
    (Arc::new(transport), socket, rx)
}

/// Whether the transport let go of `socket`: no reference to it is left,
/// so it is dropped, which closes it and frees its port. Unlike a bind of
/// the port, nothing outside this process can change the answer.
fn closed(socket: &Weak<BipSocket>) -> bool {
    socket.strong_count() == 0
}

/// Poll `future` once.
async fn poll_once<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
    std::future::poll_fn(|cx| Poll::Ready(Pin::new(&mut *future).poll(cx))).await
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
    let (transport, socket, mut rx) = shared().await;
    let other = Arc::clone(&transport);
    let (peer, mac) = peer();
    other.send_unicast(&[0x01, 0x00], &mac).await.unwrap();
    let mut buf = [0; 16];
    peer.recv(&mut buf).unwrap();

    transport.stop().await.unwrap();
    // Closed at once, while `other` still holds the transport.
    assert!(closed(&socket), "the stop let go of the socket");
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

/// A send before start is refused as the state it finds, not running, as
/// one after a stop is, and so is a second start while the transport runs
/// (#1833): each is the transport's own refusal, with nothing sent.
#[tokio::test]
async fn a_send_before_start_and_a_second_start_are_state_refusals() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let (peer, mac) = peer();
    for sent in [
        transport.send_unicast(&[0x01, 0x00], &mac).await,
        transport.send_broadcast(&[0x01, 0x00]).await,
    ] {
        assert!(stopped(&sent), "{sent:?}");
    }
    let _rx = transport.start().await.unwrap();
    let again = transport.start().await;
    assert!(
        matches!(&again, Err(Error::State(Lifecycle::AlreadyStarted, _))),
        "{again:?}"
    );
    assert!(received_nothing(&peer));
    transport.stop().await.unwrap();
}

/// Two stops at once both succeed, and the second, which comes while the
/// first still waits for the tasks that hold the socket, returns only once
/// the socket is closed. The order is forced: a stop spawned and left to
/// run finishes before the test goes on, so the second would find nothing
/// running (#2657).
#[tokio::test]
async fn stops_beside_each_other_both_return_once_the_socket_is_closed() {
    let (transport, socket, _rx) = shared().await;
    let other = Arc::clone(&transport);
    // The first stop takes the gate, aborts the tasks and waits for the
    // first of them, which nothing has run since.
    let mut first = Box::pin(async move { transport.stop().await });
    assert!(poll_once(&mut first).await.is_pending());
    let first = tokio::spawn(first);
    let mut second = Box::pin(other.stop());
    assert!(
        poll_once(&mut second).await.is_pending(),
        "the second stop waits for the first"
    );
    assert!(!closed(&socket), "it came while the socket was open");
    second.await.unwrap();
    assert!(
        closed(&socket),
        "the second stop returned with the socket closed"
    );
    first.await.unwrap().unwrap();
}

/// Sends from several tasks race one stop. Each send either went out or
/// failed with the stopped error; none hangs or panics, and none holds the
/// socket once the stop returns. Real UDP, so real time: each sender ends
/// at its first failure, which the stop brings at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sends_racing_a_stop_complete_or_fail_cleanly() {
    let (transport, socket, _rx) = shared().await;
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
    // The senders may still be running: none of them holds the socket.
    assert!(closed(&socket), "the stop let go of the socket");
    for sender in senders {
        tokio::time::timeout(Duration::from_secs(5), sender)
            .await
            .expect("every sender ends")
            .expect("no sender panicked");
    }
}

/// A send the task's cooperative budget holds back at its first poll holds
/// nothing: once the stop returns the socket is closed, and the send,
/// polled then, fails with the stopped error and sends nothing.
#[tokio::test]
async fn a_send_held_back_by_the_budget_holds_no_socket() {
    let (transport, socket, _rx) = shared().await;
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
    assert!(closed(&socket), "the held-back send held no socket");
    assert!(stopped(&send.await));
    assert!(received_nothing(&peer));
}

/// A send the socket makes wait holds no socket either: a stop closes the
/// socket while the send future lives, not polled again, and the send then
/// fails with the stopped error, sending nothing.
#[tokio::test]
async fn a_send_waiting_for_the_socket_holds_none_and_fails_after_the_stop() {
    let (transport, socket, _rx) = shared().await;
    let (peer, mac) = peer();
    block_every_try(&transport);
    let mut send = Box::pin(transport.send_unicast(&[0x01, 0x00], &mac));
    let first = std::future::poll_fn(|cx| Poll::Ready(send.as_mut().poll(cx))).await;
    assert!(first.is_pending(), "it waits");
    transport.stop().await.unwrap();
    assert!(closed(&socket), "the waiting send held no socket");
    assert!(stopped(&send.await));
    assert!(received_nothing(&peer));
}

/// A waiting send that the runtime goes on polling wakes at the stop and
/// fails with the stopped error.
#[tokio::test]
async fn a_waiting_send_wakes_at_the_stop() {
    let (transport, socket, _rx) = shared().await;
    let (peer, mac) = peer();
    block_every_try(&transport);
    let send = tokio::spawn({
        let transport = Arc::clone(&transport);
        async move { transport.send_unicast(&[0x01, 0x00], &mac).await }
    });
    tokio::task::yield_now().await;
    assert!(!send.is_finished(), "it waits");
    transport.stop().await.unwrap();
    assert!(closed(&socket), "the stop let go of the socket");
    let sent = tokio::time::timeout(Duration::from_secs(1), send)
        .await
        .expect("the stop woke the send")
        .unwrap();
    assert!(stopped(&sent));
    assert!(received_nothing(&peer));
}

/// An abort through a shared reference (#2617): every later send fails
/// with the stopped error at once, the socket closes as the runtime drops
/// the aborted tasks, and a start is refused until a stop has joined them;
/// then the transport starts again, as after any stop. Each run starts on
/// a fresh port; see `restart` for why it can lose it.
#[tokio::test]
async fn an_abort_fails_later_sends_and_a_start_waits_for_a_stop() {
    for attempt in 1..=crate::port_ownership::ATTEMPTS {
        let (transport, socket, mut rx) = shared().await;
        let other = Arc::clone(&transport);
        let (_peer, mac) = peer();
        other.abort();
        for sent in [
            transport.send_unicast(&[0x01, 0x00], &mac).await,
            transport.send_broadcast(&[0x01, 0x00]).await,
        ] {
            assert!(stopped(&sent), "{sent:?}");
        }
        assert!(rx.recv().await.is_none(), "the receiver ends");
        tokio::time::timeout(Duration::from_secs(1), async {
            while !closed(&socket) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the socket closed");
        drop(other);
        let mut transport = Arc::into_inner(transport).unwrap();
        let refused = transport.start().await;
        assert!(
            matches!(&refused, Err(Error::State(Lifecycle::Stopping, _))),
            "{refused:?}"
        );
        transport.stop().await.unwrap();
        let Some(started) = crate::port_ownership::restart(&mut transport, attempt).await else {
            continue;
        };
        let _rx = started.unwrap();
        transport.stop().await.unwrap();
        return;
    }
}
