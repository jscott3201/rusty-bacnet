//! The router's own Router-Busy and Router-Available for a clogged egress
//! port, and the order its outputs leave in (#1697; Clauses 6.5.4,
//! 6.6.3.6, 6.6.3.7, 6.6.4).

mod fixture;

use bacnet_router_core::npdu::{NetworkMessageType, NetworkPriority, Npdu, RejectMessageReason};
use bacnet_router_core::router::{
    Action, Backlog, Cause, Congestion, Frame, Kind, Options, Permissive, SendRefusal,
};
use bacnet_router_core::{Instant, PortId};
use fixture::{
    addr, apdu_to, at, at_ms, bytes, drain, learn_9_via_r7, listed, port_on, router_with,
    sent_reject, two_ports, Sent, TestRouter, NODE_A, R7, UNCONFIRMED,
};

const BUSY: Kind = Kind::Congestion(NetworkMessageType::ROUTER_BUSY_TO_NETWORK);
const AVAILABLE: Kind = Kind::Congestion(NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK);

/// The hosted options with the default congestion policy.
fn congested_options() -> Options {
    fixture::options(|options| options.congestion = Some(Congestion::DEFAULT))
}

/// The standard router, with network 9 beyond R7 on port 1, under
/// `options`.
fn router(options: Options) -> TestRouter {
    let mut router = router_with(&two_ports(), options);
    learn_9_via_r7(&mut router, Instant::ZERO);
    router
}

/// Report port 1's backlog at `now` and poll what it prompted.
fn report(router: &mut TestRouter, queued: u32, now: Instant) -> Vec<Sent> {
    router.egress_backlog(PortId(1), Backlog::new(queued, 256), now);
    drain(router)
}

/// Port 1 clogged: full at t = 0 and still full 2 s later.
fn clogged() -> TestRouter {
    let mut router = router(congested_options());
    assert!(report(&mut router, 256, at(0)).is_empty());
    let busy = report(&mut router, 256, at(2));
    assert_eq!(busy.len(), 1);
    router
}

/// A unicast for station 5 on network 9, from node A on port 0, at `now`:
/// receive it and poll its forward toward R7, refusing it as `why` says.
fn forward_refused(router: &mut TestRouter, why: Option<SendRefusal>, now: Instant) -> Vec<Sent> {
    let npdu = apdu_to(9, &[5], UNCONFIRMED);
    let frame = Frame {
        port: PortId(0),
        source: NODE_A,
        link_group: false,
        npdu: &npdu,
    };
    let _ = router.receive(frame, now, &mut Permissive);
    let Action::Send(send) = router.poll_action() else {
        panic!("the forward");
    };
    assert_eq!((send.port, send.kind), (PortId(1), Kind::Forward));
    if let Some(why) = why {
        router.refused(why);
    }
    drain(router)
}

#[test]
fn off_by_default_nothing_is_announced_or_rejected() {
    let mut router = router(Options::HOSTED);
    for second in 0..60 {
        assert!(report(&mut router, 256, at(second)).is_empty());
    }
    assert!(!router.port_congested(PortId(1)));
    assert!(forward_refused(&mut router, Some(SendRefusal::Full), at(61)).is_empty());
    let counts = router.counters();
    assert_eq!((counts.busy_announced, counts.rejects), (0, 0));
}

#[test]
fn busy_after_clog_time_then_available_under_the_low_water_mark() {
    let mut router = router(congested_options());
    assert!(report(&mut router, 256, at(0)).is_empty());
    assert!(router.port_congested(PortId(1)), "waiting to clog");
    assert!(report(&mut router, 256, at_ms(1_999)).is_empty());

    let busy = report(&mut router, 256, at(2));
    assert_eq!(busy.len(), 1, "on every port but the clogged one");
    assert_eq!((busy[0].port, busy[0].kind), (0, BUSY));
    assert!(busy[0].is_broadcast());
    assert_eq!(busy[0].cause, Cause::Local);
    assert_eq!(busy[0].priority, NetworkPriority::Normal);
    // The networks reached through port 1: its own and R7's.
    assert_eq!(fixture::listed(&busy[0]), [2, 9]);
    let parsed = busy[0].parsed();
    assert_eq!((parsed.source, parsed.destination), (None, None));

    // Draining, but not yet under half: still clogged.
    assert!(report(&mut router, 200, at(3)).is_empty());
    assert!(report(&mut router, 128, at(4)).is_empty());
    let available = report(&mut router, 127, at(5));
    assert_eq!(available.len(), 1);
    assert_eq!((available[0].port, available[0].kind), (0, AVAILABLE));
    assert_eq!(listed(&available[0]), [2, 9]);
    assert!(!router.port_congested(PortId(1)));
    assert!(report(&mut router, 0, at(6)).is_empty(), "one Available");

    let counts = router.counters();
    assert_eq!((counts.busy_announced, counts.available_announced), (1, 1));
}

#[test]
fn a_dip_under_the_low_water_mark_restarts_the_wait() {
    let mut router = router(congested_options());
    assert!(report(&mut router, 256, at(0)).is_empty());
    assert!(report(&mut router, 10, at(1)).is_empty());
    assert!(!router.port_congested(PortId(1)));
    assert!(
        report(&mut router, 256, at(2)).is_empty(),
        "the wait starts again"
    );
    assert!(report(&mut router, 256, at_ms(3_999)).is_empty());
    assert_eq!(report(&mut router, 256, at(4))[0].kind, BUSY);
    // Not full, but over the low-water mark, the wait isn't reset either.
    let mut router = self::router(congested_options());
    let _ = report(&mut router, 256, at(0));
    assert!(report(&mut router, 255, at(1)).is_empty());
    assert_eq!(report(&mut router, 256, at(2))[0].kind, BUSY);
}

#[test]
fn a_lasting_clog_repeats_busy_and_drains_once() {
    let mut router = clogged();
    assert!(report(&mut router, 256, at(21)).is_empty());
    assert_eq!(report(&mut router, 256, at(22))[0].kind, BUSY);
    assert!(report(&mut router, 256, at(41)).is_empty());
    assert_eq!(router.counters().busy_announced, 2);
}

#[test]
fn a_full_queue_on_a_clogged_port_draws_reason_2() {
    let mut router = clogged();
    let sent = forward_refused(&mut router, Some(SendRefusal::Full), at(3));
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (0, Some(NODE_A)));
    assert_eq!(
        sent_reject(&sent[0]),
        (RejectMessageReason::ROUTER_BUSY, 9, None)
    );
    // Another reason, or an admitted send, draws none.
    assert!(forward_refused(&mut router, Some(SendRefusal::PortDown), at(3)).is_empty());
    assert!(forward_refused(&mut router, None, at(3)).is_empty());

    // Waiting to clog, and once drained, a refusal only counts.
    let mut router = router_waiting();
    assert!(forward_refused(&mut router, Some(SendRefusal::Full), at(1)).is_empty());
    let mut router = clogged();
    assert_eq!(report(&mut router, 0, at(3))[0].kind, AVAILABLE);
    assert!(forward_refused(&mut router, Some(SendRefusal::Full), at(4)).is_empty());
}

/// Port 1 at capacity since t = 0, not yet clogged.
fn router_waiting() -> TestRouter {
    let mut router = router(congested_options());
    assert!(report(&mut router, 256, at(0)).is_empty());
    router
}

#[test]
fn unreachable_networks_are_left_out_and_long_lists_split() {
    let mut router = router(congested_options());
    router.table_mut().mark_unreachable(9);
    let r7 = bacnet_router_core::npdu::Mac::from_slice(R7).unwrap();
    for network in 100..140 {
        assert!(router
            .table_mut()
            .add_learned(network, PortId(1), r7, Instant::ZERO));
    }
    // Port 0 carries broadcasts of 64 octets: 30 networks a message.
    let mut ports = two_ports();
    ports[0] = port_on(1, &[10, 0, 0, 1, 0xBA, 0xC0]);
    ports[0].max_npdu.broadcast = 64;
    let mut small = router_with(&ports, congested_options());
    for network in 100..140 {
        assert!(small
            .table_mut()
            .add_learned(network, PortId(1), r7, Instant::ZERO));
    }
    let _ = report(&mut small, 256, at(0));
    let busy = report(&mut small, 256, at(2));
    let lists: Vec<Vec<u16>> = busy.iter().map(listed).collect();
    let all: Vec<u16> = core::iter::once(2).chain(100..140).collect();
    assert_eq!(lists.len(), 2);
    assert_eq!(lists[0], all[..30]);
    assert_eq!(lists[1], all[30..]);
    assert!(busy.iter().all(|sent| sent.npdu.len() <= 64));

    let _ = report(&mut router, 256, at(0));
    let busy = report(&mut router, 256, at(2));
    let listed: Vec<u16> = busy.iter().flat_map(listed).collect();
    assert!(
        !listed.contains(&9),
        "a network failed for good isn't listed"
    );
    assert_eq!(listed[0], 2);
}

#[test]
fn a_call_drops_an_unpolled_announcement_and_counts_it() {
    let mut router = router(congested_options());
    router.egress_backlog(PortId(1), Backlog::new(256, 256), at(0));
    router.egress_backlog(PortId(1), Backlog::new(256, 256), at(2));
    router.start();
    let counts = router.counters();
    assert_eq!(counts.undrained, 1);
    let announcements = drain(&mut router);
    assert!(announcements.iter().all(|sent| sent.kind != BUSY));
}

/// Within one call, the frame's own output leaves first and the router's
/// own messages, all at Normal priority, after it: one port's sends never
/// put a lower priority ahead of a higher one (Clause 6.5.4).
#[test]
fn one_ports_sends_leave_in_priority_order() {
    for priority in [
        NetworkPriority::Normal,
        NetworkPriority::Urgent,
        NetworkPriority::CriticalEquipment,
        NetworkPriority::LifeSafety,
    ] {
        let mut router = router(congested_options());
        let _ = report(&mut router, 256, at(0));
        // A global broadcast from R7's side, at `priority`, fans out to
        // port 0; port 1 clogs between the polls, as a hosted drain
        // reports it, and its Router-Busy for port 0 queues behind.
        let mut npdu = Npdu::apdu(UNCONFIRMED);
        npdu.destination = Some(addr(0xFFFF, &[]));
        npdu.priority = priority;
        let frame = bytes(&npdu);
        let _ = router.receive(
            Frame {
                port: PortId(1),
                source: R7,
                link_group: true,
                npdu: &frame,
            },
            at(2),
            &mut Permissive,
        );
        let Action::Send(first) = router.poll_action() else {
            panic!("the fan-out");
        };
        assert_eq!((first.port, first.priority), (PortId(0), priority));
        router.egress_backlog(PortId(1), Backlog::new(256, 256), at(2));
        let rest = drain(&mut router);
        assert_eq!(rest.len(), 1);
        assert_eq!((rest[0].port, rest[0].kind), (0, BUSY));
        assert!(rest[0].priority <= priority);
    }
}
