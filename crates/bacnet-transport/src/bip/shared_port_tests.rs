//! Several B/IP transports on one host share an explicitly requested port,
//! one per interface address, as several devices would at 47808, once each
//! opts in with `set_share_port_by_address` (#1538). Each gets only its own
//! unicast, sends from its own address, keeps its address to itself, and gets
//! the broadcasts sent to its subnet.

use super::*;
use crate::port_ownership::{lost_port, ATTEMPTS};
use socket::udp_socket;
use tokio::time::timeout;

/// Two local addresses a test can bind and reach from this host, or why
/// there aren't two. Linux answers for all of 127.0.0.0/8 on loopback.
/// macOS assigns only 127.0.0.1 there unless an alias is added, so macOS
/// and Windows pair it with the default-route address.
fn two_local_addresses() -> Result<[Ipv4Addr; 2], &'static str> {
    if cfg!(target_os = "linux") {
        return Ok([Ipv4Addr::new(127, 0, 0, 2), Ipv4Addr::new(127, 0, 0, 3)]);
    }
    match crate::local_addresses::route_ipv4() {
        Some(ip) if !ip.is_loopback() => Ok([Ipv4Addr::LOCALHOST, ip]),
        _ => Err("no default-route IPv4 address to pair with 127.0.0.1"),
    }
}

struct Node {
    transport: BipTransport,
    rx: mpsc::Receiver<ReceivedNpdu>,
}

impl Node {
    fn address(&self) -> SocketAddrV4 {
        let (ip, port) = decode_bip_mac(self.transport.local_mac()).unwrap();
        SocketAddrV4::new(Ipv4Addr::from(ip), port)
    }

    async fn next(&mut self) -> ReceivedNpdu {
        timeout(Duration::from_secs(2), self.rx.recv())
            .await
            .unwrap_or_else(|_| panic!("{} timed out waiting for an NPDU", self.address()))
            .expect("receive channel closed")
    }
}

/// Start one transport per address, with the broadcast address beside it,
/// all on one requested port that they share by address. The OS picks the
/// port for a probe that releases it, and another process can take it in
/// between (#1032), so a lost port starts over on a fresh one. `configure`
/// gets each transport before it starts, with its index and the port.
async fn start_on_one_port<const N: usize>(
    addresses: [Ipv4Addr; N],
    broadcasts: [Ipv4Addr; N],
    configure: impl Fn(usize, u16, &mut BipTransport),
) -> [Node; N] {
    for attempt in 1..=ATTEMPTS {
        let port = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut nodes = Vec::new();
        for (i, (ip, broadcast)) in addresses.into_iter().zip(broadcasts).enumerate() {
            let mut transport = BipTransport::new(ip, port, broadcast);
            transport.set_share_port_by_address(true);
            configure(i, port, &mut transport);
            match transport.start().await {
                Ok(rx) => nodes.push(Node { transport, rx }),
                Err(Error::Transport(ref e)) if lost_port(attempt, e) => break,
                Err(e) => panic!("start {ip}:{port}: {e}"),
            }
        }
        if let Ok(nodes) = <[Node; N]>::try_from(nodes) {
            return nodes;
        }
    }
    unreachable!("the last attempt panics instead of retrying")
}

fn frame(function: BvlcFunction, npdu: &[u8]) -> BytesMut {
    let mut buf = BytesMut::new();
    encode_bvll(&mut buf, function, npdu).unwrap();
    buf
}

/// A non-routed NPDU expecting no reply, tagged so each one is told apart.
fn npdu(tag: u8) -> [u8; 4] {
    [0x01, 0x00, 0x10, tag]
}

#[tokio::test]
async fn transports_on_one_port_each_get_their_own_unicast_and_send_from_it() {
    let addresses = match two_local_addresses() {
        Ok(addresses) => addresses,
        Err(why) => return eprintln!("skipped: {why}"),
    };
    let broadcasts = [Ipv4Addr::BROADCAST; 2];
    let mut nodes = start_on_one_port(addresses, broadcasts, |_, _, _| {}).await;
    let port = nodes[0].address().port();
    for (node, ip) in nodes.iter().zip(addresses) {
        assert_eq!(node.address(), SocketAddrV4::new(ip, port));
    }
    let peer = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        .await
        .unwrap();
    let peer_port = peer.local_addr().unwrap().port();

    // Interleaved, so a transport that took the other's unicast, or lost
    // its own to the other, shows up as a wrong or missing NPDU.
    for round in 0..3 {
        for (i, node) in nodes.iter().enumerate() {
            let unicast = frame(
                BvlcFunction::ORIGINAL_UNICAST_NPDU,
                &npdu(16 * i as u8 + round),
            );
            peer.send_to(&unicast, node.address()).await.unwrap();
        }
    }
    for (i, node) in nodes.iter_mut().enumerate() {
        for round in 0..3 {
            let received = node.next().await;
            assert_eq!(received.npdu.as_ref(), npdu(16 * i as u8 + round));
            assert!(!received.link_layer_group);
        }
    }

    // A reply leaves from the transport's own address and the shared port,
    // which is where its peers send to it next. The peer is addressed on the
    // same address, which every OS routes back to this host.
    for (i, node) in nodes.iter().enumerate() {
        let ip = node.address().ip().octets();
        let reply = npdu(0x80 + i as u8);
        let to = encode_bip_mac(ip, peer_port);
        node.transport.send_unicast(&reply, &to).await.unwrap();
        let mut buf = [0u8; 64];
        let (len, from) = timeout(Duration::from_secs(2), peer.recv_from(&mut buf))
            .await
            .expect("timed out waiting for the reply")
            .unwrap();
        assert_eq!(from, std::net::SocketAddr::V4(node.address()));
        let reply_frame = decode_bvll(&buf[..len]).unwrap();
        assert_eq!(reply_frame.function, BvlcFunction::ORIGINAL_UNICAST_NPDU);
        assert_eq!(reply_frame.payload.as_ref(), reply);
    }

    for node in &mut nodes {
        assert!(node.rx.try_recv().is_err(), "{}", node.address());
        node.transport.stop().await.unwrap();
    }
}

/// A foreign device and its BBMD on two addresses of one host, sharing one
/// port: registration, Distribute-Broadcast-To-Network and the BBMD's
/// Forwarded-NPDU all travel between the two interface addresses. Windows
/// pairs a loopback address with a non-loopback one here, and isn't relied
/// on to carry traffic between them.
#[cfg(not(windows))]
#[tokio::test]
async fn a_bbmd_and_a_foreign_device_share_a_port_on_two_addresses() {
    let addresses = match two_local_addresses() {
        Ok(addresses) => addresses,
        Err(why) => return eprintln!("skipped: {why}"),
    };
    let [bbmd_ip, device_ip] = addresses;
    // The BBMD, on loopback, sets its own address as the broadcast address,
    // so its local broadcast returns to itself; only the forwarding is under
    // test. A foreign device broadcasts through its BBMD.
    let broadcasts = [bbmd_ip, Ipv4Addr::BROADCAST];
    let [mut bbmd, mut device] = start_on_one_port(addresses, broadcasts, |i, port, t| {
        if i == 0 {
            t.enable_bbmd(vec![]);
            t.enable_foreign_device_registration(ForeignDevicePolicy::default());
        } else {
            t.register_as_foreign_device(ForeignDeviceConfig {
                bbmd_ip,
                bbmd_port: port,
                ttl: 60,
            });
        }
    })
    .await;
    let device_mac = device.address();

    // Registration came from the device's own address and the shared port.
    let registered = timeout(Duration::from_secs(2), async {
        loop {
            let registered = bbmd
                .transport
                .bbmd_state()
                .unwrap()
                .lock()
                .unwrap()
                .is_registered_foreign_device(device_ip.octets(), device_mac.port());
            if registered {
                break;
            }
            tokio::task::yield_now().await;
        }
    });
    registered.await.expect("the device never registered");

    device.transport.send_broadcast(&npdu(1)).await.unwrap();
    let distributed = bbmd.next().await;
    assert_eq!(distributed.npdu.as_ref(), npdu(1));
    assert_eq!(*distributed.source_mac, device.transport.local_mac()[..]);

    bbmd.transport.send_broadcast(&npdu(2)).await.unwrap();
    let forwarded = device.next().await;
    assert_eq!(forwarded.npdu.as_ref(), npdu(2));
    assert_eq!(*forwarded.source_mac, bbmd.transport.local_mac()[..]);
    assert!(forwarded.link_layer_group);

    device.transport.stop().await.unwrap();
    bbmd.transport.stop().await.unwrap();
}

/// Linux delivers a broadcast to every listener bound to its destination,
/// and its loopback carries 127.255.255.255. macOS gives loopback no
/// broadcast address, and Windows isn't relied on to hand one to a socket
/// bound to a loopback address; both are left to the subnet broadcast test.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_broadcast_reaches_every_transport_on_the_port() {
    let subnet = Ipv4Addr::new(127, 255, 255, 255);
    let addresses = two_local_addresses().unwrap();
    let mut nodes = start_on_one_port(addresses, [subnet; 2], |_, _, _| {}).await;
    let port = nodes[0].address().port();
    // A third device's pre-start check sees the port as free to share.
    let mut third = BipTransport::new(Ipv4Addr::new(127, 0, 0, 5), port, subnet);
    third.set_share_port_by_address(true);
    third.check_bind().unwrap();
    let peer = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    peer.set_broadcast(true).unwrap();

    for (tag, to) in [(1, subnet), (2, Ipv4Addr::BROADCAST)] {
        let broadcast = frame(BvlcFunction::ORIGINAL_BROADCAST_NPDU, &npdu(tag));
        peer.send_to(&broadcast, SocketAddrV4::new(to, port))
            .await
            .unwrap();
        for node in &mut nodes {
            let received = node.next().await;
            assert_eq!(received.npdu.as_ref(), npdu(tag), "to {to}");
            assert!(received.link_layer_group);
        }
    }

    // Unicast to an address on the port that no transport is bound to
    // reaches no socket at all, not even a listener: the kernel answers it
    // with port unreachable. The next NPDU each one hands up is the unicast
    // sent to it afterwards.
    let stray = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, &npdu(3));
    let unbound = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 4), port);
    let probe = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    probe.connect(unbound).await.unwrap();
    probe.send(&stray).await.unwrap();
    let refused = timeout(Duration::from_secs(2), probe.recv(&mut [0u8; 64]))
        .await
        .expect("port unreachable arrives")
        .expect_err("nothing answers");
    assert_eq!(refused.kind(), std::io::ErrorKind::ConnectionRefused);
    for node in &mut nodes {
        let own = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, &npdu(4));
        peer.send_to(&own, node.address()).await.unwrap();
        assert_eq!(node.next().await.npdu.as_ref(), npdu(4));
    }

    // One transport's broadcast reaches the other, from its own address,
    // and not itself.
    let [first, second] = &mut nodes;
    first.transport.send_broadcast(&npdu(5)).await.unwrap();
    let received = second.next().await;
    assert_eq!(received.npdu.as_ref(), npdu(5));
    assert_eq!(*received.source_mac, first.transport.local_mac()[..]);
    let fence = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, &npdu(6));
    peer.send_to(&fence, first.address()).await.unwrap();
    assert_eq!(first.next().await.npdu.as_ref(), npdu(6));

    for node in &mut nodes {
        node.transport.stop().await.unwrap();
    }
}

/// Another socket can't take a shared-by-address transport's own address
/// and port, but another address can still share the port.
#[tokio::test]
async fn an_address_and_port_are_not_shared_twice() {
    let addresses = match two_local_addresses() {
        Ok(addresses) => addresses,
        Err(why) => return eprintln!("skipped: {why}"),
    };
    let [node] = start_on_one_port([addresses[0]], [Ipv4Addr::BROADCAST], |_, _, _| {}).await;
    let taken = node.address();
    let checked = |ip| {
        let mut transport = BipTransport::new(ip, taken.port(), Ipv4Addr::BROADCAST);
        transport.set_share_port_by_address(true);
        transport.check_bind()
    };
    checked(addresses[1]).expect("another address shares the port");
    assert!(checked(addresses[0]).is_err(), "the same address twice");
    // Not even with SO_REUSEADDR: Linux leaves the address socket unshared,
    // macOS would need SO_REUSEPORT on both, Windows claims it.
    let squatter = |ip| {
        let socket = udp_socket(socket::SocketRole::SharedWildcard).unwrap();
        socket.bind(&SocketAddrV4::new(ip, taken.port()).into())
    };
    assert!(squatter(*taken.ip()).is_err(), "SO_REUSEADDR on {taken}");
    // Nor the wildcard address beside it on Unix, which would compete for
    // its unicast on Linux, or for the broadcast listener's on macOS.
    #[cfg(unix)]
    assert!(
        squatter(Ipv4Addr::UNSPECIFIED).is_err(),
        "SO_REUSEADDR on 0.0.0.0"
    );
    let [mut node] = [node];
    node.transport.stop().await.unwrap();
}

/// A broadcast on the default-route subnet reaches a transport bound to
/// that subnet's address, on every OS. Skipped without a default route or a
/// broadcast-capable interface for it, or where the host can't send one.
#[tokio::test]
async fn a_subnet_broadcast_reaches_a_transport_sharing_its_port_by_address() {
    let Some(route) = crate::local_addresses::route_ipv4().filter(|ip| !ip.is_loopback()) else {
        return eprintln!("skipped: no default-route IPv4 address");
    };
    let Some(subnet) = crate::local_addresses::subnet_broadcast(route) else {
        return eprintln!("skipped: {route} has no subnet broadcast address");
    };
    let [mut node] = start_on_one_port([route], [subnet], |_, _, _| {}).await;
    let peer = UdpSocket::bind(SocketAddrV4::new(route, 0)).await.unwrap();
    peer.set_broadcast(true).unwrap();
    let broadcast = frame(BvlcFunction::ORIGINAL_BROADCAST_NPDU, &npdu(7));
    let to = SocketAddrV4::new(subnet, node.address().port());
    if let Err(e) = peer.send_to(&broadcast, to).await {
        node.transport.stop().await.unwrap();
        return eprintln!("skipped: this host can't send to {to}: {e}");
    }
    let received = node.next().await;
    assert_eq!(received.npdu.as_ref(), npdu(7));
    assert!(received.link_layer_group);
    eprintln!("ran: a broadcast to {to} reached the transport on {route}");
    node.transport.stop().await.unwrap();
}

/// A limited broadcast sent on another interface reaches a loopback
/// transport's listener (on Linux the 255.255.255.255 listener, on macOS
/// the wildcard one), which drops it: only broadcasts that arrived on the
/// transport's own interface count (#1538). A control socket bound where
/// the listener is shows the broadcast did come back to this host. Skipped
/// without a default route, or where the host can't send a limited
/// broadcast or doesn't loop it back.
#[cfg(unix)]
#[tokio::test]
async fn a_broadcast_from_another_interface_is_dropped() {
    let Some(route) = crate::local_addresses::route_ipv4().filter(|ip| !ip.is_loopback()) else {
        return eprintln!("skipped: no default-route IPv4 address");
    };
    let loopback = if cfg!(target_os = "linux") {
        Ipv4Addr::new(127, 0, 0, 2)
    } else {
        Ipv4Addr::LOCALHOST
    };
    let [mut node] = start_on_one_port([loopback], [Ipv4Addr::BROADCAST], |_, _, _| {}).await;
    let port = node.address().port();
    // Shares the port with the listener on the same address and options.
    let listening = if cfg!(target_os = "linux") {
        Ipv4Addr::BROADCAST
    } else {
        Ipv4Addr::UNSPECIFIED
    };
    let control = udp_socket(socket::SocketRole::BroadcastListener).unwrap();
    control
        .bind(&SocketAddrV4::new(listening, port).into())
        .unwrap();
    let control = UdpSocket::from_std(control.into()).unwrap();
    let to = SocketAddrV4::new(Ipv4Addr::BROADCAST, port);
    let peer = UdpSocket::bind(SocketAddrV4::new(route, 0)).await.unwrap();
    peer.set_broadcast(true).unwrap();
    let broadcast = frame(BvlcFunction::ORIGINAL_BROADCAST_NPDU, &npdu(8));
    if let Err(e) = peer.send_to(&broadcast, to).await {
        node.transport.stop().await.unwrap();
        return eprintln!("skipped: this host can't send to {to} from {route}: {e}");
    }
    let mut seen = [0u8; 64];
    let looped = timeout(Duration::from_secs(2), control.recv_from(&mut seen)).await;
    if !matches!(looped, Ok(Ok((len, _))) if seen[..len] == broadcast[..]) {
        node.transport.stop().await.unwrap();
        return eprintln!("skipped: this host didn't loop the broadcast to {to} back");
    }
    // The unicast after it is the first NPDU handed up.
    let fence = UdpSocket::bind(SocketAddrV4::new(loopback, 0))
        .await
        .unwrap();
    let unicast = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, &npdu(9));
    fence.send_to(&unicast, node.address()).await.unwrap();
    assert_eq!(node.next().await.npdu.as_ref(), npdu(9));
    eprintln!("ran: a broadcast from {route} did not reach the transport on {loopback}");
    node.transport.stop().await.unwrap();
}
