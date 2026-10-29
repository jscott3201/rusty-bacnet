//! The neighbour table and its resolutions as plain state, stepped through
//! explicit instants (U.5, #1637): when Address-Resolutions are due, what
//! completes or fails a resolution, the negative cache, every bound, the
//! table's eviction and trust rules, and revalidation.

use std::net::{Ipv6Addr, SocketAddrV6};
use std::time::Duration;

use tokio::sync::oneshot::error::TryRecvError;
use tokio::time::Instant;

use super::resolve::{
    Failure, Limit, MAX_QUEUED, MAX_QUEUED_PER_VMAC, MAX_RESOLVING, MAX_VIRTUAL, MAX_WAITERS,
    MAX_WAITERS_PER_VMAC, Neighbours, Route, VirtualRoute,
};
use super::resolve_counters::{Bip6ResolutionCounters, ResolutionCounts};
use super::vmac_table::{
    DIRECT_PRECEDENCE, Heard, MAX_UNREACHABLE, MAX_VMAC_TABLE_ENTRIES, REVALIDATE_AFTER,
    UNREACHABLE_FOR,
};
use super::{Bip6Vmac, UnresolvedSend};

const OWN: Bip6Vmac = Bip6Vmac::new([0x00, 0x00, 0x0A]);
const PEER: Bip6Vmac = Bip6Vmac::new([0x00, 0x04, 0xD2]);
const OTHER: Bip6Vmac = Bip6Vmac::new([0x00, 0x04, 0xD3]);
const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];
const WAIT: UnresolvedSend = UnresolvedSend::Wait;
const QUEUE: UnresolvedSend = UnresolvedSend::Queue;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn addr(host: u16) -> SocketAddrV6 {
    SocketAddrV6::new(
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, host),
        0xBAC0,
        0,
        0,
    )
}

/// The `n`th VMAC of a run of distinct ones, all different from the named.
fn nth(n: usize) -> Bip6Vmac {
    let [_, high, middle, low] = (0x10_0000 + n as u32).to_be_bytes();
    Bip6Vmac::new([high, middle, low])
}

/// The state of a running node whose VMAC is [`OWN`].
fn neighbours() -> Neighbours {
    let mut neighbours = Neighbours::default();
    neighbours.table.set_own(Some(OWN));
    neighbours
}

fn wait(
    neighbours: &mut Neighbours,
    vmac: Bip6Vmac,
    now: Instant,
    counts: &ResolutionCounts,
) -> (
    tokio::sync::oneshot::Receiver<Result<SocketAddrV6, Failure>>,
    bool,
) {
    match neighbours.route(vmac, NPDU, WAIT, now, counts) {
        Route::Wait { rx, wake } => (rx, wake),
        other => panic!("expected to wait, got {other:?}"),
    }
}

fn counters(set: impl FnOnce(&mut Bip6ResolutionCounters)) -> Bip6ResolutionCounters {
    let mut counters = Bip6ResolutionCounters::default();
    set(&mut counters);
    counters
}

/// A miss starts one resolution and wakes the driver once: its
/// Address-Resolutions are due at once and a second and two seconds later,
/// and it fails three seconds after the first, failing every waiter.
#[test]
fn a_miss_is_resolved_three_times_a_second_apart_then_fails() {
    let counts = ResolutionCounts::default();
    let mut state = neighbours();
    let t0 = Instant::now();
    let (mut first, wake) = wait(&mut state, PEER, t0, &counts);
    assert!(wake, "a new resolution wakes the driver");
    let (mut second, wake) = wait(&mut state, PEER, t0, &counts);
    assert!(!wake, "a second send joins the first");

    for (at, due, next) in [
        (t0, true, t0 + secs(1)),
        (t0 + secs(1) - Duration::from_millis(1), false, t0 + secs(1)),
        (t0 + secs(1), true, t0 + secs(2)),
        (t0 + secs(2), true, t0 + secs(3)),
        (t0 + secs(3) - Duration::from_millis(1), false, t0 + secs(3)),
    ] {
        let polled = state.poll(at, &counts);
        let expected: &[Bip6Vmac] = if due { &[PEER] } else { &[] };
        assert_eq!(polled.resolutions, expected, "{:?}", at - t0);
        assert_eq!(polled.next, Some(next), "{:?}", at - t0);
        assert_eq!(first.try_recv(), Err(TryRecvError::Empty));
    }
    let polled = state.poll(t0 + secs(3), &counts);
    assert!(polled.resolutions.is_empty());
    assert_eq!(polled.next, None, "nothing left to wake for");
    assert_eq!(first.try_recv(), Ok(Err(Failure::TimedOut)));
    assert_eq!(second.try_recv(), Ok(Err(Failure::TimedOut)));
    assert_eq!(
        counts.snapshot(),
        counters(|c| {
            c.resolutions_started = 1;
            c.resolutions_timed_out = 1;
        })
    );
}

/// An Address-Resolution-ACK completes every waiter with the address; so
/// does any other learn of the VMAC, such as a Forwarded-NPDU's origin or an
/// I-Am broadcast from the target.
#[test]
fn any_learn_of_the_vmac_completes_its_waiters() {
    for heard in [Heard::Direct, Heard::Carried] {
        let counts = ResolutionCounts::default();
        let mut state = neighbours();
        let t0 = Instant::now();
        let (mut first, _) = wait(&mut state, PEER, t0, &counts);
        let (mut second, _) = wait(&mut state, PEER, t0, &counts);
        state.poll(t0, &counts);
        assert!(!state.learn(PEER, addr(4), heard, t0 + secs(1), &counts));
        assert_eq!(first.try_recv(), Ok(Ok(addr(4))), "{heard:?}");
        assert_eq!(second.try_recv(), Ok(Ok(addr(4))), "{heard:?}");
        assert_eq!(state.resolving(), 0);
        let polled = state.poll(t0 + secs(2), &counts);
        assert!(polled.resolutions.is_empty(), "no retry after it resolved");
        assert!(matches!(
            state.route(PEER, NPDU, WAIT, t0 + secs(2), &counts),
            Route::Send { addr: to, wake: false } if to == addr(4)
        ));
        assert_eq!(
            counts.snapshot(),
            counters(|c| {
                c.resolutions_started = 1;
                c.resolutions_resolved = 1;
            })
        );
    }
}

/// A VMAC that failed fails a send at once for ten seconds; afterwards a
/// send resolves it again. A learn clears it at once.
#[test]
fn a_failed_vmac_fails_sends_at_once_for_a_while() {
    let counts = ResolutionCounts::default();
    let mut state = neighbours();
    let t0 = Instant::now();
    let _ = wait(&mut state, PEER, t0, &counts);
    let failed_at = t0 + secs(3);
    state.poll(failed_at, &counts);
    for at in [
        failed_at,
        failed_at + UNREACHABLE_FOR - Duration::from_millis(1),
    ] {
        for hold in [WAIT, QUEUE] {
            assert!(matches!(
                state.route(PEER, NPDU, hold, at, &counts),
                Route::Unreachable
            ));
        }
    }
    assert_eq!(counts.snapshot().sends_unreachable, 4);
    let (_rx, wake) = wait(&mut state, PEER, failed_at + UNREACHABLE_FOR, &counts);
    assert!(wake, "a new resolution once the VMAC is no longer held");

    let _ = wait(&mut state, OTHER, t0, &counts);
    state.poll(failed_at, &counts);
    assert!(matches!(
        state.route(OTHER, NPDU, WAIT, failed_at, &counts),
        Route::Unreachable
    ));
    state.learn(OTHER, addr(5), Heard::Direct, failed_at, &counts);
    assert!(matches!(
        state.route(OTHER, NPDU, WAIT, failed_at, &counts),
        Route::Send { .. }
    ));
}

/// The negative cache holds at most [`MAX_UNREACHABLE`] VMACs, dropping the
/// one marked first.
#[test]
fn the_negative_cache_is_bounded() {
    let mut state = neighbours();
    let t0 = Instant::now();
    for n in 0..=MAX_UNREACHABLE {
        state
            .table
            .mark_unreachable(nth(n), t0 + Duration::from_millis(n as u64));
    }
    assert_eq!(state.table.unreachable_len(), MAX_UNREACHABLE);
    let now = t0 + secs(1);
    assert!(!state.table.is_unreachable(nth(0), now), "the oldest went");
    assert!(state.table.is_unreachable(nth(1), now));
    assert!(state.table.is_unreachable(nth(MAX_UNREACHABLE), now));
}

/// Each bound refuses the newest send and counts it: VMACs resolving at
/// once, sends waiting on one VMAC and in all.
#[test]
fn each_wait_bound_refuses_the_newest_send() {
    let t0 = Instant::now();

    let counts = ResolutionCounts::default();
    let mut state = neighbours();
    let held: Vec<_> = (0..MAX_RESOLVING)
        .map(|n| wait(&mut state, nth(n), t0, &counts))
        .collect();
    assert!(matches!(
        state.route(nth(MAX_RESOLVING), NPDU, WAIT, t0, &counts),
        Route::Full(Limit::Resolving)
    ));
    assert!(
        matches!(
            state.route(nth(0), NPDU, WAIT, t0, &counts),
            Route::Wait { .. }
        ),
        "a VMAC already resolving still takes a waiter"
    );
    assert_eq!(counts.snapshot().sends_over_limit, 1);
    drop(held);

    let counts = ResolutionCounts::default();
    let mut state = neighbours();
    let mut held: Vec<_> = (0..MAX_WAITERS_PER_VMAC)
        .map(|_| wait(&mut state, PEER, t0, &counts))
        .collect();
    assert!(matches!(
        state.route(PEER, NPDU, WAIT, t0, &counts),
        Route::Full(Limit::PerVmac)
    ));
    // A waiter whose send was dropped no longer holds a place.
    held.pop();
    held.push(wait(&mut state, PEER, t0, &counts));

    let per_vmac = MAX_WAITERS / MAX_WAITERS_PER_VMAC;
    for n in 0..per_vmac - 1 {
        for _ in 0..MAX_WAITERS_PER_VMAC {
            held.push(wait(&mut state, nth(n), t0, &counts));
        }
    }
    assert!(matches!(
        state.route(OTHER, NPDU, WAIT, t0, &counts),
        Route::Full(Limit::Total)
    ));
    assert_eq!(counts.snapshot().sends_over_limit, 2);
}

/// Queued NPDUs have their own bounds, per VMAC and in all.
#[test]
fn each_queue_bound_refuses_the_newest_send() {
    let t0 = Instant::now();
    let counts = ResolutionCounts::default();
    let mut state = neighbours();
    for _ in 0..MAX_QUEUED_PER_VMAC {
        assert!(matches!(
            state.route(PEER, NPDU, QUEUE, t0, &counts),
            Route::Queued { .. }
        ));
    }
    assert!(matches!(
        state.route(PEER, NPDU, QUEUE, t0, &counts),
        Route::Full(Limit::PerVmac)
    ));
    for n in 0..MAX_QUEUED / MAX_QUEUED_PER_VMAC - 1 {
        for _ in 0..MAX_QUEUED_PER_VMAC {
            state.route(nth(n), NPDU, QUEUE, t0, &counts);
        }
    }
    assert!(matches!(
        state.route(OTHER, NPDU, QUEUE, t0, &counts),
        Route::Full(Limit::Total)
    ));
    assert_eq!(counts.snapshot().sends_over_limit, 2);
}

/// Queue mode returns at once; the queue goes out in order once the VMAC
/// resolves, and is dropped and counted when it doesn't.
#[test]
fn a_queue_flushes_in_order_on_resolution_and_drops_on_failure() {
    let counts = ResolutionCounts::default();
    let mut state = neighbours();
    let t0 = Instant::now();
    for (index, npdu) in [[1u8], [2], [3]].iter().enumerate() {
        let route = state.route(PEER, npdu, QUEUE, t0, &counts);
        assert!(matches!(route, Route::Queued { wake } if wake == (index == 0)));
    }
    assert!(state.poll(t0, &counts).flush.is_empty());
    assert!(
        state.learn(PEER, addr(4), Heard::Direct, t0, &counts),
        "ready to flush"
    );
    let flushed: Vec<_> = state
        .poll(t0, &counts)
        .flush
        .into_iter()
        .map(|(to, vmac, npdu)| (to, vmac, npdu.to_vec()))
        .collect();
    assert_eq!(flushed, [1, 2, 3].map(|octet| (addr(4), PEER, vec![octet])));

    for _ in 0..2 {
        state.route(OTHER, NPDU, QUEUE, t0, &counts);
    }
    state.poll(t0, &counts);
    let polled = state.poll(t0 + secs(3), &counts);
    assert!(polled.flush.is_empty());
    assert_eq!(counts.snapshot().queued_dropped, 2);
    assert_eq!(counts.snapshot().resolutions_timed_out, 1);
}

/// A refusal from the BBMD doesn't end a resolution early, since the
/// registration it lacks may still land, but the failure says so.
#[test]
fn a_refused_resolution_fails_as_refused() {
    let counts = ResolutionCounts::default();
    let mut state = neighbours();
    let t0 = Instant::now();
    let (mut rx, _) = wait(&mut state, PEER, t0, &counts);
    state.poll(t0, &counts);
    state.refuse_resolutions();
    assert_eq!(state.poll(t0 + secs(1), &counts).resolutions, [PEER]);
    state.poll(t0 + secs(3), &counts);
    assert_eq!(rx.try_recv(), Ok(Err(Failure::Refused)));
    assert_eq!(
        counts.snapshot(),
        counters(|c| {
            c.resolutions_started = 1;
            c.resolutions_refused = 1;
        })
    );
}

/// This node's own VMAC is never learned, by either path.
#[test]
fn this_nodes_own_vmac_is_never_learned() {
    let counts = ResolutionCounts::default();
    let mut state = neighbours();
    let t0 = Instant::now();
    for heard in [Heard::Direct, Heard::Carried] {
        assert!(!state.table.learn(OWN, addr(9), heard, t0));
        assert!(!state.learn(OWN, addr(9), heard, t0, &counts));
    }
    assert_eq!(state.table.lookup(OWN), None);
    assert_eq!(state.table.len(), 0);
}

/// A full table evicts the peer heard from least recently, not the newest
/// one: hearing a peer again keeps it.
#[test]
fn a_full_table_evicts_the_peer_heard_from_least_recently() {
    let mut state = neighbours();
    let t0 = Instant::now();
    for n in 0..MAX_VMAC_TABLE_ENTRIES {
        let at = t0 + Duration::from_millis(n as u64);
        assert!(
            state
                .table
                .learn(nth(n), addr(n as u16 + 1), Heard::Direct, at)
        );
    }
    let later = t0 + secs(10);
    state.table.learn(nth(0), addr(1), Heard::Direct, later);
    assert!(state.table.learn(PEER, addr(0xFFFF), Heard::Direct, later));
    assert_eq!(state.table.len(), MAX_VMAC_TABLE_ENTRIES);
    assert_eq!(state.table.lookup(PEER), Some(addr(0xFFFF)));
    assert_eq!(state.table.lookup(nth(0)), Some(addr(1)), "heard again");
    assert_eq!(state.table.lookup(nth(1)), None, "the least recent went");
}

/// An address carried in a forwarded message doesn't replace one heard
/// directly within the last minute, for the same VMAC or by taking over its
/// address; after that minute, or heard directly, it does.
#[test]
fn an_address_heard_directly_outranks_a_carried_one_for_a_minute() {
    let mut state = neighbours();
    let t0 = Instant::now();
    state.table.learn(PEER, addr(1), Heard::Direct, t0);
    let within = t0 + DIRECT_PRECEDENCE - Duration::from_millis(1);
    assert!(!state.table.learn(PEER, addr(2), Heard::Carried, within));
    assert!(!state.table.learn(OTHER, addr(1), Heard::Carried, within));
    assert_eq!(state.table.lookup(PEER), Some(addr(1)));
    assert_eq!(state.table.lookup(OTHER), None);
    assert!(
        state.table.learn(PEER, addr(1), Heard::Carried, within),
        "same address"
    );

    let after = t0 + DIRECT_PRECEDENCE;
    assert!(state.table.learn(PEER, addr(2), Heard::Carried, after));
    assert_eq!(state.table.lookup(PEER), Some(addr(2)));
    // Once carried, another carried address may replace it at once.
    assert!(state.table.learn(PEER, addr(3), Heard::Carried, after));
    assert!(state.table.learn(PEER, addr(1), Heard::Direct, after));
    assert_eq!(state.table.lookup(PEER), Some(addr(1)));
}

/// One VMAC per exact address: a new VMAC there replaces the old, and two
/// VMACs at the same link-local address on different links stay apart, so
/// a lookup by an address without a scope is ambiguous and answers none.
#[test]
fn an_address_has_one_owner_and_the_scope_tells_links_apart() {
    let mut state = neighbours();
    let t0 = Instant::now();
    let ip = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
    let on = |scope| SocketAddrV6::new(ip, 0xBAC0, 0, scope);
    state.table.learn(PEER, on(4), Heard::Direct, t0);
    assert_eq!(state.table.by_address(on(0)), Some(PEER));
    state.table.learn(OTHER, on(4), Heard::Direct, t0);
    assert_eq!(state.table.lookup(PEER), None, "replaced at its address");
    assert_eq!(state.table.by_address(on(4)), Some(OTHER));

    state.table.learn(PEER, on(7), Heard::Direct, t0);
    assert_eq!(
        state.table.by_address(on(0)),
        None,
        "ambiguous without a scope"
    );
    assert_eq!(state.table.by_address(on(7)), Some(PEER));
    assert_eq!(state.table.by_address(on(4)), Some(OTHER));
}

/// A send to a peer silent for five minutes still goes to its address, and
/// starts one check; an answer keeps the peer, silence removes it.
#[test]
fn a_quiet_peer_is_checked_and_removed_when_silent() {
    for answered in [true, false] {
        let counts = ResolutionCounts::default();
        let mut state = neighbours();
        let t0 = Instant::now();
        state.learn(PEER, addr(4), Heard::Direct, t0, &counts);
        let quiet = |at| matches!(at, Route::Send { addr: to, wake: false } if to == addr(4));
        assert!(quiet(state.route(
            PEER,
            NPDU,
            WAIT,
            t0 + REVALIDATE_AFTER - secs(1),
            &counts
        )));
        let at = t0 + REVALIDATE_AFTER;
        assert!(matches!(
            state.route(PEER, NPDU, WAIT, at, &counts),
            Route::Send { addr: to, wake: true } if to == addr(4)
        ));
        assert!(
            quiet(state.route(PEER, NPDU, WAIT, at, &counts)),
            "one check at a time"
        );
        // The check asks the address on file, not the group.
        let polled = state.poll(at, &counts);
        assert_eq!(polled.revalidations, [addr(4)]);
        assert!(polled.resolutions.is_empty());
        if answered {
            state.learn(PEER, addr(4), Heard::Direct, at + secs(1), &counts);
        } else {
            let retries = state.poll(at + secs(1), &counts).revalidations;
            assert_eq!(retries, [addr(4)], "retried a second later");
        }
        state.poll(at + secs(3), &counts);
        assert_eq!(state.table.lookup(PEER).is_some(), answered);
        let expected = counters(|c| c.revalidations_failed = u64::from(!answered));
        assert_eq!(counts.snapshot(), expected, "answered={answered}");
        if answered {
            assert!(quiet(state.route(PEER, NPDU, WAIT, at + secs(4), &counts)));
        } else {
            assert!(matches!(
                state.route(PEER, NPDU, WAIT, at + secs(4), &counts),
                Route::Unreachable
            ));
        }
    }
}

/// A Virtual-Address-Resolution goes to the peer three times a second
/// apart; its answer or its NAK ends it, silence fails it, and an address
/// the table maps to one VMAC is answered at once.
#[test]
fn a_virtual_address_lookup_follows_the_same_schedule() {
    let counts = ResolutionCounts::default();
    let mut state = neighbours();
    let t0 = Instant::now();
    state.table.learn(PEER, addr(4), Heard::Direct, t0);
    assert!(matches!(
        state.resolve_virtual(addr(4), t0),
        VirtualRoute::Known(vmac) if vmac == PEER
    ));

    let VirtualRoute::Wait {
        rx: mut answered,
        wake: true,
    } = state.resolve_virtual(addr(5), t0)
    else {
        panic!("a new lookup");
    };
    let VirtualRoute::Wait {
        rx: mut refused,
        wake: true,
    } = state.resolve_virtual(addr(6), t0)
    else {
        panic!("a new lookup");
    };
    let VirtualRoute::Wait {
        rx: mut silent,
        wake: true,
    } = state.resolve_virtual(addr(7), t0)
    else {
        panic!("a new lookup");
    };
    assert!(matches!(
        state.resolve_virtual(addr(7), t0),
        VirtualRoute::Wait { wake: false, .. }
    ));
    let mut polled = state.poll(t0, &counts).virtual_resolutions;
    polled.sort_by_key(|peer| *peer.ip());
    assert_eq!(polled, [addr(5), addr(6), addr(7)]);
    state.virtual_answered(addr(5), OTHER, &counts);
    state.virtual_refused(addr(6), &counts);
    assert_eq!(answered.try_recv(), Ok(Ok(OTHER)));
    assert_eq!(refused.try_recv(), Ok(Err(Failure::Refused)));
    assert_eq!(
        state.poll(t0 + secs(1), &counts).virtual_resolutions,
        [addr(7)]
    );
    assert_eq!(
        state.poll(t0 + secs(2), &counts).virtual_resolutions,
        [addr(7)]
    );
    state.poll(t0 + secs(3), &counts);
    assert_eq!(silent.try_recv(), Ok(Err(Failure::TimedOut)));
    let snapshot = counts.snapshot();
    assert_eq!(
        (
            snapshot.virtual_resolved,
            snapshot.virtual_refused,
            snapshot.virtual_timed_out
        ),
        (1, 1, 1)
    );

    let mut held = Vec::new();
    for n in 0..MAX_VIRTUAL {
        held.push(state.resolve_virtual(addr(100 + n as u16), t0));
    }
    assert!(matches!(
        state.resolve_virtual(addr(200), t0),
        VirtualRoute::Full(Limit::Resolving)
    ));
}

/// Stop forgets the table and fails every waiter: its sender is gone.
#[test]
fn clearing_fails_every_waiter() {
    let counts = ResolutionCounts::default();
    let mut state = neighbours();
    let t0 = Instant::now();
    let (mut rx, _) = wait(&mut state, PEER, t0, &counts);
    let VirtualRoute::Wait { rx: mut lookup, .. } = state.resolve_virtual(addr(5), t0) else {
        panic!("a new lookup");
    };
    state.learn(OTHER, addr(4), Heard::Direct, t0, &counts);
    // Two NPDUs queued, one of them already handed to the flush.
    let third = nth(3);
    state.route(third, NPDU, QUEUE, t0, &counts);
    state.learn(third, addr(6), Heard::Direct, t0, &counts);
    state.route(nth(4), NPDU, QUEUE, t0, &counts);
    state.clear(&counts);
    assert_eq!(rx.try_recv(), Err(TryRecvError::Closed));
    assert_eq!(lookup.try_recv(), Err(TryRecvError::Closed));
    assert_eq!(state.table.lookup(OTHER), None);
    assert_eq!(state.resolving(), 0);
    assert!(state.poll(t0, &counts).flush.is_empty());
    assert_eq!(counts.snapshot().queued_dropped, 2, "stop counts the queue");
    let _ = wait(&mut state, PEER, t0, &counts);
}

/// Fill the whole queue: [`MAX_QUEUED`] NPDUs, [`MAX_QUEUED_PER_VMAC`] for
/// each of the VMACs from `first` on.
fn fill_queue(state: &mut Neighbours, first: usize, now: Instant, counts: &ResolutionCounts) {
    for n in first..first + MAX_QUEUED / MAX_QUEUED_PER_VMAC {
        for _ in 0..MAX_QUEUED_PER_VMAC {
            let route = state.route(nth(n), NPDU, QUEUE, now, counts);
            assert!(matches!(route, Route::Queued { .. }), "{n}: {route:?}");
        }
    }
    assert!(matches!(
        state.route(OTHER, NPDU, QUEUE, now, counts),
        Route::Full(Limit::Total)
    ));
}

/// The queue's places come back once its NPDUs leave it, whether they go
/// out or are dropped: a full queue that resolves, or fails, takes a full
/// queue again. Until the driver takes a resolved queue, it still counts.
#[test]
fn a_full_queue_takes_as_many_again_once_it_empties() {
    let counts = ResolutionCounts::default();
    let mut state = neighbours();
    let t0 = Instant::now();
    let per = MAX_QUEUED / MAX_QUEUED_PER_VMAC;

    fill_queue(&mut state, 0, t0, &counts);
    for n in 0..per {
        state.learn(nth(n), addr(n as u16 + 1), Heard::Direct, t0, &counts);
    }
    assert!(
        matches!(
            state.route(OTHER, NPDU, QUEUE, t0, &counts),
            Route::Full(Limit::Total)
        ),
        "the resolved queue counts until the driver takes it"
    );
    assert_eq!(state.poll(t0, &counts).flush.len(), MAX_QUEUED);
    fill_queue(&mut state, per, t0, &counts);

    let t1 = t0 + secs(1);
    let mut state = neighbours();
    fill_queue(&mut state, 0, t1, &counts);
    state.poll(t1 + secs(3), &counts);
    assert_eq!(counts.snapshot().queued_dropped, MAX_QUEUED as u64);
    fill_queue(&mut state, per, t1 + secs(3), &counts);
}

/// A check the resolving bound leaves no room for isn't recorded: the next
/// send, once there is room, starts it.
#[test]
fn a_check_with_no_room_is_tried_again_at_the_next_send() {
    let counts = ResolutionCounts::default();
    let mut state = neighbours();
    let t0 = Instant::now();
    state.learn(PEER, addr(4), Heard::Direct, t0, &counts);
    let held: Vec<_> = (0..MAX_RESOLVING)
        .map(|n| wait(&mut state, nth(n), t0, &counts))
        .collect();
    let at = t0 + REVALIDATE_AFTER;
    assert!(matches!(
        state.route(PEER, NPDU, WAIT, at, &counts),
        Route::Send { wake: false, .. }
    ));
    state.learn(nth(0), addr(9), Heard::Direct, at, &counts);
    assert!(matches!(
        state.route(PEER, NPDU, WAIT, at + secs(1), &counts),
        Route::Send { wake: true, .. }
    ));
    drop(held);
}

/// A send to a quiet peer whose entry went while it was being checked
/// joins the check, which becomes that send's resolution: it asks the group
/// from then on, and when nothing answers, the send's own outcome counts as
/// timed out besides the failed check.
#[test]
fn a_send_that_joins_a_check_counts_its_own_outcome() {
    let counts = ResolutionCounts::default();
    let mut state = neighbours();
    let t0 = Instant::now();
    state.learn(PEER, addr(4), Heard::Direct, t0, &counts);
    let at = t0 + REVALIDATE_AFTER;
    state.route(PEER, NPDU, WAIT, at, &counts);
    assert_eq!(state.poll(at, &counts).revalidations, [addr(4)]);
    state.table.forget(PEER);
    let (mut rx, wake) = wait(&mut state, PEER, at, &counts);
    assert!(!wake, "it joins the check");
    let polled = state.poll(at + secs(1), &counts);
    assert_eq!(
        (polled.resolutions, polled.revalidations),
        (vec![PEER], vec![])
    );
    state.poll(at + secs(3), &counts);
    assert_eq!(rx.try_recv(), Ok(Err(Failure::TimedOut)));
    assert_eq!(
        counts.snapshot(),
        counters(|c| {
            c.resolutions_started = 1;
            c.resolutions_timed_out = 1;
            c.revalidations_failed = 1;
        })
    );
}
