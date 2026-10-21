//! B/IP-M nodes (Annex J.8, #941): the group's configuration, what a member
//! takes from where, two members on one host exchanging Who-Is and I-Am
//! through their group, and a group changed at run time through the
//! Network Port's live view.
// The loopback tests run on Linux and macOS only; elsewhere their helpers
// go unused.
#![cfg_attr(
    not(any(target_os = "linux", target_vendor = "apple")),
    allow(dead_code, unused_imports)
)]

use super::ingress::{handle_datagram, Arrival as SocketArrival, IngressAddresses};
use super::*;
use crate::udp_metadata::ReceivedDatagram;
use bacnet_types::enums::ErrorCode;
use tokio::time::timeout;

/// A global Who-Is.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
const WHO_IS: &[u8] = &[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];
/// A local I-Am for Device 7.
const I_AM: &[u8] = &[
    0x01, 0x00, 0x10, 0x00, 0xC4, 0x02, 0x00, 0x00, 0x07, 0x22, 0x05, 0xC4, 0x91, 0x00, 0x21, 0x0F,
];
const LOCAL: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 10);
const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 94, 1);

fn group(port: u16) -> BipMulticast {
    BipMulticast::new(GROUP).unwrap().with_port(port).unwrap()
}

fn error_code(error: Error) -> ErrorCode {
    match error {
        Error::Protocol { code, .. } => code,
        other => panic!("expected a protocol error, got {other:?}"),
    }
}

#[test]
fn a_group_must_be_class_d_with_a_ttl_and_a_port() {
    for ip in [[224, 0, 1, 0], [239, 255, 255, 255], [239, 255, 94, 1]] {
        let ip = Ipv4Addr::from(ip);
        assert_eq!(BipMulticast::new(ip).unwrap().group(), ip);
    }
    for ip in [
        [10, 0, 0, 1],
        [255, 255, 255, 255],
        [0, 0, 0, 0],
        [240, 0, 0, 1],
    ] {
        let error = BipMulticast::new(Ipv4Addr::from(ip)).unwrap_err();
        assert!(error.to_string().contains("class D"), "{error}");
    }
    // The local network control block is refused.
    for ip in [[224, 0, 0, 0], [224, 0, 0, 1], [224, 0, 0, 255]] {
        let error = BipMulticast::new(Ipv4Addr::from(ip)).unwrap_err();
        assert!(error.to_string().contains("224.0.0.0/24"), "{error}");
    }
    let multicast = BipMulticast::new(GROUP).unwrap();
    assert_eq!(
        (multicast.ttl(), multicast.port()),
        (BipMulticast::DEFAULT_TTL, None)
    );
    assert_eq!(BipMulticast::DEFAULT_TTL, 1);
    assert!(multicast.with_ttl(0).is_err());
    assert_eq!(multicast.with_ttl(1).unwrap().ttl(), 1);
    assert_eq!(multicast.with_ttl(255).unwrap().ttl(), 255);
    assert!(multicast.with_port(0).is_err());
    assert_eq!(multicast.with_port(47809).unwrap().port(), Some(47809));
}

#[test]
fn multicast_needs_an_explicit_interface_and_a_port() {
    let multicast = BipMulticast::new(GROUP).unwrap();
    let mut wildcard = BipTransport::new(Ipv4Addr::UNSPECIFIED, 47808, Ipv4Addr::BROADCAST);
    let error = wildcard.set_multicast(Some(multicast)).unwrap_err();
    assert!(error.to_string().contains("explicit interface"), "{error}");
    assert_eq!(wildcard.multicast(), None);

    let mut picked = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let error = picked.set_multicast(Some(multicast)).unwrap_err();
    assert!(error.to_string().contains("needs a port"), "{error}");
    let own_port = multicast.with_port(47809).unwrap();
    if cfg!(windows) {
        // The primary socket joins the group there, so the group's port is
        // the transport's: a picked port can't have its own group port.
        let error = picked.set_multicast(Some(own_port)).unwrap_err();
        assert!(error.to_string().contains("transport's own"), "{error}");
    } else {
        picked.set_multicast(Some(own_port)).unwrap();
        assert_eq!(picked.multicast().and_then(|m| m.port()), Some(47809));
    }
    // A group port equal to the transport's is taken everywhere.
    let mut same = BipTransport::new(Ipv4Addr::LOCALHOST, 47809, Ipv4Addr::LOCALHOST);
    same.set_multicast(Some(own_port)).unwrap();

    let mut explicit = BipTransport::new(Ipv4Addr::LOCALHOST, 47808, Ipv4Addr::LOCALHOST);
    explicit.set_multicast(Some(multicast)).unwrap();
    assert_eq!(explicit.multicast(), Some(multicast));
    assert_eq!(
        explicit.bip_broadcast_endpoint(),
        Some(SocketAddrV4::new(GROUP, 47808))
    );
    assert!(explicit.is_broadcast_mac(&encode_bip_mac(GROUP.octets(), 47808)));
    assert!(!explicit.is_broadcast_mac(&encode_bip_mac([127, 0, 0, 1], 47808)));
    explicit.set_multicast(None).unwrap();
    assert_eq!(explicit.multicast(), None);
}

#[tokio::test]
async fn a_member_cant_be_a_foreign_device_or_share_its_port_by_address() {
    // The transports take the group's port, which Windows requires; each
    // start is refused before it binds anything. A BBMD may serve a group
    // (multicast_bbmd_tests.rs).
    let multicast = group(47809);
    let mut foreign = BipTransport::new(Ipv4Addr::LOCALHOST, 47809, Ipv4Addr::LOCALHOST);
    foreign.set_multicast(Some(multicast)).unwrap();
    foreign.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::new(192, 0, 2, 1),
        bbmd_port: 47808,
        ttl: 60,
        renewal_interval: None,
    });
    let error = foreign.start().await.unwrap_err();
    assert!(error.to_string().contains("foreign device"), "{error}");

    let mut shared = BipTransport::new(Ipv4Addr::LOCALHOST, 47809, Ipv4Addr::LOCALHOST);
    shared.set_multicast(Some(multicast)).unwrap();
    shared.set_share_port_by_address(true);
    let error = shared.start().await.unwrap_err();
    assert!(error.to_string().contains("by address"), "{error}");
}

/// Feed `frame` to a member of [`GROUP`] at [`LOCAL`] from a peer, as
/// though the OS reported it sent to `destination`.
async fn member_receives(destination: Ipv4Addr, function: BvlcFunction) -> (bool, ReceiveCounters) {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let (npdu_tx, mut npdu_rx) = mpsc::channel(8);
    let ctx = RecvContext::for_test(
        Arc::new(BipSocket::new(socket, None)),
        npdu_tx,
        SocketAddrV4::new(LOCAL, 0xBAC0),
        SocketAddrV4::new(GROUP, 0xBAC0),
        None,
    );
    let frame = raw_frame(function, I_AM);
    let received = ReceivedDatagram {
        len: frame.len(),
        peer: SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 30), 0xBAC0).into(),
        destination: destination.into(),
        arrival_index: None,
        os_group_delivery: None,
        arrival_stamp: None,
    };
    let local = IngressAddresses {
        unicast_ips: vec![LOCAL],
        wildcard_bind: false,
        listener_interface: None,
        interface_mismatch_seen: Default::default(),
    };
    handle_datagram(&frame, &received, SocketArrival::Primary, &local, &ctx).await;
    let delivered = npdu_rx.try_recv().ok();
    if let Some(npdu) = &delivered {
        assert_eq!(&npdu.npdu[..], I_AM);
        assert_eq!(
            npdu.link_layer_group,
            function != BvlcFunction::ORIGINAL_UNICAST_NPDU
        );
    }
    (delivered.is_some(), ctx.drops.snapshot())
}

#[tokio::test]
async fn a_member_takes_broadcasts_from_its_group_only() {
    let broadcast = BvlcFunction::ORIGINAL_BROADCAST_NPDU;
    assert!(member_receives(GROUP, broadcast).await.0);
    for subnet in [Ipv4Addr::BROADCAST, Ipv4Addr::new(192, 0, 2, 255)] {
        let (delivered, counters) = member_receives(subnet, broadcast).await;
        assert!(!delivered, "{subnet}");
        assert_eq!(counters.destination_mismatch, 1, "{subnet}");
    }
    let other_group = Ipv4Addr::new(239, 255, 94, 2);
    assert!(!member_receives(other_group, broadcast).await.0);
    // A unicast function sent to the group is refused, and unicast is
    // unchanged.
    let unicast = BvlcFunction::ORIGINAL_UNICAST_NPDU;
    let (delivered, counters) = member_receives(GROUP, unicast).await;
    assert!(!delivered);
    assert_eq!(counters.destination_mismatch, 1);
    assert!(member_receives(LOCAL, unicast).await.0);
}

/// A UDP port no socket holds now, for a group.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
fn free_port() -> u16 {
    std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[cfg(any(target_os = "linux", target_vendor = "apple"))]
struct Member {
    transport: BipTransport,
    rx: mpsc::Receiver<ReceivedNpdu>,
}

#[cfg(any(target_os = "linux", target_vendor = "apple"))]
impl Member {
    /// A member on 127.0.0.1 at a port the OS picks, in `multicast`'s group.
    async fn start(multicast: BipMulticast) -> Self {
        let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
        transport.set_multicast(Some(multicast)).unwrap();
        let rx = transport.start().await.unwrap();
        Self { transport, rx }
    }

    async fn next(&mut self) -> ReceivedNpdu {
        timeout(Duration::from_secs(2), self.rx.recv())
            .await
            .expect("timed out waiting for an NPDU")
            .expect("receive channel closed")
    }

    /// Nothing more arrives for a while.
    async fn hears_nothing(&mut self) {
        let heard = timeout(Duration::from_millis(300), self.rx.recv()).await;
        assert!(heard.is_err(), "unexpected NPDU: {heard:?}");
    }

    /// Who-Is from `self` through the group, then I-Am back from `peer`.
    async fn exchange(&mut self, peer: &mut Member) {
        self.transport.send_broadcast(WHO_IS).await.unwrap();
        let who_is = peer.next().await;
        assert_eq!(&who_is.npdu[..], WHO_IS);
        assert_eq!(&who_is.source_mac[..], self.transport.local_mac());
        assert!(who_is.link_layer_group);
        peer.transport.send_broadcast(I_AM).await.unwrap();
        let i_am = self.next().await;
        assert_eq!(&i_am.npdu[..], I_AM);
        assert_eq!(&i_am.source_mac[..], peer.transport.local_mac());
        assert!(i_am.link_layer_group);
        // Neither took its own looped-back copy.
        self.hears_nothing().await;
        peer.hears_nothing().await;
    }
}

// Loopback multicast with loop on works on Linux and macOS; Windows
// multicast is untested (multicast.rs).
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
#[tokio::test]
async fn two_members_on_one_host_exchange_who_is_and_i_am_through_the_group() {
    let multicast = group(free_port());
    let mut a = Member::start(multicast).await;
    let mut b = Member::start(multicast).await;
    assert_ne!(a.transport.local_mac(), b.transport.local_mac());
    let endpoint = SocketAddrV4::new(GROUP, multicast.port().unwrap());
    assert_eq!(a.transport.bip_broadcast_endpoint(), Some(endpoint));

    a.exchange(&mut b).await;
    for member in [&a, &b] {
        // Each dropped its own copy of the broadcast it sent.
        assert_eq!(member.transport.receive_counters().own_echo, 1);
    }
    a.transport.stop().await.unwrap();
    b.transport.stop().await.unwrap();
}

#[cfg(any(target_os = "linux", target_vendor = "apple"))]
#[tokio::test]
async fn an_activated_group_moves_the_members_and_zero_ends_multicast() {
    let port = free_port();
    let first = group(port);
    let mut a = Member::start(first).await;
    let mut b = Member::start(first).await;
    let views: Vec<_> = [&a, &b]
        .iter()
        .map(|member| member.transport.bip_port().unwrap().multicast.unwrap())
        .collect();
    assert_eq!(views[0].group(), GROUP.octets());
    assert!(views[0].can_change());

    // Not a group: refused at check and activation alike.
    let out_of_range = error_code(views[0].check([10, 0, 0, 1]).unwrap_err());
    assert_eq!(out_of_range, ErrorCode::VALUE_OUT_OF_RANGE);
    assert_eq!(
        error_code(views[0].activate([10, 0, 0, 1]).unwrap_err()),
        ErrorCode::VALUE_OUT_OF_RANGE
    );
    // Nor is the local network control block.
    assert_eq!(
        error_code(views[0].check([224, 0, 0, 1]).unwrap_err()),
        ErrorCode::VALUE_OUT_OF_RANGE
    );

    let moved = Ipv4Addr::new(239, 255, 94, 3);
    // A listener that fails to open fails the activation, and nothing
    // changes: the old group stays in use.
    a.transport
        .multicast
        .fail_next_listener
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(matches!(
        views[0].activate(moved.octets()),
        Err(Error::Transport(_))
    ));
    assert_eq!(views[0].group(), GROUP.octets());
    assert_eq!(
        a.transport.bip_broadcast_endpoint(),
        Some(SocketAddrV4::new(GROUP, port))
    );
    a.exchange(&mut b).await;

    for view in &views {
        view.check(moved.octets()).unwrap();
        view.activate(moved.octets()).unwrap();
        assert_eq!(view.group(), moved.octets());
    }
    let endpoint = SocketAddrV4::new(moved, port);
    assert_eq!(a.transport.bip_broadcast_endpoint(), Some(endpoint));
    assert_eq!(a.transport.multicast().map(|m| m.group()), Some(moved));
    a.exchange(&mut b).await;

    // Off: broadcasts go to the configured broadcast address again.
    views[0].activate([0; 4]).unwrap();
    assert_eq!(views[0].group(), [0; 4]);
    assert_eq!(a.transport.multicast(), None);
    let local_port = a.transport.bip_port().unwrap().endpoint.port();
    assert_eq!(
        a.transport.bip_broadcast_endpoint(),
        Some(SocketAddrV4::new(Ipv4Addr::LOCALHOST, local_port))
    );
    b.transport.send_broadcast(I_AM).await.unwrap();
    a.hears_nothing().await;

    a.transport.stop().await.unwrap();
    b.transport.stop().await.unwrap();
    // A stopped transport takes no change.
    assert_eq!(
        error_code(views[1].check(GROUP.octets()).unwrap_err()),
        ErrorCode::WRITE_ACCESS_DENIED
    );
    // A restart keeps the group activated last.
    let rx = b.transport.start().await.unwrap();
    drop(rx);
    assert_eq!(b.transport.multicast().map(|m| m.group()), Some(moved));
    b.transport.stop().await.unwrap();
}

#[test]
fn a_node_with_an_interface_lends_its_group_unless_foreign() {
    let lent = |transport: &BipTransport| transport.bip_port().unwrap().multicast.is_some();
    let explicit = BipTransport::new(Ipv4Addr::LOCALHOST, 47808, Ipv4Addr::LOCALHOST);
    assert!(lent(&explicit), "off, but able");
    let wildcard = BipTransport::new(Ipv4Addr::UNSPECIFIED, 47808, Ipv4Addr::BROADCAST);
    assert!(!lent(&wildcard));
    let picked = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    assert!(!lent(&picked), "no port for a group");
    let mut shared = BipTransport::new(Ipv4Addr::LOCALHOST, 47808, Ipv4Addr::LOCALHOST);
    shared.set_share_port_by_address(true);
    assert!(!lent(&shared));
    let mut foreign = BipTransport::new(Ipv4Addr::LOCALHOST, 47808, Ipv4Addr::LOCALHOST);
    foreign.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::new(192, 0, 2, 1),
        bbmd_port: 47808,
        ttl: 60,
        renewal_interval: None,
    });
    assert!(!lent(&foreign));
}

#[test]
fn a_members_configured_broadcast_stays_a_group_destination() {
    // At the group's port, as Windows requires.
    let mut member = BipTransport::new(
        Ipv4Addr::new(192, 0, 2, 20),
        47809,
        Ipv4Addr::new(192, 0, 2, 255),
    );
    member.set_multicast(Some(group(47809))).unwrap();
    let mac = |ip: [u8; 4], port| encode_bip_mac(ip, port);
    // The group, any multicast address, and the subnet's directed
    // broadcast at any port reach more than one node.
    for (ip, port) in [
        (GROUP.octets(), 47809),
        ([239, 1, 2, 3], 47808),
        ([192, 0, 2, 255], 47808),
        ([192, 0, 2, 255], 47810),
    ] {
        assert!(member.is_group_destination(&mac(ip, port)), "{ip:?}:{port}");
    }
    assert!(!member.is_group_destination(&mac([192, 0, 2, 30], 47808)));
    // Only the group is this link's broadcast.
    assert!(member.is_broadcast_mac(&mac(GROUP.octets(), 47809)));
    assert!(!member.is_broadcast_mac(&mac([192, 0, 2, 255], 47809)));
}
