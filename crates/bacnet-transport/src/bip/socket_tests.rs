//! Socket options the B/IP transport sets at start.

use super::*;
use crate::local_addresses::LocalInterface;
use crate::port_ownership::{restart, ATTEMPTS};
use socket::{udp_socket, BindPlan, BoundSockets, SocketRole};

#[tokio::test]
async fn socket_is_broadcast_capable_and_binds_inaddr_any() {
    // Regression for the "user-supplied interface IP" silently rejecting
    // broadcast traffic. Unless the port is shared by address (#1538), even
    // when the caller passes a specific interface, the one socket binds
    // 0.0.0.0 so the kernel delivers subnet- and limited-broadcast packets to
    // it. The interface IP is still used for the announced local MAC.
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();

    let local = transport
        .socket
        .as_ref()
        .expect("socket exists after start")
        .local_addr()
        .expect("local_addr is queryable");

    assert!(
        local.ip().is_unspecified(),
        "BIP socket must bind to 0.0.0.0 for broadcast reception; got {local}"
    );
    assert!(
        socket2::SockRef::from(
            &**transport
                .socket
                .as_ref()
                .expect("socket exists after start")
                .as_ref()
        )
        .broadcast()
        .expect("SO_BROADCAST is queryable"),
        "BIP socket must enable SO_BROADCAST for Original-Broadcast-NPDU sends"
    );

    // The announced local MAC must still reflect the user-supplied interface,
    // not the bind address.
    let mac = transport.local_mac();
    assert_eq!(
        &mac[..4],
        &Ipv4Addr::LOCALHOST.octets(),
        "announced IP must match interface"
    );

    transport.stop().await.unwrap();
}

fn reuses_address(transport: &BipTransport) -> bool {
    let socket = transport
        .socket
        .as_ref()
        .expect("socket exists after start");
    socket2::SockRef::from(&***socket)
        .reuse_address()
        .expect("SO_REUSEADDR is queryable")
}

#[test]
fn only_an_explicitly_requested_port_opts_into_address_reuse() {
    // Linux can hand two SO_REUSEADDR sockets the same ephemeral port, and
    // unicast to it then reaches only one of them (#892).
    let reuses = |role| udp_socket(role).unwrap().reuse_address().unwrap();
    assert!(!reuses(SocketRole::PrivateWildcard));
    assert!(reuses(SocketRole::SharedWildcard));
    // In per-address mode (#1538) only macOS and the BSDs share the address,
    // to bind beside their wildcard listener; Linux needs no listener there,
    // and Windows claims the address.
    let macos_or_bsd = cfg!(all(
        unix,
        not(any(target_os = "linux", target_os = "android"))
    ));
    assert_eq!(reuses(SocketRole::Address), macos_or_bsd);
    assert!(reuses(SocketRole::BroadcastListener));
    assert!(!BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST).share_port);
    assert!(BipTransport::new(Ipv4Addr::LOCALHOST, 0xBAC0, Ipv4Addr::BROADCAST).share_port);
}

/// Where `bind` put a socket.
fn address_of(socket: &socket2::Socket) -> SocketAddrV4 {
    socket.local_addr().unwrap().as_socket_ipv4().unwrap()
}

/// The plan for `interface` on an explicitly requested `port`, with the
/// loopback subnet's broadcast address.
fn plan(interface: Ipv4Addr, port: u16, by_address: bool) -> BindPlan {
    BindPlan {
        interface,
        port,
        share_port: port != 0,
        by_address,
        broadcast: Ipv4Addr::new(127, 255, 255, 255),
        local: None,
    }
}

/// `socket::bind` for an explicitly requested port, which the OS picks here
/// and the probe releases first. Another process can take it in between
/// (#1032), so each run of `check` gets a fresh one.
fn on_a_requested_port(interface: Ipv4Addr, by_address: bool, check: impl Fn(u16, BoundSockets)) {
    for attempt in 1..=ATTEMPTS {
        let port = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        match socket::bind(plan(interface, port, by_address)) {
            Ok(bound) => return check(port, bound),
            Err(e) if crate::port_ownership::lost_port(attempt, &e) => continue,
            Err(e) => panic!("bind {interface}:{port}: {e}"),
        }
    }
}

#[test]
fn an_explicit_interface_keeps_one_wildcard_socket_by_default() {
    // Its single socket receives unicast and broadcasts in arrival order.
    on_a_requested_port(Ipv4Addr::LOCALHOST, false, |port, bound| {
        assert_eq!(
            address_of(&bound.primary),
            SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)
        );
        assert!(bound.primary.reuse_address().unwrap());
        assert!(bound.listeners.is_empty());
    });
}

#[test]
fn sharing_the_port_by_address_binds_the_interface_address() {
    on_a_requested_port(Ipv4Addr::LOCALHOST, true, |port, bound| {
        assert_eq!(
            address_of(&bound.primary),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
        );
        // Windows hands a socket bound to the interface address the
        // broadcasts that arrive there. Linux listens on the broadcast
        // addresses themselves; macOS and the BSDs on the wildcard address.
        let listening: Vec<_> = bound.listeners.iter().map(address_of).collect();
        let expected: Vec<Ipv4Addr> = if cfg!(windows) {
            vec![]
        } else if cfg!(any(target_os = "linux", target_os = "android")) {
            vec![Ipv4Addr::new(127, 255, 255, 255), Ipv4Addr::BROADCAST]
        } else {
            vec![Ipv4Addr::UNSPECIFIED]
        };
        let expected: Vec<_> = expected
            .into_iter()
            .map(|ip| SocketAddrV4::new(ip, port))
            .collect();
        assert_eq!(listening, expected);
        for listener in &bound.listeners {
            assert!(listener.reuse_address().unwrap());
            assert!(listener.broadcast().unwrap());
            // Several wildcard listeners on one port need it there.
            #[cfg(any(
                target_vendor = "apple",
                target_os = "freebsd",
                target_os = "dragonfly",
                target_os = "openbsd",
                target_os = "netbsd",
            ))]
            assert!(listener.reuse_port().unwrap());
        }
    });
}

#[test]
fn sharing_the_port_by_address_needs_an_address_and_a_port() {
    for plan in [
        plan(Ipv4Addr::UNSPECIFIED, 0xBAC0, true),
        plan(Ipv4Addr::LOCALHOST, 0, true),
    ] {
        let refused = socket::bind(plan).err().expect("refused");
        assert_eq!(refused.kind(), std::io::ErrorKind::InvalidInput, "{plan:?}");
    }
}

#[test]
fn a_wildcard_or_a_private_port_keeps_one_wildcard_socket() {
    on_a_requested_port(Ipv4Addr::UNSPECIFIED, false, |port, bound| {
        assert_eq!(
            address_of(&bound.primary),
            SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)
        );
        assert!(bound.primary.reuse_address().unwrap());
        assert!(bound.listeners.is_empty());
    });
    // An ephemeral port is never shared (#892).
    let bound = socket::bind(plan(Ipv4Addr::LOCALHOST, 0, false)).unwrap();
    assert!(address_of(&bound.primary).ip().is_unspecified());
    assert!(!bound.primary.reuse_address().unwrap());
    assert!(bound.listeners.is_empty());
}

#[tokio::test]
async fn an_ephemeral_port_stays_private_across_restart() {
    // A restart rebinds the remembered actual port, which must not opt the
    // socket into sharing it. Each run starts on a fresh port; see `restart`
    // for why it can lose it.
    for attempt in 1..=ATTEMPTS {
        let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let _rx = transport.start().await.unwrap();
        assert!(!reuses_address(&transport));
        transport.stop().await.unwrap();
        let Some(started) = restart(&mut transport, attempt).await else {
            continue;
        };
        let _rx = started.unwrap();
        assert!(!reuses_address(&transport));
        transport.stop().await.unwrap();
        return;
    }
}

#[tokio::test]
async fn check_bind_refuses_what_start_would() {
    let typo = BipTransport::new(Ipv4Addr::new(192, 0, 2, 1), 0, Ipv4Addr::BROADCAST);
    assert!(typo.check_bind().is_err());
    let mut holder = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = holder.start().await.unwrap();
    // A private port refuses a requested-port bind on every OS, in either
    // mode. macOS lets the address socket beside it bind, but not the
    // broadcast listener.
    for (interface, by_address) in [
        (Ipv4Addr::UNSPECIFIED, false),
        (Ipv4Addr::LOCALHOST, false),
        (Ipv4Addr::LOCALHOST, true),
    ] {
        let mut checked = BipTransport::new(interface, holder.port, Ipv4Addr::BROADCAST);
        checked.set_share_port_by_address(by_address);
        assert!(checked.check_bind().is_err(), "{interface} {by_address}");
    }
    holder.stop().await.unwrap();
}

#[tokio::test]
async fn start_refuses_sharing_by_address_without_an_address() {
    let mut transport = BipTransport::new(Ipv4Addr::UNSPECIFIED, 0xBAC0, Ipv4Addr::BROADCAST);
    transport.set_share_port_by_address(true);
    let Err(Error::Transport(refused)) = transport.start().await else {
        panic!("start must refuse");
    };
    assert_eq!(refused.kind(), std::io::ErrorKind::InvalidInput);
}

/// The broadcast-address rule for sharing a port by address, against a /24.
#[test]
fn sharing_by_address_takes_only_the_subnet_or_limited_broadcast() {
    let interface = Ipv4Addr::new(192, 0, 2, 10);
    let check = |interface, netmask: Option<Ipv4Addr>, broadcast| {
        let mut plan = plan(interface, 0xBAC0, true);
        plan.broadcast = broadcast;
        plan.local = Some(LocalInterface {
            index: Some(2),
            netmask,
            broadcast: true,
        });
        socket::check_broadcast(&plan)
    };
    let on = |netmask, broadcast| check(interface, netmask, broadcast);
    let slash_24 = Some(Ipv4Addr::new(255, 255, 255, 0));
    for broadcast in [Ipv4Addr::new(192, 0, 2, 255), Ipv4Addr::BROADCAST] {
        on(slash_24, broadcast).unwrap();
    }
    // A loopback interface may name itself, as loopback tests do, whatever
    // its netmask; any other interface may not, with or without one.
    let loopback = Ipv4Addr::LOCALHOST;
    check(loopback, Some(Ipv4Addr::new(255, 0, 0, 0)), loopback).unwrap();
    check(loopback, None, loopback).unwrap();
    for netmask in [slash_24, None] {
        let refused = on(netmask, interface).expect_err("its own address");
        assert!(refused.to_string().contains("own address"), "{refused}");
    }
    // Another subnet's broadcast, a host on this one, and any address on a
    // /32 lose this subnet's broadcasts.
    for (netmask, broadcast) in [
        (slash_24, Ipv4Addr::new(198, 51, 100, 255)),
        (slash_24, Ipv4Addr::new(192, 0, 2, 254)),
        (Some(Ipv4Addr::BROADCAST), Ipv4Addr::new(192, 0, 2, 255)),
    ] {
        let refused = on(netmask, broadcast).expect_err("refused");
        assert_eq!(
            refused.kind(),
            std::io::ErrorKind::InvalidInput,
            "{broadcast}"
        );
    }
    // Without a netmask to judge by, the bind decides.
    on(None, Ipv4Addr::new(198, 51, 100, 255)).unwrap();
}

/// On this host: loopback's subnet broadcast is taken, another subnet's is
/// refused before anything binds.
#[test]
fn check_bind_refuses_another_subnets_broadcast() {
    let mut addresses = vec![Ipv4Addr::LOCALHOST];
    // Linux finds 127.0.0.2 on loopback's subnet.
    if cfg!(target_os = "linux") {
        addresses.push(Ipv4Addr::new(127, 0, 0, 2));
    }
    for interface in addresses {
        let Some(_) = crate::local_addresses::interface_of(interface).unwrap() else {
            return eprintln!("skipped: the host reports no interface for {interface}");
        };
        let mut wrong = vec![Ipv4Addr::new(192, 0, 2, 255)];
        if interface != Ipv4Addr::LOCALHOST {
            // A local address binds fine on Linux, and would hear nothing.
            wrong.push(Ipv4Addr::LOCALHOST);
        }
        for broadcast in wrong {
            let mut transport = BipTransport::new(interface, 0xBAC0, broadcast);
            transport.set_share_port_by_address(true);
            let Err(Error::Transport(refused)) = transport.check_bind() else {
                panic!("{interface} with {broadcast} must be refused");
            };
            assert_eq!(refused.kind(), std::io::ErrorKind::InvalidInput);
            assert!(
                refused.to_string().contains("subnet broadcast"),
                "{refused}"
            );
        }
    }
    // Loopback's own subnet broadcast, 127.255.255.255, is taken.
    let local = crate::local_addresses::interface_of(Ipv4Addr::LOCALHOST).unwrap();
    for attempt in 1..=ATTEMPTS {
        let port = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut plan = plan(Ipv4Addr::LOCALHOST, port, true);
        plan.local = local;
        match socket::bind(plan) {
            Ok(_) => return,
            Err(e) if crate::port_ownership::lost_port(attempt, &e) => continue,
            Err(e) => panic!("loopback's subnet broadcast: {e}"),
        }
    }
}
