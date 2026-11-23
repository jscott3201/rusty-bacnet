//! The sender checks of `tests/support/sender.rs` (#2718) with a stray
//! datagram from another socket waiting first in line, as a late
//! datagram meant for another test's process would be. Each test peeks
//! until the stray is queued first, so it is read before the sender's.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use tokio::net::UdpSocket;

use super::sender::{next_from, queued_from};

async fn bound() -> (UdpSocket, SocketAddr) {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let at = socket.local_addr().unwrap();
    (socket, at)
}

/// `peer` with a stray from a socket of its own queued first.
async fn stray_first(peer: SocketAddr, receiver: &UdpSocket) -> SocketAddr {
    let (other, at) = bound().await;
    other.send_to(b"stray", peer).await.unwrap();
    let mut head = [0u8; 16];
    let (_, from) = tokio::time::timeout(Duration::from_secs(5), receiver.peek_from(&mut head))
        .await
        .expect("the stray arrives")
        .unwrap();
    assert_eq!(from, at, "the stray is first in line");
    at
}

#[tokio::test]
async fn the_wait_reads_past_a_stray_to_the_senders_datagram() {
    let (peer, peer_at) = bound().await;
    let (sender, sender_at) = bound().await;
    stray_first(peer_at, &peer).await;
    sender.send_to(b"ours", peer_at).await.unwrap();
    let datagram = tokio::time::timeout(Duration::from_secs(5), next_from(&peer, sender_at))
        .await
        .expect("the sender's datagram");
    assert_eq!(datagram, b"ours");
}

#[tokio::test]
async fn a_stray_alone_is_not_the_senders() {
    let (peer, peer_at) = bound().await;
    let (_sender, sender_at) = bound().await;
    let stray = stray_first(peer_at, &peer).await;
    let queued = queued_from(sender_at, |buffer| peer.try_recv_from(buffer));
    assert_eq!(queued.unwrap(), None);
    // The stray was read and dropped: nothing is left for its own sender.
    let queued = queued_from(stray, |buffer| peer.try_recv_from(buffer));
    assert_eq!(queued.unwrap(), None);
}
