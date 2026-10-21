//! A foreign device moved to another BBMD or time-to-live while it runs
//! ([`Node::retarget`], #1825): the registration lifecycle restarts there,
//! and a refused move changes nothing.

mod node_support;

use bacnet_bip_codec::Bvll;
use bacnet_bip_core::node::{
    Action, Admission, AttemptOutcome, ConfigError, ExchangeId, Foreign, Inbound, Mode, Node,
    Permissive,
};
use bacnet_bip_core::port::{HeldBroadcast, NodePort, PortError};
use bacnet_bip_core::Instant;
use node_support::*;

/// The BBMD the device moves to.
const NEW_BBMD: core::net::SocketAddrV4 = addr(10, 0, 1, 1, PORT);

const NPDU: &[u8] = &[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];

fn start(foreign: Foreign) -> Node {
    Node::new(foreign_link(foreign), Instant::ZERO).unwrap()
}

/// The registration the node asks for, sent at `now`: where and with what
/// time-to-live.
fn sent_attempt(
    node: &mut Node,
    now: Instant,
) -> Option<(ExchangeId, core::net::SocketAddrV4, u16)> {
    let mut attempt = None;
    for action in drain(node) {
        if let Action::Register { exchange, to, ttl } = action {
            assert!(!node.exchange_sent(exchange, now));
            attempt = Some((exchange, to, ttl));
        }
    }
    attempt
}

/// `from` answers with `code` at `now`.
fn says<'d>(
    node: &mut Node,
    payload: &'d [u8],
    from: core::net::SocketAddrV4,
    now: Instant,
) -> Inbound<'d> {
    node.receive(&datagram(payload, from, UNICAST), now, &mut Permissive)
}

#[test]
fn the_next_registration_goes_to_the_new_bbmd_with_the_new_ttl() {
    let mut node = start(foreign());
    assert!(sent_attempt(&mut node, at(0)).is_some());
    let success = result(SUCCESS);
    assert!(matches!(
        says(&mut node, &success, BBMD, at(10)),
        Inbound::Matched { .. }
    ));
    drain(&mut node);
    assert!(node.registration(at(10)).unwrap().registered);

    node.retarget(Foreign::new(NEW_BBMD, 120), at(20_000))
        .unwrap();
    assert_eq!(node.link().mode, Mode::Foreign(Foreign::new(NEW_BBMD, 120)));
    let (_, to, ttl) = sent_attempt(&mut node, at(20_000)).expect("an attempt at once");
    assert_eq!((to, ttl), (NEW_BBMD, 120));
    // The old BBMD's lease proves nothing about the new one.
    let view = node.registration(at(20_000)).unwrap();
    assert!(!view.registered && view.lease.is_none() && view.attempt_in_flight);

    // The new BBMD's success proves a lease of the new time-to-live.
    assert!(matches!(
        says(&mut node, &success, NEW_BBMD, at(20_010)),
        Inbound::Matched { settled: true, .. }
    ));
    drain(&mut node);
    let view = node.registration(at(20_010)).unwrap();
    assert_eq!(view.lease.map(span), Some((at(20_000), at(140_000))));
    assert_eq!(view.next_attempt, Some(at(80_000)));
    let last = node.take_counters().last_registration_attempt.unwrap();
    assert_eq!((last.bbmd, last.ttl), (NEW_BBMD, 120));
    assert_eq!(last.outcome, AttemptOutcome::Accepted);
}

#[test]
fn the_attempt_in_flight_is_cancelled_and_its_late_answer_matches_nothing() {
    let mut node = start(foreign());
    let (old, _, _) = sent_attempt(&mut node, at(0)).unwrap();
    node.retarget(Foreign::new(NEW_BBMD, 60), at(1_000))
        .unwrap();
    let (new, to, _) = sent_attempt(&mut node, at(1_000)).unwrap();
    assert_ne!(old, new);
    assert_eq!(to, NEW_BBMD);
    // The old BBMD answers the cancelled attempt: no lease from it.
    let success = result(SUCCESS);
    assert!(matches!(
        says(&mut node, &success, BBMD, at(1_010)),
        Inbound::Unmatched { peer: BBMD, .. }
    ));
    assert!(!node.registration(at(1_010)).unwrap().registered);
    // A move with an attempt admitted but not yet polled drops it unsent.
    node.retarget(Foreign::new(BBMD, 60), at(2_000)).unwrap();
    node.retarget(Foreign::new(NEW_BBMD, 90), at(2_000))
        .unwrap();
    let attempts: Vec<_> = drain(&mut node)
        .into_iter()
        .filter_map(|action| match action {
            Action::Register { to, ttl, .. } => Some((to, ttl)),
            _ => None,
        })
        .collect();
    assert_eq!(attempts, [(NEW_BBMD, 90)]);
}

#[test]
fn a_new_ttl_for_the_same_bbmd_keeps_its_lease() {
    let mut node = start(foreign());
    sent_attempt(&mut node, at(0)).unwrap();
    let success = result(SUCCESS);
    says(&mut node, &success, BBMD, at(10));
    drain(&mut node);
    node.retarget(Foreign::new(BBMD, 300), at(5_000)).unwrap();
    let (_, to, ttl) = sent_attempt(&mut node, at(5_000)).unwrap();
    assert_eq!((to, ttl), (BBMD, 300));
    let view = node.registration(at(5_000)).unwrap();
    assert!(view.registered);
    assert_eq!(view.lease.map(span), Some((at(0), at(60_000))));
}

/// An attempt admitted for the old BBMD but not yet polled is dropped even
/// when the new BBMD's can't start yet: here a manual Write-BDT to it holds
/// the success-answered slot.
#[test]
fn an_unpolled_attempt_for_the_old_bbmd_is_never_handed_out() {
    let mut node = start(foreign());
    let (attempt, _, _) = sent_attempt(&mut node, at(0)).unwrap();
    let success = result(SUCCESS);
    says(&mut node, &success, BBMD, at(10));
    drain(&mut node);
    // The renewal is admitted at 30 s, and not polled.
    node.on_deadline(at(30_000));
    let write =
        Bvll::WriteBroadcastDistributionTable(bacnet_bip_codec::BdtEntries::from_entries(&[]));
    node.begin_exchange(NEW_BBMD, write, ms(1_000), at(30_000))
        .unwrap();
    node.retarget(Foreign::new(NEW_BBMD, 60), at(30_000))
        .unwrap();
    let registers: Vec<_> = drain(&mut node)
        .into_iter()
        .filter(|action| matches!(action, Action::Register { .. }))
        .collect();
    assert!(registers.is_empty(), "{registers:?} after {attempt:?}");
    assert!(node
        .registration(at(30_000))
        .unwrap()
        .next_attempt
        .is_some());
}

#[test]
fn a_refused_move_changes_nothing() {
    let mut node = start(foreign());
    sent_attempt(&mut node, at(0)).unwrap();
    let before = (*node.link(), node.registration(at(100)));
    let self_bbmd = Foreign::new(OWN, 60);
    let no_ttl = with(Foreign::new(NEW_BBMD, 60), |f| f.ttl = 0);
    let group = Foreign::new(BROADCAST, 60);
    for (foreign, why) in [
        (self_bbmd, ConfigError::BbmdIsSelf),
        (no_ttl, ConfigError::Renewal),
        (group, ConfigError::BbmdAddress),
    ] {
        assert_eq!(node.retarget(foreign, at(100)), Err(why));
        assert_eq!((*node.link(), node.registration(at(100))), before);
        assert!(drain(&mut node).is_empty());
    }
    let mut normal = Node::<4>::new(link(Mode::Normal), Instant::ZERO).unwrap();
    assert_eq!(
        normal.retarget(foreign(), at(0)),
        Err(ConfigError::NotForeign)
    );
    assert_eq!(normal.link().mode, Mode::Normal);
}

#[test]
fn broadcasts_held_for_the_first_attempt_wait_for_the_new_bbmd() {
    let held = with(foreign(), |f| f.hold = 2);
    let mut node = start(held);
    sent_attempt(&mut node, at(0)).unwrap();
    assert_eq!(node.admit_broadcast(at(1)), Ok(Admission::Hold));
    node.retarget(with(Foreign::new(NEW_BBMD, 60), |f| f.hold = 2), at(2))
        .unwrap();
    sent_attempt(&mut node, at(2)).unwrap();
    assert_eq!(node.admit_broadcast(at(3)), Ok(Admission::Hold));
    let success = result(SUCCESS);
    says(&mut node, &success, NEW_BBMD, at(12));
    assert!(drain(&mut node).contains(&Action::ReleaseHeld));
}

#[test]
fn a_port_sends_the_moved_registration_and_refuses_what_its_slots_cant_hold() {
    let mut slots: [HeldBroadcast; 1] = [HeldBroadcast::EMPTY];
    let node = start(with(foreign(), |f| f.hold = 1));
    let mut port = NodePort::new(node, &mut slots).unwrap();
    let mut socket = Socket::default();
    port.start(at(0), &mut socket);
    assert_eq!(
        socket.take(),
        [(BBMD, wire(Bvll::RegisterForeignDevice { ttl: 60 }))]
    );
    assert_eq!(
        port.broadcast(NPDU, at(1), &mut socket),
        Ok(Admission::Hold)
    );

    let two = with(Foreign::new(NEW_BBMD, 60), |f| f.hold = 2);
    assert_eq!(
        port.retarget(two, at(2), &mut socket),
        Err(PortError::HeldStorage {
            needed: 2,
            available: 1
        })
    );
    assert_eq!(
        port.retarget(Foreign::new(OWN, 60), at(2), &mut socket),
        Err(PortError::Link(ConfigError::BbmdIsSelf))
    );
    assert!(socket.take().is_empty());

    let one = with(Foreign::new(NEW_BBMD, 45), |f| f.hold = 1);
    port.retarget(one, at(2), &mut socket).unwrap();
    assert_eq!(
        socket.take(),
        [(NEW_BBMD, wire(Bvll::RegisterForeignDevice { ttl: 45 }))]
    );
    // Its success releases the broadcast held since before the move, to the
    // new BBMD.
    let success = result(SUCCESS);
    port.receive(
        &datagram(&success, NEW_BBMD, UNICAST),
        at(12),
        &mut Permissive,
        &mut socket,
    );
    assert_eq!(
        socket.take(),
        [(NEW_BBMD, wire(Bvll::DistributeBroadcastToNetwork(NPDU)))]
    );
}

/// A move to another BBMD holds broadcasts again until it answers, since it
/// has no registration for the device yet and would refuse to distribute
/// them (Annex J.4.5); a new lifetime at the same BBMD doesn't.
#[test]
fn a_move_to_another_bbmd_holds_broadcasts_until_it_answers() {
    let holding = |bbmd| with(Foreign::new(bbmd, 60), |f| f.hold = 2);
    let mut node = start(holding(BBMD));
    sent_attempt(&mut node, at(0)).unwrap();
    let success = result(SUCCESS);
    says(&mut node, &success, BBMD, at(10));
    drain(&mut node);
    assert_eq!(node.admit_broadcast(at(20)), Ok(Admission::Send));

    node.retarget(with(holding(BBMD), |f| f.ttl = 120), at(30))
        .unwrap();
    sent_attempt(&mut node, at(30)).unwrap();
    assert_eq!(node.admit_broadcast(at(31)), Ok(Admission::Send));

    node.retarget(holding(NEW_BBMD), at(40)).unwrap();
    sent_attempt(&mut node, at(40)).unwrap();
    assert_eq!(node.admit_broadcast(at(41)), Ok(Admission::Hold));
    says(&mut node, &success, NEW_BBMD, at(50));
    assert!(drain(&mut node).contains(&Action::ReleaseHeld));
    assert_eq!(node.admit_broadcast(at(51)), Ok(Admission::Send));
}

/// A smaller hold than the broadcasts already held keeps them all.
#[test]
fn a_smaller_hold_keeps_the_broadcasts_already_held() {
    let mut node = start(with(foreign(), |f| f.hold = 2));
    sent_attempt(&mut node, at(0)).unwrap();
    for millis in [1, 2] {
        assert_eq!(node.admit_broadcast(at(millis)), Ok(Admission::Hold));
    }
    node.retarget(with(Foreign::new(NEW_BBMD, 60), |f| f.hold = 1), at(3))
        .unwrap();
    sent_attempt(&mut node, at(3)).unwrap();
    assert_eq!(node.registration(at(3)).unwrap().held, 2);
    assert_eq!(
        node.admit_broadcast(at(4)),
        Err(bacnet_bip_core::node::SendRefusal::HoldFull)
    );
    let success = result(SUCCESS);
    says(&mut node, &success, NEW_BBMD, at(13));
    assert!(drain(&mut node).contains(&Action::ReleaseHeld));
    assert_eq!(node.take_counters().broadcasts_released, 2);
}
