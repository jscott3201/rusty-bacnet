//! A B/IP-M node (Annex J.8): its multicast group stands in for the
//! subnet's broadcast address, for what it sends and what it takes (#941).

mod node_support;

use core::net::Ipv4Addr;

use bacnet_bip_codec::Bvll;
use bacnet_bip_core::LocalSubnet;
use bacnet_bip_core::node::{
    Addressed, ConfigError, Discard, Link, LocalAddresses, Mode, Node, Received, SendRefusal,
};
use node_support::*;

const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];

/// The node's multicast group, at the network's port.
const GROUP: core::net::SocketAddrV4 = addr(239, 255, 0, 47, PORT);

/// A member of [`GROUP`].
fn member() -> bacnet_bip_core::node::Link {
    with(link(Mode::Normal), |link| {
        link.subnet = LocalSubnet::member(GROUP, Some(*BROADCAST.ip()))
    })
}

/// What a member's host reports for a datagram sent to `ip`: its socket
/// is bound to the node's own address.
fn classify(ip: Ipv4Addr) -> Addressed {
    let link = member();
    let local = LocalAddresses::new(*link.address.ip(), *link.subnet.broadcast.ip());
    Addressed::classify(ip, None, &local)
}

#[test]
fn the_group_is_a_members_only_broadcast_destination() {
    let to_group = classify(*GROUP.ip());
    assert_eq!((to_group.node, to_group.broadcast), (false, true));
    // A subnet broadcast isn't this network's, nor is the limited one.
    for subnet in [Ipv4Addr::BROADCAST, *BROADCAST.ip()] {
        let to = classify(subnet);
        assert_eq!((to.node, to.broadcast), (false, false), "{subnet}");
    }
    // Another group isn't this one.
    let other = classify(Ipv4Addr::new(239, 255, 0, 48));
    assert_eq!((other.node, other.broadcast), (false, false));
    // Unicast is unchanged.
    let own = classify(*OWN.ip());
    assert_eq!((own.node, own.broadcast), (true, false));
    // A host that reports the delivery as a group's agrees.
    let link = member();
    let local = LocalAddresses::new(*link.address.ip(), *link.subnet.broadcast.ip());
    let reported = Addressed::classify(*GROUP.ip(), Some(true), &local);
    assert_eq!((reported.node, reported.broadcast), (false, true));
    // A plain node still takes the limited broadcast.
    let plain = LocalAddresses::new(*OWN.ip(), *BROADCAST.ip());
    assert!(Addressed::classify(Ipv4Addr::BROADCAST, None, &plain).broadcast);
}

#[test]
fn a_member_takes_broadcast_functions_from_the_group_and_unicast_ones_only_by_unicast() {
    let link = member();
    let broadcast = wire(Bvll::OriginalBroadcastNpdu(NPDU));
    let forwarded = wire(Bvll::ForwardedNpdu {
        origin: REMOTE,
        npdu: NPDU,
    });
    let unicast = wire(Bvll::OriginalUnicastNpdu(NPDU));
    let mismatch = Received::Discarded(Discard::DestinationMismatch);
    let to_group = classify(*GROUP.ip());

    let Received::Deliver(delivery) = receive(&link, &broadcast, PEER, to_group) else {
        panic!("an Original-Broadcast-NPDU to the group is delivered");
    };
    assert_eq!((delivery.source, delivery.group), (PEER, true));
    let Received::Deliver(delivery) = receive(&link, &forwarded, BBMD, to_group) else {
        panic!("a Forwarded-NPDU to the group is delivered");
    };
    assert_eq!((delivery.source, delivery.group), (REMOTE, true));
    assert_eq!(receive(&link, &unicast, PEER, to_group), mismatch);

    // The subnet's broadcasts are refused.
    for subnet in [Ipv4Addr::BROADCAST, *BROADCAST.ip()] {
        let to = classify(subnet);
        assert_eq!(receive(&link, &broadcast, PEER, to), mismatch, "{subnet}");
        assert_eq!(receive(&link, &forwarded, BBMD, to), mismatch, "{subnet}");
    }
    // Directed traffic still comes by unicast.
    let Received::Deliver(delivery) = receive(&link, &unicast, PEER, classify(*OWN.ip())) else {
        panic!("unicast is delivered");
    };
    assert_eq!((delivery.source, delivery.group), (PEER, false));
}

#[test]
fn a_members_own_echo_from_the_group_is_dropped() {
    let link = member();
    let broadcast = wire(Bvll::OriginalBroadcastNpdu(NPDU));
    assert_eq!(
        receive(&link, &broadcast, OWN, classify(*GROUP.ip())),
        Received::Discarded(Discard::OwnEcho)
    );
}

#[test]
fn a_member_broadcasts_to_the_group_and_never_unicasts_there() {
    let link = member();
    let (outgoing, forward) = link.broadcast(NPDU).expect("fits");
    assert_eq!(sent(outgoing), (GROUP, Bvll::OriginalBroadcastNpdu(NPDU)));
    assert!(!forward);
    assert_eq!(
        link.unicast(GROUP, NPDU).map(sent),
        Err(SendRefusal::GroupDestination)
    );
    assert!(link.groups().contains(GROUP));
    assert!(link.groups().contains_ip(*GROUP.ip()));
}

#[test]
fn set_subnet_joins_and_leaves_the_group() {
    let mut node: Node = Node::new(link(Mode::Normal), at(0)).expect("valid");
    node.set_subnet(LocalSubnet::new(GROUP)).expect("valid");
    let (outgoing, _) = node.link().broadcast(NPDU).expect("fits");
    assert_eq!(outgoing.to, GROUP);
    node.set_subnet(SUBNET).expect("valid");
    let (outgoing, _) = node.link().broadcast(NPDU).expect("fits");
    assert_eq!(outgoing.to, BROADCAST);
}

#[test]
fn a_members_directed_broadcast_stays_a_group() {
    let link = member();
    let groups = link.groups();
    for port in [PORT, PORT + 1] {
        assert!(groups.contains(addr(192, 168, 1, 255, port)), "{port}");
    }
    assert!(groups.contains_ip(*BROADCAST.ip()));
    let unicast = wire(Bvll::OriginalUnicastNpdu(NPDU));
    // A datagram from the subnet's directed broadcast is dropped first.
    assert_eq!(
        receive(&link, &unicast, BROADCAST, classify(*OWN.ip())),
        Received::Discarded(Discard::GroupSource)
    );
    // So is a Forwarded-NPDU naming it as the origin.
    let forwarded = wire(Bvll::ForwardedNpdu {
        origin: BROADCAST,
        npdu: NPDU,
    });
    assert_eq!(
        receive(&link, &forwarded, BBMD, classify(*GROUP.ip())),
        Received::Discarded(Discard::GroupOrigin)
    );
    // A member that doesn't know its directed broadcast has only the group.
    let unknown = with(member(), |link| link.subnet = LocalSubnet::new(GROUP));
    assert!(!unknown.groups().contains_ip(*BROADCAST.ip()));
}

#[test]
fn a_foreign_device_takes_no_group() {
    let mode = Mode::Foreign(foreign());
    assert_eq!(
        Link::new(OWN, LocalSubnet::new(GROUP), mode),
        Err(ConfigError::MulticastMode)
    );
    let mut node: Node = Node::new(link(mode), at(0)).expect("valid");
    assert_eq!(
        node.set_subnet(LocalSubnet::new(GROUP)),
        Err(ConfigError::MulticastMode)
    );
    assert_eq!(node.link().subnet, SUBNET, "unchanged");
    // A normal node is a member, and a BBMD serves the group (J.8.3,
    // `port_bbmd_multicast.rs`).
    for mode in [Mode::Normal, Mode::Bbmd] {
        assert!(Link::new(OWN, LocalSubnet::new(GROUP), mode).is_ok());
        let mut node: Node = Node::new(link(mode), at(0)).expect("valid");
        node.set_subnet(LocalSubnet::new(GROUP)).expect("valid");
    }
}
