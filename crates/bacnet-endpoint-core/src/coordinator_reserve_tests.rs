//! The reserve each owner keeps in a shared pool (#2617): an owner under its
//! reserve is never refused because of the other, either may borrow what
//! neither keeps, and [`OutboundTransactionCoordinator::room`] says what
//! [`OutboundTransactionCoordinator::reserve`] would do.

use std::sync::Arc;
use std::thread;

use super::*;

const REQUESTER: LeaseOwner = LeaseOwner::Requester;
const NOTIFICATION: LeaseOwner = LeaseOwner::Notification;

fn metadata(owner: LeaseOwner, peer: u8) -> LeaseMetadata {
    let peer = CanonicalPeer::direct(&[peer]);
    match owner {
        LeaseOwner::Requester => LeaseMetadata::requester(
            peer,
            ConfirmedServiceChoice::READ_PROPERTY,
            TerminalPolicy::ComplexAck,
        ),
        LeaseOwner::Notification => {
            LeaseMetadata::notification(peer, ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION)
        }
    }
}

fn lease(coordinator: &OutboundTransactionCoordinator, owner: LeaseOwner) -> Option<LeaseToken> {
    match coordinator.reserve(metadata(owner, 1)) {
        Ok(token) => Some(token),
        Err(ReserveError::Exhausted) => None,
        Err(error) => panic!("{error}"),
    }
}

/// Lease for `owner` until refused.
fn fill(coordinator: &OutboundTransactionCoordinator, owner: LeaseOwner) -> Vec<LeaseToken> {
    std::iter::from_fn(|| lease(coordinator, owner)).collect()
}

fn node() -> OutboundTransactionCoordinator {
    OutboundTransactionCoordinator::with_reserves(REQUESTER_RESERVE, NOTIFICATION_RESERVE)
}

fn room(coordinator: &OutboundTransactionCoordinator, owner: LeaseOwner) -> Room {
    coordinator.room(owner).unwrap()
}

/// A pool that keeps no reserve lets either owner take all of it.
#[test]
fn without_reserves_either_owner_takes_the_whole_pool() {
    let coordinator = OutboundTransactionCoordinator::new();
    assert_eq!(room(&coordinator, NOTIFICATION).reserve_left, None);
    for (owner, other) in [(REQUESTER, NOTIFICATION), (NOTIFICATION, REQUESTER)] {
        let held = fill(&coordinator, owner);
        assert_eq!(held.len(), 256, "{owner:?}");
        assert!(lease(&coordinator, other).is_none());
        assert_eq!(room(&coordinator, other).available, 0);
        for token in held {
            coordinator.release(token).unwrap();
        }
    }
}

/// With the other owner holding all it may, each still finds its whole
/// reserve, and not one more.
#[test]
fn each_owner_finds_its_reserve_with_the_other_at_its_ceiling() {
    for (owner, reserve, other) in [
        (REQUESTER, REQUESTER_RESERVE, NOTIFICATION),
        (NOTIFICATION, NOTIFICATION_RESERVE, REQUESTER),
    ] {
        let coordinator = node();
        let flood = fill(&coordinator, other);
        assert_eq!(flood.len(), 256 - reserve, "{other:?}'s ceiling");
        assert_eq!(
            room(&coordinator, owner),
            Room {
                available: reserve,
                reserve_left: Some(reserve),
            }
        );
        let kept = fill(&coordinator, owner);
        assert_eq!(kept.len(), reserve, "{owner:?}'s reserve");
        assert_eq!(coordinator.active_count().unwrap(), 256);
        assert_eq!(room(&coordinator, other).reserve_left, Some(0));
    }
}

/// An owner borrows what neither reserve keeps; what it gives back the other
/// may take, up to its own ceiling, and past that only the first owner's
/// reserve is refilled.
#[test]
fn borrowing_from_the_shared_part_and_giving_it_back() {
    let coordinator = node();
    let mut server: Vec<_> = (0..100)
        .map(|_| lease(&coordinator, NOTIFICATION).expect("within its ceiling"))
        .collect();
    let mut client = fill(&coordinator, REQUESTER);
    assert_eq!(
        client.len(),
        156,
        "the pool is full below the client's ceiling"
    );
    for token in server.drain(64..) {
        coordinator.release(token).unwrap();
    }
    client.extend(fill(&coordinator, REQUESTER));
    assert_eq!(client.len(), 192, "the client's ceiling");
    assert_eq!(room(&coordinator, REQUESTER).available, 0);
    coordinator.release(server.pop().unwrap()).unwrap();
    assert!(
        lease(&coordinator, REQUESTER).is_none(),
        "the server's reserve"
    );
    assert_eq!(
        room(&coordinator, NOTIFICATION),
        Room {
            available: 1,
            reserve_left: Some(1),
        }
    );
    server.push(lease(&coordinator, NOTIFICATION).expect("its reserve"));
    // The client gives its borrowed part back: the server may borrow it.
    for token in client.drain(REQUESTER_RESERVE..) {
        coordinator.release(token).unwrap();
    }
    server.extend(fill(&coordinator, NOTIFICATION));
    assert_eq!(server.len(), 128, "the server's ceiling");
    assert_eq!(room(&coordinator, REQUESTER).available, 0);
    assert_eq!(room(&coordinator, REQUESTER).reserve_left, Some(0));
}

/// A refusal at an owner's ceiling changes nothing: the next lease takes the
/// invoke ID the refused one would have.
#[test]
fn a_refusal_at_the_ceiling_changes_nothing() {
    let coordinator = OutboundTransactionCoordinator::with_reserves(250, 0);
    let held = fill(&coordinator, NOTIFICATION);
    assert_eq!(held.len(), 6);
    assert!(lease(&coordinator, NOTIFICATION).is_none());
    let next = lease(&coordinator, REQUESTER).unwrap();
    assert_eq!(next.invoke_id(), 6);
    assert_eq!(next.generation, 7, "no generation spent on the refusal");
}

/// Reserves that together pass the pool are refused at construction.
#[test]
#[should_panic(expected = "exceed the pool")]
fn reserves_past_the_pool_are_refused() {
    let _ = OutboundTransactionCoordinator::with_reserves(200, 57);
}

/// A small deterministic generator, so the walk below repeats exactly.
struct Steps(u64);

impl Steps {
    fn next(&mut self, below: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as usize % below
    }
}

/// Over long walks of leases and releases on pools of several shapes, the
/// ceilings hold, a refusal comes only to an owner at or past its reserve,
/// and `room` says exactly what `reserve` then does.
#[test]
fn room_says_what_reserve_does_on_every_step() {
    for (requester, notification) in [(128, 64), (64, 64), (0, 0), (256, 0), (0, 256), (10, 3)] {
        let coordinator = OutboundTransactionCoordinator::with_reserves(requester, notification);
        let reserves = [requester, notification];
        let mut held: [Vec<LeaseToken>; 2] = [Vec::new(), Vec::new()];
        let mut steps = Steps(u64::try_from(requester * 1000 + notification).unwrap());
        for step in 0..20_000 {
            let owner = [REQUESTER, NOTIFICATION][steps.next(2)];
            let at = owner as usize;
            // Lean towards leasing, so the pool fills and stays near full.
            if steps.next(5) < 3 {
                let before = room(&coordinator, owner);
                let free = 256 - held[0].len() - held[1].len();
                let ceiling = 256 - reserves[1 - at];
                assert_eq!(
                    before.available,
                    free.min(ceiling - held[at].len()),
                    "step {step}"
                );
                assert_eq!(
                    before.reserve_left,
                    (reserves[at] > 0).then(|| reserves[at].saturating_sub(held[at].len()))
                );
                match lease(&coordinator, owner) {
                    Some(token) => {
                        assert!(before.available > 0, "step {step}");
                        held[at].push(token);
                    }
                    None => {
                        assert_eq!(before.available, 0, "step {step}");
                        assert!(held[at].len() >= reserves[at], "under its reserve, refused");
                    }
                }
            } else if !held[at].is_empty() {
                let token = held[at].swap_remove(steps.next(held[at].len()));
                assert_eq!(
                    coordinator.release(token).unwrap(),
                    ReleaseOutcome::Released
                );
            }
            for owner in [REQUESTER, NOTIFICATION] {
                let at = owner as usize;
                assert!(held[at].len() <= 256 - reserves[1 - at], "ceiling");
            }
            assert_eq!(
                coordinator.active_count().unwrap(),
                held[0].len() + held[1].len()
            );
        }
    }
}

/// Two threads per owner leasing and releasing at once never pass either
/// ceiling, and every count comes back whole.
#[test]
fn racing_owners_keep_the_ceilings() {
    let coordinator = Arc::new(node());
    let workers: Vec<_> = [REQUESTER, REQUESTER, NOTIFICATION, NOTIFICATION]
        .into_iter()
        .map(|owner| {
            let coordinator = Arc::clone(&coordinator);
            thread::spawn(move || {
                let mut held = Vec::new();
                for round in 0..5_000 {
                    if round % 3 == 2 {
                        if let Some(token) = held.pop() {
                            coordinator.release(token).unwrap();
                        }
                    } else if let Some(token) = lease(&coordinator, owner) {
                        held.push(token);
                    }
                    let state = coordinator.state.lock().unwrap();
                    for at in 0..2 {
                        assert!(state.room[at] <= state.ceiling[at]);
                    }
                    assert!(state.active() <= 256);
                }
                (owner, held)
            })
        })
        .collect();
    // Every thread has ended before any of what they hold is let go.
    let ended: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    let mut totals = [0; 2];
    for (owner, held) in ended {
        totals[owner as usize] += held.len();
        for token in held {
            coordinator.release(token).unwrap();
        }
    }
    assert!(totals[REQUESTER as usize] <= 192);
    assert!(totals[NOTIFICATION as usize] <= 128);
    assert_eq!(coordinator.active_count().unwrap(), 0);
    assert_eq!(fill(&coordinator, REQUESTER).len(), 192, "every count back");
}
