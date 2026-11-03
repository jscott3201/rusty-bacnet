//! A foreign device whose BBMD isn't known yet ([`Foreign::bbmd`] `None`,
//! #2308), as when its host name hasn't resolved: it waits, with no
//! registration attempt due, holds broadcasts up to its hold and refuses
//! the rest, trusts no Forwarded-NPDU, and leaves the wait at its first
//! move to a known BBMD. Hand-set clocks throughout.

mod node_support;

use core::net::SocketAddrV4;

use bacnet_bip_codec::{BvlcResultCode, Bvll};
use bacnet_bip_core::Instant;
use bacnet_bip_core::node::{
    Action, Admission, Discard, Foreign, Inbound, Node, Permissive, Received, SendRefusal,
};
use bacnet_bip_core::port::{HeldBroadcast, NodePort};
use node_support::*;

const NPDU: &[u8] = &[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];

/// Registration for 60 s with no BBMD known yet, holding `hold`
/// broadcasts, and releasing the BBMD a move leaves.
fn waiting(hold: u16) -> Foreign {
    with(Foreign::waiting(60), |f| {
        f.hold = hold;
        f.release_previous = true;
    })
}

/// `foreign` with its BBMD known: `bbmd`.
fn at(foreign: Foreign, bbmd: SocketAddrV4) -> Foreign {
    with(foreign, |f| f.bbmd = Some(bbmd))
}

fn start(foreign: Foreign) -> Node {
    Node::new(foreign_link(foreign), Instant::ZERO).unwrap()
}

/// Every Register-Foreign-Device among `actions`, reported sent at `now`:
/// where each goes.
fn send_registers(node: &mut Node, now: Instant) -> Vec<SocketAddrV4> {
    let mut to = Vec::new();
    for action in drain(node) {
        if let Action::Register {
            exchange, to: bbmd, ..
        } = action
        {
            node.exchange_sent(exchange, now);
            to.push(bbmd);
        }
    }
    to
}

#[test]
fn a_node_without_a_bbmd_waits_with_no_attempt_due() {
    let mut node = start(waiting(2));
    assert!(drain(&mut node).is_empty());
    assert_eq!(node.next_deadline(), None);
    node.on_deadline(at_ms(3_600_000));
    assert!(drain(&mut node).is_empty());
    let view = node.registration(at_ms(10)).unwrap();
    assert!(!view.registered && !view.attempt_in_flight && !view.first_resolved);
    assert_eq!(view.next_attempt, None);
    assert_eq!(node.take_counters().registration_attempts, 0);
}

/// Broadcasts are held up to the hold and refused beyond it; with no hold,
/// every one is refused. The link has nowhere to send one.
#[test]
fn broadcasts_wait_in_the_hold_or_are_refused() {
    let mut node = start(waiting(2));
    assert_eq!(node.admit_broadcast(at_ms(1)), Ok(Admission::Hold));
    assert_eq!(node.admit_broadcast(at_ms(2)), Ok(Admission::Hold));
    assert_eq!(node.admit_broadcast(at_ms(3)), Err(SendRefusal::HoldFull));
    let counters = node.take_counters();
    assert_eq!(
        (counters.broadcasts_held, counters.broadcasts_hold_full),
        (2, 1)
    );
    assert_eq!(node.link().broadcast(NPDU), Err(SendRefusal::NoBbmd));

    let mut none = start(waiting(0));
    for millis in 1..=3 {
        assert_eq!(
            none.admit_broadcast(at_ms(millis)),
            Err(SendRefusal::HoldFull)
        );
    }
    assert_eq!(none.take_counters().broadcasts_hold_full, 3);
    // Once its BBMD is known, a node with no hold sends at once.
    none.retarget(at(waiting(0), BBMD), at_ms(4)).unwrap();
    assert_eq!(none.admit_broadcast(at_ms(5)), Ok(Admission::Send));
}

/// With no BBMD, no Forwarded-NPDU comes from it; and a
/// Distribute-Broadcast-To-Network NAK from anyone is no hint.
#[test]
fn nothing_is_trusted_or_taken_as_a_hint_while_waiting() {
    let mut node = start(waiting(0));
    let forwarded = wire(Bvll::ForwardedNpdu {
        origin: REMOTE,
        npdu: NPDU,
    });
    assert_eq!(
        receive(node.link(), &forwarded, BBMD, UNICAST),
        Received::Discarded(Discard::ForwardedUntrusted)
    );
    let nak = result(BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK);
    let datagram = datagram(&nak, BBMD, UNICAST);
    assert!(matches!(
        node.receive(&datagram, at_ms(5), &mut Permissive),
        Inbound::Unmatched { .. }
    ));
    let counters = node.take_counters();
    assert_eq!(
        (counters.renewal_hints, counters.unmatched_responses),
        (0, 1)
    );
    assert_eq!(node.next_deadline(), None);
}

/// The first move to a known BBMD ends the wait: its attempt goes at once,
/// there is nothing to release, and the broadcasts held while waiting go
/// out, in order, once that BBMD answers.
#[test]
fn the_first_move_to_a_known_bbmd_ends_the_wait() {
    let mut node = start(waiting(2));
    assert_eq!(node.admit_broadcast(at_ms(1)), Ok(Admission::Hold));
    // A new lifetime with no BBMD still waits.
    node.retarget(with(waiting(2), |f| f.ttl = 90), at_ms(2))
        .unwrap();
    assert!(drain(&mut node).is_empty());
    assert_eq!(node.registration(at_ms(2)).unwrap().held, 1);

    node.retarget(at(waiting(2), BBMD), at_ms(3)).unwrap();
    assert_eq!(send_registers(&mut node, at_ms(3)), [BBMD]);
    let counters = node.take_counters();
    assert_eq!(counters.previous_releases, 0);
    assert_eq!(counters.registration_attempts, 1);
    node.receive(
        &datagram(&result(SUCCESS), BBMD, UNICAST),
        at_ms(13),
        &mut Permissive,
    );
    assert!(drain(&mut node).contains(&Action::ReleaseHeld));
    let view = node.registration(at_ms(13)).unwrap();
    assert!(view.registered && view.first_resolved);
}

/// A move from a known BBMD to none waits again: no attempt is due, the
/// lease from the BBMD it left no longer counts, broadcasts are held again,
/// and that BBMD is released.
#[test]
fn a_move_to_no_bbmd_waits_again() {
    let mut node = start(at(waiting(1), BBMD));
    assert_eq!(send_registers(&mut node, at_ms(0)), [BBMD]);
    node.receive(
        &datagram(&result(SUCCESS), BBMD, UNICAST),
        at_ms(10),
        &mut Permissive,
    );
    drain(&mut node);
    node.retarget(waiting(1), at_ms(20)).unwrap();
    let actions = drain(&mut node);
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, Action::Register { .. }))
    );
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, Action::ReleasePrevious { to: BBMD, .. }))
    );
    let view = node.registration(at_ms(20)).unwrap();
    assert!(!view.registered && view.lease.is_none());
    assert_eq!(view.next_attempt, None);
    assert_eq!(node.admit_broadcast(at_ms(21)), Ok(Admission::Hold));
}

/// A port holds a waiting node's broadcast in its slot and refuses the one
/// past it, then sends the registration as the node moves to a known BBMD,
/// and the held broadcast to that BBMD once it answers.
#[test]
fn a_port_holds_while_waiting_and_registers_after_the_move() {
    let mut slots: [HeldBroadcast; 1] = [HeldBroadcast::EMPTY];
    let mut port = NodePort::new(start(waiting(1)), &mut slots).unwrap();
    let mut socket = Socket::default();
    port.start(at_ms(0), &mut socket);
    assert!(socket.take().is_empty());
    assert_eq!(
        port.broadcast(NPDU, at_ms(1), &mut socket),
        Ok(Admission::Hold)
    );
    assert_eq!(
        port.broadcast(NPDU, at_ms(2), &mut socket),
        Err(SendRefusal::HoldFull)
    );
    // Too long for the link: refused before it is held, as ever.
    let long = vec![0; 1498];
    assert!(matches!(
        port.broadcast(&long, at_ms(2), &mut socket),
        Err(SendRefusal::TooLong { .. })
    ));
    assert!(socket.take().is_empty());

    port.retarget(at(waiting(1), BBMD), at_ms(3), &mut socket)
        .unwrap();
    assert_eq!(
        socket.take(),
        [(BBMD, wire(Bvll::RegisterForeignDevice { ttl: 60 }))]
    );
    let success = result(SUCCESS);
    port.receive(
        &datagram(&success, BBMD, UNICAST),
        at_ms(13),
        &mut Permissive,
        &mut socket,
    );
    assert_eq!(
        socket.take(),
        [(BBMD, wire(Bvll::DistributeBroadcastToNetwork(NPDU)))]
    );
}

/// `millis` milliseconds after the epoch.
fn at_ms(millis: u64) -> Instant {
    node_support::at(millis)
}
