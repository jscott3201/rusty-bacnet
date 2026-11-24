//! The slots' sharing rule (#2652), polled by hand: no runtime, no clock.
use super::*;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
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

/// One request's wait for a slot, polled by hand.
struct Waiting {
    future: Pin<Box<dyn Future<Output = Option<Slot>>>>,
    wakes: Arc<Wakes>,
}

impl Waiting {
    fn new(slots: &Arc<Slots>, lane: Lane) -> Self {
        let slots = Arc::clone(slots);
        Self {
            future: Box::pin(async move { slots.admit(lane).await }),
            wakes: Arc::default(),
        }
    }

    fn poll(&mut self) -> Poll<Option<Slot>> {
        let waker = Waker::from(Arc::clone(&self.wakes));
        (self.future.as_mut()).poll(&mut Context::from_waker(&waker))
    }

    fn woken(&self) -> usize {
        self.wakes.0.load(Ordering::SeqCst)
    }

    /// The slot it was granted, as its next poll takes it.
    fn slot(mut self) -> Slot {
        match self.poll() {
            Poll::Ready(Some(slot)) => slot,
            _ => panic!("granted a slot"),
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
    assert!(waiting.poll().is_pending(), "it waits");
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
/// while a slot is free, and the waiter goes when its own lane frees one.
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
    drop(other);
    assert_eq!((over.woken(), behind.woken()), (0, 0));
    drop(silent.pop());
    assert_eq!((over.woken(), behind.woken()), (1, 0));
    let _over = over.slot();
    assert!(behind.poll().is_pending());
    drop(routed);
}

/// The pool holds 64. A slot that comes back goes to the lane holding the
/// fewest, ahead of one that waited longer but holds more.
#[test]
fn a_full_pool_serves_the_lane_holding_fewest_first() {
    let slots = Arc::new(Slots::new());
    let mut lanes = hold(&slots, 16, LANE_SHARE);
    assert_eq!(slots.available(), 0);
    let first = waiting(&slots, station(100));
    let fresh = waiting(&slots, station(50));
    drop(lanes[0].pop());
    assert_eq!((first.woken(), fresh.woken()), (0, 1));
    let _fresh = fresh.slot();
    assert_eq!(slots.available(), 0);
    drop(lanes);
    assert_eq!(first.woken(), 1, "its own lane's slot came back");
}

/// Lanes holding as many take turns: one that just got a slot goes behind
/// the others that wait, so each lane waiting ahead gets at most one first.
#[test]
fn lanes_holding_as_many_take_turns() {
    let slots = Arc::new(Slots::new());
    let mut lanes = hold(&slots, 64, 1);
    assert_eq!(slots.available(), 0);
    let first = waiting(&slots, station(100));
    let mut second = waiting(&slots, station(100));
    let late = waiting(&slots, station(99));
    // Both hold none once station 100's slot comes back; it waited longer.
    drop(lanes[0].pop());
    assert_eq!((first.woken(), late.woken()), (1, 0));
    // Again none, but it has just been served: the late lane's turn.
    drop(first.slot());
    assert_eq!((second.woken(), late.woken()), (0, 1));
    let _late = late.slot();
    assert!(second.poll().is_pending());
}

/// With k lanes holding none ahead of it, a lane's waiter is served by the
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

/// One lane's waiters go first come, first served, one wake per slot.
#[test]
fn a_lanes_waiters_go_in_order() {
    let slots = Arc::new(Slots::new());
    let mut lane = hold(&slots, 1, LANE_SHARE).remove(0);
    let mut queue: Vec<_> = (0..3).map(|_| waiting(&slots, station(100))).collect();
    drop(lane.pop());
    let woken: Vec<_> = queue.iter().map(Waiting::woken).collect();
    assert_eq!(woken, [1, 0, 0]);
    let _first = queue.remove(0).slot();
    assert!(queue[0].poll().is_pending() && queue[1].poll().is_pending());
    drop(lane.pop());
    assert_eq!((queue[0].woken(), queue[1].woken()), (1, 0));
}

/// A waiter dropped in the queue leaves it; one dropped after it was
/// granted a slot, before it took it, gives the slot on at once. Neither
/// holds anything after, and the lane's entry goes with its last slot.
#[test]
fn a_dropped_wait_holds_nothing() {
    let slots = Arc::new(Slots::new());
    let mut lane = hold(&slots, 1, LANE_SHARE).remove(0);
    let mut queue: Vec<_> = (0..3).map(|_| waiting(&slots, station(100))).collect();
    drop(queue.remove(1));
    drop(lane.pop());
    assert_eq!((queue[0].woken(), queue[1].woken()), (1, 0));
    // Granted and never polled: it holds its lane's slot, no other lane's.
    let other = held(&slots, station(7));
    drop(queue.remove(0));
    assert_eq!(queue[0].woken(), 1, "the grant went on");
    let last = queue.remove(0).slot();
    assert_eq!(slots.available(), SLOTS - LANE_SHARE - 1);
    drop((lane, last, other));
    assert_eq!(slots.available(), SLOTS);
    assert_eq!(slots.lanes(), 0);
}

/// Closing wakes every waiter once, each to fail; slots that come back
/// after are granted to none of them, and nothing more is admitted.
#[test]
fn closing_ends_every_wait() {
    let slots = Arc::new(Slots::new());
    let lane = hold(&slots, 1, LANE_SHARE).remove(0);
    let mut queue: Vec<_> = (0..2).map(|_| waiting(&slots, station(100))).collect();
    slots.close();
    assert!(slots.is_closed());
    drop(lane);
    assert_eq!(slots.available(), SLOTS, "none granted");
    for waiter in &mut queue {
        assert_eq!(waiter.woken(), 1);
        assert!(matches!(waiter.poll(), Poll::Ready(None)));
    }
    assert!(matches!(
        Waiting::new(&slots, station(7)).poll(),
        Poll::Ready(None)
    ));
    drop(queue);
    assert_eq!(slots.available(), SLOTS);
    assert_eq!(slots.lanes(), 0);
}
