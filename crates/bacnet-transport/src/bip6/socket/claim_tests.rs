//! A requested B/IPv6 port is claimed unless sharing is asked for (#2385):
//! which sockets can bind it beside the transport on each OS, in normal and
//! in foreign mode, the error a transport that finds it held starts with,
//! and what a claimed normal node still receives on its one socket.

use super::*;
use crate::bip6::{Bip6ForeignDeviceConfig, Bip6Transport};
use crate::port::TransportPort;
use crate::port_retry::{ATTEMPTS, bind_error, lost_to_another_socket, rerun_on_lost_port};
use bacnet_types::error::Error;

/// Whether this OS lets a socket that sets `SO_REUSEADDR` (or
/// `SO_REUSEPORT`) bind a specific address beside a wildcard socket, or the
/// wildcard address beside a specific one, when the first socket set neither
/// (`port_ownership`).
const SPECIFIC_ADDRESS_GAP: bool = cfg!(all(
    unix,
    not(any(target_os = "linux", target_os = "android"))
));

/// Whether two sockets on one address that both set `SO_REUSEADDR` share a
/// port: macOS and the BSDs also want `SO_REUSEPORT` on both.
const SHARING: bool = cfg!(any(windows, target_os = "linux", target_os = "android"));

/// Every combination of `SO_REUSEADDR` and `SO_REUSEPORT` a squatter might
/// set.
#[cfg(unix)]
const EVERY_OPTION: [(bool, bool); 4] =
    [(false, false), (true, false), (false, true), (true, true)];

const LOCAL: Ipv6Addr = Ipv6Addr::LOCALHOST;
const ANY: Ipv6Addr = Ipv6Addr::UNSPECIFIED;

/// A UDP port no socket holds now. Another process can take it before the
/// test binds it, so its users rerun on a lost port.
fn free_port() -> u16 {
    std::net::UdpSocket::bind((ANY, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Another program's socket on `[ip]:port`, with `SO_REUSEADDR` and
/// `SO_REUSEPORT` as asked (the latter only on Unix).
fn squatter(
    ip: Ipv6Addr,
    port: u16,
    reuse_address: bool,
    reuse_port: bool,
) -> io::Result<socket2::Socket> {
    let socket = socket2::Socket::new(socket2::Domain::IPV6, socket2::Type::DGRAM, None)?;
    socket.set_only_v6(true)?;
    socket.set_reuse_address(reuse_address)?;
    #[cfg(unix)]
    socket.set_reuse_port(reuse_port)?;
    #[cfg(not(unix))]
    let _ = reuse_port;
    socket.bind(&SocketAddrV6::new(ip, port, 0, 0).into())?;
    Ok(socket)
}

/// A socket bound for `role` on `::1` at a fresh requested port, and the
/// port.
async fn bound(port_use: PortUse, role: Role) -> (u16, Bip6Socket) {
    for attempt in 1..=ATTEMPTS {
        let port = free_port();
        match Bip6Socket::bind(LOCAL, port, port_use, role).await {
            Ok(socket) => return (port, socket),
            Err(e) if attempt < ATTEMPTS && lost_to_another_socket(&e) => continue,
            Err(e) => panic!("bind port {port} for {role:?}: {e}"),
        }
    }
    unreachable!("the last attempt returns or panics")
}

fn reuses_address(socket: &Bip6Socket) -> bool {
    socket2::SockRef::from(&socket.udp).reuse_address().unwrap()
}

/// The error of a second bind of `port` for `role`, which the first holds.
async fn second_bind(port: u16, port_use: PortUse, role: Role) -> io::Error {
    Bip6Socket::bind(LOCAL, port, port_use, role)
        .await
        .err()
        .expect("a second bind of a held port")
}

#[tokio::test]
async fn only_a_requested_port_shared_on_request_sets_address_reuse() {
    let reuse = |port, port_use| udp_socket(port, port_use).unwrap().reuse_address().unwrap();
    assert!(!reuse(0xBAC0, PortUse::Claimed));
    assert!(reuse(0xBAC0, PortUse::Shared));
    // A port the OS picks is never shared (#892), whatever is asked.
    assert!(!reuse(0, PortUse::Claimed));
    assert!(!reuse(0, PortUse::Shared));
    for role in [Role::Normal, Role::Foreign(None)] {
        let ephemeral = Bip6Socket::bind(LOCAL, 0, PortUse::Shared, role)
            .await
            .unwrap();
        assert!(!reuses_address(&ephemeral), "{role:?}");
    }
    assert_eq!(PortUse::sharing(false), PortUse::Claimed);
    assert_eq!(PortUse::sharing(true), PortUse::Shared);
}

/// A transport built plainly claims its requested port: nothing sets the
/// opt-in, and a later bind of the port fails on every OS.
#[tokio::test]
async fn a_plain_transport_claims_its_requested_port() {
    rerun_on_lost_port(async || {
        let port = free_port();
        let mut transport = Bip6Transport::new(LOCAL, port, Some(10));
        let _rx = transport.start().await.map_err(bind_error)?;
        assert!(!reuses_address(transport.socket.as_ref().unwrap()));
        let refused = squatter(ANY, port, true, false).unwrap_err();
        assert!(lost_to_another_socket(&refused), "{refused}");
        transport.stop().await.unwrap();
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn a_requested_port_refuses_a_second_bind_by_default() {
    let (port, socket) = bound(PortUse::Claimed, Role::Normal).await;
    assert_eq!(
        socket.local_address().unwrap(),
        SocketAddrV6::new(ANY, port, 0, 0)
    );
    assert!(!reuses_address(&socket));
    // Another wildcard socket asking to share: refused everywhere.
    let refused = squatter(ANY, port, true, false).unwrap_err();
    assert!(lost_to_another_socket(&refused), "{refused}");
    // A specific address asking to share: refused but on macOS and the
    // BSDs, which have no option to stop it.
    let specific = squatter(LOCAL, port, true, false);
    assert_eq!(
        specific.is_ok(),
        SPECIFIC_ADDRESS_GAP,
        "{:?}",
        specific.as_ref().err()
    );
    drop(specific);
    #[cfg(unix)]
    for (reuse_address, reuse_port) in EVERY_OPTION {
        let wildcard = squatter(ANY, port, reuse_address, reuse_port);
        assert!(wildcard.is_err(), ":: {reuse_address} {reuse_port}");
        let specific = squatter(LOCAL, port, reuse_address, reuse_port);
        // macOS and the BSDs: either option skips the wildcard check.
        let gap = SPECIFIC_ADDRESS_GAP && (reuse_address || reuse_port);
        assert_eq!(specific.is_ok(), gap, "::1 {reuse_address} {reuse_port}");
    }
    // A second transport's bind says why, and keeps the OS error.
    let second = second_bind(port, PortUse::Claimed, Role::Normal).await;
    assert!(lost_to_another_socket(&second), "{second}");
    let shown = second.to_string();
    assert!(
        shown.contains(&format!("UDP port {port} is in use")),
        "{shown}"
    );
    assert!(shown.contains("a B/IPv6 transport"), "{shown}");
    assert!(shown.contains("share_wildcard_port"), "{shown}");
    assert!(shown.contains("--share-port"), "{shown}");
    // One that asks to share is told the holder doesn't.
    let sharing = second_bind(port, PortUse::Shared, Role::Normal).await;
    assert!(lost_to_another_socket(&sharing), "{sharing}");
    assert!(sharing.to_string().contains("can't be shared"), "{sharing}");
}

#[tokio::test]
async fn a_requested_port_shared_on_request_takes_a_second_sharing_bind() {
    let (port, socket) = bound(PortUse::Shared, Role::Normal).await;
    assert!(reuses_address(&socket));
    let second = Bip6Socket::bind(LOCAL, port, PortUse::Shared, Role::Normal).await;
    assert_eq!(second.is_ok(), SHARING, "{:?}", second.err());
    drop(second);
    let wildcard = squatter(ANY, port, true, false);
    assert_eq!(wildcard.is_ok(), SHARING, "{:?}", wildcard.err());
    // A specific address asking to share binds beside it everywhere.
    squatter(LOCAL, port, true, false).unwrap();
    // A claiming bind beside it is refused, and told how to share.
    let claiming = second_bind(port, PortUse::Claimed, Role::Normal).await;
    assert!(
        claiming.to_string().contains("share_wildcard_port"),
        "{claiming}"
    );
}

/// A foreign device binds its own address, and by default claims it: no
/// other socket binds that address and port, whatever it sets. Linux
/// refuses `[::]` on the port too; macOS and the BSDs let a socket that
/// sets a reuse option bind it, and it then gets none of the unicast sent
/// to the device's address.
#[tokio::test]
async fn a_foreign_device_claims_its_address_and_port() {
    let (port, socket) = bound(PortUse::Claimed, Role::Foreign(None)).await;
    let own = SocketAddrV6::new(LOCAL, port, 0, 0);
    assert_eq!(socket.local_address().unwrap(), own);
    assert!(!reuses_address(&socket));
    let refused = squatter(LOCAL, port, true, false).unwrap_err();
    assert!(lost_to_another_socket(&refused), "{refused}");
    #[cfg(unix)]
    for (reuse_address, reuse_port) in EVERY_OPTION {
        let same = squatter(LOCAL, port, reuse_address, reuse_port);
        assert!(same.is_err(), "::1 {reuse_address} {reuse_port}");
        let wildcard = squatter(ANY, port, reuse_address, reuse_port);
        let gap = SPECIFIC_ADDRESS_GAP && (reuse_address || reuse_port);
        assert_eq!(wildcard.is_ok(), gap, ":: {reuse_address} {reuse_port}");
        let Ok(wildcard) = wildcard else { continue };
        // The device's unicast still reaches the device.
        wildcard.set_nonblocking(true).unwrap();
        let peer = std::net::UdpSocket::bind((LOCAL, 0)).unwrap();
        peer.send_to(&[0x82, 0x00], own).unwrap();
        let mut buf = [0u8; 16];
        let read = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            socket.recv_from(&mut buf).await
        });
        let Ok(Ok(Inbound::Datagram(datagram))) = read.await else {
            panic!("the device's unicast went astray ({reuse_address} {reuse_port})");
        };
        assert_eq!(datagram.len, 2);
        let astray = std::net::UdpSocket::from(wildcard).recv_from(&mut buf);
        assert_eq!(astray.unwrap_err().kind(), io::ErrorKind::WouldBlock);
    }
    // A second foreign device's bind says why, naming the address.
    let second = second_bind(port, PortUse::Claimed, Role::Foreign(None)).await;
    assert!(lost_to_another_socket(&second), "{second}");
    let shown = second.to_string();
    assert!(
        shown.contains(&format!("UDP port {port} on ::1 is in use")),
        "{shown}"
    );
    assert!(shown.contains("share_wildcard_port"), "{shown}");
}

/// With sharing on, a foreign device's address takes a second sharing bind
/// where the OS shares one address between two `SO_REUSEADDR` sockets.
#[tokio::test]
async fn a_foreign_device_shares_its_address_on_request() {
    let (port, socket) = bound(PortUse::Shared, Role::Foreign(None)).await;
    assert!(reuses_address(&socket));
    let same = squatter(LOCAL, port, true, false);
    assert_eq!(same.is_ok(), SHARING, "{:?}", same.err());
    let second = Bip6Socket::bind(LOCAL, port, PortUse::Shared, Role::Foreign(None)).await;
    assert_eq!(second.is_ok(), SHARING, "{:?}", second.err());
}

/// The transport's setter reaches the bind, in normal and in foreign mode:
/// two transports on one requested port start only when both share it, and
/// only where the OS shares.
#[tokio::test]
async fn a_second_transport_on_a_requested_port_starts_only_when_both_share() {
    // A foreign device's BBMD: it only has to exist.
    let bbmd = std::net::UdpSocket::bind((LOCAL, 0)).unwrap();
    let bbmd = (LOCAL, bbmd.local_addr().unwrap().port());
    for (foreign, place) in [
        (false, "UDP port {port} is"),
        (true, "UDP port {port} on ::1 is"),
    ] {
        rerun_on_lost_port(async || {
            let port = free_port();
            let place = place.replace("{port}", &port.to_string());
            let transport = |instance, share| {
                let mut transport = Bip6Transport::new(LOCAL, port, Some(instance));
                if foreign {
                    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
                        bbmd: bbmd.into(),
                        ttl: 300,
                    });
                }
                transport.set_share_wildcard_port(share);
                transport
            };
            let mut first = transport(10, false);
            let _rx = first.start().await.map_err(bind_error)?;
            let Err(Error::Transport(refused)) = transport(11, true).start().await else {
                panic!("a second transport started on a claimed port");
            };
            assert!(lost_to_another_socket(&refused), "{refused}");
            let shown = refused.to_string();
            assert!(shown.contains("can't be shared"), "{shown}");
            assert!(shown.contains(&place), "{shown}");
            first.stop().await.unwrap();

            let mut first = transport(10, true);
            let _rx = first.start().await.map_err(bind_error)?;
            let mut second = transport(11, true);
            let started = second.start().await;
            assert_eq!(started.is_ok(), SHARING, "{:?}", started.err());
            if SHARING {
                second.stop().await.unwrap();
            }
            let Err(Error::Transport(refused)) = transport(12, false).start().await else {
                panic!("a claiming transport started beside a shared one");
            };
            let shown = refused.to_string();
            assert!(shown.contains("share_wildcard_port"), "{shown}");
            assert!(shown.contains(&place), "{shown}");
            first.stop().await.unwrap();
            Ok(())
        })
        .await;
    }
}

/// A normal node and a foreign device on one port of one host, which both
/// set `SO_REUSEADDR` and both started before #2385, in either order: `[::]`
/// and a specific address. Claiming, the later one is refused on every OS.
/// macOS and the BSDs let the later one in once it alone shares (their
/// specific-address gap); Linux wants both to share.
#[cfg(unix)]
#[tokio::test]
async fn a_normal_node_and_a_foreign_device_on_one_port_need_the_opt_in() {
    let bbmd = std::net::UdpSocket::bind((LOCAL, 0)).unwrap();
    let bbmd = (LOCAL, bbmd.local_addr().unwrap().port());
    for foreign_first in [false, true] {
        rerun_on_lost_port(async || {
            let port = free_port();
            // `later` is the second to start: the foreign device, or the
            // normal node when the foreign device goes first.
            let transport = |later: bool, share| {
                let foreign = later != foreign_first;
                let instance = if foreign { 11 } else { 10 };
                let mut transport = Bip6Transport::new(LOCAL, port, Some(instance));
                if foreign {
                    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
                        bbmd: bbmd.into(),
                        ttl: 300,
                    });
                }
                transport.set_share_wildcard_port(share);
                transport
            };
            for (first_shares, later_shares, starts) in [
                (false, false, false),
                (false, true, SPECIFIC_ADDRESS_GAP),
                (true, true, true),
            ] {
                let mut first = transport(false, first_shares);
                let _rx = first.start().await.map_err(bind_error)?;
                let mut later = transport(true, later_shares);
                let started = later.start().await;
                assert_eq!(
                    started.is_ok(),
                    starts,
                    "foreign first: {foreign_first}, shares: {first_shares} {later_shares}: {:?}",
                    started.err()
                );
                if starts {
                    later.stop().await.unwrap();
                }
                first.stop().await.unwrap();
            }
            Ok(())
        })
        .await;
    }
}

/// A claimed B/IP transport and a claimed B/IPv6 one take the same port
/// number on one host, as a router between the two does on 47808, whichever
/// starts first: the B/IPv6 socket is IPv6 only, so the two never meet.
#[tokio::test]
async fn a_claimed_bip_and_a_claimed_bip6_transport_take_one_port_number() {
    use crate::bip::BipTransport;
    use std::net::Ipv4Addr;

    /// A port number free on IPv4 and IPv6 alike.
    fn free_on_both() -> u16 {
        loop {
            let port = free_port();
            if std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).is_ok() {
                return port;
            }
        }
    }
    for bip_first in [true, false] {
        rerun_on_lost_port(async || {
            let port = free_on_both();
            let mut bip = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::BROADCAST);
            let mut bip6 = Bip6Transport::new(LOCAL, port, Some(10));
            let (_rx, _rx6) = if bip_first {
                let rx = bip.start().await.map_err(bind_error)?;
                (rx, bip6.start().await.map_err(bind_error)?)
            } else {
                let rx6 = bip6.start().await.map_err(bind_error)?;
                (bip.start().await.map_err(bind_error)?, rx6)
            };
            // A B/IP MAC is the address and then the port.
            assert_eq!(bip.local_mac()[4..], port.to_be_bytes());
            assert_eq!(bip6.local_address().map(|bound| bound.port()), Some(port));
            bip.stop().await.unwrap();
            bip6.stop().await.unwrap();
            Ok(())
        })
        .await;
    }
}

/// A normal node has one socket, on `[::]`, which joins the BACnet groups
/// itself. Claimed, it takes what it took before: a group's datagrams, its
/// own broadcast coming back, and its unicast.
///
/// The test multicasts to FF02::BAC0 on the loopback interface. macOS loops
/// that back; a Linux loopback without the multicast flag does not, and
/// there the isolated-link fixture
/// (`tests/ipv6_selected_link/claim.rs`) carries the evidence. On macOS the
/// test fails if the group isn't reachable; elsewhere it skips.
#[cfg(unix)]
#[tokio::test]
async fn a_claimed_port_still_takes_its_groups_and_its_own_echo() {
    use crate::bip6::BACNET_IPV6_MULTICAST_LINK_LOCAL as GROUP;
    use crate::bip6::Bip6BroadcastScope;
    use std::time::Duration;

    const EVIDENCE_OS: bool = cfg!(target_os = "macos");
    const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];
    const DEADLINE: Duration = Duration::from_secs(5);
    #[allow(clippy::print_stderr)] // a skipped setup is noted on the console
    fn skip(why: &str) {
        if EVIDENCE_OS {
            panic!("macOS loops FF02::BAC0 back on lo0, and this test needs it: {why}");
        }
        eprintln!("skipping: {why}");
    }
    let index = ["lo0", "lo"]
        .into_iter()
        .find_map(|name| nix::net::if_::if_nametoindex(name).ok());
    let Some(index) = index else {
        return skip("no loopback interface index");
    };

    rerun_on_lost_port(async || {
        let port = free_port();
        let mut transport = Bip6Transport::new(LOCAL, port, Some(10));
        transport.set_broadcast_scope(Bip6BroadcastScope::LinkLocal);
        let mut rx = transport.start().await.map_err(bind_error)?;
        assert!(!reuses_address(transport.socket.as_ref().unwrap()));
        let own = transport.local_address().unwrap();

        // An Original-Broadcast-NPDU from VMAC 00 04 D2, sent to the group.
        let peer = tokio::net::UdpSocket::bind((LOCAL, 0)).await.unwrap();
        socket2::SockRef::from(&peer)
            .set_multicast_if_v6(index)
            .unwrap();
        let mut broadcast = vec![0x82, 0x02, 0x00, 0x0B, 0x00, 0x04, 0xD2];
        broadcast.extend_from_slice(NPDU);
        let group = SocketAddrV6::new(GROUP, port, 0, index);
        if let Err(error) = peer.send_to(&broadcast, group).await {
            skip(&format!("cannot send to the group: {error}"));
            return transport.stop().await.map_err(io::Error::other);
        }
        let Ok(heard) = tokio::time::timeout(DEADLINE, rx.recv()).await else {
            skip("the group's datagram never reached the claimed socket");
            return transport.stop().await.map_err(io::Error::other);
        };
        let heard = heard.expect("a running transport");
        assert_eq!(heard.npdu.as_ref(), NPDU);
        assert!(heard.link_layer_group);
        assert_eq!(heard.source_mac.as_slice(), [0x00, 0x04, 0xD2]);

        // Its own broadcast comes back on the same socket, and is dropped
        // as its own.
        assert_eq!(transport.receive_counters().own_echo, 0);
        transport.send_broadcast(NPDU).await.unwrap();
        let echoed = async {
            while transport.receive_counters().own_echo == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        };
        tokio::time::timeout(DEADLINE, echoed)
            .await
            .expect("the node's own broadcast came back");

        // Unicast to the node arrives as before.
        let mut unicast = vec![0x82, 0x01, 0x00, 0x0E, 0x00, 0x04, 0xD2, 0x00, 0x00, 0x0A];
        unicast.extend_from_slice(NPDU);
        peer.send_to(&unicast, own).await.unwrap();
        let directed = tokio::time::timeout(DEADLINE, rx.recv())
            .await
            .expect("a unicast to the claimed port")
            .expect("a running transport");
        assert_eq!(directed.npdu.as_ref(), NPDU);
        assert!(!directed.link_layer_group);
        transport.stop().await.map_err(io::Error::other)
    })
    .await;
}
