//! A foreign device that moves to another BBMD asks the one it left to
//! delete its entry ([`Foreign::release_previous`], #2307): one
//! Delete-Foreign-Device-Table-Entry for its own address (Annex J.2.9),
//! best effort, with its outcome counted and reported as an event. The old
//! BBMD answers with success or its NAK, or stays silent (J.5.2.1.2); none
//! of the three changes the move. Hand-set clocks throughout.

mod node_support;

use core::net::SocketAddrV4;

use bacnet_bip_codec::{BvlcResultCode, Bvll};
use bacnet_bip_core::Instant;
use bacnet_bip_core::node::{
    Action, AttemptOutcome, ExchangeId, Foreign, Inbound, LocalFailure, Node, NodeCounters,
    Permissive, RegistrationEvent,
};
use bacnet_bip_core::port::NodePort;
use node_support::*;

/// The BBMD the device moves to.
const NEW_BBMD: SocketAddrV4 = addr(10, 0, 1, 1, PORT);

const DELETE_NAK: BvlcResultCode = BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK;

/// Registration with `bbmd` for 60 s, releasing the BBMD a move leaves.
fn releasing(bbmd: SocketAddrV4) -> Foreign {
    with(Foreign::new(bbmd, 60), |foreign| {
        foreign.release_previous = true
    })
}

/// A node registered with [`BBMD`] at 10 ms, its counters taken.
fn registered<const X: usize>() -> Node<X> {
    let mut node = Node::<X>::new(foreign_link(releasing(BBMD)), Instant::ZERO).unwrap();
    send_all(&mut node, at(0));
    says(&mut node, &result(SUCCESS), BBMD, at(10));
    drain(&mut node);
    assert!(node.registration(at(10)).unwrap().registered);
    let _ = node.take_counters();
    node
}

/// Report every request the node asks for sent at `now`, and return the
/// actions in order.
fn send_all<const X: usize>(node: &mut Node<X>, now: Instant) -> Vec<Action> {
    let actions = drain(node);
    for action in &actions {
        if let Action::Register { exchange, .. } | Action::ReleasePrevious { exchange, .. } =
            *action
        {
            node.exchange_sent(exchange, now);
        }
    }
    actions
}

fn says<'d, const X: usize>(
    node: &mut Node<X>,
    payload: &'d [u8],
    from: SocketAddrV4,
    now: Instant,
) -> Inbound<'d> {
    node.receive(&datagram(payload, from, UNICAST), now, &mut Permissive)
}

/// The release among `actions`, if any.
fn release_of(actions: &[Action]) -> Option<(ExchangeId, SocketAddrV4, SocketAddrV4)> {
    actions.iter().find_map(|action| match *action {
        Action::ReleasePrevious {
            exchange,
            to,
            entry,
        } => Some((exchange, to, entry)),
        _ => None,
    })
}

/// The releases' ends among `actions`: where each went and what came of it.
fn ends(actions: &[Action]) -> Vec<(SocketAddrV4, AttemptOutcome)> {
    actions
        .iter()
        .filter_map(|action| match *action {
            Action::Registration(RegistrationEvent::PreviousRelease { bbmd, outcome }) => {
                Some((bbmd, outcome))
            }
            _ => None,
        })
        .collect()
}

/// Where each Register-Foreign-Device among `actions` goes.
fn registers(actions: &[Action]) -> Vec<SocketAddrV4> {
    actions
        .iter()
        .filter_map(|action| match *action {
            Action::Register { to, .. } => Some(to),
            _ => None,
        })
        .collect()
}

/// The four release counts.
fn releases(counters: &NodeCounters) -> [u32; 4] {
    [
        counters.previous_releases,
        counters.previous_released,
        counters.previous_release_refused,
        counters.previous_release_unanswered,
    ]
}

#[test]
fn a_move_asks_the_old_bbmd_to_delete_the_entry_and_its_success_counts() {
    let mut node: Node = registered();
    node.retarget(releasing(NEW_BBMD), at(20_000)).unwrap();
    let actions = send_all(&mut node, at(20_000));
    // The registration with the new BBMD goes first, then the release, to
    // the old BBMD, for this node's own address.
    assert!(matches!(actions[0], Action::Register { to: NEW_BBMD, .. }));
    let (_, to, entry) = release_of(&actions).expect("a release");
    assert_eq!((to, entry), (BBMD, OWN));
    assert_eq!(
        actions[1].message(),
        Some(Bvll::DeleteForeignDeviceTableEntry(OWN))
    );
    assert_eq!(actions.len(), 2);

    // The old BBMD's success settles the release, which is reported, and
    // proves nothing about the registration.
    assert!(matches!(
        says(&mut node, &result(SUCCESS), BBMD, at(20_010)),
        Inbound::Matched { settled: true, .. }
    ));
    let actions = drain(&mut node);
    assert_eq!(ends(&actions), [(BBMD, AttemptOutcome::Accepted)]);
    assert_eq!(actions.len(), 1);
    assert!(!node.registration(at(20_010)).unwrap().registered);
    let counters = node.take_counters();
    assert_eq!(releases(&counters), [1, 1, 0, 0]);
    let delete = counters.delete_fdt_entry;
    assert_eq!((delete.sent, delete.results), (1, 1));
    assert_eq!(delete.last_result, Some(SUCCESS));

    // The new BBMD's success registers the device as without a release.
    says(&mut node, &result(SUCCESS), NEW_BBMD, at(20_020));
    assert!(node.registration(at(20_020)).unwrap().registered);
}

#[test]
fn a_nak_from_the_old_bbmd_is_counted_and_the_move_stands() {
    let mut node: Node = registered();
    node.retarget(releasing(NEW_BBMD), at(20_000)).unwrap();
    send_all(&mut node, at(20_000));
    assert!(matches!(
        says(&mut node, &result(DELETE_NAK), BBMD, at(20_010)),
        Inbound::Matched { settled: true, .. }
    ));
    assert_eq!(ends(&drain(&mut node)), [(BBMD, AttemptOutcome::Refused)]);
    let counters = node.take_counters();
    assert_eq!(releases(&counters), [1, 0, 1, 0]);
    assert_eq!(counters.delete_fdt_entry.last_result, Some(DELETE_NAK));
    let view = node.registration(at(20_010)).unwrap();
    assert!(view.attempt_in_flight);
    says(&mut node, &result(SUCCESS), NEW_BBMD, at(20_020));
    assert!(node.registration(at(20_020)).unwrap().registered);
}

/// Silence: the release times out after the old registration's response
/// timeout, 3 s, unanswered. It hands out no `ExchangeEnded`, which only a
/// manual exchange gets, only its event, and a late answer matches nothing.
#[test]
fn silence_from_the_old_bbmd_is_counted_as_unanswered() {
    let mut node: Node = registered();
    node.retarget(releasing(NEW_BBMD), at(20_000)).unwrap();
    send_all(&mut node, at(20_000));
    says(&mut node, &result(SUCCESS), NEW_BBMD, at(20_010));
    drain(&mut node);
    assert_eq!(node.next_deadline(), Some(at(23_000)));
    node.on_deadline(at(23_000));
    let actions = drain(&mut node);
    assert_eq!(ends(&actions), [(BBMD, AttemptOutcome::TimedOut)]);
    assert_eq!(actions.len(), 1);
    let counters = node.take_counters();
    assert_eq!(releases(&counters), [1, 0, 0, 1]);
    assert_eq!(counters.delete_fdt_entry.timeouts, 1);
    assert!(matches!(
        says(&mut node, &result(SUCCESS), BBMD, at(23_010)),
        Inbound::Unmatched { late: true, .. }
    ));
    assert!(node.registration(at(23_010)).unwrap().registered);
}

/// No release without the option, and none for a new time-to-live at the
/// same BBMD.
#[test]
fn only_a_move_to_another_bbmd_with_the_option_releases() {
    let mut node: Node = registered();
    node.retarget(with(releasing(BBMD), |f| f.ttl = 120), at(1_000))
        .unwrap();
    assert!(release_of(&send_all(&mut node, at(1_000))).is_none());
    node.retarget(Foreign::new(NEW_BBMD, 60), at(2_000))
        .unwrap();
    assert!(release_of(&send_all(&mut node, at(2_000))).is_none());
    assert_eq!(releases(&node.take_counters()), [0; 4]);
}

/// The new BBMD's first attempt is admitted before the release, so with
/// one slot free it takes it, and the release, refused for want of a slot,
/// counts as unanswered.
#[test]
fn the_registration_takes_the_last_slot_and_the_release_counts_unanswered() {
    let mut node: Node<2> = registered();
    node.begin_exchange(
        PEER,
        Bvll::ReadBroadcastDistributionTable,
        ms(3_000),
        at(1_000),
    )
    .unwrap();
    node.retarget(releasing(NEW_BBMD), at(1_000)).unwrap();
    let actions = send_all(&mut node, at(1_000));
    assert_eq!(registers(&actions), [NEW_BBMD]);
    assert!(release_of(&actions).is_none());
    assert_eq!(ends(&actions), [(BBMD, AttemptOutcome::Cancelled)]);
    let counters = node.take_counters();
    assert_eq!(releases(&counters), [1, 0, 0, 1]);
    assert_eq!(counters.delete_fdt_entry.busy, 1);
}

/// A release whose send fails, or that the caller gives up, is unanswered.
#[test]
fn a_release_that_isnt_sent_counts_as_unanswered() {
    for why in [LocalFailure::SendFailed, LocalFailure::Cancelled] {
        let mut node: Node = registered();
        node.retarget(releasing(NEW_BBMD), at(1_000)).unwrap();
        let (exchange, _, _) = release_of(&drain(&mut node)).unwrap();
        node.exchange_failed(exchange, why, at(1_000));
        let outcome = match why {
            LocalFailure::SendFailed => AttemptOutcome::LocalError,
            _ => AttemptOutcome::Cancelled,
        };
        assert_eq!(ends(&drain(&mut node)), [(BBMD, outcome)]);
        let counters = node.take_counters();
        assert_eq!(releases(&counters), [1, 0, 0, 1], "{why:?}");
        let local = u32::from(why == LocalFailure::SendFailed);
        assert_eq!(counters.delete_fdt_entry.local_errors, local);
    }
}

/// A move back to the BBMD a release is still in flight to can't register
/// there while it is (a success couldn't tell the two apart), but does as
/// soon as the release ends, answered or not, rather than after a retry's
/// wait of one response timeout.
#[test]
fn a_move_back_registers_as_soon_as_the_release_in_flight_ends() {
    for answered in [true, false] {
        let mut node: Node = registered();
        node.retarget(releasing(NEW_BBMD), at(1_000)).unwrap();
        send_all(&mut node, at(1_000));
        node.retarget(releasing(BBMD), at(1_005)).unwrap();
        let actions = send_all(&mut node, at(1_005));
        // No attempt to the old BBMD yet; the release to the new one goes.
        assert!(registers(&actions).is_empty());
        assert_eq!(release_of(&actions).map(|(_, to, _)| to), Some(NEW_BBMD));
        // The retry would wait until 4.005 s.
        let view = node.registration(at(1_005)).unwrap();
        assert_eq!(view.next_attempt, Some(at(4_005)));
        let (ended, outcome) = if answered {
            says(&mut node, &result(SUCCESS), BBMD, at(1_010));
            (at(1_010), AttemptOutcome::Accepted)
        } else {
            // Sent at 1 s, the release times out at 4 s.
            node.on_deadline(at(4_000));
            (at(4_000), AttemptOutcome::TimedOut)
        };
        let actions = send_all(&mut node, ended);
        assert_eq!(registers(&actions), [BBMD], "answered: {answered}");
        assert_eq!(ends(&actions), [(BBMD, outcome)]);
        assert_eq!(node.take_counters().previous_releases, 2);
    }
}

/// A release that ends while the device registers with another BBMD pulls
/// no attempt forward, even when that BBMD refused it and its next attempt
/// waits a renewal interval.
#[test]
fn a_release_elsewhere_pulls_no_attempt_forward() {
    let mut node: Node = registered();
    node.retarget(releasing(NEW_BBMD), at(1_000)).unwrap();
    send_all(&mut node, at(1_000));
    let refused = result(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK);
    says(&mut node, &refused, NEW_BBMD, at(1_010));
    drain(&mut node);
    let due = node.registration(at(1_010)).unwrap().next_attempt;
    assert_eq!(due, Some(at(31_000)));
    says(&mut node, &result(SUCCESS), BBMD, at(1_020));
    assert!(registers(&drain(&mut node)).is_empty());
    assert_eq!(node.registration(at(1_020)).unwrap().next_attempt, due);
}

/// A release queued and never polled is dropped unsent: by a move back to
/// its BBMD, where it would delete the new entry, and by the next move's
/// release, which takes its place.
#[test]
fn an_unpolled_release_is_dropped_unsent() {
    let mut node: Node = registered();
    node.retarget(releasing(NEW_BBMD), at(1_000)).unwrap();
    node.retarget(Foreign::new(BBMD, 60), at(1_000)).unwrap();
    let actions = send_all(&mut node, at(1_000));
    assert_eq!(registers(&actions), [BBMD]);
    assert!(release_of(&actions).is_none());
    assert_eq!(ends(&actions), [(BBMD, AttemptOutcome::Cancelled)]);
    assert_eq!(releases(&node.take_counters()), [1, 0, 0, 1]);

    let mut node: Node = registered();
    let third = addr(10, 0, 2, 1, PORT);
    node.retarget(releasing(NEW_BBMD), at(1_000)).unwrap();
    node.retarget(releasing(third), at(1_000)).unwrap();
    let actions = send_all(&mut node, at(1_000));
    assert_eq!(registers(&actions), [third]);
    assert_eq!(
        release_of(&actions).map(|(_, to, _)| to),
        Some(NEW_BBMD),
        "{actions:?}"
    );
    assert_eq!(ends(&actions), [(BBMD, AttemptOutcome::Cancelled)]);
    assert_eq!(actions.len(), 3);
    assert_eq!(releases(&node.take_counters()), [2, 0, 0, 1]);
}

/// A port sends the release after the moved registration, from its own
/// socket, and the node counts the old BBMD's NAK.
#[test]
fn a_port_sends_the_release_after_the_moved_registration() {
    let node = Node::new(foreign_link(releasing(BBMD)), Instant::ZERO).unwrap();
    let mut port: NodePort<'_> = NodePort::new(node, &mut []).unwrap();
    let mut socket = Socket::default();
    port.start(at(0), &mut socket);
    socket.take();
    port.retarget(releasing(NEW_BBMD), at(1_000), &mut socket)
        .unwrap();
    assert_eq!(
        socket.take(),
        [
            (NEW_BBMD, wire(Bvll::RegisterForeignDevice { ttl: 60 })),
            (BBMD, wire(Bvll::DeleteForeignDeviceTableEntry(OWN))),
        ]
    );
    let nak = result(DELETE_NAK);
    port.receive(
        &datagram(&nak, BBMD, UNICAST),
        at(1_010),
        &mut Permissive,
        &mut socket,
    );
    assert!(socket.take().is_empty());
    assert_eq!(releases(&port.node_mut().take_counters()), [1, 0, 1, 0]);
}
