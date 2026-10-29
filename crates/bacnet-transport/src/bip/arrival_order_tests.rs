//! A transport sharing its port by address reads several sockets, and hands
//! up their datagrams in the order the kernel received them (#1546). The
//! default transport reads one socket, which keeps arrival order by itself,
//! and asks for no receive timestamps.

use super::ingress::{Held, Listeners, Source};
use super::socket::BoundSockets;
use super::*;
use crate::udp_metadata::ReceivedDatagram;
use std::net::{IpAddr, SocketAddr};

/// How many times each ordering test sends a pair: the acceptance count.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
const RUNS: usize = 1_000;

/// Whether this platform's kernel stamps arrivals: Linux, macOS and the
/// BSDs do. Elsewhere (Windows) the order is the read's.
const STAMPS: bool = cfg!(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
));

fn held(arrival_stamp: Option<u128>, read: u64) -> Held {
    Held {
        datagram: ReceivedDatagram {
            len: 0,
            peer: SocketAddr::from((Ipv4Addr::LOCALHOST, 47808)),
            destination: IpAddr::V4(Ipv4Addr::LOCALHOST),
            arrival_index: None,
            os_group_delivery: None,
            arrival_stamp,
        },
        read,
    }
}

#[test]
fn the_arrival_stamp_decides_and_the_read_order_breaks_ties() {
    let (early, late) = (Some(5), Some(9));
    // Read second but received first: it goes first.
    assert!(held(early, 2).before(&held(late, 1)));
    assert!(!held(late, 1).before(&held(early, 2)));
    // The same time, or one without a time: the earlier read goes first.
    assert!(held(early, 1).before(&held(early, 2)));
    assert!(!held(early, 2).before(&held(early, 1)));
    assert!(held(None, 1).before(&held(late, 2)));
    assert!(!held(early, 2).before(&held(None, 1)));
}

/// A plain socket bound to `127.0.0.1` on a port the OS picks.
fn loopback_socket() -> socket2::Socket {
    let socket = socket::udp_socket(socket::SocketRole::PrivateWildcard).unwrap();
    socket
        .bind(&SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0).into())
        .unwrap();
    socket
}

fn port_of(socket: &socket2::Socket) -> SocketAddr {
    socket.local_addr().unwrap().as_socket().unwrap()
}

/// Listeners over a primary and, with `listener`, one more socket standing
/// in for a broadcast listener, and where each one receives.
fn open(listener: bool) -> (Listeners, SocketAddr, Option<SocketAddr>) {
    let primary = loopback_socket();
    let primary_at = port_of(&primary);
    let listeners: Vec<_> = listener.then(loopback_socket).into_iter().collect();
    let listener_at = listeners.first().map(port_of);
    let bound = BoundSockets { primary, listeners };
    let listeners = Listeners::open(bound, None, &crate::tap::FrameTap::new()).unwrap();
    (listeners, primary_at, listener_at)
}

/// A socket that receives stamped datagrams, once the kernel stamps them:
/// Linux starts stamping arrivals a moment after the first socket on the
/// host asks, and stops after the last one closes (see
/// `request_arrival_stamps`). Held open, it keeps stamping on for the test.
/// Elsewhere the first datagram is stamped already, and where nothing
/// stamps (Windows) the socket is just a socket.
async fn stamping_on() -> UdpSocket {
    let socket = loopback_socket();
    if !STAMPS {
        return UdpSocket::from_std(socket.into()).unwrap();
    }
    crate::udp_metadata::request_arrival_stamps(&socket).unwrap();
    let receiver = DestinationReceiver::configure(&socket, IpVersion::V4).unwrap();
    let socket = UdpSocket::from_std(socket.into()).unwrap();
    let own = socket.local_addr().unwrap();
    let stamped = async {
        let mut buf = [0u8; 8];
        loop {
            socket.send_to(b"warm", own).await.unwrap();
            if receiver
                .recv_from(&socket, &mut buf)
                .await
                .unwrap()
                .arrival_stamp
                .is_some()
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(Duration::from_secs(2), stamped)
        .await
        .expect("the kernel never stamped a datagram");
    socket
}

async fn next(listeners: &mut Listeners) -> (Source, ReceivedDatagram) {
    let (source, received) = tokio::time::timeout(Duration::from_secs(2), listeners.recv())
        .await
        .expect("timed out waiting for a datagram");
    (source, received.unwrap())
}

/// The default single socket asks for no timestamps, so the kernel attaches
/// none, while every socket of an ordered transport gets them.
#[tokio::test]
async fn only_a_transport_reading_several_sockets_asks_for_receive_times() {
    let _stamping = stamping_on().await;
    let sender = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let (mut single, primary_at, _) = open(false);
    sender.send_to(b"one", primary_at).await.unwrap();
    let (source, received) = next(&mut single).await;
    assert_eq!((source, received.arrival_stamp), (None, None));

    let (mut ordered, primary_at, listener_at) = open(true);
    sender.send_to(b"two", primary_at).await.unwrap();
    sender
        .send_to(b"three", listener_at.unwrap())
        .await
        .unwrap();
    for _ in 0..2 {
        let (_, received) = next(&mut ordered).await;
        assert_eq!(received.arrival_stamp.is_some(), STAMPS);
    }
}

/// One datagram to the listener, then one to the primary, sent back to back
/// so both are queued before the loop reads either: every pair comes up in
/// the order sent. Without ordering, either socket's datagram can come up
/// first. Windows binds one socket in per-address mode, so it has nothing
/// to order.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
#[tokio::test]
async fn datagrams_on_two_sockets_come_up_in_the_order_the_kernel_received_them() {
    let _stamping = stamping_on().await;
    let (mut listeners, primary_at, listener_at) = open(true);
    let listener_at = listener_at.unwrap();
    let sender = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    for run in 0..RUNS {
        let tag = (run as u32).to_be_bytes();
        // Alternate which socket goes first, so neither wins by position.
        let (first, second) = if run % 2 == 0 {
            ((Some(0), listener_at), (None, primary_at))
        } else {
            ((None, primary_at), (Some(0), listener_at))
        };
        sender.send_to(&tag, first.1).await.unwrap();
        sender.send_to(&tag, second.1).await.unwrap();
        for (expected, _) in [first, second] {
            let (source, received) = next(&mut listeners).await;
            assert_eq!(source, expected, "run {run}");
            assert_eq!(listeners_data(&listeners, source, received.len), tag);
        }
    }
}

#[cfg(any(target_os = "linux", target_vendor = "apple"))]
fn listeners_data(listeners: &Listeners, source: Source, len: usize) -> [u8; 4] {
    listeners.data(source, len).try_into().unwrap()
}

/// A broadcast and then a unicast from one peer, sent back to back to a
/// transport sharing its port by address, come up in that order every
/// time: the broadcast arrives on a listener, the unicast on the address
/// socket. Linux carries 127.255.255.255 on loopback, so nothing leaves the
/// host. macOS gives loopback no broadcast address, and the socket-level
/// test above covers its ordering on loopback; no test here sends off it.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_broadcast_then_a_unicast_come_up_in_that_order() {
    use crate::port_ownership::{ATTEMPTS, lost_port};

    let (ip, subnet) = (
        Ipv4Addr::new(127, 0, 0, 2),
        Ipv4Addr::new(127, 255, 255, 255),
    );
    let _stamping = stamping_on().await;
    let mut started = None;
    for attempt in 1..=ATTEMPTS {
        let port = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut transport = BipTransport::new(ip, port, subnet);
        transport.set_share_port_by_address(true);
        match transport.start().await {
            Ok(rx) => {
                started = Some((transport, rx, port));
                break;
            }
            Err(Error::Transport(ref e)) if lost_port(attempt, e) => {}
            Err(e) => panic!("start {ip}:{port}: {e}"),
        }
    }
    let (mut transport, mut rx, port) = started.expect("the last attempt panics instead");
    let peer = UdpSocket::bind((ip, 0)).await.unwrap();
    peer.set_broadcast(true).unwrap();
    for run in 0..RUNS {
        let tag = (run as u16).to_be_bytes();
        let npdu = [0x01, 0x00, 0x10, tag[0], tag[1]];
        let broadcast = raw_frame(BvlcFunction::ORIGINAL_BROADCAST_NPDU, &npdu);
        let unicast = raw_frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, &npdu);
        peer.send_to(&broadcast, (subnet, port)).await.unwrap();
        peer.send_to(&unicast, (ip, port)).await.unwrap();
        for group in [true, false] {
            let received = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("timed out waiting for an NPDU")
                .expect("receive channel closed");
            assert_eq!(received.link_layer_group, group, "run {run}");
            assert_eq!(received.npdu.as_ref(), npdu);
        }
    }
    transport.stop().await.unwrap();
}
