//! The B/IP glue (feature `bip`): a router's B/IP port over
//! `bacnet-bip-core`'s node engine, in normal and BBMD mode.

#![cfg(feature = "bip")]

mod fixture;
mod glue;

use bacnet_bip_codec::{BdtEntries, BdtEntry, BvlcFunction, BvlcResultCode, Bvll};
use bacnet_bip_core::bbmd::{ForwardRefusal, RegistrationLimits, Reservation, Tables, Target};
use bacnet_bip_core::node::{Cast, Discard, ForBbmd, Mode, Node, Permissive};
use bacnet_router_core::bip::{
    group, port_config, BbmdPort, BipPort, HeldBroadcast, Ingress, PortError,
};
use bacnet_router_core::router::{Group, Limits, Options, PortConfig, Received, SendRefusal};
use bacnet_router_core::Instant;

use fixture::*;
use glue::*;

fn bip_port(mode: Mode, held: &mut [HeldBroadcast]) -> BipPort<'_> {
    BipPort::new(BIP_PORT, node(mode, Instant::ZERO), held).unwrap()
}

/// The router, with `bip` as port 0 and an MS/TP-like port 1.
fn router_on(bip: PortConfig) -> TestRouter {
    router_with(&[bip, port_on(2, OWN_1)], Options::HOSTED)
}

#[test]
fn port_config_reads_the_link() {
    let link = link(Mode::Normal);
    let config = port_config(&link, 1);
    assert_eq!(config.network, 1);
    assert_eq!(config.mac.as_slice(), mac(OWN));
    assert_eq!(config.max_npdu, Limits::both(1497));
    // A smaller budget: a unicast's datagram adds the 4-octet header, a
    // broadcast's also the origin a BBMD forwards it with.
    let small = with(link, |link| link.budget = Some(510));
    assert_eq!(
        port_config(&small, 1).max_npdu,
        Limits {
            unicast: 506,
            broadcast: 500,
        }
    );
    // No budget: B/IP's largest NPDU all the same (Table 6-1).
    let unbounded = with(link, |link| link.budget = None);
    assert_eq!(port_config(&unbounded, 1).max_npdu, Limits::both(1497));
}

#[test]
fn groups_are_local_broadcasts_only_at_the_links_own_port() {
    let link = link(Mode::Normal);
    for local in [
        SUBNET.broadcast,
        sock(255, 255, 255, 255, PORT),
        sock(239, 1, 2, 3, PORT),
    ] {
        assert_eq!(group(&link, &mac(local)), Group::Broadcast, "{local}");
    }
    for elsewhere in [
        sock(192, 0, 2, 255, PORT + 1),
        sock(255, 255, 255, 255, PORT + 1),
    ] {
        assert_eq!(group(&link, &mac(elsewhere)), Group::Refuse, "{elsewhere}");
    }
    assert_eq!(group(&link, &mac(PEER)), Group::No);
    assert_eq!(group(&link, &[0x05]), Group::No);
}

#[test]
fn ports_refuse_storage_that_doesnt_fit_the_node() {
    let now = Instant::ZERO;
    let mut none: [HeldBroadcast; 0] = [];
    let mut one = [HeldBroadcast::EMPTY];
    let refused =
        |mode, held: &mut [HeldBroadcast]| BipPort::<4>::new(BIP_PORT, node(mode, now), held).err();
    assert_eq!(refused(Mode::Bbmd, &mut none), Some(PortError::Mode));
    assert_eq!(
        refused(foreign(2), &mut one),
        Some(PortError::HeldStorage {
            needed: 2,
            available: 1
        })
    );
    assert_eq!(refused(foreign(1), &mut one), None);
    assert_eq!(refused(Mode::Normal, &mut none), None);
    // Without a send budget a held broadcast could be as long as the
    // largest BVLL message less its header: far past a slot.
    let unbounded = with(link(foreign(1)), |link| link.budget = None);
    let unbounded = Node::<4>::new(unbounded, now).unwrap();
    assert_eq!(
        BipPort::new(BIP_PORT, unbounded, &mut one).err(),
        Some(PortError::HeldLength {
            needed: 65_525,
            available: 1497
        })
    );

    // Slots of N octets take a link whose broadcasts are no longer: a
    // budget of 510 lets broadcasts of 500 through.
    let budget = |octets| with(link(foreign(1)), |link| link.budget = Some(octets));
    assert_eq!(budget(510).max_npdu(Cast::Broadcast), 500);
    let mut short = [HeldBroadcast::<500>::EMPTY];
    let port = BipPort::new(
        BIP_PORT,
        Node::<4>::new(budget(510), now).unwrap(),
        &mut short,
    );
    assert!(port.is_ok());
    let mut shorter = [HeldBroadcast::<499>::EMPTY];
    let port = BipPort::new(
        BIP_PORT,
        Node::<4>::new(budget(510), now).unwrap(),
        &mut shorter,
    );
    assert_eq!(
        port.err(),
        Some(PortError::HeldLength {
            needed: 500,
            available: 499
        })
    );
    // The default budget sends B/IP's largest NPDU.
    let mut small = [HeldBroadcast::<501>::EMPTY];
    let port = BipPort::new(BIP_PORT, node(foreign(1), now), &mut small);
    assert_eq!(
        port.err(),
        Some(PortError::HeldLength {
            needed: 1497,
            available: 501
        })
    );
    // A normal node holds nothing, so any slots do.
    assert!(BipPort::new(BIP_PORT, node(Mode::Normal, now), &mut shorter).is_ok());

    let mut tables: Tables<2, 2> = Tables::new(OWN);
    let mut targets = [Target::UNUSED; 5];
    assert_eq!(
        BbmdPort::<2, 2>::new(
            BIP_PORT,
            self::node(Mode::Normal, now),
            &mut tables,
            &mut targets
        )
        .err(),
        Some(PortError::Mode)
    );
    assert_eq!(
        BbmdPort::<2, 2>::new(
            BIP_PORT,
            self::node(Mode::Bbmd, now),
            &mut tables,
            &mut targets[..4]
        )
        .err(),
        Some(PortError::TargetStorage {
            needed: Tables::<2, 2>::MAX_TARGETS,
            available: 4
        })
    );
}

/// A device on the subnet reaches a node on network 2, and back; a
/// broadcast goes both ways too.
#[test]
fn deliveries_become_frames_and_sends_become_annex_j_messages() {
    let now = Instant::ZERO;
    let mut bip = bip_port(Mode::Normal, &mut []);
    let mut router = router_on(bip.config(1));
    let (mut out, mut host) = (Wire::default(), Recorder::default());

    // An Original-Unicast from the peer, for node 5 on network 2.
    let npdu = apdu_to(2, &[5], CONFIRMED);
    let payload = encode(&Bvll::OriginalUnicastNpdu(&npdu));
    let ingress = bip.receive(
        &datagram(&payload, PEER, UNICAST),
        now,
        &mut Permissive,
        &mut out,
    );
    let Ingress::Npdu(arrived) = ingress else {
        panic!("{ingress:?}");
    };
    assert_eq!(
        (arrived.port, arrived.source.as_slice(), arrived.link_group),
        (BIP_PORT, &mac(PEER)[..], false)
    );
    assert_eq!(
        router.receive(arrived.frame(), now, &mut host),
        Received::Handled
    );
    let sent = drain(&mut router);
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (1, Some(&[5][..])));
    let source = sent[0].parsed().source.unwrap();
    assert_eq!((source.network, source.mac), (1, &mac(PEER)[..]));

    // Node 5 answers: an Original-Unicast to the peer.
    let answer = apdu_to(1, &mac(PEER), CONFIRMED);
    let _ = router.receive(frame(1, &[5], false, &answer), now, &mut host);
    let results = pump(&mut router, |send| bip.send(send, now, &mut out));
    assert_eq!(results, [(0, Ok(()))]);
    let wire = out.take();
    assert_eq!(wire.len(), 1);
    assert_eq!(
        (wire[0].to, wire[0].function),
        (PEER, BvlcFunction::ORIGINAL_UNICAST_NPDU)
    );
    let relayed = bacnet_router_core::npdu::Npdu::parse(&wire[0].npdu).unwrap();
    assert_eq!(relayed.destination, None);
    assert_eq!(
        relayed.source,
        Some(bacnet_router_core::npdu::Address {
            network: 2,
            mac: &[5]
        })
    );

    // A global broadcast from network 2 is an Original-Broadcast here.
    let _ = router.receive(frame(1, &[5], true, &GLOBAL_WHO_IS), now, &mut host);
    let results = pump(&mut router, |send| bip.send(send, now, &mut out));
    assert_eq!(results, [(0, Ok(()))]);
    let wire = out.take();
    assert_eq!(
        (wire[0].to, wire[0].function),
        (SUBNET.broadcast, BvlcFunction::ORIGINAL_BROADCAST_NPDU)
    );

    // And one from the subnet reaches the router as a link broadcast.
    let payload = encode(&Bvll::OriginalBroadcastNpdu(&GLOBAL_WHO_IS));
    let ingress = bip.receive(
        &datagram(&payload, PEER, TO_BROADCAST),
        now,
        &mut Permissive,
        &mut out,
    );
    let Ingress::Npdu(arrived) = ingress else {
        panic!("{ingress:?}");
    };
    assert!(arrived.link_group);
    assert!(matches!(
        router.receive(arrived.frame(), now, &mut host),
        Received::Local(local) if local.global
    ));
    assert_eq!(drain(&mut router).len(), 1, "relayed to network 2");
}

/// Each refusal reaches the router as its own, and is counted there once.
#[test]
fn refusals_map_to_the_routers() {
    let now = Instant::ZERO;
    let mut host = Recorder::default();
    // A 64-octet budget: the router believes the port carries 1,497.
    let small = with(link(Mode::Normal), |link| link.budget = Some(64));
    let mut bip = BipPort::<4>::new(
        BIP_PORT,
        bacnet_bip_core::node::Node::new(small, now).unwrap(),
        &mut [],
    )
    .unwrap();
    let mut router = router_on(port_config(&link(Mode::Normal), 1));
    let mut out = Wire::default();

    // Too long for the budget.
    let long = apdu_to(1, &mac(PEER), &[0x10; 80]);
    let _ = router.receive(frame(1, &[5], false, &long), now, &mut host);
    let results = pump(&mut router, |send| bip.send(send, now, &mut out));
    assert_eq!(results, [(0, Err(SendRefusal::TooLong))]);
    // A DADR that isn't six octets.
    let short = apdu_to(1, &[7], CONFIRMED);
    let _ = router.receive(frame(1, &[5], false, &short), now, &mut host);
    let results = pump(&mut router, |send| bip.send(send, now, &mut out));
    assert_eq!(results, [(0, Err(SendRefusal::Other))]);
    assert_eq!(out.calls, 0, "nothing reached the socket");
    // The socket is full.
    out.refuse = Some(SendRefusal::Full);
    let fits = apdu_to(1, &mac(PEER), CONFIRMED);
    let _ = router.receive(frame(1, &[5], false, &fits), now, &mut host);
    let results = pump(&mut router, |send| bip.send(send, now, &mut out));
    assert_eq!(results, [(0, Err(SendRefusal::Full))]);

    let counts = router.counters();
    assert_eq!((counts.refused, counts.egress_too_long), (3, 1));
}

/// Requests only a BBMD serves draw their NAK from the port, and a
/// datagram the node drops goes nowhere.
#[test]
fn naks_and_drops_stay_at_the_port() {
    let now = Instant::ZERO;
    let mut bip = bip_port(Mode::Normal, &mut []);
    let mut out = Wire::default();
    let payload = encode(&Bvll::ReadBroadcastDistributionTable);
    assert_eq!(
        bip.receive(
            &datagram(&payload, PEER, UNICAST),
            now,
            &mut Permissive,
            &mut out
        ),
        Ingress::Handled
    );
    let nak = BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK.to_raw();
    assert_eq!(
        out.take(),
        [Out::new(
            PEER,
            BvlcFunction::BVLC_RESULT,
            &nak.to_be_bytes()
        )]
    );
    assert_eq!(
        bip.receive(
            &datagram(&[0x81, 0x0A], PEER, UNICAST),
            now,
            &mut Permissive,
            &mut out
        ),
        Ingress::Discarded(Discard::TooShort)
    );
    assert_eq!(out.calls, 1);
}

/// A BBMD port with one peer BBMD, reached two-hop, and one registered
/// foreign device: what each broadcast brings the router, and where the
/// port forwards it (Annex J.4.5).
#[test]
fn bbmd_port_delivers_and_forwards() {
    let now = Instant::from_secs(1);
    let mut tables: Tables<2, 2> = Tables::new(OWN);
    let mut targets = [Target::UNUSED; Tables::<2, 2>::MAX_TARGETS];
    let mut bbmd = bbmd_port(&mut tables, &mut targets, now);
    let mut router = router_on(bbmd.config(1));
    let (mut out, mut host) = (Wire::default(), Recorder::default());

    // The router's own broadcast: the subnet, then the peer and the
    // device, as the BBMD's own broadcast.
    let _ = router.receive(frame(1, &[5], true, &GLOBAL_WHO_IS), now, &mut host);
    let results = pump(&mut router, |send| bbmd.send(send, now, &mut out));
    assert_eq!(results, [(0, Ok(()))]);
    let relayed = out.take();
    let npdu = relayed[0].npdu.clone();
    assert_eq!(
        relayed,
        [
            Out::new(
                SUBNET.broadcast,
                BvlcFunction::ORIGINAL_BROADCAST_NPDU,
                &npdu
            ),
            Out::forwarded(BBMD, OWN, &npdu),
            Out::forwarded(DEVICE, OWN, &npdu),
        ]
    );

    // A broadcast on the subnet: to the router, and on to the others.
    let who_is = [0x01, 0x00, 0x10, 0x08];
    let message = Bvll::OriginalBroadcastNpdu(&who_is);
    let payload = encode(&message);
    let ingress = bbmd.receive(
        &datagram(&payload, PEER, TO_BROADCAST),
        now,
        &mut Permissive,
        &mut out,
    );
    assert_eq!(npdu_from(&ingress), Some(mac(PEER)));
    assert_eq!(
        out.take(),
        [
            Out::forwarded(BBMD, PEER, &who_is),
            Out::forwarded(DEVICE, PEER, &who_is)
        ]
    );

    // The peer's forward, by unicast: to the router from its origin, and
    // on to the subnet and the device.
    let message = Bvll::ForwardedNpdu {
        origin: ORIGIN,
        npdu: &who_is,
    };
    let payload = encode(&message);
    let ingress = bbmd.receive(
        &datagram(&payload, BBMD, UNICAST),
        now,
        &mut Permissive,
        &mut out,
    );
    assert_eq!(npdu_from(&ingress), Some(mac(ORIGIN)));
    assert_eq!(
        out.take(),
        [
            Out::forwarded(SUBNET.broadcast, ORIGIN, &who_is),
            Out::forwarded(DEVICE, ORIGIN, &who_is),
        ]
    );
    // The same from a sender with no BDT row: refused, and nowhere.
    let ingress = bbmd.receive(
        &datagram(&payload, DEVICE, UNICAST),
        now,
        &mut Permissive,
        &mut out,
    );
    assert_eq!(ingress, Ingress::NotForwarded(ForwardRefusal::NotFromPeer));
    assert!(out.take().is_empty());

    // The device's broadcast: to the router, the subnet and the peer.
    let message = Bvll::DistributeBroadcastToNetwork(&who_is);
    let payload = encode(&message);
    let ingress = bbmd.receive(
        &datagram(&payload, DEVICE, UNICAST),
        now,
        &mut Permissive,
        &mut out,
    );
    assert_eq!(npdu_from(&ingress), Some(mac(DEVICE)));
    assert_eq!(
        out.take(),
        [
            Out::forwarded(SUBNET.broadcast, DEVICE, &who_is),
            Out::forwarded(BBMD, DEVICE, &who_is),
        ]
    );
    // One from a device that isn't registered draws the NAK, and nothing
    // reaches the router.
    let ingress = bbmd.receive(
        &datagram(&payload, PEER, UNICAST),
        now,
        &mut Permissive,
        &mut out,
    );
    assert_eq!(ingress, Ingress::Handled);
    let nak = BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK.to_raw();
    assert_eq!(
        out.take(),
        [Out::new(
            PEER,
            BvlcFunction::BVLC_RESULT,
            &nak.to_be_bytes()
        )]
    );

    // A registration is the caller's to serve, through the tables.
    let payload = encode(&Bvll::RegisterForeignDevice { ttl: 30 });
    let ingress = bbmd.receive(
        &datagram(&payload, PEER, UNICAST),
        now,
        &mut Permissive,
        &mut out,
    );
    assert_eq!(
        ingress,
        Ingress::Management(ForBbmd::Register {
            sender: PEER,
            ttl: 30
        })
    );
    assert_eq!(bbmd.counters().refused, 0);
}

/// The source of the NPDU `ingress` hands the router, which a BBMD port
/// always hands over as a link broadcast.
fn npdu_from(ingress: &Ingress<'_>) -> Option<Vec<u8>> {
    match ingress {
        Ingress::Npdu(arrived) => {
            assert!(arrived.link_group);
            Some(arrived.source.as_slice().to_vec())
        }
        _ => None,
    }
}

/// A datagram the node can't take on trust reaches neither the router nor
/// the socket: one whose destination the host couldn't report (#1678), one
/// cut short, one whose destination doesn't fit its function, and one
/// whose Length field is wrong.
///
/// The port's half of ACE IoT's
/// `ingress_fails_closed_for_destination_mismatch_missing_metadata_and_truncation`
/// (ACE-IoT-Solutions/rusty-bacnet#5, thanks @acedrew), which
/// `bacnet-bip-core`'s `tests/node_receive.rs` ports whole.
#[test]
fn ingress_fails_closed_before_the_router() {
    let now = Instant::ZERO;
    let mut bip = bip_port(Mode::Normal, &mut []);
    let mut out = Wire::default();
    let npdu = apdu_to(2, &[5], CONFIRMED);
    let payload = encode(&Bvll::OriginalUnicastNpdu(&npdu));
    let mut long = payload.clone();
    long[3] += 1;
    let unknown = with(datagram(&payload, PEER, UNICAST), |datagram| {
        datagram.to = None;
    });
    let truncated = with(datagram(&payload, PEER, UNICAST), |datagram| {
        datagram.truncated = true;
    });
    for (datagram, reason) in [
        (unknown, Discard::UnknownDestination),
        (truncated, Discard::Truncated),
        (
            datagram(&payload, PEER, TO_BROADCAST),
            Discard::DestinationMismatch,
        ),
        (datagram(&long, PEER, UNICAST), Discard::LengthMismatch),
    ] {
        assert_eq!(
            bip.receive(&datagram, now, &mut Permissive, &mut out),
            Ingress::Discarded(reason)
        );
    }
    assert_eq!(out.calls, 0);
}

/// Nothing goes to a group address as a unicast: a router send to the
/// subnet broadcast, the limited broadcast or a multicast address, which a
/// host that knows no groups lets through, is refused before anything is
/// encoded or reaches the socket (#1677, decision L19).
///
/// Ported from ACE IoT's bacnet-router-adapters
/// (ACE-IoT-Solutions/rusty-bacnet#5), thanks @acedrew, where it is
/// `unicast_group_destinations_are_rejected_before_encoding_or_send`;
/// `bacnet-bip-core`'s `tests/node_send.rs` ports its link half.
#[test]
fn unicast_group_destinations_are_rejected_before_encoding_or_send() {
    let now = Instant::ZERO;
    let mut bip = bip_port(Mode::Normal, &mut []);
    let mut router = router_on(bip.config(1));
    let (mut out, mut host) = (Wire::default(), Recorder::default());
    for group in [
        SUBNET.broadcast,
        sock(255, 255, 255, 255, PORT),
        sock(239, 1, 2, 3, PORT),
    ] {
        let npdu = apdu_to(1, &mac(group), UNCONFIRMED);
        let _ = router.receive(frame(1, &[5], false, &npdu), now, &mut host);
        let results = pump(&mut router, |send| bip.send(send, now, &mut out));
        assert_eq!(results, [(0, Err(SendRefusal::Other))], "{group}");
    }
    assert_eq!(out.calls, 0);
    assert_eq!(router.counters().refused, 3);
}

/// A BBMD port at [`OWN`] with one peer BBMD, reached two-hop, and one
/// foreign device registered at `now`, over `tables` and `targets`.
fn bbmd_port<'t>(
    tables: &'t mut Tables<2, 2>,
    targets: &'t mut [Target],
    now: Instant,
) -> BbmdPort<'t, 2, 2> {
    let peer_row = [BdtEntry::new(BBMD, core::net::Ipv4Addr::BROADCAST)];
    tables.bdt.set(BdtEntries::from_entries(&peer_row)).unwrap();
    let limits = RegistrationLimits::default();
    let registered = tables
        .fdt
        .register(DEVICE, 60, now, &limits, Reservation::Ordinary, || true);
    assert!(registered.result.is_ok());
    BbmdPort::new(BIP_PORT, node(Mode::Bbmd, now), tables, targets).unwrap()
}

/// The clock rule holds for a BBMD's tables, which keep no clock of their
/// own. Once the port has been given 200 s, the device registered at 1 s
/// for 60 s has lapsed (its TTL and the 30 s grace), and a receive or a
/// send stamped 10 s doesn't bring it back: neither forwards to it, where
/// at its stamp each would.
#[test]
fn a_bbmd_port_hands_its_tables_the_latest_time() {
    let mut tables: Tables<2, 2> = Tables::new(OWN);
    let mut targets = [Target::UNUSED; Tables::<2, 2>::MAX_TARGETS];
    let mut bbmd = bbmd_port(&mut tables, &mut targets, Instant::from_secs(1));
    let mut router = router_on(bbmd.config(1));
    let (mut out, mut host) = (Wire::default(), Recorder::default());
    bbmd.on_deadline(Instant::from_secs(200));
    let early = Instant::from_secs(10);

    // A broadcast on the subnet, stamped early: on to the peer only.
    let who_is = [0x01, 0x00, 0x10, 0x08];
    let payload = encode(&Bvll::OriginalBroadcastNpdu(&who_is));
    let ingress = bbmd.receive(
        &datagram(&payload, PEER, TO_BROADCAST),
        early,
        &mut Permissive,
        &mut out,
    );
    assert_eq!(npdu_from(&ingress), Some(mac(PEER)));
    assert_eq!(out.take(), [Out::forwarded(BBMD, PEER, &who_is)]);

    // The router's own broadcast, sent early: the subnet and the peer.
    let _ = router.receive(frame(1, &[5], true, &GLOBAL_WHO_IS), early, &mut host);
    let results = pump(&mut router, |send| bbmd.send(send, early, &mut out));
    assert_eq!(results, [(0, Ok(()))]);
    let sent = out.take();
    let npdu = sent[0].npdu.clone();
    assert_eq!(
        sent,
        [
            Out::new(
                SUBNET.broadcast,
                BvlcFunction::ORIGINAL_BROADCAST_NPDU,
                &npdu
            ),
            Out::forwarded(BBMD, OWN, &npdu),
        ]
    );
}

/// Each datagram of a BBMD port's own that the socket refuses is counted
/// once: a NAK, and each forwarded copy.
#[test]
fn refused_datagrams_of_a_bbmd_port_are_counted() {
    let mut out = Wire {
        refuse: Some(SendRefusal::Full),
        ..Wire::default()
    };
    let now = Instant::from_secs(1);
    let mut tables: Tables<2, 2> = Tables::new(OWN);
    let mut targets = [Target::UNUSED; Tables::<2, 2>::MAX_TARGETS];
    let mut bbmd = bbmd_port(&mut tables, &mut targets, now);
    let who_is = [0x01, 0x00, 0x10, 0x08];
    let forwarded = Bvll::ForwardedNpdu {
        origin: ORIGIN,
        npdu: &who_is,
    };
    for (message, from, to, refused) in [
        // A subnet broadcast, to the peer and the device.
        (Bvll::OriginalBroadcastNpdu(&who_is), PEER, TO_BROADCAST, 2),
        // The peer's forward, to the subnet and the device.
        (forwarded, BBMD, UNICAST, 4),
        // A broadcast from a device that isn't registered: its NAK.
        (
            Bvll::DistributeBroadcastToNetwork(&who_is),
            PEER,
            UNICAST,
            5,
        ),
    ] {
        let payload = encode(&message);
        let ingress = bbmd.receive(
            &datagram(&payload, from, to),
            now,
            &mut Permissive,
            &mut out,
        );
        assert!(!matches!(ingress, Ingress::Discarded(_)), "{ingress:?}");
        assert_eq!(bbmd.counters().refused, refused, "{message:?}");
    }
    // Taking the counts starts them again from zero.
    assert_eq!(bbmd.take_counters().refused, 5);
    assert_eq!(bbmd.counters().refused, 0);
}

/// A BBMD port whose socket refuses the router's broadcast on the subnet
/// forwards it nowhere, and the router hears of the refusal.
#[test]
fn a_bbmd_broadcast_the_socket_refuses_goes_nowhere() {
    let now = Instant::from_secs(1);
    let mut tables: Tables<2, 2> = Tables::new(OWN);
    let mut targets = [Target::UNUSED; Tables::<2, 2>::MAX_TARGETS];
    let mut bbmd = bbmd_port(&mut tables, &mut targets, now);
    let mut router = router_on(bbmd.config(1));
    let mut out = Wire {
        refuse: Some(SendRefusal::Full),
        ..Wire::default()
    };
    let mut host = Recorder::default();
    let _ = router.receive(frame(1, &[5], true, &GLOBAL_WHO_IS), now, &mut host);
    let results = pump(&mut router, |send| bbmd.send(send, now, &mut out));
    assert_eq!(results, [(0, Err(SendRefusal::Full))]);
    assert_eq!(out.calls, 1, "no copy follows the refused broadcast");
    assert_eq!(bbmd.counters().refused, 0, "the router counts it");
    assert_eq!(router.counters().refused, 1);
}
