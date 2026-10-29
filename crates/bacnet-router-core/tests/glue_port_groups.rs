//! A port's group rules work with the router's default host.

#![cfg(all(feature = "bip", feature = "mstp"))]

mod fixture;
mod glue;

use bacnet_bip_core::codec::BvlcFunction;
use bacnet_bip_core::node::Mode;
use bacnet_bip_core::port::NodePort;
use bacnet_router_core::Instant;
use bacnet_router_core::bip::BipPort;
use bacnet_router_core::mstp::port_config;
use bacnet_router_core::npdu::Npdu;
use bacnet_router_core::npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_router_core::router::{Discard, Group, GroupClassifier, Options, Permissive, Received};

use fixture::{
    CONFIRMED, Outcome, TestRouter, UNCONFIRMED, addr, apdu_to, bytes, deliver, learn_9_via_r7,
    reject_data, router_with,
};
use glue::*;

#[test]
fn default_host_delivers_a_subnet_group_through_the_bip_link() {
    let now = Instant::ZERO;
    let glue = BipPort::new(BIP_PORT);
    let mut bip: NodePort<'_> = NodePort::new(node(Mode::Normal, now), &mut []).unwrap();
    let mut router = router_with(
        &[
            BipPort::config(bip.node().link(), 1),
            port_config(&config(true), 2).unwrap(),
        ],
        Options::HOSTED,
    );
    let npdu = apdu_to(1, &mac(SUBNET.broadcast), UNCONFIRMED);
    assert_eq!(
        router.receive(frame(1, &[2], false, &npdu), now, &mut Permissive),
        Received::Handled
    );
    let mut wire = Wire::default();
    let results = pump(&mut router, |send| {
        glue.send(&mut bip, send, now, &mut wire)
    });
    assert_eq!(results, [(0, Ok(()))]);
    let sent = wire.take();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to, SUBNET.broadcast);
    assert_eq!(sent[0].function, BvlcFunction::ORIGINAL_BROADCAST_NPDU);
    assert_eq!(router.counters().refused, 0);
}

fn configured() -> TestRouter {
    router_with(
        &[
            BipPort::config(&link(Mode::Normal), 1),
            port_config(&config(true), 2).unwrap(),
        ],
        Options::HOSTED,
    )
}

#[test]
fn default_host_drops_unsupported_apdus_and_network_messages_once() {
    let mut router = configured();
    let mut drops = 0;
    let reject_data = reject_data(RejectMessageReason::NOT_DIRECTLY_CONNECTED, 9);
    learn_9_via_r7(&mut router, Instant::ZERO);
    for target in [SUBNET.broadcast, sock(192, 0, 2, 255, PORT + 1)] {
        let dadr = mac(target);
        let mut message = Npdu::message(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0, 44]);
        message.destination = Some(addr(1, &dadr));
        let mut reject = Npdu::message(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, &reject_data);
        reject.destination = Some(addr(1, &dadr));
        let mut proprietary = Npdu::message(NetworkMessageType::from_raw(0x80), UNCONFIRMED);
        proprietary.vendor_id = Some(260);
        proprietary.destination = message.destination;
        let mut frames = vec![
            apdu_to(1, &dadr, CONFIRMED),
            bytes(&message),
            bytes(&proprietary),
            bytes(&reject),
        ];
        if target.port() != PORT {
            frames.push(apdu_to(1, &dadr, UNCONFIRMED));
        }
        for npdu in frames {
            let (outcome, sent) =
                deliver(&mut router, &mut Permissive, 1, &[7], &npdu, Instant::ZERO);
            drops += 1;
            assert_eq!(outcome, Outcome::Discarded(Discard::GroupDadr));
            assert!(sent.is_empty());
            assert_eq!(router.counters().group_dadr_drops, drops);
            assert_eq!(router.counters().rejects, 0);
            assert_eq!(router.counters().refused, 0);
        }
    }
    assert!(router.table().lookup(44).is_none());
}

#[test]
fn default_host_delivers_same_network_groups_and_preserves_other_destinations() {
    let mut router = configured();
    let (local, sent) = deliver(
        &mut router,
        &mut Permissive,
        0,
        &mac(PEER),
        &apdu_to(1, &mac(SUBNET.broadcast), UNCONFIRMED),
        Instant::ZERO,
    );
    assert!(matches!(
        local,
        Outcome::Local {
            group: true,
            global: false,
            ..
        }
    ));
    assert_eq!(sent.len(), 1);
    assert!(sent[0].is_broadcast());
    assert!(sent[0].parsed().destination.is_none());

    let (_, sent) = deliver(
        &mut router,
        &mut Permissive,
        0,
        &mac(PEER),
        &apdu_to(2, &[255], UNCONFIRMED),
        Instant::ZERO,
    );
    assert_eq!((sent[0].port, sent[0].is_broadcast()), (1, true));
    let (local, _) = deliver(
        &mut router,
        &mut Permissive,
        1,
        &[7],
        &apdu_to(2, &[255], UNCONFIRMED),
        Instant::ZERO,
    );
    assert!(matches!(local, Outcome::Local { group: true, .. }));

    let (_, sent) = deliver(
        &mut router,
        &mut Permissive,
        1,
        &[7],
        &apdu_to(1, &mac(PEER), CONFIRMED),
        Instant::ZERO,
    );
    assert_eq!(sent[0].to.as_deref(), Some(mac(PEER).as_slice()));
    // A one-octet MS/TP group has no meaning on the B/IP port.
    let (_, sent) = deliver(
        &mut router,
        &mut Permissive,
        1,
        &[7],
        &apdu_to(1, &[255], CONFIRMED),
        Instant::ZERO,
    );
    assert_eq!(sent[0].to.as_deref(), Some([255].as_slice()));
    learn_9_via_r7(&mut router, Instant::ZERO);
    let (_, sent) = deliver(
        &mut router,
        &mut Permissive,
        0,
        &mac(PEER),
        &apdu_to(9, &[255], CONFIRMED),
        Instant::ZERO,
    );
    assert_eq!(sent[0].to.as_deref(), Some([7].as_slice()));
    assert_eq!(sent[0].parsed().destination, Some(addr(9, &[255])));
    assert_eq!(router.counters().group_dadr_drops, 0);
}

#[test]
fn port_rules_preserve_loopback_and_interface_address_exceptions() {
    for ip in [core::net::Ipv4Addr::LOCALHOST, *OWN.ip()] {
        let mut link = link(Mode::Normal);
        link.subnet.broadcast = core::net::SocketAddrV4::new(ip, PORT);
        let rules = BipPort::config(&link, 1).groups;
        assert_eq!(
            rules.classify(&mac(link.subnet.broadcast)),
            Group::Broadcast
        );
        assert_eq!(
            rules.classify(&mac(core::net::SocketAddrV4::new(ip, PORT + 1))),
            Group::No
        );
    }
}
