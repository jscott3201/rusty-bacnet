//! A requested B/IP port is claimed unless sharing is asked for (#2366):
//! which sockets can bind it beside the transport on each OS, the error a
//! transport that finds it held starts with, and a B/IP-M node whose group
//! sits on its own port (on Linux bound to its interface address, or
//! switched to `SO_REUSEPORT` by a later activation).

use super::*;
use crate::port_retry::{ATTEMPTS, bind_error, lost_to_another_socket, rerun_on_lost_port};
use socket::{BindPlan, BoundSockets, SocketRole, udp_socket, wildcard_role};
use std::io;

/// Whether this OS lets a socket that sets `SO_REUSEADDR` (or
/// `SO_REUSEPORT`) bind a specific address beside a claimed wildcard socket
/// (`port_ownership`).
const SPECIFIC_ADDRESS_GAP: bool = cfg!(all(
    unix,
    not(any(target_os = "linux", target_os = "android"))
));

/// Whether two wildcard sockets that both set `SO_REUSEADDR` share a port:
/// macOS and the BSDs also want `SO_REUSEPORT` on both.
const WILDCARD_SHARING: bool = cfg!(any(windows, target_os = "linux", target_os = "android"));

/// Whether this OS binds a B/IP-M node with its group on its own port to
/// its interface address (socket.rs).
const GROUP_BY_ADDRESS: bool = cfg!(any(target_os = "linux", target_os = "android"));

/// Every combination of `SO_REUSEADDR` and `SO_REUSEPORT` a squatter might
/// set.
#[cfg(unix)]
const EVERY_OPTION: [(bool, bool); 4] =
    [(false, false), (true, false), (false, true), (true, true)];

/// A UDP port no socket holds now. Another process can take it before the
/// test binds it, so its users rerun on a lost port.
fn free_port() -> u16 {
    std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Another program's socket on `ip:port`, with `SO_REUSEADDR` and
/// `SO_REUSEPORT` as asked (the latter only on Unix).
fn squatter(
    ip: Ipv4Addr,
    port: u16,
    reuse_address: bool,
    reuse_port: bool,
) -> io::Result<socket2::Socket> {
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None)?;
    socket.set_reuse_address(reuse_address)?;
    #[cfg(unix)]
    socket.set_reuse_port(reuse_port)?;
    #[cfg(not(unix))]
    let _ = reuse_port;
    socket.bind(&SocketAddrV4::new(ip, port).into())?;
    Ok(socket)
}

/// `socket::bind` for the plan on `interface` that `adjust` makes, on a
/// fresh requested port, then `check` with the port and what was bound.
fn on_a_requested_port(
    interface: Ipv4Addr,
    adjust: impl Fn(&mut BindPlan),
    check: impl Fn(u16, &BoundSockets),
) {
    for attempt in 1..=ATTEMPTS {
        let port = free_port();
        let mut plan = plan(interface, port, false);
        adjust(&mut plan);
        match socket::bind(plan) {
            Ok(bound) => return check(port, &bound),
            Err(e) if attempt < ATTEMPTS && lost_to_another_socket(&e) => continue,
            Err(e) => panic!("bind {interface}:{port}: {e}"),
        }
    }
}

const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 94, 3);

/// A group at the node's own port.
fn own_port_group() -> BipMulticast {
    BipMulticast::new(GROUP).unwrap()
}

#[test]
fn the_bind_shape_follows_the_port_the_opt_in_and_the_group() {
    let shape = |interface, port, share, group: Option<BipMulticast>| {
        let mut plan = plan(interface, port, false);
        plan.share_wildcard = share;
        plan.multicast = group;
        (wildcard_role(&plan), plan.per_address())
    };
    let own = Some(own_port_group());
    let apart = Some(own_port_group().with_port(47809).unwrap());
    let at_47808 = Some(own_port_group().with_port(47808).unwrap());
    let local = Ipv4Addr::LOCALHOST;
    // A port the OS picks is never shared (#892), whatever is asked.
    for (share, group) in [(false, None), (true, None), (false, own), (true, own)] {
        assert_eq!(
            shape(local, 0, share, group),
            (SocketRole::ClaimedWildcard, false)
        );
    }
    for group in [None, apart] {
        let claimed = (SocketRole::ClaimedWildcard, false);
        assert_eq!(shape(local, 47808, false, group), claimed);
    }
    // A group on the node's own port: by address on Linux.
    for group in [own, at_47808] {
        let by_address = shape(local, 47808, false, group).1;
        assert_eq!(by_address, GROUP_BY_ADDRESS, "{group:?}");
    }
    // Not without an interface address, nor when sharing is asked for.
    assert!(!shape(Ipv4Addr::UNSPECIFIED, 47808, false, own).1);
    for group in [None, own, apart] {
        let shared = (SocketRole::SharedWildcard, false);
        assert_eq!(shape(local, 47808, true, group), shared);
    }
}

/// A transport built plainly claims its requested port: nothing sets the
/// opt-in, and a later bind of the port fails on every OS.
#[tokio::test]
async fn a_plain_transport_claims_its_requested_port() {
    rerun_on_lost_port(async || {
        let port = free_port();
        let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::BROADCAST);
        let _rx = transport.start().await.map_err(bind_error)?;
        let primary = socket2::SockRef::from(&***transport.socket.as_ref().unwrap());
        assert!(!primary.reuse_address().unwrap());
        let refused = squatter(Ipv4Addr::UNSPECIFIED, port, true, false).unwrap_err();
        assert!(lost_to_another_socket(&refused), "{refused}");
        transport.stop().await.unwrap();
        Ok(())
    })
    .await;
}

#[test]
fn a_requested_port_refuses_a_second_bind_by_default() {
    on_a_requested_port(
        Ipv4Addr::UNSPECIFIED,
        |_| {},
        |port, bound| {
            assert!(!bound.primary.reuse_address().unwrap());
            // Another wildcard socket asking to share: refused everywhere.
            let refused = squatter(Ipv4Addr::UNSPECIFIED, port, true, false).unwrap_err();
            assert!(lost_to_another_socket(&refused), "{refused}");
            // A specific address asking to share: refused but on macOS and
            // the BSDs, which have no option to stop it.
            let specific = squatter(Ipv4Addr::LOCALHOST, port, true, false);
            assert_eq!(
                specific.is_ok(),
                SPECIFIC_ADDRESS_GAP,
                "{:?}",
                specific.err()
            );
            // A second transport's bind says why, and keeps the OS error.
            let second = socket::bind(plan(Ipv4Addr::UNSPECIFIED, port, false))
                .err()
                .expect("a second bind of a claimed port");
            assert!(lost_to_another_socket(&second), "{second}");
            let shown = second.to_string();
            assert!(
                shown.contains(&format!("UDP port {port} is in use")),
                "{shown}"
            );
            assert!(shown.contains("share_wildcard_port"), "{shown}");
            assert!(shown.contains("--share-port"), "{shown}");
        },
    );
}

#[test]
fn a_requested_port_shared_on_request_takes_a_second_sharing_bind() {
    on_a_requested_port(
        Ipv4Addr::UNSPECIFIED,
        |plan| plan.share_wildcard = true,
        |port, bound| {
            assert!(bound.primary.reuse_address().unwrap());
            let mut shared = plan(Ipv4Addr::UNSPECIFIED, port, false);
            shared.share_wildcard = true;
            let second = socket::bind(shared);
            assert_eq!(second.is_ok(), WILDCARD_SHARING, "{:?}", second.err());
            if WILDCARD_SHARING {
                squatter(Ipv4Addr::UNSPECIFIED, port, true, false).unwrap();
            }
            // A specific address asking to share binds beside it everywhere.
            squatter(Ipv4Addr::LOCALHOST, port, true, false).unwrap();
        },
    );
}

/// A B/IP-M node with its group on its own port binds the group's listener
/// beside its primary socket. On Linux the primary binds the interface
/// address with no reuse option and broadcast listeners beside it, so every
/// squatter is refused; macOS binds the claimed wildcard socket, beside
/// which a specific address with `SO_REUSEADDR` or `SO_REUSEPORT` still
/// binds.
#[cfg(unix)]
#[test]
fn a_group_on_the_nodes_own_port_binds_beside_its_primary() {
    on_a_requested_port(
        Ipv4Addr::LOCALHOST,
        |plan| plan.multicast = Some(own_port_group()),
        |port, bound| {
            let primary = if GROUP_BY_ADDRESS {
                Ipv4Addr::LOCALHOST
            } else {
                Ipv4Addr::UNSPECIFIED
            };
            assert_eq!(address_of(&bound.primary), SocketAddrV4::new(primary, port));
            assert!(!bound.primary.reuse_address().unwrap());
            assert!(!bound.primary.reuse_port().unwrap());
            let group = bound.group.as_ref().expect("the group's listener");
            assert_eq!(address_of(group), SocketAddrV4::new(GROUP, port));
            assert_eq!(bound.listeners.is_empty(), !GROUP_BY_ADDRESS);
            for (reuse_address, reuse_port) in EVERY_OPTION {
                let wildcard = squatter(Ipv4Addr::UNSPECIFIED, port, reuse_address, reuse_port);
                assert!(wildcard.is_err(), "0.0.0.0 {reuse_address} {reuse_port}");
                let specific = squatter(Ipv4Addr::LOCALHOST, port, reuse_address, reuse_port);
                // macOS and the BSDs: either option skips the wildcard check.
                let gap = SPECIFIC_ADDRESS_GAP && (reuse_address || reuse_port);
                assert_eq!(
                    specific.is_ok(),
                    gap,
                    "127.0.0.1 {reuse_address} {reuse_port}"
                );
            }
            // Another member's listener on the group binds beside it.
            udp_socket(SocketRole::GroupListener)
                .unwrap()
                .bind(&SocketAddrV4::new(GROUP, port).into())
                .unwrap();
        },
    );
}

/// The transport's setter reaches the bind: two transports on one requested
/// port start only when both share it, and only where the OS shares a
/// wildcard port.
#[tokio::test]
async fn a_second_transport_on_a_requested_port_starts_only_when_both_share() {
    rerun_on_lost_port(async || {
        let port = free_port();
        let transport = |share| {
            let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::BROADCAST);
            transport.set_share_wildcard_port(share);
            transport
        };
        let mut first = transport(false);
        let _rx = first.start().await.map_err(bind_error)?;
        let Err(Error::Transport(refused)) = transport(true).start().await else {
            panic!("a second transport started on a claimed port");
        };
        assert!(lost_to_another_socket(&refused), "{refused}");
        assert!(refused.to_string().contains("refuses sharing"), "{refused}");
        first.stop().await.unwrap();

        let mut first = transport(true);
        let _rx = first.start().await.map_err(bind_error)?;
        let mut second = transport(true);
        let started = second.start().await;
        assert_eq!(started.is_ok(), WILDCARD_SHARING, "{:?}", started.err());
        if WILDCARD_SHARING {
            second.stop().await.unwrap();
        }
        let Err(Error::Transport(refused)) = transport(false).start().await else {
            panic!("a claiming transport started beside a shared one");
        };
        assert!(
            refused.to_string().contains("share_wildcard_port"),
            "{refused}"
        );
        first.stop().await.unwrap();
        Ok(())
    })
    .await;
}

/// A group-driven broadcast listener that can't bind: a held address fails
/// the start as a held port, naming it, and any other error is only
/// warned about (#2366).
#[test]
fn a_held_broadcast_address_fails_the_start_and_other_errors_only_warn() {
    let holder = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let held = holder.local_addr().unwrap();
    let in_use = std::net::UdpSocket::bind(held).unwrap_err();
    let ip = Ipv4Addr::new(127, 255, 255, 255);
    let refused = socket::group_shape_listener_failed(ip, held.port(), in_use)
        .expect_err("a held address fails the start");
    assert!(lost_to_another_socket(&refused), "{refused}");
    let shown = refused.to_string();
    assert!(
        shown.contains(&format!("UDP port {} on {ip}", held.port())),
        "{shown}"
    );
    // An address this host doesn't have.
    let unbindable = std::net::UdpSocket::bind((Ipv4Addr::new(192, 0, 2, 1), 0)).unwrap_err();
    assert!(!lost_to_another_socket(&unbindable), "{unbindable}");
    socket::group_shape_listener_failed(ip, held.port(), unbindable).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod linux {
    use super::*;
    use std::net::SocketAddr;
    use tokio::time::timeout;

    /// A Who-Is, as the NPDU every test frame carries.
    const WHO_IS: &[u8] = &[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];

    fn frame(function: u8) -> Vec<u8> {
        let mut frame = vec![0x81, function, 0, (4 + WHO_IS.len()) as u8];
        frame.extend_from_slice(WHO_IS);
        frame
    }

    /// A plain peer on 127.0.0.1 that can send to the group on loopback.
    fn peer() -> std::net::UdpSocket {
        let peer = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        peer.set_broadcast(true).unwrap();
        socket2::SockRef::from(&peer)
            .set_multicast_if_v4(&Ipv4Addr::LOCALHOST)
            .unwrap();
        peer
    }

    async fn next(rx: &mut mpsc::Receiver<ReceivedNpdu>) -> ReceivedNpdu {
        timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("nothing heard")
            .expect("receive channel open")
    }

    async fn hears_nothing(rx: &mut mpsc::Receiver<ReceivedNpdu>) {
        let heard = timeout(Duration::from_millis(300), rx.recv()).await;
        assert!(heard.is_err(), "unexpected NPDU {heard:?}");
    }

    fn primary_address(node: &BipTransport) -> SocketAddr {
        node.socket.as_ref().unwrap().local_addr().unwrap()
    }

    /// A member that starts with its group on its own port binds its
    /// interface address (socket.rs) and refuses every squatter. It takes
    /// unicast and its group's broadcasts, drops its own echo, and drops a
    /// plain peer's directed and limited broadcasts, as the wildcard member
    /// did, now on the broadcast listeners (#2366).
    #[tokio::test]
    async fn a_member_on_its_own_port_binds_its_address_and_hears_as_before() {
        rerun_on_lost_port(async || {
            let port = free_port();
            let subnet = Ipv4Addr::new(127, 255, 255, 255);
            let mut member = BipTransport::new(Ipv4Addr::LOCALHOST, port, subnet);
            member.set_multicast(Some(own_port_group())).unwrap();
            let mut rx = member.start().await.map_err(bind_error)?;
            let own = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
            assert_eq!(primary_address(&member), own);
            for ip in [Ipv4Addr::UNSPECIFIED, Ipv4Addr::LOCALHOST] {
                for (reuse_address, reuse_port) in EVERY_OPTION {
                    let taken = squatter(ip, port, reuse_address, reuse_port);
                    assert!(taken.is_err(), "{ip} {reuse_address} {reuse_port}");
                }
            }
            let peer = peer();
            peer.send_to(&frame(0x0A), own).unwrap();
            let heard = next(&mut rx).await;
            assert_eq!((&heard.npdu[..], heard.link_layer_group), (WHO_IS, false));
            peer.send_to(&frame(0x0B), (GROUP, port)).unwrap();
            let heard = next(&mut rx).await;
            assert_eq!((&heard.npdu[..], heard.link_layer_group), (WHO_IS, true));
            for broadcast in [subnet, Ipv4Addr::BROADCAST] {
                peer.send_to(&frame(0x0B), (broadcast, port)).unwrap();
            }
            member.send_broadcast(WHO_IS).await.unwrap();
            hears_nothing(&mut rx).await;
            let counters = member.receive_counters();
            assert_eq!(counters.listener_mismatch, 2, "{counters:?}");
            assert_eq!(counters.own_echo, 1, "{counters:?}");
            member.stop().await.unwrap();
            Ok(())
        })
        .await;
    }

    /// A group activated on the port of a node bound to 0.0.0.0 switches the
    /// primary to `SO_REUSEPORT` and binds the listener beside it: the group
    /// is heard, a socket that sets only `SO_REUSEADDR` is still refused, and
    /// a same-user one that sets `SO_REUSEPORT` can bind until the node
    /// restarts, which binds the interface address instead.
    #[tokio::test]
    async fn a_group_activated_on_a_claimed_port_switches_roles() {
        rerun_on_lost_port(async || {
            let port = free_port();
            let mut node = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::LOCALHOST);
            let mut rx = node.start().await.map_err(bind_error)?;
            let primary = || socket2::SockRef::from(&***node.socket.as_ref().unwrap());
            assert!(!primary().reuse_port().unwrap());
            let view = node.bip_port().unwrap().multicast.unwrap();
            view.activate(GROUP.octets()).map_err(bind_error)?;
            assert_eq!(node.multicast(), Some(own_port_group()));
            assert!(primary().reuse_port().unwrap());
            assert!(!primary().reuse_address().unwrap());
            peer().send_to(&frame(0x0B), (GROUP, port)).unwrap();
            let heard = next(&mut rx).await;
            assert_eq!((&heard.npdu[..], heard.link_layer_group), (WHO_IS, true));
            assert!(squatter(Ipv4Addr::UNSPECIFIED, port, true, false).is_err());
            let same_user = squatter(Ipv4Addr::UNSPECIFIED, port, false, true);
            assert!(same_user.is_ok(), "{:?}", same_user.err());
            drop(same_user);
            node.stop().await.unwrap();
            let _rx = node.start().await.map_err(bind_error)?;
            let own = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
            assert_eq!(primary_address(&node), own);
            assert!(squatter(Ipv4Addr::UNSPECIFIED, port, false, true).is_err());
            node.stop().await.unwrap();
            Ok(())
        })
        .await;
    }

    /// A peer's subnet-directed and limited broadcasts on loopback, which a
    /// member's broadcast listeners take and drop (#2366).
    fn send_subnet_broadcasts(port: u16) {
        let peer = peer();
        for broadcast in [Ipv4Addr::new(127, 255, 255, 255), Ipv4Addr::BROADCAST] {
            peer.send_to(&frame(0x0B), (broadcast, port)).unwrap();
        }
    }

    /// A member whose configured broadcast address is neither its subnet's
    /// nor 255.255.255.255 still starts with its group on its own port, as
    /// it did on `0.0.0.0`, and its listeners still hear the subnet-directed
    /// and limited broadcasts, worked out from the netmask: at start, and at
    /// the restart after a remote group write.
    #[tokio::test]
    async fn an_unusual_broadcast_address_never_refuses_the_start() {
        rerun_on_lost_port(async || {
            let unusual = Ipv4Addr::new(192, 0, 2, 255);
            for written in [false, true] {
                let port = free_port();
                let mut member = BipTransport::new(Ipv4Addr::LOCALHOST, port, unusual);
                if written {
                    let _rx = member.start().await.map_err(bind_error)?;
                    let view = member.bip_port().unwrap().multicast.unwrap();
                    view.activate(GROUP.octets()).map_err(bind_error)?;
                    member.stop().await.unwrap();
                } else {
                    member.set_multicast(Some(own_port_group())).unwrap();
                }
                let mut rx = member.start().await.map_err(bind_error)?;
                let own = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
                assert_eq!(primary_address(&member), own, "written: {written}");
                send_subnet_broadcasts(port);
                hears_nothing(&mut rx).await;
                let counters = member.receive_counters();
                assert_eq!(counters.listener_mismatch, 2, "{written}: {counters:?}");
                member.stop().await.unwrap();
            }
            Ok(())
        })
        .await;
    }

    /// A role switch whose listener fails is undone: the primary's
    /// `SO_REUSEPORT` is clear again, the group stays as it was, and the
    /// next activation switches and joins.
    #[tokio::test]
    async fn a_failed_switch_is_undone() {
        rerun_on_lost_port(async || {
            let port = free_port();
            let mut node = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::LOCALHOST);
            let mut rx = node.start().await.map_err(bind_error)?;
            let primary = || socket2::SockRef::from(&***node.socket.as_ref().unwrap());
            let view = node.bip_port().unwrap().multicast.unwrap();
            node.multicast
                .fail_next_switch
                .store(true, std::sync::atomic::Ordering::Relaxed);
            assert!(view.activate(GROUP.octets()).is_err());
            assert!(!primary().reuse_port().unwrap());
            assert_eq!(node.multicast(), None);
            // Nothing listens on the group's port for the node: a datagram
            // sent there is not heard.
            peer().send_to(&frame(0x0B), (GROUP, port)).unwrap();
            hears_nothing(&mut rx).await;
            view.activate(GROUP.octets()).map_err(bind_error)?;
            assert!(primary().reuse_port().unwrap());
            peer().send_to(&frame(0x0B), (GROUP, port)).unwrap();
            assert!(next(&mut rx).await.link_layer_group);
            node.stop().await.unwrap();
            Ok(())
        })
        .await;
    }

    /// A group activated through the Network Port on a member bound by
    /// address moves its listener to the new group on its own port, with no
    /// switch: the primary keeps its address and no reuse option.
    #[tokio::test]
    async fn a_group_activated_on_a_member_bound_by_address_joins_without_a_switch() {
        rerun_on_lost_port(async || {
            let port = free_port();
            let mut member = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::LOCALHOST);
            member.set_multicast(Some(own_port_group())).unwrap();
            let mut rx = member.start().await.map_err(bind_error)?;
            let moved = Ipv4Addr::new(239, 255, 94, 4);
            let view = member.bip_port().unwrap().multicast.unwrap();
            view.activate(moved.octets()).map_err(bind_error)?;
            assert_eq!(member.multicast(), Some(BipMulticast::new(moved).unwrap()));
            let primary = socket2::SockRef::from(&***member.socket.as_ref().unwrap());
            assert!(!primary.reuse_port().unwrap());
            let own = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
            assert_eq!(primary_address(&member), own);
            let peer = peer();
            peer.send_to(&frame(0x0B), (GROUP, port)).unwrap();
            hears_nothing(&mut rx).await;
            peer.send_to(&frame(0x0B), (moved, port)).unwrap();
            assert!(next(&mut rx).await.link_layer_group);
            member.stop().await.unwrap();
            Ok(())
        })
        .await;
    }

    /// A BBMD serving its group on its own port (#941) binds by address
    /// too: a broadcast heard through the group goes up, and on to its
    /// two-hop BDT peer as a Forwarded-NPDU from the BBMD's address.
    #[tokio::test]
    async fn a_group_bbmd_on_its_own_port_binds_by_address_and_forwards() {
        rerun_on_lost_port(async || {
            let port = free_port();
            let bdt_peer = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            bdt_peer
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let row = |port| crate::bbmd::BdtEntry {
                ip: Ipv4Addr::LOCALHOST.octets(),
                port,
                broadcast_mask: [0xFF; 4],
            };
            let peer_port = bdt_peer.local_addr().unwrap().port();
            let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::LOCALHOST);
            bbmd.set_multicast(Some(own_port_group())).unwrap();
            bbmd.enable_bbmd(vec![row(port), row(peer_port)]);
            let mut rx = bbmd.start().await.map_err(bind_error)?;
            let own = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
            assert_eq!(primary_address(&bbmd), own);
            let member = peer();
            member.send_to(&frame(0x0B), (GROUP, port)).unwrap();
            assert!(next(&mut rx).await.link_layer_group);
            let mut forwarded = [0; 64];
            let (len, from) = bdt_peer.recv_from(&mut forwarded).unwrap();
            assert_eq!(from, own);
            assert_eq!(&forwarded[..2], &[0x81, 0x04]);
            assert!(forwarded[..len].ends_with(WHO_IS));
            bbmd.stop().await.unwrap();
            Ok(())
        })
        .await;
    }

    /// Another socket holding the subnet broadcast on the port fails a
    /// member's start with a held-port error naming the port, as its
    /// `0.0.0.0` bind would have.
    #[tokio::test]
    async fn a_held_subnet_broadcast_fails_a_members_start() {
        let subnet = Ipv4Addr::new(127, 255, 255, 255);
        let (port, _holder) = loop {
            let port = free_port();
            if let Ok(holder) = std::net::UdpSocket::bind((subnet, port)) {
                break (port, holder);
            }
        };
        let mut member = BipTransport::new(Ipv4Addr::LOCALHOST, port, subnet);
        member.set_multicast(Some(own_port_group())).unwrap();
        let Err(Error::Transport(refused)) = member.start().await else {
            panic!("a member started beside a held subnet broadcast");
        };
        assert!(lost_to_another_socket(&refused), "{refused}");
        let shown = refused.to_string();
        assert!(
            shown.contains(&format!("UDP port {port} on {subnet}")),
            "{shown}"
        );
    }

    /// A second switch that fails leaves the first one's `SO_REUSEPORT` set
    /// and the first group in use and heard.
    #[tokio::test]
    async fn a_failed_second_switch_keeps_the_first() {
        rerun_on_lost_port(async || {
            let port = free_port();
            let mut node = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::LOCALHOST);
            let mut rx = node.start().await.map_err(bind_error)?;
            let primary = || socket2::SockRef::from(&***node.socket.as_ref().unwrap());
            let view = node.bip_port().unwrap().multicast.unwrap();
            view.activate(GROUP.octets()).map_err(bind_error)?;
            assert!(primary().reuse_port().unwrap());
            node.multicast
                .fail_next_switch
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let moved = Ipv4Addr::new(239, 255, 94, 4);
            assert!(view.activate(moved.octets()).is_err());
            assert!(primary().reuse_port().unwrap());
            assert_eq!(node.multicast(), Some(own_port_group()));
            peer().send_to(&frame(0x0B), (GROUP, port)).unwrap();
            assert!(next(&mut rx).await.link_layer_group);
            node.stop().await.unwrap();
            Ok(())
        })
        .await;
    }
}
