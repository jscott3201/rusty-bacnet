//! What a node sends for an NPDU, the datagram budget, and the checks on a
//! link's configuration (Annex J.2.10 to J.2.12, J.3, J.4.1; Table 6-1).

mod node_support;

use core::net::SocketAddrV4;

use bacnet_bip_codec::Bvll;
use bacnet_bip_core::node::{
    Cast, ConfigError, Foreign, ForwardedTrust, Link, Mode, Retry, SendRefusal, MAX_DATAGRAM,
    MIN_RENEWAL,
};
use bacnet_bip_core::{Duration, LocalSubnet};
use bacnet_npdu::MAX_NPDU;
use node_support::*;

const NPDU: &[u8] = &[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];

/// What `link` sends to broadcast [`NPDU`], with [`sent`]'s pair for the
/// message, and whether to forward it too.
fn broadcast(link: &Link) -> Result<((SocketAddrV4, Bvll<'static>), bool), SendRefusal> {
    link.broadcast(NPDU)
        .map(|(outgoing, forward)| (sent(outgoing), forward))
}

#[test]
fn unicast_is_original_unicast() {
    for mode in [Mode::Normal, Mode::Foreign(foreign()), Mode::Bbmd] {
        let peer = addr(192, 168, 1, 20, 50_000);
        assert_eq!(
            link(mode).unicast(peer, NPDU).map(sent),
            Ok((peer, Bvll::OriginalUnicastNpdu(NPDU)))
        );
    }
}

/// A unicast to one of the link's group addresses (the subnet's broadcast
/// IP at any port, the limited broadcast, a multicast address) is refused
/// before there is a message to encode, in every mode, whatever the NPDU's
/// length: an Original-Unicast-NPDU goes to one node (#1677). A broadcast
/// still reaches the group.
///
/// Ported from ACE IoT's bacnet-router-adapters
/// (ACE-IoT-Solutions/rusty-bacnet#5), thanks @acedrew.
#[test]
fn unicast_group_destinations_are_rejected_before_encoding_or_send() {
    let over = vec![0x01; MAX_NPDU + 1];
    for mode in [Mode::Normal, Mode::Foreign(foreign()), Mode::Bbmd] {
        let link = link(mode);
        for group in [
            BROADCAST,
            addr(192, 168, 1, 255, 47_809),
            addr(255, 255, 255, 255, PORT),
            addr(239, 1, 2, 3, PORT),
        ] {
            let mut scratch = [0xA5; 32];
            let before = scratch;
            let sent = link
                .unicast(group, NPDU)
                .map(|outgoing| outgoing.message.encode_into(&mut scratch));
            assert_eq!(
                sent.map(drop),
                Err(SendRefusal::GroupDestination),
                "{mode:?} to {group}"
            );
            assert_eq!(scratch, before, "nothing encoded");
            assert_eq!(
                link.unicast(group, &over).map(drop),
                Err(SendRefusal::GroupDestination),
                "the destination is judged first"
            );
        }
        assert!(link.broadcast(NPDU).is_ok());
        // One node's address takes it, on the subnet or off it.
        assert!(link.unicast(PEER, NPDU).is_ok());
        assert!(link.unicast(addr(198, 51, 100, 7, 47_809), NPDU).is_ok());
    }
}

#[test]
fn normal_broadcast_is_original_broadcast_at_the_network_port() {
    assert_eq!(
        broadcast(&link(Mode::Normal)),
        Ok(((BROADCAST, Bvll::OriginalBroadcastNpdu(NPDU)), false))
    );
    // The configured broadcast address, port and all, even for a node on
    // an ephemeral port.
    let other = LocalSubnet::new(addr(10, 1, 255, 255, 47_809));
    let link = Link::new(addr(10, 1, 0, 3, 54_321), other, Mode::Normal).unwrap();
    let (outgoing, _) = link.broadcast(NPDU).unwrap();
    assert_eq!(outgoing.to, other.broadcast);
}

#[test]
fn foreign_broadcast_is_always_dbtn_to_the_bbmd() {
    let bbmd = addr(10, 0, 0, 1, 47_809);
    let link = foreign_link(Foreign::new(bbmd, 60));
    assert_eq!(
        broadcast(&link),
        Ok(((bbmd, Bvll::DistributeBroadcastToNetwork(NPDU)), false))
    );
}

#[test]
fn a_bbmd_broadcasts_locally_and_forwards_its_own() {
    assert_eq!(
        broadcast(&link(Mode::Bbmd)),
        Ok(((BROADCAST, Bvll::OriginalBroadcastNpdu(NPDU)), true))
    );
}

#[test]
fn the_budget_caps_each_cast() {
    // The default refuses only what Table 6-1 does.
    let default = link(Mode::Normal);
    assert_eq!(default.budget, Some(MAX_DATAGRAM as u16));
    assert_eq!(MAX_DATAGRAM, 1507);
    assert_eq!(default.max_npdu(Cast::Unicast), MAX_NPDU);
    assert_eq!(default.max_npdu(Cast::Broadcast), MAX_NPDU);
    let largest = vec![0x01; MAX_NPDU];
    let over = vec![0x01; MAX_NPDU + 1];
    assert!(default.unicast(PEER, &largest).is_ok());
    assert!(default.broadcast(&largest).is_ok());
    let too_long = Err(SendRefusal::TooLong { max: MAX_NPDU });
    assert_eq!(default.unicast(PEER, &over).map(|_| ()), too_long);
    assert_eq!(default.broadcast(&over).map(|_| ()), too_long);

    // A 1500-octet MTU less the IP and UDP headers: a BBMD's forward of a
    // broadcast, 6 octets longer, must fit too.
    let mtu = with(default, |link| link.budget = Some(1472));
    assert_eq!(mtu.max_npdu(Cast::Unicast), 1468);
    assert_eq!(mtu.max_npdu(Cast::Broadcast), 1462);
    assert!(mtu.unicast(PEER, &largest[..1468]).is_ok());
    assert_eq!(
        mtu.unicast(PEER, &largest[..1469]).map(|_| ()),
        Err(SendRefusal::TooLong { max: 1468 })
    );
    assert!(mtu.broadcast(&largest[..1462]).is_ok());
    assert_eq!(
        mtu.broadcast(&largest[..1463]).map(|_| ()),
        Err(SendRefusal::TooLong { max: 1462 })
    );
    let foreign = with(foreign_link(foreign()), |link| link.budget = Some(1472));
    assert_eq!(
        foreign.broadcast(&largest[..1463]).map(|_| ()),
        Err(SendRefusal::TooLong { max: 1462 })
    );

    // Without a budget, nothing is refused here.
    let unchecked = with(default, |link| link.budget = None);
    let huge = vec![0x01; 60_000];
    assert!(unchecked.unicast(PEER, &huge).is_ok());
    assert!(unchecked.broadcast(&huge).is_ok());
    assert_eq!(unchecked.max_npdu(Cast::Unicast), 65_531);
    assert_eq!(unchecked.max_npdu(Cast::Broadcast), 65_525);
}

#[test]
fn foreign_defaults_follow_the_time_to_live() {
    let foreign = Foreign::new(BBMD, 60);
    assert_eq!(foreign.renewal, Duration::from_secs(30));
    assert_eq!(foreign.response_timeout, Duration::from_secs(3));
    assert_eq!(foreign.retry, Retry::Backoff);
    assert_eq!(foreign.hold, 0);
    assert_eq!(foreign.trust, ForwardedTrust::ConfiguredBbmd);
    // Half the time-to-live, to the millisecond, at the longest one.
    let longest = Foreign::new(BBMD, u16::MAX);
    assert_eq!(longest.renewal, ms(32_767_500));
    assert_eq!(longest.response_timeout, Duration::from_secs(3));
    let short = Foreign::new(BBMD, 1);
    assert_eq!(short.renewal, ms(500));
    assert_eq!(short.response_timeout, ms(500));
    // The floor applies after the checks.
    let fast = with(short, |foreign| {
        foreign.renewal = ms(50);
        foreign.response_timeout = ms(40);
    });
    assert_eq!(fast.effective_renewal(), MIN_RENEWAL);
    assert_eq!(fast.effective_response_timeout(), ms(40));
    let slow_answer = with(Foreign::new(BBMD, 4), |foreign| {
        foreign.response_timeout = Duration::from_secs(5);
    });
    assert_eq!(
        slow_answer.effective_response_timeout(),
        Duration::from_secs(2)
    );
}

#[test]
fn a_link_is_checked_before_it_runs() {
    let mode = |foreign: Foreign| Mode::Foreign(foreign);
    // A valid link with the fields under test set on it, unchecked.
    let check = |own, mode, budget| {
        let link = with(link(Mode::Normal), |link| {
            link.address = own;
            link.mode = mode;
            link.budget = budget;
        });
        link.validate()
    };
    let budget = Some(MAX_DATAGRAM as u16);
    assert_eq!(check(OWN, Mode::Normal, budget), Ok(()));
    assert_eq!(
        check(addr(192, 168, 1, 10, 0), Mode::Normal, budget),
        Err(ConfigError::OwnPort)
    );
    assert_eq!(check(OWN, Mode::Normal, Some(63)), Err(ConfigError::Budget));
    assert_eq!(check(OWN, Mode::Normal, Some(64)), Ok(()));
    assert_eq!(check(OWN, Mode::Bbmd, None), Ok(()));
    let base = foreign();
    assert_eq!(check(OWN, mode(base), budget), Ok(()));
    let cases = [
        (with(base, |f| f.ttl = 0), ConfigError::Renewal),
        (
            with(base, |f| f.renewal = Duration::ZERO),
            ConfigError::Renewal,
        ),
        (
            with(base, |f| f.renewal = Duration::from_secs(60)),
            ConfigError::Renewal,
        ),
        (
            with(base, |f| f.response_timeout = Duration::ZERO),
            ConfigError::ResponseTimeout,
        ),
        (
            with(base, |f| f.bbmd = addr(10, 0, 0, 1, 0)),
            ConfigError::BbmdPort,
        ),
        (with(base, |f| f.bbmd = OWN), ConfigError::BbmdIsSelf),
    ];
    for (foreign, error) in cases {
        assert_eq!(check(OWN, mode(foreign), budget), Err(error), "{foreign:?}");
        assert_eq!(Link::new(OWN, SUBNET, mode(foreign)), Err(error));
    }
    // Just under the time-to-live, and the floor raising a tiny renewal.
    let tight = with(base, |f| {
        f.renewal = Duration::from_secs(60) - Duration::from_nanos(1);
    });
    assert_eq!(check(OWN, mode(tight), budget), Ok(()));
    let tiny = with(Foreign::new(BBMD, 1), |f| f.renewal = ms(1));
    assert_eq!(check(OWN, mode(tiny), budget), Ok(()));
    // The BBMD at the node's IP but another port is another node.
    let beside = with(base, |f| f.bbmd = addr(192, 168, 1, 10, 47_809));
    assert_eq!(check(OWN, mode(beside), budget), Ok(()));
    assert!(!ConfigError::Renewal.to_string().is_empty());
}

#[test]
fn a_bbmd_at_no_single_address_is_refused() {
    let base = foreign();
    let check = |bbmd| {
        let link = with(link(Mode::Normal), |link| {
            link.mode = Mode::Foreign(with(base, |f| f.bbmd = bbmd));
            link.budget = None;
        });
        link.validate()
    };
    for bbmd in [
        addr(0, 0, 0, 0, PORT),
        addr(255, 255, 255, 255, PORT),
        addr(224, 0, 0, 1, PORT),
        addr(239, 1, 2, 3, 47_809),
        BROADCAST,
        // The subnet's broadcast IP at any port.
        addr(192, 168, 1, 255, 47_809),
    ] {
        assert_eq!(check(bbmd), Err(ConfigError::BbmdAddress), "{bbmd}");
    }
    assert_eq!(check(BBMD), Ok(()));
    // A loopback test's broadcast address is a group only at its port.
    let loopback = LocalSubnet::new(addr(127, 0, 0, 1, PORT));
    let own = addr(127, 0, 0, 1, 47_809);
    let on_loopback = |bbmd| Link::new(own, loopback, Mode::Foreign(Foreign::new(bbmd, 60)));
    assert!(on_loopback(addr(127, 0, 0, 1, 47_810)).is_ok());
    assert_eq!(
        on_loopback(addr(127, 0, 0, 1, PORT)),
        Err(ConfigError::BbmdAddress)
    );
}
