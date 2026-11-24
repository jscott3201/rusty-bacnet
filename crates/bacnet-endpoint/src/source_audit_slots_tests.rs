//! The slots' sharing rule (#2652), polled by hand: no runtime, no clock.
use super::*;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::Wake;

/// A waker that counts its wakes.
#[derive(Default)]
struct Wakes(AtomicUsize);

impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// One request's wait for a slot, polled by hand; its condition holds
/// while `ready` does.
struct Waiting {
    future: Pin<Box<dyn Future<Output = Admission>>>,
    wakes: Arc<Wakes>,
    ready: Arc<AtomicBool>,
}

impl Waiting {
    fn new(slots: &Arc<Slots>, lane: Lane) -> Self {
        let slots = Arc::clone(slots);
        let ready = Arc::new(AtomicBool::new(true));
        let condition = Arc::clone(&ready);
        Self {
            future: Box::pin(async move {
                slots.admit(lane, || condition.load(Ordering::SeqCst)).await
            }),
            wakes: Arc::default(),
            ready,
        }
    }

    fn poll(&mut self) -> Poll<Admission> {
        let wakes = Arc::clone(&self.wakes);
        self.poll_with(&wakes)
    }

    /// Poll it with a waker counting into `wakes`, as a task that moved
    /// would.
    fn poll_with(&mut self, wakes: &Arc<Wakes>) -> Poll<Admission> {
        let waker = Waker::from(Arc::clone(wakes));
        (self.future.as_mut()).poll(&mut Context::from_waker(&waker))
    }

    fn pending(&mut self) -> bool {
        self.poll().is_pending()
    }

    fn woken(&self) -> usize {
        self.wakes.0.load(Ordering::SeqCst)
    }

    /// The slot its next poll takes.
    fn slot(mut self) -> Slot {
        match self.poll() {
            Poll::Ready(Admission::Admitted(slot)) => slot,
            _ => panic!("it takes a slot"),
        }
    }
}

fn station(last: u8) -> Lane {
    Lane::Station(MacAddr::from_slice(&[10, 0, 0, last, 0xBA, 0xC0]))
}

/// A slot of `lane`, admitted at once.
fn held(slots: &Arc<Slots>, lane: Lane) -> Slot {
    Waiting::new(slots, lane).slot()
}

/// A request to `lane` that has to wait.
fn waiting(slots: &Arc<Slots>, lane: Lane) -> Waiting {
    let mut waiting = Waiting::new(slots, lane);
    assert!(waiting.pending(), "it waits");
    waiting
}

/// `lanes` lanes, stations from 100, holding `each` slots apiece.
fn hold(slots: &Arc<Slots>, lanes: u8, each: usize) -> Vec<Vec<Slot>> {
    (0..lanes)
        .map(|lane| {
            (0..each)
                .map(|_| held(slots, station(100 + lane)))
                .collect()
        })
        .collect()
}

#[test]
fn a_station_here_is_a_lane_and_stations_elsewhere_share_their_networks() {
    let at = |network_number, last: u8| {
        Lane::of(&BACnetAddress {
            network_number,
            mac_address: MacAddr::from_slice(&[last]),
        })
    };
    assert_eq!(at(0, 1), Lane::Station(MacAddr::from_slice(&[1])));
    assert_ne!(at(0, 1), at(0, 2));
    assert_eq!(at(5, 1), at(5, 2));
    assert_eq!(at(5, 1), Lane::Network(5));
    assert_ne!(at(5, 1), at(6, 1));
}

/// A lane at its share waits behind itself only: another lane is admitted
/// while a slot is free, and the waiter is woken when its own lane frees
/// one.
#[test]
fn a_lane_at_its_share_waits_while_others_go() {
    let slots = Arc::new(Slots::new());
    let mut silent = hold(&slots, 1, LANE_SHARE).remove(0);
    let over = waiting(&slots, station(100));
    let routed: Vec<_> = (0..LANE_SHARE)
        .map(|_| held(&slots, Lane::Network(5)))
        .collect();
    let mut behind = waiting(&slots, Lane::Network(5));
    let other = held(&slots, station(7));
    assert_eq!(slots.available(), SLOTS - 2 * LANE_SHARE - 1);
    drop(silent.pop());
    assert_eq!((over.woken(), behind.woken()), (1, 0));
    let _over = over.slot();
    assert!(behind.pending());
    drop((routed, other));
}

/// The pool holds 64. A slot that comes back wakes the lane holding the
/// fewest, ahead of one that waited longer but holds more.
#[test]
fn a_full_pool_wakes_the_lane_holding_fewest_first() {
    let slots = Arc::new(Slots::new());
    let mut lanes = hold(&slots, 16, LANE_SHARE);
    assert_eq!(slots.available(), 0);
    let first = waiting(&slots, station(100));
    let fresh = waiting(&slots, station(50));
    drop(lanes[0].pop());
    assert_eq!((first.woken(), fresh.woken()), (0, 1));
    let _fresh = fresh.slot();
    assert_eq!(slots.available(), 0);
    drop(lanes[0].pop());
    assert_eq!(first.woken(), 1, "its own lane's slot came back");
}

/// Lanes holding as many take turns at being woken: one woken goes behind
/// the others that wait, so each lane waiting ahead is woken at most once
/// first. One slot wakes one waiter, the oldest.
#[test]
fn lanes_holding_as_many_take_turns() {
    let slots = Arc::new(Slots::new());
    let mut lanes = hold(&slots, 64, 1);
    let first = waiting(&slots, station(100));
    let mut second = waiting(&slots, station(100));
    let late = waiting(&slots, station(99));
    // Both hold none once station 100's slot comes back; it waited longer.
    drop(lanes[0].pop());
    assert_eq!((first.woken(), second.woken(), late.woken()), (1, 0, 0));
    // Again none, but it has just been woken: the late lane's turn.
    drop(first.slot());
    assert_eq!((second.woken(), late.woken()), (0, 1));
    let _late = late.slot();
    assert!(second.pending());
}

/// With k lanes ahead of it, each woken once, a lane is woken by the
/// (k + 1)th slot to come back.
#[test]
fn a_lane_waits_one_slot_for_each_lane_ahead() {
    let slots = Arc::new(Slots::new());
    let mut lanes = hold(&slots, 16, LANE_SHARE);
    let ahead: Vec<_> = (0..3).map(|n| waiting(&slots, station(n))).collect();
    let last = waiting(&slots, station(9));
    let mut served = Vec::new();
    for (n, lane) in ahead.into_iter().enumerate() {
        drop(lanes[n].pop());
        assert_eq!((lane.woken(), last.woken()), (1, 0));
        served.push(lane.slot());
    }
    drop(lanes[3].pop());
    assert_eq!(last.woken(), 1);
}

/// Within a lane, the first waiter polled with a slot free goes, woken or
/// not.
#[test]
fn within_a_lane_the_first_polled_goes() {
    let slots = Arc::new(Slots::new());
    let mut lane = hold(&slots, 1, LANE_SHARE).remove(0);
    let mut queue: Vec<_> = (0..3).map(|_| waiting(&slots, station(100))).collect();
    drop(lane.pop());
    let woken: Vec<_> = queue.iter().map(Waiting::woken).collect();
    assert_eq!(woken, [1, 0, 0], "one slot, one wake");
    let _last = queue.remove(2).slot();
    assert!(queue[0].pending() && queue[1].pending());
}

/// The interleaving a caller can make with a buffered stream (the review of
/// #2652): four requests to a lane run, four more wait and are then never
/// polled, and the caller sends another to the same lane while it handles
/// an answer. It goes once a slot is free: the waiters it never polls hold
/// nothing, before or after they are woken.
#[test]
fn waiters_never_polled_again_hold_nothing() {
    let slots = Arc::new(Slots::new());
    let mut running = hold(&slots, 1, LANE_SHARE).remove(0);
    let stuck: Vec<_> = (0..LANE_SHARE)
        .map(|_| waiting(&slots, station(100)))
        .collect();
    drop(running.pop());
    assert_eq!((stuck[0].woken(), stuck[1].woken()), (1, 0));
    let write = held(&slots, station(100));
    drop(running);
    assert_eq!(slots.available(), SLOTS - 1, "only the write holds one");
    assert!(stuck.iter().all(|waiter| waiter.woken() == 1), "each once");
    let again = held(&slots, station(100));
    drop((write, again, stuck));
    assert_eq!(slots.available(), SLOTS);
    assert_eq!(slots.lanes(), 0);
}

/// A lane whose waiters were woken and have not polled since is woken past:
/// a slot that comes back wakes a waiter of the next lane, even when the
/// lane passed over holds fewer and so comes first in order.
#[test]
fn a_lane_woken_and_not_polled_since_is_woken_past() {
    let slots = Arc::new(Slots::new());
    let mut lanes = hold(&slots, 15, LANE_SHARE);
    let mut busy = vec![held(&slots, station(51))];
    busy.extend((0..3).map(|_| held(&slots, station(52))));
    assert_eq!(slots.available(), 0);
    let stuck = waiting(&slots, station(50));
    let live = waiting(&slots, station(51));
    drop(lanes[0].pop());
    assert_eq!((stuck.woken(), live.woken()), (1, 0), "it holds fewer");
    // A newcomer takes the slot the stuck lane was woken for.
    let newcomer = held(&slots, station(53));
    drop(lanes[1].pop());
    assert_eq!((stuck.woken(), live.woken()), (1, 1), "the same release");
    let live = live.slot();
    drop((lanes, busy, newcomer, live, stuck));
    assert_eq!(slots.lanes(), 0);
}

/// A waiter woken for a free slot that leaves without it, dropped or with
/// its condition false, passes the wake on to the next asleep waiter; one
/// that was not woken passes nothing on.
#[test]
fn a_woken_waiter_that_leaves_passes_its_wake_on() {
    let slots = Arc::new(Slots::new());
    let mut lane = hold(&slots, 1, LANE_SHARE).remove(0);
    let mut queue: Vec<_> = (0..4).map(|_| waiting(&slots, station(100))).collect();
    drop(queue.remove(1));
    assert!(queue.iter().all(|waiter| waiter.woken() == 0));
    drop(lane.pop());
    assert_eq!(queue[0].woken(), 1);
    drop(queue.remove(0));
    assert_eq!((queue[0].woken(), queue[1].woken()), (1, 0));
    // Its turn at the destination was taken meanwhile: it takes nothing.
    queue[0].ready.store(false, Ordering::SeqCst);
    assert!(matches!(queue[0].poll(), Poll::Ready(Admission::NotReady)));
    assert_eq!(queue[1].woken(), 1);
    assert_eq!(slots.available(), SLOTS - LANE_SHARE + 1);
    let last = queue.remove(1).slot();
    drop((lane, last, queue));
    assert_eq!(slots.available(), SLOTS);
    assert_eq!(slots.lanes(), 0);
}

/// The last slot to come back, with nothing left held, wakes every asleep
/// waiter, since no later one would: past any number of waiters that were
/// woken before and never poll.
#[test]
fn the_last_slot_back_wakes_every_waiter() {
    let slots = Arc::new(Slots::new());
    let mut lane = hold(&slots, 1, LANE_SHARE).remove(0);
    let stuck: Vec<_> = (0..10).map(|_| waiting(&slots, station(100))).collect();
    let live = waiting(&slots, station(100));
    while lane.len() > 1 {
        drop(lane.pop());
        assert_eq!(live.woken(), 0);
    }
    drop(lane);
    assert_eq!(live.woken(), 1);
    assert!(stuck.iter().all(|waiter| waiter.woken() == 1));
    let _live = live.slot();
}

/// Draining one lane's queue costs one wake per waiter, not one per waiter
/// per slot that comes back. Every waiter woken is polled, as its task
/// would poll it.
#[test]
fn a_long_queue_drains_with_one_wake_per_waiter() {
    const WAITERS: usize = 1_000;
    let slots = Arc::new(Slots::new());
    let mut running = hold(&slots, 1, LANE_SHARE).remove(0);
    let mut queue: Vec<_> = (0..WAITERS)
        .map(|_| Some((waiting(&slots, station(100)), 0)))
        .collect();
    let mut polls = 0;
    while !running.is_empty() {
        drop(running.remove(0));
        for entry in &mut queue {
            let Some((waiter, seen)) = entry else {
                continue;
            };
            if waiter.woken() == *seen {
                continue;
            }
            polls += 1;
            *seen = waiter.woken();
            if let Poll::Ready(Admission::Admitted(slot)) = waiter.poll() {
                running.push(slot);
                *entry = None;
            }
        }
    }
    assert!(queue.iter().all(Option::is_none), "every waiter served");
    assert_eq!(polls, WAITERS, "one wake and one poll each");
}

/// A woken waiter that polls and can't take a slot sleeps again, keeping
/// its age: the next slot back wakes it again, ahead of those that came
/// after it. Once its lane is at its share again, once the pool is full.
#[test]
fn a_woken_waiter_that_cannot_take_sleeps_again_keeping_its_age() {
    let slots = Arc::new(Slots::new());
    let mut lane = hold(&slots, 1, LANE_SHARE).remove(0);
    let mut first = waiting(&slots, station(100));
    let later = waiting(&slots, station(100));
    drop(lane.pop());
    lane.push(held(&slots, station(100)));
    assert!(first.pending(), "its lane is at its share again");
    drop(lane.pop());
    assert_eq!((first.woken(), later.woken()), (2, 0));
    drop((lane, first, later));

    let mut lanes = hold(&slots, 16, LANE_SHARE);
    let mut first = waiting(&slots, station(50));
    let later = waiting(&slots, station(50));
    drop(lanes[0].pop());
    let newcomer = held(&slots, station(51));
    assert!(first.pending(), "the pool is full again");
    drop(lanes[1].pop());
    assert_eq!((first.woken(), later.woken()), (2, 0));
    let _first = first.slot();
    drop((lanes, newcomer, later));
}

/// A waiter polled again with another waker, as a task that moved polls
/// it, is woken through the new one only; a poll that finds no slot leaves
/// it waiting where it was.
#[test]
fn a_waiter_is_woken_through_its_latest_waker() {
    let slots = Arc::new(Slots::new());
    let mut lane = hold(&slots, 1, LANE_SHARE).remove(0);
    let mut moved = waiting(&slots, station(100));
    let behind = waiting(&slots, station(100));
    let latest = Arc::new(Wakes::default());
    assert!(moved.poll_with(&latest).is_pending());
    drop(lane.pop());
    assert_eq!(moved.woken(), 0, "the first waker is gone");
    assert_eq!(latest.0.load(Ordering::SeqCst), 1);
    assert_eq!(behind.woken(), 0, "it kept its place");
    assert!(matches!(
        moved.poll_with(&latest),
        Poll::Ready(Admission::Admitted(_))
    ));
    drop((lane, behind));
}

/// Closing wakes every waiter once, each to fail; slots that come back
/// after wake no one, and nothing more is admitted.
#[test]
fn closing_ends_every_wait() {
    let slots = Arc::new(Slots::new());
    let lane = hold(&slots, 1, LANE_SHARE).remove(0);
    let mut queue: Vec<_> = (0..2).map(|_| waiting(&slots, station(100))).collect();
    slots.close();
    assert!(slots.is_closed());
    drop(lane);
    assert_eq!(slots.available(), SLOTS);
    for waiter in &mut queue {
        assert_eq!(waiter.woken(), 1);
        assert!(matches!(waiter.poll(), Poll::Ready(Admission::Closed)));
    }
    assert!(matches!(
        Waiting::new(&slots, station(7)).poll(),
        Poll::Ready(Admission::Closed)
    ));
    drop(queue);
    assert_eq!(slots.lanes(), 0);
}
