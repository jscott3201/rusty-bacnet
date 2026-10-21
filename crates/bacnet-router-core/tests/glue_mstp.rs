//! The MS/TP glue (feature `mstp`): a router's MS/TP port over
//! `bacnet-mstp-core`'s master core.

#![cfg(feature = "mstp")]

mod fixture;
mod glue;

use bacnet_mstp_codec::{Frame as MstpFrame, FrameType, BROADCAST_MAC, MAX_EXTENDED_FRAME_LENGTH};
use bacnet_mstp_core::{
    Action, DequeuedNpdu, ExtendedActionBuffers, Instant, MasterCore, QueueCounters, QueueItemInfo,
    QueueKind, QueueRefusal, QueueStorage, ReplyDecision, RequestError, RequestId,
};
use bacnet_router_core::mstp::{MstpPort, NotExtended, PortAction};
use bacnet_router_core::npdu::Mac;
use bacnet_router_core::npdu::Npdu;
use bacnet_router_core::npdu::NpduLimits;
use bacnet_router_core::router::{Dest, Group, GroupClassifier, Options, Received, SendRefusal};
use bacnet_router_core::PortId;

use bacnet_router_core::npdu::NetworkPriority;
use fixture::*;
use glue::*;

/// The router, its MS/TP port (port 1, on network 2) configured from the
/// core's configuration. Port 0 is a B/IP-like port on network 1.
fn router() -> TestRouter {
    let mstp = bacnet_router_core::mstp::port_config(&config(true), 2).unwrap();
    router_with(&[port_on(1, OWN_0), mstp], Options::HOSTED)
}

/// An APDU for `dnet`/`dadr` at network priority `priority`.
fn apdu_at(dnet: u16, dadr: &[u8], apdu: &[u8], priority: u8) -> Vec<u8> {
    let mut npdu = Npdu::apdu(apdu);
    npdu.destination = Some(addr(dnet, dadr));
    npdu.priority = NetworkPriority::try_from(priority).unwrap();
    bytes(&npdu)
}

/// What the port hands the router next: the source station, the
/// destination (255 for a link broadcast, else this station), the request
/// and the NPDU.
fn delivered<const U: usize>(port: &mut Port<'_, U>) -> (u8, u8, Option<RequestId>, Vec<u8>) {
    loop {
        match port.poll_action() {
            PortAction::Arrived { arrived, request } => {
                assert_eq!(arrived.port, PortId(1));
                let destination = if arrived.link_group {
                    BROADCAST_MAC
                } else {
                    STATION
                };
                return (
                    arrived.source.as_slice()[0],
                    destination,
                    request,
                    arrived.npdu.to_vec(),
                );
            }
            PortAction::Core(Action::SetDirection(_)) => {}
            other => panic!("expected a delivery, got {other:?}"),
        }
    }
}

#[test]
fn config_needs_extended_frames() {
    assert_eq!(
        bacnet_router_core::mstp::port_config(&config(false), 2),
        Err(NotExtended)
    );
    let port = bacnet_router_core::mstp::port_config(&config(true), 2).unwrap();
    assert_eq!(port.network, 2);
    assert_eq!(port.mac, Mac::from_array([STATION]));
    assert_eq!(port.max_npdu, NpduLimits::both(1497));
}

/// The port owns its core: built only over one with extended frames, it
/// reads its configuration from it, hands each NPDU up as the router's
/// frame and every other action as the core's, and gives the core back.
#[test]
fn the_port_owns_its_core() {
    let mut bus = Bus::<1>::new();
    assert!(matches!(
        MstpPort::new(PortId(1), bus.core(false)),
        Err(NotExtended)
    ));
    let mut port = bus.port();
    assert_eq!(port.port(), PortId(1));
    assert_eq!(
        port.port_config(2),
        bacnet_router_core::mstp::port_config(&config(true), 2).unwrap()
    );
    let now = Instant::from_millis(1);
    let npdu = [0x01, 0x00, 0x10, 0x08];
    let frame = MstpFrame::new(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        STATION,
        7,
        &npdu,
    );
    port.core_mut().on_validated_frame(frame, now);
    assert_eq!(
        port.poll_action(),
        PortAction::Arrived {
            arrived: mstp_arrived(7, STATION, &npdu),
            request: None,
        }
    );
    assert_eq!(port.poll_action(), PortAction::Core(Action::Idle));
    assert_eq!(port.into_core().counters().npdus_delivered, 1);
}

#[test]
fn the_broadcast_station_is_the_only_group() {
    assert_eq!(
        bacnet_router_core::mstp::port_config(&config(true), 2)
            .unwrap()
            .groups
            .classify(&[BROADCAST_MAC]),
        Group::Broadcast
    );
    assert_eq!(
        bacnet_router_core::mstp::port_config(&config(true), 2)
            .unwrap()
            .groups
            .classify(&[5]),
        Group::No
    );
    assert_eq!(
        bacnet_router_core::mstp::port_config(&config(true), 2)
            .unwrap()
            .groups
            .classify(&[]),
        Group::No
    );
}

/// A broadcast on the MS/TP port reaches the router as a link broadcast
/// from its station, and a directed frame as a unicast.
#[test]
fn deliveries_become_frames() {
    let now = Instant::from_millis(1);
    let mut bus = Bus::<2>::new();
    let mut port = bus.port();
    let mut router = router();
    let mut host = Recorder::default();

    let global = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];
    let frame = MstpFrame::new(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        BROADCAST_MAC,
        7,
        &global,
    );
    port.core_mut().on_validated_frame(frame, now);
    let (source, destination, request, npdu) = delivered(&mut port);
    let arrived = mstp_arrived(source, destination, &npdu);
    assert_eq!(
        (
            arrived.port,
            arrived.source.as_slice(),
            arrived.link_group,
            request
        ),
        (PortId(1), &[7][..], true, None)
    );
    assert!(matches!(
        router.receive(arrived.frame(), now, &mut host),
        Received::Local(local) if local.global
    ));
    assert_eq!(drain(&mut router).len(), 1, "relayed to network 1");

    let directed = mstp_arrived(7, STATION, &npdu);
    assert!(!directed.link_group);
}

/// A request for another network gets Reply Postponed at once; one the
/// router's own application answered keeps its answer (Clauses 6.5.4,
/// 9.5.6.9).
#[test]
fn relayed_requests_are_postponed() {
    let now = Instant::from_millis(1);
    let mut bus = Bus::<2>::new();
    let mut port = bus.port();
    let mut router = router();
    let mut host = Recorder::default();

    let mut request = Npdu::apdu(CONFIRMED);
    request.destination = Some(addr(1, NODE_A));
    request.expecting_reply = true;
    let request = bytes(&request);
    let frame = MstpFrame::new(FrameType::BACNET_DATA_EXPECTING_REPLY, STATION, 7, &request);
    port.core_mut().on_validated_frame(frame, now);
    let (source, destination, id, npdu) = delivered(&mut port);
    assert!(id.is_some());
    let arrived = mstp_arrived(source, destination, &npdu);
    assert_eq!(
        router.receive(arrived.frame(), now, &mut host),
        Received::Handled
    );
    let sent = drain(&mut router);
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (0, Some(NODE_A)));

    port.release(id, now).unwrap();
    assert_eq!(port.core_mut().snapshot().request_id, None);
    let answer = next_frame(port.core_mut(), now).unwrap();
    assert_eq!(
        (answer.frame_type, answer.destination),
        (FrameType::REPLY_POSTPONED, 7)
    );

    // A request for the router's own application, which answers it.
    let mut local = Npdu::apdu(CONFIRMED);
    local.expecting_reply = true;
    let local = bytes(&local);
    let later = now + bacnet_router_core::Duration::from_millis(100);
    let frame = MstpFrame::new(FrameType::BACNET_DATA_EXPECTING_REPLY, STATION, 7, &local);
    port.core_mut().on_validated_frame(frame, later);
    let (source, destination, id, npdu) = delivered(&mut port);
    let arrived = mstp_arrived(source, destination, &npdu);
    assert!(matches!(
        router.receive(arrived.frame(), later, &mut host),
        Received::Local(local) if local.expecting_reply
    ));
    let reply = [0x30, 0x05, 0x0C];
    port.core_mut()
        .complete_request(id.unwrap(), ReplyDecision::Data(&reply), later)
        .unwrap();
    port.release(id, later).unwrap();
    port.release(None, later).unwrap();
    let answer = next_frame(port.core_mut(), later).unwrap();
    assert_eq!(
        (answer.frame_type, answer.destination, answer.data),
        (
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            7,
            reply.to_vec()
        )
    );
    // A release stamped before the core's clock is taken at the core's
    // time (the core's clock rule); a stale request is still nothing to
    // answer.
    let stale = RequestId::new(99);
    port.release(Some(stale), Instant::ZERO).unwrap();
    // The core's other refusals come back as they are: a request released
    // once its requester may have stopped waiting.
    let late = later + bacnet_router_core::Duration::from_millis(10);
    let frame = MstpFrame::new(FrameType::BACNET_DATA_EXPECTING_REPLY, STATION, 7, &local);
    port.core_mut().on_validated_frame(frame, late);
    let (.., id, _) = delivered(&mut port);
    let gone = late + bacnet_router_core::Duration::from_millis(300);
    assert_eq!(port.release(id, gone), Err(RequestError::Expired));
}

/// The release of a request the application answered, made once the next
/// request is already waiting, leaves that one alone; its own release then
/// answers it Reply Postponed.
#[test]
fn a_late_release_leaves_the_next_request_alone() {
    let now = Instant::from_millis(1);
    let mut bus = Bus::<2>::new();
    let mut port = bus.port();
    let mut local = Npdu::apdu(CONFIRMED);
    local.expecting_reply = true;
    let local = bytes(&local);
    let request = MstpFrame::new(FrameType::BACNET_DATA_EXPECTING_REPLY, STATION, 7, &local);

    port.core_mut().on_validated_frame(request, now);
    let (.., first, _) = delivered(&mut port);
    let reply = [0x30, 0x05, 0x0C];
    port.core_mut()
        .complete_request(first.unwrap(), ReplyDecision::Data(&reply), now)
        .unwrap();
    let answer = next_frame(port.core_mut(), now).unwrap();
    assert_eq!(
        answer.frame_type,
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
    );

    let later = now + bacnet_router_core::Duration::from_millis(100);
    port.core_mut().on_validated_frame(request, later);
    let (.., second, _) = delivered(&mut port);
    assert_ne!(first, second);
    port.release(first, later).unwrap();
    assert_eq!(
        port.core_mut().snapshot().request_id,
        second,
        "still waiting"
    );
    port.release(second, later).unwrap();
    let answer = next_frame(port.core_mut(), later).unwrap();
    assert_eq!(
        (answer.frame_type, answer.destination),
        (FrameType::REPLY_POSTPONED, 7)
    );
}

/// What the router relays at life-safety priority leaves the MS/TP queues
/// before what it relayed earlier at normal priority.
#[test]
fn priority_reaches_the_queue() {
    let now = Instant::from_millis(1);
    let mut bus = Bus::<2>::new();
    let normal = apdu_at(2, &[5], CONFIRMED, 0);
    let urgent = apdu_at(2, &[6], CONFIRMED, 3);
    {
        let mut port = bus.port();
        let mut router = router();
        let mut host = Recorder::default();
        for npdu in [&normal, &urgent] {
            let _ = router.receive(frame_on(0, NODE_A, npdu), now, &mut host);
            let results = pump(&mut router, |send| port.send(send, now));
            assert_eq!(results, [(1, Ok(()))]);
        }
    }
    let first = bus.queues().dequeue(now, 1497).unwrap().unwrap();
    assert_eq!(
        (first.destination, first.priority),
        (6, NetworkPriority::LifeSafety)
    );
    let second = bus.queues().dequeue(now, 1497).unwrap().unwrap();
    assert_eq!(
        (second.destination, second.priority),
        (5, NetworkPriority::Normal)
    );
}

/// Each refusal reaches the router as its own, and is counted there once.
#[test]
fn refusals_map_to_the_routers() {
    let now = Instant::from_millis(1);
    // Deliberately omit the group rule to exercise the link's defense too.
    let mut mstp = bacnet_router_core::mstp::port_config(&config(true), 2).unwrap();
    mstp.groups = bacnet_router_core::router::Groups::None;
    let mut router = router_with(&[port_on(1, OWN_0), mstp], Options::HOSTED);
    let mut host = Recorder::default();
    let mut route = |router: &mut TestRouter, npdu: &[u8], port: &mut Port<'_, 1>| {
        let _ = router.receive(frame_on(0, NODE_A, npdu), now, &mut host);
        pump(router, |send| port.send(send, now))
    };

    // One unicast slot: the second waits for none.
    let mut bus = Bus::<1>::new();
    let mut port = bus.port();
    let fits = apdu_at(2, &[5], CONFIRMED, 0);
    assert_eq!(route(&mut router, &fits, &mut port), [(1, Ok(()))]);
    assert_eq!(
        route(&mut router, &fits, &mut port),
        [(1, Err(SendRefusal::Full))]
    );
    // Station 255 is no unicast destination, nor is a two-octet MAC.
    for dadr in [&[BROADCAST_MAC][..], &[1, 2]] {
        let npdu = apdu_at(2, dadr, CONFIRMED, 0);
        assert_eq!(
            route(&mut router, &npdu, &mut port),
            [(1, Err(SendRefusal::Other))]
        );
    }

    // No port is built over a core without extended frames, so none can
    // take less than the 1,497 octets its configuration claims.
    let mut bus = Bus::<1>::new();
    assert!(matches!(
        MstpPort::new(PortId(1), bus.core(false)),
        Err(NotExtended)
    ));
    // A send for another port.
    let mut bus = Bus::<1>::new();
    let mut other = MstpPort::new(PortId(0), bus.core(true)).unwrap();
    let _ = router.receive(frame_on(0, NODE_A, &fits), now, &mut host);
    let results = pump(&mut router, |send| other.send(send, now));
    assert_eq!(results, [(1, Err(SendRefusal::Other))]);

    let counts = router.counters();
    assert_eq!((counts.refused, counts.egress_too_long), (4, 0));
}

/// A frame port `port` received from `source`, by unicast.
fn frame_on<'f>(
    port: u8,
    source: &'f [u8],
    npdu: &'f [u8],
) -> bacnet_router_core::router::Frame<'f> {
    bacnet_router_core::router::Frame {
        port: PortId(port),
        source,
        link_group: false,
        npdu,
    }
}

/// An APDU from node A on network 1 for station [`NEXT`] on network 2,
/// whose relay onto MS/TP is `len` octets: the router drops the DNET, DADR
/// and hop count (7 octets of NPCI with the version) and adds SNET 1 and
/// A's six-octet SADR (11).
fn relayed_as(len: usize) -> Vec<u8> {
    let mut apdu = vec![0x5A; len - 11];
    apdu[0] = 0x10;
    apdu_to(2, &[NEXT], &apdu)
}

/// The router relays an NPDU of `len` octets onto MS/TP, in one extended
/// frame that a station taking them delivers whole.
fn relayed_in_one_extended_frame(len: usize) {
    let now = Instant::from_millis(1);
    let mut bus = Bus::<1>::new();
    let mut port = bus.port();
    let mut router = router();
    let npdu = relayed_as(len);
    let _ = router.receive(frame_on(0, NODE_A, &npdu), now, &mut Recorder::default());
    let results = pump(&mut router, |send| port.send(send, now));
    assert_eq!(results, [(1, Ok(()))]);

    let wired = token_then_frame(port.core_mut(), now).unwrap();
    assert_eq!(
        (wired.frame_type, wired.destination, wired.data.len()),
        (
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            NEXT,
            len
        )
    );
    assert_eq!(wired.data[7..11], NODE_A[2..], "the SADR names node A");
    assert_eq!(wired.data[11..], npdu[7..], "the APDU travels whole");

    let mut peer_bus = Bus::<1>::new();
    let mut peer = peer_bus.core_at(NEXT, true);
    for &octet in &wired.octets {
        peer.on_octet(octet, now);
    }
    let delivered = loop {
        match peer.poll_action() {
            Action::DeliverNpdu {
                source,
                destination,
                npdu,
                ..
            } => break (source, destination, npdu.to_vec()),
            Action::SetDirection(_) => {}
            other => panic!("expected the relay, got {other:?}"),
        }
    };
    assert_eq!(delivered, (STATION, NEXT, wired.data));
}

/// The largest NPDU B/IP carries, 1,497 octets, crosses onto MS/TP whole,
/// in one extended frame (Clause 9.3, Table 6-1).
///
/// Ported from ACE IoT's bacnet-router-adapters
/// (ACE-IoT-Solutions/rusty-bacnet#5), thanks @acedrew, where it is
/// `extended_profile_admits_1497_bytes_and_emits_decodable_extended_frame`.
#[test]
fn extended_profile_admits_1497_bytes_and_emits_decodable_extended_frame() {
    relayed_in_one_extended_frame(1497);
}

/// The shortest NPDU that needs an extended frame, 502 octets, goes in
/// one.
///
/// Ported from ACE IoT's bacnet-router-adapters
/// (ACE-IoT-Solutions/rusty-bacnet#5), thanks @acedrew, where it is
/// `extended_profile_admits_the_first_extended_boundary`.
#[test]
fn extended_profile_admits_the_first_extended_boundary() {
    relayed_in_one_extended_frame(502);
}

/// Queues that claim extended slots but refuse an NPDU over 1,000 octets as
/// oversize: queue storage of the caller's own whose limit is below what
/// the router's port carries.
struct Narrow(Queues<1>);

impl QueueStorage for Narrow {
    fn set_starvation_limit(&mut self, limit: u8) {
        self.0.set_starvation_limit(limit);
    }
    fn max_npdu(&self) -> usize {
        self.0.max_npdu()
    }
    fn enqueue_npdu(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: Instant,
        expires_at: Option<Instant>,
    ) -> Result<(), QueueRefusal> {
        if payload.len() > 1000 {
            return Err(QueueRefusal::Oversize);
        }
        self.0
            .enqueue_npdu(destination, payload, priority, now, expires_at)
    }
    fn expire(&mut self, now: Instant) -> usize {
        self.0.expire(now)
    }
    fn peek(&self, now: Instant) -> Option<QueueItemInfo> {
        self.0.peek(now)
    }
    fn peek_kind(&self, kind: QueueKind, now: Instant) -> Option<QueueItemInfo> {
        self.0.peek_kind(kind, now)
    }
    fn dequeue_kind(
        &mut self,
        kind: QueueKind,
        now: Instant,
        max_len: usize,
    ) -> Result<Option<DequeuedNpdu<'_>>, QueueRefusal> {
        self.0.dequeue_kind(kind, now, max_len)
    }
    fn depth(&self, kind: QueueKind) -> usize {
        self.0.depth(kind)
    }
    fn counters(&self, kind: QueueKind) -> QueueCounters {
        self.0.counters(kind)
    }
    fn take_counters(&mut self, kind: QueueKind) -> QueueCounters {
        self.0.take_counters(kind)
    }
}

/// A queue refusal as oversize reaches the router as
/// [`SendRefusal::TooLong`], and nothing is queued: here from queues of the
/// caller's own that take less than the 1,497 octets the port carries.
#[test]
fn an_oversize_queue_refusal_is_too_long_for_the_router() {
    let now = Instant::from_millis(1);
    let mut router = router();
    let mut rx = vec![0; MAX_EXTENDED_FRAME_LENGTH];
    let mut actions: Box<ExtendedActionBuffers<1>> = Box::default();
    let core = MasterCore::from_parts(
        config(true),
        Narrow(Queues::new()),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let mut port = MstpPort::new(PortId(1), core).unwrap();
    let npdu = relayed_as(1100);
    let _ = router.receive(frame_on(0, NODE_A, &npdu), now, &mut Recorder::default());
    let results = pump(&mut router, |send| port.send(send, now));
    assert_eq!(results, [(1, Err(SendRefusal::TooLong))]);
    assert_eq!(port.core().counters().unicast.depth, 0);
    assert_eq!(router.counters().egress_too_long, 1);
}

/// A router's MS/TP port has no standard-frame profile: a core
/// configuration without extended frames is refused (Clause 9.3). Where a
/// port is held to 501 octets anyway, the router keeps a 502-octet NPDU from it
/// with reject reason 4 to its sender (#1665), and nothing is queued; a port
/// over a core without extended frames is refused.
///
/// Ported from ACE IoT's bacnet-router-adapters
/// (ACE-IoT-Solutions/rusty-bacnet#5), thanks @acedrew, where it is
/// `standard_profile_rejects_502_bytes_before_queue_mutation` and the
/// standard profile is an adapter setting.
#[test]
fn standard_profile_rejects_502_bytes_before_queue_mutation() {
    let now = Instant::from_millis(1);
    let mut host = Recorder::default();
    assert_eq!(
        bacnet_router_core::mstp::port_config(&config(false), 2),
        Err(NotExtended)
    );
    let npdu = relayed_as(502);

    // The router's own check, on a port held to 501 octets.
    let mut held_to_501 = bacnet_router_core::mstp::port_config(&config(true), 2).unwrap();
    held_to_501.max_npdu = NpduLimits::both(501);
    let mut router = router_with(&[port_on(1, OWN_0), held_to_501], Options::HOSTED);
    let mut bus = Bus::<1>::new();
    let mut port = bus.port();
    let mut rejects = Vec::new();
    let _ = router.receive(frame_on(0, NODE_A, &npdu), now, &mut host);
    let results = pump(&mut router, |send| match send.port {
        PortId(0) => {
            let to = match send.to {
                Dest::Unicast(mac) => Some(mac.to_vec()),
                Dest::Broadcast => None,
            };
            rejects.push((to, send.npdu.to_vec()));
            Ok(())
        }
        _ => port.send(send, now),
    });
    assert_eq!(results, [(0, Ok(()))]);
    let (to, reject) = &rejects[0];
    assert_eq!(to.as_deref(), Some(NODE_A));
    let reject = Npdu::decode(reject).unwrap();
    assert_eq!(reject.payload, [4, 0, 2]);
    assert_eq!(router.counters().egress_too_long, 1);
    assert_eq!(port.core_mut().counters().unicast.depth, 0);

    // A core without extended frames can't stand behind a port claiming
    // 1,497: the port refuses it.
    let mut bus = Bus::<1>::new();
    assert!(matches!(
        MstpPort::new(PortId(1), bus.core(false)),
        Err(NotExtended)
    ));
}

/// A full MS/TP queue refuses the next part of a routing-table answer, and
/// the router sends no more of it: the refusal is counted once, and the
/// rest of the answer never comes.
///
/// Ported from ACE IoT's bacnet-router-adapters
/// (ACE-IoT-Solutions/rusty-bacnet#5), thanks @acedrew, where it is
/// `saturated_queue_backpressures_network_control`.
#[test]
fn saturated_queue_backpressures_network_control() {
    let now = Instant::from_millis(1);
    let mut router = router();
    // 250 learned routes and the two direct ones: three Acks' worth.
    let via = Mac::from_slice(NODE_A).unwrap();
    for network in 100..350 {
        assert!(router.table_mut().add_learned(network, PortId(0), via, now));
    }
    let mut bus = Bus::<1>::new();
    let mut port = bus.port();
    let query = message(
        bacnet_router_core::npdu::NetworkMessageType::INITIALIZE_ROUTING_TABLE,
        &[0],
    );
    let arrived = mstp_arrived(7, STATION, &query);
    let _ = router.receive(arrived.frame(), now, &mut Recorder::default());
    let results = pump(&mut router, |send| port.send(send, now));
    assert_eq!(results, [(1, Ok(())), (1, Err(SendRefusal::Full))]);
    assert_eq!(router.counters().refused, 1);
    assert_eq!(port.core_mut().counters().unicast.depth, 1);

    // With room, the whole answer goes: three parts.
    let mut bus = Bus::<4>::new();
    let mut port = bus.port();
    let _ = router.receive(arrived.frame(), now, &mut Recorder::default());
    let results = pump(&mut router, |send| port.send(send, now));
    assert_eq!(results, [(1, Ok(())), (1, Ok(())), (1, Ok(()))]);
}
