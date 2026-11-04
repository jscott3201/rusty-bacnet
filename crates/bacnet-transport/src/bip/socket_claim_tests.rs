//! A requested B/IP port is claimed unless sharing is asked for (#2366):
//! which sockets can bind it beside the transport on each OS, the error a
//! transport that finds it held starts with, and a B/IP-M node whose group
//! sits on its own port.

use super::*;
use crate::port_retry::{ATTEMPTS, bind_error, lost_to_another_socket, rerun_on_lost_port};
use socket::{BindPlan, BoundSockets, SocketRole, udp_socket, wildcard_role};
use std::io;

/// Whether this OS lets a socket that sets `SO_REUSEADDR` bind a specific
/// address beside a claimed wildcard socket (`port_ownership`).
const SPECIFIC_ADDRESS_GAP: bool = cfg!(all(
    unix,
    not(any(target_os = "linux", target_os = "android"))
));

/// Whether two wildcard sockets that both set `SO_REUSEADDR` share a port:
/// macOS and the BSDs also want `SO_REUSEPORT` on both.
const WILDCARD_SHARING: bool = cfg!(any(windows, target_os = "linux", target_os = "android"));

/// A UDP port no socket holds now. Another process can take it before the
/// test binds it, so its users rerun on a lost port.
fn free_port() -> u16 {
    std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Another program's socket on `ip:port`, with `SO_REUSEADDR` and, when
/// asked, `SO_REUSEPORT`.
fn squatter(ip: Ipv4Addr, port: u16, reuse_port: bool) -> io::Result<socket2::Socket> {
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None)?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    if reuse_port {
        socket.set_reuse_port(true)?;
    }
    #[cfg(not(unix))]
    let _ = reuse_port;
    socket.bind(&SocketAddrV4::new(ip, port).into())?;
    Ok(socket)
}

/// `socket::bind` for the wildcard plan `adjust` makes on a fresh requested
/// port, then `check` with the port and what was bound.
fn on_a_requested_port(adjust: impl Fn(&mut BindPlan), check: impl Fn(u16, &BoundSockets)) {
    for attempt in 1..=ATTEMPTS {
        let port = free_port();
        let mut plan = plan(Ipv4Addr::UNSPECIFIED, port, false);
        adjust(&mut plan);
        match socket::bind(plan) {
            Ok(bound) => return check(port, &bound),
            Err(e) if attempt < ATTEMPTS && lost_to_another_socket(&e) => continue,
            Err(e) => panic!("bind 0.0.0.0:{port}: {e}"),
        }
    }
}

/// A group at the node's own port.
fn own_port_group() -> BipMulticast {
    BipMulticast::new(Ipv4Addr::new(239, 255, 94, 3)).unwrap()
}

#[test]
fn the_wildcard_role_follows_the_port_the_opt_in_and_the_group() {
    let role = |port, share, group: Option<BipMulticast>| {
        let mut plan = plan(Ipv4Addr::UNSPECIFIED, port, false);
        plan.share_wildcard = share;
        plan.multicast = group;
        wildcard_role(&plan)
    };
    let own = Some(own_port_group());
    let apart = Some(own_port_group().with_port(47809).unwrap());
    let at_47808 = Some(own_port_group().with_port(47808).unwrap());
    // A port the OS picks is never shared (#892), whatever is asked.
    for (share, group) in [(false, None), (true, None), (false, own), (true, own)] {
        assert_eq!(role(0, share, group), SocketRole::ClaimedWildcard);
    }
    assert_eq!(role(47808, false, None), SocketRole::ClaimedWildcard);
    assert_eq!(role(47808, false, apart), SocketRole::ClaimedWildcard);
    assert_eq!(role(47808, false, own), SocketRole::GroupWildcard);
    assert_eq!(role(47808, false, at_47808), SocketRole::GroupWildcard);
    for group in [None, own, apart] {
        assert_eq!(role(47808, true, group), SocketRole::SharedWildcard);
    }
}

#[test]
fn a_requested_port_refuses_a_second_bind_by_default() {
    on_a_requested_port(
        |_| {},
        |port, bound| {
            assert!(!bound.primary.reuse_address().unwrap());
            // Another wildcard socket asking to share: refused everywhere.
            let refused = squatter(Ipv4Addr::UNSPECIFIED, port, false).unwrap_err();
            assert!(lost_to_another_socket(&refused), "{refused}");
            // A specific address asking to share: refused but on macOS and
            // the BSDs, which have no option to stop it.
            let specific = squatter(Ipv4Addr::LOCALHOST, port, false);
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
        },
    );
}

#[test]
fn a_requested_port_shared_on_request_takes_a_second_sharing_bind() {
    on_a_requested_port(
        |plan| plan.share_wildcard = true,
        |port, bound| {
            assert!(bound.primary.reuse_address().unwrap());
            let mut shared = plan(Ipv4Addr::UNSPECIFIED, port, false);
            shared.share_wildcard = true;
            let second = socket::bind(shared);
            assert_eq!(second.is_ok(), WILDCARD_SHARING, "{:?}", second.err());
            if WILDCARD_SHARING {
                squatter(Ipv4Addr::UNSPECIFIED, port, false).unwrap();
            }
            // A specific address asking to share binds beside it everywhere.
            squatter(Ipv4Addr::LOCALHOST, port, false).unwrap();
        },
    );
}

/// The node's group listener binds its own port beside the primary socket.
/// On Linux both set `SO_REUSEPORT` for it, so a socket that only sets
/// `SO_REUSEADDR` is still refused.
#[cfg(not(windows))]
#[test]
fn a_group_on_the_nodes_own_port_binds_beside_its_primary() {
    on_a_requested_port(
        |plan| plan.multicast = Some(own_port_group()),
        |port, bound| {
            assert!(!bound.primary.reuse_address().unwrap());
            let linux = cfg!(any(target_os = "linux", target_os = "android"));
            assert_eq!(bound.primary.reuse_port().unwrap(), linux);
            let listener = udp_socket(SocketRole::GroupListener).unwrap();
            let group = SocketAddrV4::new(Ipv4Addr::new(239, 255, 94, 3), port);
            listener.bind(&group.into()).unwrap();
            assert_eq!(address_of(&listener), group);
            assert!(squatter(Ipv4Addr::UNSPECIFIED, port, false).is_err());
            let specific = squatter(Ipv4Addr::LOCALHOST, port, false);
            assert_eq!(
                specific.is_ok(),
                SPECIFIC_ADDRESS_GAP,
                "{:?}",
                specific.err()
            );
            // What Linux can't refuse: a socket of the same user that sets
            // `SO_REUSEPORT` too.
            let same_user = squatter(Ipv4Addr::UNSPECIFIED, port, true);
            assert_eq!(same_user.is_ok(), linux, "{:?}", same_user.err());
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

/// On Linux a node that started without a group claims its port, so a group
/// activated later on that port can't bind its listener: the activation
/// says why and changes nothing. Sharing the port lets it in.
#[cfg(any(target_os = "linux", target_os = "android"))]
#[tokio::test]
async fn on_linux_a_group_activated_on_a_claimed_port_needs_it_shared() {
    rerun_on_lost_port(async || {
        let port = free_port();
        let group = own_port_group().group().octets();
        for share in [false, true] {
            let mut node = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::LOCALHOST);
            node.set_share_wildcard_port(share);
            let _rx = node.start().await.map_err(bind_error)?;
            let view = node.bip_port().unwrap().multicast.unwrap();
            let activated = view.activate(group);
            if share {
                activated.map_err(bind_error)?;
                assert_eq!(node.multicast(), Some(own_port_group()));
            } else {
                let Err(Error::Transport(refused)) = activated else {
                    panic!("a group took a claimed port: {activated:?}");
                };
                assert!(lost_to_another_socket(&refused), "{refused}");
                let shown = refused.to_string();
                assert!(shown.contains("share the port"), "{shown}");
                assert_eq!(node.multicast(), None);
            }
            node.stop().await.unwrap();
        }
        Ok(())
    })
    .await;
}
