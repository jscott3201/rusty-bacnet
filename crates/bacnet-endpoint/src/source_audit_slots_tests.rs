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

    /// The slot its next poll takes.
    fn slot(mut self) -> Slot {
        match self.poll() {
            Poll::Ready(Some(slot)) => slot,
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
    assert!(behind.poll().is_pending());
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
/// first.
#[test]
fn lanes_holding_as_many_take_turns() {
    let slots = Arc::new(Slots::new());
    let mut lanes = hold(&slots, 64, 1);
    let first = waiting(&slots, station(100));
    let mut second = waiting(&slots, station(100));
    let late = waiting(&slots, station(99));
    // Both hold none once station 100's slot comes back; it waited longer,
    // and every waiter of the lane is woken.
    drop(lanes[0].pop());
    assert_eq!((first.woken(), second.woken(), late.woken()), (1, 1, 0));
    // Again none, but it has just been woken: the late lane's turn.
    drop(first.slot());
    assert_eq!((second.woken(), late.woken()), (1, 1));
    let _late = late.slot();
    assert!(second.poll().is_pending());
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

/// Within a lane, the first waiter polled with a slot free goes, whatever
/// the order they came in.
#[test]
fn within_a_lane_the_first_polled_goes() {
    let slots = Arc::new(Slots::new());
    let mut lane = hold(&slots, 1, LANE_SHARE).remove(0);
    let mut queue: Vec<_> = (0..3).map(|_| waiting(&slots, station(100))).collect();
    drop(lane.pop());
    let woken: Vec<_> = queue.iter().map(Waiting::woken).collect();
    assert_eq!(woken, [1, 1, 1]);
    let _last = queue.remove(2).slot();
    assert!(queue[0].poll().is_pending() && queue[1].poll().is_pending());
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
    assert!(stuck.iter().all(|waiter| waiter.woken() == 1));
    let write = held(&slots, station(100));
    drop(running);
    assert_eq!(slots.available(), SLOTS - 1, "only the write holds one");
    let again = held(&slots, station(100));
    drop((write, again, stuck));
    assert_eq!(slots.available(), SLOTS);
    assert_eq!(slots.lanes(), 0);
}

/// A lane woken for a slot whose waiters are never polled holds no slot
/// and keeps no other lane out: a newcomer takes the slot at once, the
/// next slot to come back wakes the lane after it, and a slot it leaves
/// free reaches every lane with the next one to come back.
#[test]
fn a_woken_lane_that_never_polls_keeps_no_one_out() {
    let slots = Arc::new(Slots::new());
    let mut lanes = hold(&slots, 16, LANE_SHARE);
    let stuck = [waiting(&slots, station(50)), waiting(&slots, station(50))];
    let other = waiting(&slots, station(51));
    drop(lanes[0].pop());
    assert_eq!((stuck[0].woken(), other.woken()), (1, 0));
    let newcomer = held(&slots, station(52));
    drop(lanes[1].pop());
    assert_eq!(other.woken(), 1, "the stuck lane went behind");
    let other = other.slot();
    // Two lanes that won't poll are now ahead of a third that will.
    let also_stuck = waiting(&slots, station(54));
    let late = waiting(&slots, station(53));
    drop(lanes[2].pop());
    assert_eq!((stuck[0].woken(), late.woken()), (2, 0), "its turn again");
    drop(lanes[3].pop());
    assert_eq!(also_stuck.woken(), 1);
    assert_eq!(late.woken(), 1, "two free: every lane is woken");
    let late = late.slot();
    drop((lanes, newcomer, stuck, also_stuck, other, late));
    assert_eq!(slots.available(), SLOTS);
    assert_eq!(slots.lanes(), 0);
}

/// A waiter dropped, polled or not, leaves its lane; a slot that comes back
/// then goes to those still waiting. Nothing is held after, and the lane's
/// entry goes with its last slot.
#[test]
fn a_dropped_wait_holds_nothing() {
    let slots = Arc::new(Slots::new());
    let mut lane = hold(&slots, 1, LANE_SHARE).remove(0);
    let mut queue: Vec<_> = (0..3).map(|_| waiting(&slots, station(100))).collect();
    drop(queue.remove(1));
    drop(lane.pop());
    // Woken and dropped before it is polled: the slot is still free.
    drop(queue.remove(0));
    assert_eq!(queue[0].woken(), 2, "woken again as the other left");
    let last = queue.remove(0).slot();
    assert_eq!(slots.available(), SLOTS - LANE_SHARE);
    drop((lane, last));
    assert_eq!(slots.available(), SLOTS);
    assert_eq!(slots.lanes(), 0);
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
        assert!(matches!(waiter.poll(), Poll::Ready(None)));
    }
    assert!(matches!(
        Waiting::new(&slots, station(7)).poll(),
        Poll::Ready(None)
    ));
    drop(queue);
    assert_eq!(slots.lanes(), 0);
}
