//! The source Audit's operation slots, shared out by lane (#2652).
//!
//! A session holds at most [`SLOTS`] audited operations at once, each from
//! its admission to its end: its retries and, when routed, its wait for the
//! router's path. A station on this link is a lane of its own, by its MAC.
//! Every station on another network shares that network's lane, since the
//! client sends one request at a time through a router to a network: the
//! others wait there holding their slots. No lane holds more than
//! [`LANE_SHARE`] slots. A request whose lane is at its share waits behind
//! that lane only; one to another lane is admitted while a slot is free.
//!
//! The numbers: one silent station or network holds 4 slots and leaves 60,
//! eight leave 32, and it takes 16 to fill the pool. Even then a lane
//! holding none is woken ahead of every lane holding some (below), so a
//! request to a live device waits for slots to come back, not for a silent
//! device's queue. Four at once is ample for one device, which often
//! handles one transaction at a time, and for one network, where a router
//! passes one at a time.
//!
//! A slot is taken only by a waiter as it is polled: one polled while a
//! slot is free and its lane is under its share takes it, newcomer or not.
//! So a waiter that is never polled again holds nothing and keeps no one
//! else out, and within a lane the first waiter polled goes first, not the
//! first to come. Who is woken follows an order. A slot that comes back
//! wakes every waiter of the first lane under its share in that order, and
//! moves that lane behind the others holding as many. The order is fewest
//! slots held, then longest wait since the lane began to wait or was last
//! woken. A slot that comes back while another is still free (the lane
//! woken for that one has not taken it up) wakes every waiter of every lane
//! under its share, and so does a waiter that leaves while a slot is free.
//!
//! So a lane holding none is woken by the (k + 1)th slot to come back,
//! where k counts the lanes that have waited longer since they were last
//! woken, whatever they hold now: each of those is woken at most once
//! first. Woken, it takes the slot unless a request polled before it did;
//! a slot it leaves free reaches everyone with the next one. Every held
//! slot comes back within its operation's life: the APDU timeout times the
//! retries plus one (24 s at the session defaults), and for a routed one
//! its wait at the router behind the others of its lane.
//!
//! Wakers are collected under the lock and woken once it is let go; closing
//! wakes every waiter once. The lock is a leaf: held only inside these
//! methods, never across an await, and no other lock is taken under it, so
//! it orders after the database's and is never held with the
//! coordinator's or a client's transaction state. A lane's entry goes with
//! its last slot and waiter.
//!
//! A request goes to one station (`SourceAudit::one_station`), checked once
//! it holds its slot: a broadcast or group address takes the lane its form
//! names, its MAC on this link or its network, and is then refused. A
//! routed station whose router isn't known yet is in its network's lane.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::poll_fn;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetAddress;

/// The audited operations a session holds at once.
pub(crate) const SLOTS: usize = 64;

/// The most slots one lane holds at once.
pub(crate) const LANE_SHARE: usize = 4;

/// What the slots are shared out by.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Lane {
    /// A station on this link, by its MAC.
    Station(MacAddr),
    /// Every station on another network, by its number.
    Network(u16),
}

impl Lane {
    /// The lane of a request to `station`, network 0 for one on this link.
    pub(crate) fn of(station: &BACnetAddress) -> Self {
        match station.network_number {
            0 => Self::Station(MacAddr::from_slice(&station.mac_address)),
            network => Self::Network(network),
        }
    }
}

/// The slots, by lane, and the requests waiting for one.
pub(crate) struct Slots {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Slots held.
    held: usize,
    lanes: HashMap<Lane, LaneState>,
    /// Each lane with a waiter, by the slots it holds and then its turn:
    /// the order lanes are woken in.
    order: BTreeMap<(usize, u64), Lane>,
    /// The next turn and the next waiter's number; both only grow.
    next_turn: u64,
    next_waiter: u64,
    closed: bool,
}

#[derive(Default)]
struct LaneState {
    held: usize,
    /// Each with the waker its last poll left; numbers rise front to back.
    waiters: VecDeque<Waiter>,
    /// Its turn in `order` while it has waiters: when it began to wait, or
    /// was last woken.
    turn: u64,
}

struct Waiter {
    number: u64,
    waker: Waker,
}

impl State {
    fn free(&self) -> usize {
        SLOTS - self.held
    }

    /// Waiter `number` of `lane`, polled: it takes a slot (`true`) if one
    /// is free and the lane is under its share, or waits with `cx`'s waker.
    /// `false` once closed.
    fn poll(&mut self, cx: &Context<'_>, lane: &Lane, number: u64) -> Poll<bool> {
        if self.closed {
            self.remove(lane, number);
            return Poll::Ready(false);
        }
        let free = self.held < SLOTS;
        let entry = self.lanes.entry(lane.clone()).or_default();
        let at = entry.waiters.binary_search_by_key(&number, |w| w.number);
        if free && entry.held < LANE_SHARE {
            let key = (entry.held, entry.turn);
            if let Ok(at) = at {
                entry.waiters.remove(at);
            }
            if at.is_ok() || !entry.waiters.is_empty() {
                self.order.remove(&key);
            }
            entry.held += 1;
            self.held += 1;
            if !entry.waiters.is_empty() {
                self.order.insert((entry.held, entry.turn), lane.clone());
            }
            return Poll::Ready(true);
        }
        match at {
            Ok(at) => {
                let waker = &mut entry.waiters[at].waker;
                if !waker.will_wake(cx.waker()) {
                    *waker = cx.waker().clone();
                }
            }
            Err(_) => {
                if entry.waiters.is_empty() {
                    entry.turn = self.next_turn;
                    self.next_turn += 1;
                    self.order.insert((entry.held, entry.turn), lane.clone());
                }
                entry.waiters.push_back(Waiter {
                    number,
                    waker: cx.waker().clone(),
                });
            }
        }
        Poll::Pending
    }

    /// Waiter `number` of `lane` leaves, if it waits.
    fn remove(&mut self, lane: &Lane, number: u64) {
        let Some(entry) = self.lanes.get_mut(lane) else {
            return;
        };
        let Ok(at) = (entry.waiters).binary_search_by_key(&number, |w| w.number) else {
            return;
        };
        entry.waiters.remove(at);
        if entry.waiters.is_empty() {
            self.order.remove(&(entry.held, entry.turn));
            self.forget_if_idle(lane);
        }
    }

    /// Waiter `number` of `lane` gives up waiting. A slot left free for
    /// it goes to everyone.
    fn leave(&mut self, lane: &Lane, number: u64, wakers: &mut Vec<Waker>) {
        self.remove(lane, number);
        if self.free() > 0 {
            self.wake(true, wakers);
        }
    }

    /// A slot of `lane` comes back. What it wakes is in the module
    /// documentation.
    fn release(&mut self, lane: &Lane, wakers: &mut Vec<Waker>) {
        let entry = self.lanes.get_mut(lane).expect("a held slot's lane");
        if !entry.waiters.is_empty() {
            self.order.remove(&(entry.held, entry.turn));
            self.order
                .insert((entry.held - 1, entry.turn), lane.clone());
        }
        entry.held -= 1;
        self.held -= 1;
        self.forget_if_idle(lane);
        let everyone = self.free() > 1;
        self.wake(everyone, wakers);
    }

    /// Wake every waiter of the first lane under its share in order, or of
    /// every such lane, and move each behind those holding as many.
    fn wake(&mut self, everyone: bool, wakers: &mut Vec<Waker>) {
        if self.closed {
            return;
        }
        let woken: Vec<(usize, u64)> = (self.order.keys())
            .take_while(|(held, _)| *held < LANE_SHARE)
            .take(if everyone { usize::MAX } else { 1 })
            .copied()
            .collect();
        for key in woken {
            let lane = self.order.remove(&key).expect("a waiting lane");
            let entry = self.lanes.get_mut(&lane).expect("a waiting lane");
            wakers.extend(entry.waiters.iter().map(|w| w.waker.clone()));
            entry.turn = self.next_turn;
            self.next_turn += 1;
            self.order.insert((entry.held, entry.turn), lane);
        }
    }

    fn forget_if_idle(&mut self, lane: &Lane) {
        let idle =
            (self.lanes.get(lane)).is_some_and(|entry| entry.held == 0 && entry.waiters.is_empty());
        if idle {
            self.lanes.remove(lane);
        }
    }
}

/// Wake `wakers`, the lock let go.
fn wake_all(wakers: Vec<Waker>) {
    for waker in wakers {
        waker.wake();
    }
}

impl Slots {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wait for a slot in `lane`, as the module documentation describes.
    /// `None` once the slots are closed. Dropping the wait gives up its
    /// place; it holds nothing until it has its slot.
    pub(crate) async fn admit(self: &Arc<Self>, lane: Lane) -> Option<Slot> {
        let mut place = Place {
            slots: self,
            lane: &lane,
            number: None,
        };
        let admitted = poll_fn(|cx| {
            let mut state = self.lock();
            let number = *place.number.get_or_insert_with(|| {
                state.next_waiter += 1;
                state.next_waiter
            });
            state.poll(cx, &lane, number)
        })
        .await;
        // Admitted or closed, it waits no more.
        place.number = None;
        drop(place);
        admitted.then(|| Slot {
            slots: Arc::clone(self),
            lane,
        })
    }

    /// End every wait, and admit nothing more. Slots already held stay held
    /// until they drop.
    pub(crate) fn close(&self) {
        let wakers: Vec<Waker> = {
            let mut state = self.lock();
            state.closed = true;
            (state.lanes.values())
                .flat_map(|entry| entry.waiters.iter())
                .map(|waiter| waiter.waker.clone())
                .collect()
        };
        wake_all(wakers);
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.lock().closed
    }

    /// How many slots are free.
    #[cfg(test)]
    pub(crate) fn available(&self) -> usize {
        self.lock().free()
    }

    /// How many lanes have a slot or a waiter.
    #[cfg(test)]
    pub(crate) fn lanes(&self) -> usize {
        self.lock().lanes.len()
    }
}

/// One held slot, given back as it drops.
pub(crate) struct Slot {
    slots: Arc<Slots>,
    lane: Lane,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut wakers = Vec::new();
        self.slots.lock().release(&self.lane, &mut wakers);
        wake_all(wakers);
    }
}

/// A waiter's place, given up as it drops before it has its slot.
struct Place<'a> {
    slots: &'a Slots,
    lane: &'a Lane,
    /// Its number once polled; `None` once it waits no more.
    number: Option<u64>,
}

impl Drop for Place<'_> {
    fn drop(&mut self) {
        if let Some(number) = self.number.take() {
            let mut wakers = Vec::new();
            self.slots.lock().leave(self.lane, number, &mut wakers);
            wake_all(wakers);
        }
    }
}

#[cfg(test)]
#[path = "source_audit_slots_tests.rs"]
mod tests;
