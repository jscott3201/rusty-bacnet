//! Both glue features at once, one router joining a B/IP node to an MS/TP
//! master: the compositions ACE IoT's bacnet-router-adapters tested
//! (ACE-IoT-Solutions/rusty-bacnet#5), ported with thanks to @acedrew.
//!
//! Port 0 is a B/IP node in normal mode on network 1, port 1 an MS/TP
//! master, station 1, on network 2. Each send goes to its port's glue, and
//! each refusal back to the router, as a firmware loop does.

#![cfg(all(feature = "bip", feature = "mstp"))]

mod fixture;
mod glue;

use bacnet_bip_core::codec::{BvlcFunction, Bvll};
use bacnet_bip_core::node::{self, Discard, Link, Mode, Permissive};
use bacnet_bip_core::port::{Ingress, NodePort};
use bacnet_mstp_codec::{Frame as MstpFrame, FrameType};
use bacnet_mstp_core::{Action, Instant};
use bacnet_router_core::PortId;
use bacnet_router_core::bip::BipPort;
use bacnet_router_core::mstp::MstpPort;
use bacnet_router_core::npdu::NpduLimits;
use bacnet_router_core::npdu::{Address, Npdu};
use bacnet_router_core::router::{Options, PortConfig, Received, SendRefusal};

use bacnet_router_core::npdu::NetworkPriority;
use fixture::{CONFIRMED, TestRouter, UNCONFIRMED, apdu_to, router_with};
use glue::*;

const BIP_NET: u16 = 1;
const MSTP_NET: u16 = 2;
const BIP: PortId = PortId(0);
const BIP_PORT: BipPort = BipPort::new(BIP);

/// A BBMD on the router's subnet, which rebroadcasts a forward there.
const LOCAL_BBMD: core::net::SocketAddrV4 = sock(192, 0, 2, 1, PORT);
/// A device on network 1 that the MS/TP side answers.
const DEVICE_44: core::net::SocketAddrV4 = sock(192, 0, 2, 44, 0xBAC1);

/// The B/IP port, a normal node at [`OWN`] with `link`'s budget.
fn bip_port(link: Link) -> NodePort<'static> {
    NodePort::new(node::Node::new(link, Instant::ZERO).unwrap(), &mut []).unwrap()
}

/// The router over `bip`, with the MS/TP port as the core's configuration
/// gives it, or as `mstp` overrides it.
fn router(bip: &NodePort<'_>, mstp: Option<PortConfig>, options: Options) -> TestRouter {
    let mstp = mstp
        .unwrap_or_else(|| bacnet_router_core::mstp::port_config(&config(true), MSTP_NET).unwrap());
    router_with(
        &[BipPort::config(bip.node().link(), BIP_NET), mstp],
        options,
    )
}

/// Carry out every send the router has, each on its port.
fn carry<const U: usize>(
    router: &mut TestRouter,
    bip: &mut NodePort<'_>,
    out: &mut Wire,
    port: &mut Port<'_, U>,
    now: Instant,
) -> Vec<(u8, Result<(), SendRefusal>)> {
    pump(router, |send| match send.port {
        BIP => BIP_PORT.send(bip, send, now, out),
        _ => port.send(send, now),
    })
}

/// What the B/IP port makes of `message` from `source`, sent as `to`.
fn into_bip<'d>(
    bip: &mut NodePort<'_>,
    payload: &'d [u8],
    source: core::net::SocketAddrV4,
    to: bacnet_bip_core::node::Addressed,
    out: &mut Wire,
    now: Instant,
) -> Ingress<'d> {
    bip.receive(&datagram(payload, source, to), now, &mut Permissive, out)
}

/// An NPDU of `len` octets for `dnet`/`dadr` at life-safety priority, as
/// ACE builds it: the APDU is filler after an Error PDU's type octet.
fn routed(len: usize, dnet: u16, dadr: &[u8]) -> Vec<u8> {
    let mut npdu = vec![0x5A; len];
    let [high, low] = dnet.to_be_bytes();
    npdu[..5].copy_from_slice(&[0x01, 0x23, high, low, dadr.len() as u8]);
    npdu[5..5 + dadr.len()].copy_from_slice(dadr);
    npdu[5 + dadr.len()] = 0xFF;
    npdu
}

/// #1678 case 1: when the router relays a Forwarded-NPDU onto MS/TP, the
/// SADR names the forward's origin, not the BBMD that forwarded it (Annex
/// J.2.5), and the life-safety priority survives.
///
/// Ported from ACE IoT's
/// `forwarded_origin_and_priority_survive_bip_to_mstp_composition`.
#[test]
fn forwarded_origin_and_priority_survive_bip_to_mstp_composition() {
    let now = Instant::from_millis(1);
    let mut bip = bip_port(link(Mode::Normal));
    let mut router = router(&bip, None, Options::HOSTED);
    let mut host = bacnet_router_core::router::Permissive;
    let (mut bus, mut out) = (Bus::<1>::new(), Wire::default());
    let mut port = bus.port();

    let npdu = routed(100, MSTP_NET, &[5]);
    let forwarded = encode(&Bvll::ForwardedNpdu {
        origin: ORIGIN,
        npdu: &npdu,
    });
    let ingress = into_bip(
        &mut bip,
        &forwarded,
        LOCAL_BBMD,
        TO_BROADCAST,
        &mut out,
        now,
    );
    let Ingress::Npdu(delivery) = ingress else {
        panic!("{ingress:?}");
    };
    let arrived = BIP_PORT.arrived(&delivery);
    assert_eq!(arrived.source.as_slice(), mac(ORIGIN));
    assert_eq!(
        router.receive(arrived.frame(), now, &mut host),
        Received::Handled
    );
    let results = carry(&mut router, &mut bip, &mut out, &mut port, now);
    assert_eq!(results, [(1, Ok(()))]);
    assert_eq!(port.core_mut().counters().unicast.depth, 1);

    let wired = token_then_frame(port.core_mut(), now).unwrap();
    assert_eq!(wired.destination, 5);
    let mut npci = vec![0x01, 0x0B, 0x00, BIP_NET as u8, 0x06];
    npci.extend_from_slice(&mac(ORIGIN));
    assert_eq!(wired.data[..11], npci);
    assert_eq!(wired.data[1] & 0x03, 3, "life-safety priority");
    assert_eq!(wired.data.len(), 104);
    assert_eq!(wired.data[11..], npdu[7..]);
}

/// The router's egress check keeps an NPDU one octet too long for a port
/// held to 501 octets off it, with reject reason 4 back to its sender
/// (#1665); a burst then
/// fills the MS/TP queue's one slot, and the refusal is counted once.
///
/// Ported from ACE IoT's
/// `standard_boundary_and_burst_backpressure_are_visible_at_mstp_queue`.
#[test]
fn standard_boundary_and_burst_backpressure_are_visible_at_mstp_queue() {
    let now = Instant::from_millis(1);
    let mut bip = bip_port(link(Mode::Normal));
    let mut held_to_501 = bacnet_router_core::mstp::port_config(&config(true), MSTP_NET).unwrap();
    held_to_501.max_npdu = NpduLimits::both(501);
    let mut router = router(&bip, Some(held_to_501), Options::HOSTED);
    let mut host = bacnet_router_core::router::Permissive;
    let (mut bus, mut out) = (Bus::<1>::new(), Wire::default());
    let mut port = bus.port();
    // From the peer, the relay gains SNET and a six-octet SADR (11 octets)
    // and loses the DNET, DADR and hop count (7 with the version).
    let mut relay = |router: &mut TestRouter, port: &mut Port<'_, 1>, len: usize| {
        let npdu = routed(len - 4, MSTP_NET, &[5]);
        let payload = encode(&Bvll::OriginalUnicastNpdu(&npdu));
        let Ingress::Npdu(delivery) = into_bip(&mut bip, &payload, PEER, UNICAST, &mut out, now)
        else {
            panic!("not delivered");
        };
        let _ = router.receive(BIP_PORT.arrived(&delivery).frame(), now, &mut host);
        let results = pump(router, |send| match send.port {
            BIP => BIP_PORT.send(&mut bip, send, now, &mut out),
            _ => port.send(send, now),
        });
        (results, out.take())
    };

    let (results, _) = relay(&mut router, &mut port, 501);
    assert_eq!(results, [(1, Ok(()))]);
    let (results, wire) = relay(&mut router, &mut port, 502);
    assert_eq!(results, [(0, Ok(()))], "only the reject goes");
    assert_eq!(
        (wire[0].to, wire[0].function),
        (PEER, BvlcFunction::ORIGINAL_UNICAST_NPDU)
    );
    let reject = Npdu::decode(&wire[0].npdu).unwrap();
    assert_eq!(reject.payload, [4, 0, MSTP_NET as u8]);
    let (results, _) = relay(&mut router, &mut port, 501);
    assert_eq!(results, [(1, Err(SendRefusal::Full))]);

    assert_eq!(port.core_mut().counters().unicast.depth, 1);
    let counts = router.counters();
    assert_eq!((counts.egress_too_long, counts.refused), (1, 1));
}

/// A request from MS/TP for a device on network 1 leaves the B/IP port as
/// an Original-Unicast, with SNET 2, the station as SADR and its priority
/// kept; the MS/TP side answers Reply Postponed at once (Clauses 6.5.4,
/// 9.5.6.9).
///
/// Ported from ACE IoT's
/// `mstp_delivery_becomes_bip_original_unicast_and_binds_reply_id`; the
/// binding of the far side's reply to the request isn't adopted here, a
/// follow-up.
#[test]
fn mstp_delivery_becomes_bip_original_unicast_with_reply_postponed() {
    let now = Instant::from_millis(1);
    let mut bip = bip_port(link(Mode::Normal));
    let mut router = router(&bip, None, Options::HOSTED);
    let mut host = bacnet_router_core::router::Permissive;
    let (mut bus, mut out) = (Bus::<1>::new(), Wire::default());
    let mut port = bus.port();

    let mut request = routed(40, BIP_NET, &mac(DEVICE_44));
    request[1] |= 0x04;
    let frame = MstpFrame::new(FrameType::BACNET_DATA_EXPECTING_REPLY, STATION, 7, &request);
    port.core_mut().on_validated_frame(frame, now);
    let (id, received) = match port.core_mut().poll_action() {
        Action::DeliverNpdu {
            source,
            destination,
            npdu,
            request_id,
        } => {
            let arrived = mstp_arrived(source, destination, npdu);
            let received = router.receive(arrived.frame(), now, &mut host);
            (request_id, received == Received::Handled)
        }
        other => panic!("{other:?}"),
    };
    assert!(received, "routed to network 1");
    port.release(id, now).unwrap();
    let results = carry(&mut router, &mut bip, &mut out, &mut port, now);
    assert_eq!(results, [(0, Ok(()))]);

    let wire = out.take();
    assert_eq!(
        (wire[0].to, wire[0].function),
        (DEVICE_44, BvlcFunction::ORIGINAL_UNICAST_NPDU)
    );
    let relayed = Npdu::decode(&wire[0].npdu).unwrap();
    assert_eq!(relayed.priority, NetworkPriority::LifeSafety);
    assert!(relayed.expecting_reply);
    assert_eq!(
        relayed.source,
        Some(Address {
            network: MSTP_NET,
            mac: &[7]
        })
    );
    assert_eq!(relayed.destination, None);
    assert_eq!(wire[0].npdu.len(), 34);

    let answer = next_frame(port.core_mut(), now).unwrap();
    assert_eq!(
        (answer.frame_type, answer.destination),
        (FrameType::REPLY_POSTPONED, 7)
    );
}

/// A datagram that fails B/IP's checks changes nothing: driven as a
/// firmware loop drives the port, the router never sees it, and the MS/TP
/// queue stays empty. Each carries an NPDU for station 5 on MS/TP, which
/// the same datagram, well formed, does queue.
///
/// Ported from ACE IoT's `strict_bip_failure_precedes_router_or_queue_mutation`.
#[test]
fn strict_bip_failure_precedes_router_or_queue_mutation() {
    let now = Instant::from_millis(1);
    let mut bip = bip_port(link(Mode::Normal));
    let mut router = router(&bip, None, Options::HOSTED);
    let mut host = bacnet_router_core::router::Permissive;
    let (mut bus, mut out) = (Bus::<1>::new(), Wire::default());
    let mut port = bus.port();
    let npdu = routed(20, MSTP_NET, &[5]);
    let good = encode(&Bvll::OriginalUnicastNpdu(&npdu));
    let mut long = good.clone();
    long[3] += 1;
    let unknown = with(datagram(&good, PEER, UNICAST), |datagram| {
        datagram.to = None
    });
    // What the port made of a datagram, with any NPDU handed to the router
    // and every send carried out.
    let mut drive = |datagram: &bacnet_bip_core::node::Datagram<'_>,
                     router: &mut TestRouter,
                     out: &mut Wire| {
        let dropped = match bip.receive(datagram, now, &mut Permissive, out) {
            Ingress::Npdu(delivery) => {
                let _ = router.receive(BIP_PORT.arrived(&delivery).frame(), now, &mut host);
                None
            }
            Ingress::Discarded(reason) => Some(reason),
            other => panic!("{other:?}"),
        };
        pump(router, |send| match send.port {
            BIP => Err(SendRefusal::Other),
            _ => port.send(send, now),
        });
        dropped
    };

    for (datagram, reason) in [
        (datagram(&[0x81, 0x0A], PEER, UNICAST), Discard::TooShort),
        (datagram(&long, PEER, UNICAST), Discard::LengthMismatch),
        (
            datagram(&good, PEER, TO_BROADCAST),
            Discard::DestinationMismatch,
        ),
        (unknown, Discard::UnknownDestination),
    ] {
        assert_eq!(drive(&datagram, &mut router, &mut out), Some(reason));
    }
    assert_eq!(router.counters(), Default::default());
    assert_eq!(router.table().len(), 2, "the two direct routes");
    assert_eq!(out.calls, 0);

    let well_formed = datagram(&good, PEER, UNICAST);
    assert_eq!(drive(&well_formed, &mut router, &mut out), None);
    assert_eq!(
        port.core_mut().counters().unicast.depth,
        1,
        "the well-formed one"
    );
}

/// Every way the MS/TP port can take or refuse a relay from B/IP, each
/// counted once by the router: taken; a full queue; taken though stamped
/// before the core's clock; expired at once on a core whose clock has run
/// out; and a send for another port. A core without extended frames, which
/// would take less than the port claims, is no port at all.
///
/// Ported from ACE IoT's
/// `real_mstp_success_full_oversize_expired_and_invalid_are_observed`.
#[test]
fn real_mstp_success_full_oversize_expired_and_invalid_are_observed() {
    let now = Instant::from_millis(10);
    let mut bip = bip_port(link(Mode::Normal));
    let mut router = router(&bip, None, Options::HOSTED);
    let mut host = bacnet_router_core::router::Permissive;
    let mut out = Wire::default();
    let mut relay = |router: &mut TestRouter,
                     npdu: &[u8],
                     each: &mut dyn FnMut(
        &bacnet_router_core::router::Send<'_>,
    ) -> Result<(), SendRefusal>| {
        let payload = encode(&Bvll::OriginalUnicastNpdu(npdu));
        let Ingress::Npdu(delivery) = into_bip(&mut bip, &payload, PEER, UNICAST, &mut out, now)
        else {
            panic!("not delivered");
        };
        let _ = router.receive(BIP_PORT.arrived(&delivery).frame(), now, &mut host);
        pump(router, each)
    };
    let short = routed(100, MSTP_NET, &[5]);
    let long = routed(600, MSTP_NET, &[5]);

    let mut bus = Bus::<1>::new();
    let mut port = bus.port();
    let mut to_core = |send: &bacnet_router_core::router::Send<'_>| port.send(send, now);
    assert_eq!(relay(&mut router, &short, &mut to_core), [(1, Ok(()))]);
    assert_eq!(
        relay(&mut router, &short, &mut to_core),
        [(1, Err(SendRefusal::Full))]
    );

    let mut bus = Bus::<1>::new();
    assert!(MstpPort::new(PortId(1), bus.core(false)).is_err());
    let mut bus = Bus::<1>::new();
    let mut port = bus.port();
    let mut to_core = |send: &bacnet_router_core::router::Send<'_>| port.send(send, now);
    assert_eq!(relay(&mut router, &long, &mut to_core), [(1, Ok(()))]);

    // A send stamped before the core's clock is taken at the core's time
    // (the core's clock rule), so it is queued.
    let mut bus = Bus::<1>::new();
    let mut ahead = bus.port();
    ahead.core_mut().on_deadline(Instant::from_millis(20));
    let mut to_ahead = |send: &bacnet_router_core::router::Send<'_>| ahead.send(send, now);
    assert_eq!(relay(&mut router, &short, &mut to_ahead), [(1, Ok(()))]);

    // A core whose clock has reached the end of time: an NPDU queued then
    // expires at once.
    let mut bus = Bus::<1>::new();
    let mut ended = bus.port();
    ended.core_mut().on_deadline(Instant::from_nanos(u64::MAX));
    let mut to_ended = |send: &bacnet_router_core::router::Send<'_>| ended.send(send, now);
    assert_eq!(
        relay(&mut router, &short, &mut to_ended),
        [(1, Err(SendRefusal::Other))]
    );

    let mut bus = Bus::<1>::new();
    let mut elsewhere = MstpPort::new(BIP, bus.core(true)).unwrap();
    let mut to_elsewhere = |send: &bacnet_router_core::router::Send<'_>| elsewhere.send(send, now);
    assert_eq!(
        relay(&mut router, &short, &mut to_elsewhere),
        [(1, Err(SendRefusal::Other))]
    );

    let counts = router.counters();
    assert_eq!((counts.refused, counts.egress_too_long), (3, 0));
}

/// Every way the B/IP port can take or refuse a relay from MS/TP, each
/// counted once by the router: taken; the socket full; too long for the
/// port's send budget; and the socket refusing for another reason.
///
/// Ported from ACE IoT's
/// `real_bip_success_and_admission_failures_are_observed_once`.
#[test]
fn real_bip_success_and_admission_failures_are_observed_once() {
    let now = Instant::from_millis(1);
    let small = with(link(Mode::Normal), |link| link.budget = Some(512));
    let mut bip = bip_port(small);
    // The router believes the port carries 1,497 octets.
    let mut router = router(&bip_port(link(Mode::Normal)), None, Options::HOSTED);
    let mut host = bacnet_router_core::router::Permissive;
    let mut out = Wire::default();
    let mut relay = |router: &mut TestRouter, npdu: &[u8], out: &mut Wire| {
        let arrived = mstp_arrived(7, STATION, npdu);
        let _ = router.receive(arrived.frame(), now, &mut host);
        pump(router, |send| BIP_PORT.send(&mut bip, send, now, out))
    };
    let short = routed(40, BIP_NET, &mac(DEVICE_44));
    let long = routed(600, BIP_NET, &mac(DEVICE_44));

    assert_eq!(relay(&mut router, &short, &mut out), [(0, Ok(()))]);
    assert_eq!(out.take().len(), 1);
    out.refuse = Some(node::SendRefusal::SocketFull);
    assert_eq!(
        relay(&mut router, &short, &mut out),
        [(0, Err(SendRefusal::Full))]
    );
    out.refuse = None;
    assert_eq!(
        relay(&mut router, &long, &mut out),
        [(0, Err(SendRefusal::TooLong))]
    );
    out.refuse = Some(node::SendRefusal::SocketError);
    assert_eq!(
        relay(&mut router, &short, &mut out),
        [(0, Err(SendRefusal::Other))]
    );

    let counts = router.counters();
    assert_eq!((counts.refused, counts.egress_too_long), (3, 1));
}

/// No relay from MS/TP reaches the B/IP socket as a unicast to one of the
/// link's group addresses. With the port's groups known, an
/// Unconfirmed-Request for a group at the link's own UDP port leaves as an
/// Original-Broadcast, and anything else for a group is dropped and
/// counted (#1504, #1677; decisions L19, L24), with the default host.
///
/// Ported from ACE IoT's
/// `mstp_six_byte_bip_group_destinations_never_reach_the_sink`.
#[test]
fn mstp_six_byte_bip_group_destinations_never_reach_the_sink() {
    let now = Instant::from_millis(1);
    let mut bip = bip_port(link(Mode::Normal));
    let mut router = router(&bip, None, Options::HOSTED);
    let mut groups = bacnet_router_core::router::Permissive;
    let mut out = Wire::default();
    let local = [
        SUBNET.broadcast,
        sock(255, 255, 255, 255, PORT),
        sock(239, 1, 2, 3, PORT),
    ];
    let elsewhere = [sock(192, 0, 2, 255, PORT + 1), sock(239, 1, 2, 3, PORT + 1)];

    let mut sent = Vec::new();
    for (group, apdu) in local
        .iter()
        .map(|group| (group, CONFIRMED))
        .chain(local.iter().map(|group| (group, UNCONFIRMED)))
        .chain(elsewhere.iter().map(|group| (group, UNCONFIRMED)))
    {
        let npdu = apdu_to(BIP_NET, &mac(*group), apdu);
        let _ = router.receive(mstp_arrived(7, STATION, &npdu).frame(), now, &mut groups);
        let results = pump(&mut router, |send| {
            BIP_PORT.send(&mut bip, send, now, &mut out)
        });
        assert!(results.iter().all(|(_, result)| result.is_ok()));
        sent.extend(out.take());
    }
    assert_eq!(sent.len(), local.len());
    for sent in &sent {
        assert_eq!(
            (sent.to, sent.function),
            (SUBNET.broadcast, BvlcFunction::ORIGINAL_BROADCAST_NPDU)
        );
        let relayed = Npdu::decode(&sent.npdu).unwrap();
        assert_eq!(relayed.payload, UNCONFIRMED);
        assert_eq!(
            relayed.source,
            Some(Address {
                network: MSTP_NET,
                mac: &[7]
            })
        );
    }
    assert_eq!(
        router.counters().group_dadr_drops,
        (local.len() + elsewhere.len()) as u32
    );
}
