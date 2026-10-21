//! The resolver's driver and send path (U.5, #1637): the exact requests it
//! sends and where, through a capturing socket under paused time, and the
//! same against real sockets on `::1` for a foreign device, a BBMD that
//! refuses, and Virtual-Address-Resolution.

use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bacnet_types::error::Error;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::receive::Receiver;
use super::resolve::{Failure, Route};
use super::resolver::{DatagramSender, Driver, Resolver};
use super::socket::Bip6Socket;
use super::*;
use crate::port::TransportPort;
use crate::udp_metadata::ReceivedDatagram;

/// Device instance 10's VMAC, this node's.
const OWN: Bip6Vmac = Bip6Vmac::new([0x00, 0x00, 0x0A]);
/// Device instance 1234's VMAC, the peer's.
const PEER: [u8; 3] = [0x00, 0x04, 0xD2];
const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];
/// Address-Resolution from [`OWN`] for [`PEER`].
const ADDRESS_RESOLUTION: [u8; 10] = [0x82, 0x03, 0x00, 0x0A, 0x00, 0x00, 0x0A, 0x00, 0x04, 0xD2];
/// Address-Resolution-ACK from [`PEER`] to [`OWN`].
const ACK: [u8; 10] = [0x82, 0x05, 0x00, 0x0A, 0x00, 0x04, 0xD2, 0x00, 0x00, 0x0A];
/// Original-Unicast-NPDU from [`OWN`] to [`PEER`] carrying [`NPDU`].
const UNICAST: [u8; 14] = [
    0x82, 0x01, 0x00, 0x0E, 0x00, 0x00, 0x0A, 0x00, 0x04, 0xD2, 0x01, 0x00, 0x10, 0x08,
];

fn peer_vmac() -> Bip6Vmac {
    Bip6Vmac::new(PEER)
}

fn target() -> SocketAddrV6 {
    SocketAddrV6::new(
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x20),
        0xBAC0,
        0,
        0,
    )
}

fn bbmd() -> SocketAddrV6 {
    SocketAddrV6::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1), 0xBAC0, 0, 0)
}

fn group() -> SocketAddrV6 {
    SocketAddrV6::new(BACNET_IPV6_MULTICAST_SITE_LOCAL, 0xBAC0, 0, 0)
}

/// A socket that records what it sends, and when.
#[derive(Default)]
struct Capture {
    sent: Mutex<Vec<(Instant, Vec<u8>, SocketAddrV6)>>,
}

impl DatagramSender for Capture {
    fn send_to(
        &self,
        bytes: &[u8],
        peer: SocketAddrV6,
    ) -> impl Future<Output = io::Result<usize>> + Send {
        let entry = (Instant::now(), bytes.to_vec(), peer);
        self.sent.lock().unwrap().push(entry);
        std::future::ready(Ok(bytes.len()))
    }
}

impl Capture {
    /// What went out, with each send's time since `start`.
    fn sent(&self, start: Instant) -> Vec<(Duration, Vec<u8>, SocketAddrV6)> {
        let sent = self.sent.lock().unwrap();
        sent.iter()
            .map(|(at, bytes, to)| (*at - start, bytes.clone(), *to))
            .collect()
    }
}

/// A resolver for [`OWN`] and its driver, sending through a capture, with
/// Address-Resolutions going to `resolution_to`.
fn driven(resolution_to: SocketAddrV6) -> (Arc<Resolver>, Arc<Capture>) {
    let resolver = Resolver::with_own(OWN);
    let capture = Arc::new(Capture::default());
    let driver = Driver {
        socket: Arc::clone(&capture),
        resolver: Arc::clone(&resolver),
        source: OWN,
        resolution_to,
        foreign: None,
    };
    tokio::spawn(driver.run());
    (resolver, capture)
}

fn kind(result: &Result<(), Error>) -> Option<io::ErrorKind> {
    match result {
        Err(Error::Transport(error)) => Some(error.kind()),
        _ => None,
    }
}

/// The Address-Resolution goes to the scope's group in normal mode and to
/// the BBMD by unicast for a foreign device (U.4.4), at once and a second
/// and two seconds later; the send fails three seconds after the first.
#[tokio::test(start_paused = true)]
async fn an_address_resolution_goes_to_the_group_or_to_the_bbmd() {
    for to in [group(), bbmd()] {
        let (resolver, capture) = driven(to);
        let start = Instant::now();
        let result = resolver
            .send_unicast(&*capture, OWN, UnresolvedSend::Wait, NPDU, &PEER)
            .await;
        assert_eq!(kind(&result), Some(io::ErrorKind::TimedOut), "{result:?}");
        assert!(result.unwrap_err().to_string().contains("0x0004D2"));
        assert_eq!(start.elapsed(), Duration::from_secs(3));
        let expected: Vec<_> = [0, 1, 2]
            .map(|at| (Duration::from_secs(at), ADDRESS_RESOLUTION.to_vec(), to))
            .into();
        assert_eq!(capture.sent(start), expected);
        let counters = resolver.counters();
        assert_eq!(
            (
                counters.resolutions_started,
                counters.resolutions_timed_out,
                counters.address_resolutions_sent
            ),
            (1, 1, 3)
        );
    }
}

/// A waiting send goes out the moment its VMAC is learned, to the learned
/// address, and no further Address-Resolution follows.
#[tokio::test(start_paused = true)]
async fn a_waiting_send_goes_out_once_the_vmac_resolves() {
    let (resolver, capture) = driven(group());
    let start = Instant::now();
    let send = tokio::spawn({
        let (resolver, capture) = (Arc::clone(&resolver), Arc::clone(&capture));
        async move {
            resolver
                .send_unicast(&*capture, OWN, UnresolvedSend::Wait, NPDU, &PEER)
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(1500)).await;
    resolver.learn(peer_vmac(), target(), super::vmac_table::Heard::Direct);
    send.await.unwrap().unwrap();
    assert_eq!(start.elapsed(), Duration::from_millis(1500));
    tokio::time::sleep(Duration::from_secs(5)).await;
    let sent = capture.sent(start);
    assert_eq!(
        sent,
        [
            (Duration::ZERO, ADDRESS_RESOLUTION.to_vec(), group()),
            (Duration::from_secs(1), ADDRESS_RESOLUTION.to_vec(), group()),
            (Duration::from_millis(1500), UNICAST.to_vec(), target()),
        ]
    );
    assert_eq!(resolver.counters().resolutions_resolved, 1);
}

/// A queued send returns at once; its NPDU goes out when the VMAC
/// resolves, or is dropped and counted when it doesn't.
#[tokio::test(start_paused = true)]
async fn a_queued_send_returns_at_once_and_goes_out_on_resolution() {
    let (resolver, capture) = driven(group());
    let start = Instant::now();
    for _ in 0..2 {
        resolver
            .send_unicast(&*capture, OWN, UnresolvedSend::Queue, NPDU, &PEER)
            .await
            .unwrap();
    }
    assert_eq!(start.elapsed(), Duration::ZERO);
    tokio::time::sleep(Duration::from_millis(200)).await;
    resolver.learn(peer_vmac(), target(), super::vmac_table::Heard::Direct);
    tokio::time::sleep(Duration::from_millis(1)).await;
    let flushed: Vec<_> = capture
        .sent(start)
        .into_iter()
        .filter(|(_, bytes, _)| bytes[1] == 0x01)
        .collect();
    let at = Duration::from_millis(200);
    assert_eq!(
        flushed,
        [
            (at, UNICAST.to_vec(), target()),
            (at, UNICAST.to_vec(), target())
        ]
    );

    let other = [0x00, 0x04, 0xD3];
    resolver
        .send_unicast(&*capture, OWN, UnresolvedSend::Queue, NPDU, &other)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(4)).await;
    let counters = resolver.counters();
    assert_eq!(
        (counters.queued_dropped, counters.resolutions_timed_out),
        (1, 1)
    );
    assert_eq!(counters.resolutions_resolved, 1);
}

/// A MAC that isn't three octets, or names this node, is refused before
/// anything is sent or any resolution starts.
#[tokio::test(start_paused = true)]
async fn a_mac_that_is_not_another_nodes_vmac_is_refused() {
    let (resolver, capture) = driven(group());
    let eighteen = [0u8; 18];
    for mac in [
        &eighteen[..],
        &[1, 2, 3, 4, 5, 6],
        &[],
        &[0x00, 0x00],
        &OWN.octets(),
    ] {
        for hold in [UnresolvedSend::Wait, UnresolvedSend::Queue] {
            let result = resolver.send_unicast(&*capture, OWN, hold, NPDU, mac).await;
            assert_eq!(
                kind(&result),
                Some(io::ErrorKind::InvalidInput),
                "{mac:02X?}"
            );
        }
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(capture.sent(Instant::now()).is_empty());
    assert_eq!(resolver.counters(), Bip6ResolutionCounters::default());
}

/// A transport configured as a foreign device of `bbmd`, with device
/// instance 10, started.
async fn foreign_device(bbmd: &UdpSocket) -> Bip6Transport {
    let SocketAddr::V6(bbmd) = bbmd.local_addr().unwrap() else {
        unreachable!("bound to ::1");
    };
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(10));
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: *bbmd.ip(),
        bbmd_port: bbmd.port(),
        ttl: 60,
    });
    transport
}

/// The next datagram `socket` reads that isn't a Register-Foreign-Device.
async fn next_request(socket: &UdpSocket) -> (Vec<u8>, SocketAddr) {
    let mut buf = [0u8; 1500];
    loop {
        let (len, from) = tokio::time::timeout(Duration::from_secs(10), socket.recv_from(&mut buf))
            .await
            .expect("the BBMD gets a request")
            .unwrap();
        if buf[1] != 0x09 {
            return (buf[..len].to_vec(), from);
        }
    }
}

/// On real sockets: a foreign device asks its BBMD, the target's ACK
/// resolves the VMAC, and the held NPDU arrives at the target as an
/// Original-Unicast-NPDU. The table and counts follow.
#[tokio::test]
async fn a_foreign_device_asks_its_bbmd_and_sends_once_answered() {
    let bbmd = UdpSocket::bind("[::1]:0").await.unwrap();
    let target = UdpSocket::bind("[::1]:0").await.unwrap();
    let mut transport = foreign_device(&bbmd).await;
    let _rx = transport.start().await.unwrap();
    let local = transport.local_address().unwrap();
    let answer = async {
        let (request, _) = next_request(&bbmd).await;
        assert_eq!(request, ADDRESS_RESOLUTION, "the BBMD gets the request");
        target.send_to(&ACK, local).await.unwrap();
        let mut buf = [0u8; 64];
        let (len, from) = target.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..len], UNICAST);
        assert_eq!(from, SocketAddr::V6(local));
    };
    let (sent, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(transport.send_unicast(NPDU, &PEER), answer)
    })
    .await
    .expect("resolved well inside the timeout");
    sent.unwrap();
    let SocketAddr::V6(target_address) = target.local_addr().unwrap() else {
        unreachable!("bound to ::1");
    };
    assert_eq!(transport.lookup_vmac(peer_vmac()), Some(target_address));
    let counters = transport.resolution_counters();
    assert_eq!(
        (counters.resolutions_started, counters.resolutions_resolved),
        (1, 1)
    );
    transport.stop().await.unwrap();
    assert_eq!(
        transport.lookup_vmac(peer_vmac()),
        None,
        "stop clears the table"
    );
    assert_eq!(
        transport.resolution_counters(),
        counters,
        "the counts survive"
    );
}

/// With nobody answering, the send fails with a transport TimedOut three
/// seconds after the first request, the next send to that VMAC fails at
/// once, and the BBMD got all three requests.
///
/// The timing is checked on the paused clock before the test waits on any
/// real socket. While a datagram is still crossing loopback the runtime
/// looks idle, and a paused clock then jumps to its next timer (#1885). The
/// BBMD's socket is read after the clock resumes.
#[tokio::test(start_paused = true)]
async fn a_foreign_device_send_fails_after_three_seconds_unanswered() {
    let bbmd = UdpSocket::bind("[::1]:0").await.unwrap();
    let mut transport = foreign_device(&bbmd).await;
    let _rx = transport.start().await.unwrap();
    let start = Instant::now();
    let result = transport.send_unicast(NPDU, &PEER).await;
    assert_eq!(kind(&result), Some(io::ErrorKind::TimedOut), "{result:?}");
    assert_eq!(start.elapsed(), Duration::from_secs(3));
    let again = transport.send_unicast(NPDU, &PEER).await;
    assert_eq!(
        kind(&again),
        Some(io::ErrorKind::HostUnreachable),
        "{again:?}"
    );
    assert_eq!(start.elapsed(), Duration::from_secs(3));
    tokio::time::resume();
    for _ in 0..3 {
        assert_eq!(next_request(&bbmd).await.0, ADDRESS_RESOLUTION);
    }
    transport.stop().await.unwrap();
}

/// A BBMD that refuses to pass the request on (Address-Resolution NAK)
/// makes the failure say so.
///
/// This runs on the real clock because the NAK has to cross loopback before
/// the resolution's three seconds run out. A paused clock could use them up
/// while the NAK is still on its way (#1885).
#[tokio::test]
async fn a_bbmd_refusal_fails_the_send_as_refused() {
    let bbmd = UdpSocket::bind("[::1]:0").await.unwrap();
    let mut transport = foreign_device(&bbmd).await;
    let _rx = transport.start().await.unwrap();
    let local = transport.local_address().unwrap();
    let refuse = async {
        loop {
            let (request, _) = next_request(&bbmd).await;
            assert_eq!(request, ADDRESS_RESOLUTION);
            let nak = [0x82, 0x00, 0x00, 0x09, 0x00, 0x00, 0x01, 0x00, 0x30];
            bbmd.send_to(&nak, local).await.unwrap();
        }
    };
    let result = tokio::select! {
        result = transport.send_unicast(NPDU, &PEER) => result,
        () = refuse => unreachable!("the BBMD answers for ever"),
    };
    assert_eq!(
        kind(&result),
        Some(io::ErrorKind::ConnectionRefused),
        "{result:?}"
    );
    assert!(result.unwrap_err().to_string().contains("BBMD refused"));
    assert_eq!(transport.resolution_counters().resolutions_refused, 1);
    transport.stop().await.unwrap();
}

/// A receiver for `2001:db8::10`, VMAC [`OWN`], a foreign device of
/// [`bbmd`] when `foreign`, whose neighbour table is `resolver`, and the
/// network layer's end of its channel. Its socket is never used.
async fn receiver(
    resolver: Arc<Resolver>,
    foreign: bool,
) -> (Receiver, mpsc::Receiver<crate::port::ReceivedNpdu>) {
    let socket = Bip6Socket::bind(Ipv6Addr::LOCALHOST, 0, None)
        .await
        .unwrap();
    let local_ip = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x10);
    let (tx, rx) = mpsc::channel(8);
    let receiver = Receiver {
        socket: Arc::new(socket),
        tx,
        local_port: 0xBAC0,
        vmac: OWN,
        local_ip,
        unicast_ips: vec![local_ip],
        wildcard_bind: false,
        foreign_bbmd: foreign.then(|| super::foreign_target::ForeignTarget::at(bbmd())),
        neighbours: resolver,
        forwarded_group_origin_drops: Arc::default(),
        drops: Arc::default(),
        nak_quota: Default::default(),
        far_ack_quota: Default::default(),
    };
    (receiver, rx)
}

/// Feed `message` from `from`, sent to `to`, and return the reply.
async fn feed(
    receiver: &Receiver,
    message: Bvll6<'_>,
    from: SocketAddrV6,
    to: Ipv6Addr,
) -> Option<super::receive::Reply> {
    let frame = super::frame::encoded(&message).unwrap();
    let datagram = ReceivedDatagram {
        len: frame.len(),
        peer: SocketAddr::V6(from),
        destination: IpAddr::V6(to),
        arrival_index: Some(1),
        os_group_delivery: None,
        arrival_stamp: None,
    };
    receiver.handle_datagram(&frame, &datagram).await
}

/// Another address sending from this node's VMAC is dropped and counted,
/// and never learned: its NPDUs, a Forwarded-NPDU naming it as the origin,
/// and its ACKs. An Address-Resolution from it is still answered, since a
/// node probing for its own VMAC learns of the collision that way.
#[tokio::test]
async fn frames_from_another_node_using_this_vmac_are_dropped_and_counted() {
    let resolver = Resolver::with_own(OWN);
    let (receiver, mut rx) = receiver(Arc::clone(&resolver), false).await;
    let local_ip = receiver.local_ip;
    let group = BACNET_IPV6_MULTICAST_SITE_LOCAL;
    let station = SocketAddrV6::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 9, 0, 0, 0, 1), 0xBAC0, 0, 0);
    let forwarded = Bvll6::ForwardedNpdu {
        original_source: OWN,
        original_address: station.into(),
        npdu: NPDU,
    };
    for (message, to) in [
        (
            Bvll6::OriginalUnicastNpdu {
                source: OWN,
                destination: OWN,
                npdu: NPDU,
            },
            local_ip,
        ),
        (
            Bvll6::OriginalBroadcastNpdu {
                source: OWN,
                npdu: NPDU,
            },
            group,
        ),
        (forwarded, group),
        (
            Bvll6::AddressResolutionAck {
                source: OWN,
                destination: OWN,
            },
            local_ip,
        ),
        (
            Bvll6::VirtualAddressResolutionAck {
                source: OWN,
                destination: OWN,
            },
            local_ip,
        ),
    ] {
        assert_eq!(
            feed(&receiver, message, target(), to).await,
            None,
            "{message:?}"
        );
    }
    assert!(rx.try_recv().is_err(), "nothing reached the network layer");
    let probe = Bvll6::AddressResolution {
        source: OWN,
        target: OWN,
    };
    assert!(feed(&receiver, probe, target(), group).await.is_some());
    let counters = receiver.drops.snapshot();
    assert_eq!(
        counters,
        Bip6ReceiveCounters {
            own_vmac_source: 5,
            ..Bip6ReceiveCounters::default()
        }
    );
    assert_eq!(resolver.lookup(OWN), None);
}

/// A Forwarded-NPDU by a trusted path still can't move a peer heard
/// directly within the last minute to the address it carries, which would
/// let its sender steer this node's unicasts; the NPDU itself still goes
/// up. After that minute the carried address is taken.
#[tokio::test(start_paused = true)]
async fn a_forwarded_npdu_cannot_move_a_peer_heard_directly() {
    let resolver = Resolver::with_own(OWN);
    let (receiver, mut rx) = receiver(Arc::clone(&resolver), false).await;
    let direct = Bvll6::OriginalBroadcastNpdu {
        source: peer_vmac(),
        npdu: NPDU,
    };
    let group = BACNET_IPV6_MULTICAST_SITE_LOCAL;
    feed(&receiver, direct, target(), group).await;
    let elsewhere = SocketAddrV6::new(
        Ipv6Addr::new(0x2001, 0xdb8, 0, 9, 0, 0, 0, 0x66),
        0xBAC0,
        0,
        0,
    );
    let forged = Bvll6::ForwardedNpdu {
        original_source: peer_vmac(),
        original_address: elsewhere.into(),
        npdu: NPDU,
    };
    feed(&receiver, forged, bbmd(), group).await;
    assert_eq!(resolver.lookup(peer_vmac()), Some(target()));
    for _ in 0..2 {
        assert_eq!(rx.try_recv().unwrap().source_mac.as_slice(), PEER);
    }
    tokio::time::advance(Duration::from_secs(60)).await;
    feed(&receiver, forged, bbmd(), group).await;
    assert_eq!(resolver.lookup(peer_vmac()), Some(elsewhere));
}

/// Only the configured BBMD's Address-Resolution NAK marks a resolution as
/// refused; anyone else's is only logged.
#[tokio::test]
async fn only_the_configured_bbmds_nak_refuses_a_resolution() {
    for (from, refused) in [(target(), false), (bbmd(), true)] {
        let resolver = Resolver::with_own(OWN);
        let (receiver, _rx) = receiver(Arc::clone(&resolver), true).await;
        let t0 = Instant::now();
        let mut rx = resolver.with_state(|state, counts| {
            match state.route(peer_vmac(), NPDU, UnresolvedSend::Wait, t0, counts) {
                Route::Wait { rx, .. } => rx,
                other => panic!("{other:?}"),
            }
        });
        let nak = [0x82, 0x00, 0x00, 0x09, 0x00, 0x00, 0x01, 0x00, 0x30];
        let datagram = ReceivedDatagram {
            len: nak.len(),
            peer: SocketAddr::V6(from),
            destination: IpAddr::V6(receiver.local_ip),
            arrival_index: Some(1),
            os_group_delivery: None,
            arrival_stamp: None,
        };
        assert_eq!(receiver.handle_datagram(&nak, &datagram).await, None);
        resolver.with_state(|state, counts| state.poll(t0 + Duration::from_secs(3), counts));
        let expected = if refused {
            Failure::Refused
        } else {
            Failure::TimedOut
        };
        assert_eq!(rx.try_recv(), Ok(Err(expected)), "from {from}");
    }
}

/// Virtual-Address-Resolution against real sockets: a running peer's VMAC
/// comes back and enters the table, so asking again sends nothing, and a
/// NAK fails a lookup at once.
#[tokio::test]
async fn a_virtual_address_resolution_finds_a_vmac_or_is_refused() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(10));
    let mut peer = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(1234));
    let _rx = transport.start().await.unwrap();
    let _peer_rx = peer.start().await.unwrap();
    let peer_address = peer.local_address().unwrap();
    for _ in 0..2 {
        let found = transport.resolve_virtual_address(peer_address).await;
        assert_eq!(found.unwrap(), peer_vmac());
    }
    assert_eq!(transport.lookup_vmac(peer_vmac()), Some(peer_address));
    let counters = transport.resolution_counters();
    assert_eq!(
        (counters.virtual_resolutions_sent, counters.virtual_resolved),
        (1, 1)
    );
    let own = transport.resolve_virtual_address(transport.local_address().unwrap());
    assert_eq!(own.await.unwrap(), OWN);

    let refusing = UdpSocket::bind("[::1]:0").await.unwrap();
    let SocketAddr::V6(refusing_address) = refusing.local_addr().unwrap() else {
        unreachable!("bound to ::1");
    };
    let local = transport.local_address().unwrap();
    let refuse = async {
        let mut buf = [0u8; 64];
        let (len, from) = refusing.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..len], [0x82, 0x06, 0x00, 0x07, 0x00, 0x00, 0x0A]);
        assert_eq!(from, SocketAddr::V6(local));
        let nak = [0x82, 0x00, 0x00, 0x09, 0x00, 0x00, 0x05, 0x00, 0x60];
        refusing.send_to(&nak, local).await.unwrap();
    };
    let start = Instant::now();
    let (refused, ()) = tokio::join!(transport.resolve_virtual_address(refusing_address), refuse);
    assert!(
        matches!(&refused, Err(Error::Transport(e)) if e.kind() == io::ErrorKind::ConnectionRefused),
        "{refused:?}"
    );
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "the NAK ends it at once"
    );
    assert_eq!(transport.resolution_counters().virtual_refused, 1);
    transport.stop().await.unwrap();
    peer.stop().await.unwrap();
}

/// A silent address fails a Virtual-Address-Resolution after three
/// requests and three seconds.
#[tokio::test(start_paused = true)]
async fn a_virtual_address_resolution_times_out_unanswered() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(10));
    let _rx = transport.start().await.unwrap();
    let silent = UdpSocket::bind("[::1]:0").await.unwrap();
    let SocketAddr::V6(silent_address) = silent.local_addr().unwrap() else {
        unreachable!("bound to ::1");
    };
    let start = Instant::now();
    let error = transport
        .resolve_virtual_address(silent_address)
        .await
        .unwrap_err();
    assert!(matches!(&error, Error::Transport(e) if e.kind() == io::ErrorKind::TimedOut));
    assert_eq!(start.elapsed(), Duration::from_secs(3));
    // As in the foreign-device timeout test: read the requests on the real
    // clock, so one still on its way can't let the paused clock jump.
    tokio::time::resume();
    let mut buf = [0u8; 64];
    for _ in 0..3 {
        let (len, _) = silent.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..len], [0x82, 0x06, 0x00, 0x07, 0x00, 0x00, 0x0A]);
    }
    assert_eq!(transport.resolution_counters().virtual_timed_out, 1);
    transport.stop().await.unwrap();
}

/// Before start there is nothing to resolve with.
#[tokio::test]
async fn resolving_needs_a_running_transport() {
    let transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(10));
    let peer = SocketAddrV6::new(Ipv6Addr::LOCALHOST, 47808, 0, 0);
    let error = transport.resolve_virtual_address(peer).await.unwrap_err();
    assert!(matches!(error, Error::Transport(e) if e.kind() == io::ErrorKind::NotConnected));
    let result = transport.send_unicast(NPDU, &PEER).await;
    assert_eq!(kind(&result), Some(io::ErrorKind::NotConnected));
}
