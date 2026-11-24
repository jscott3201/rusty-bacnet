//! The source Audit's operation slots, shared out by lane (#2652).
//!
//! A session holds at most [`SLOTS`] audited operations at once, each from
//! its admission to its end: its turn at the destination, its retries and,
//! when routed, its wait for the router's path. A station on this link is a
//! lane of its own, by its MAC. Every station on another network shares
//! that network's lane, since the client sends one request at a time
//! through a router to a network: the others wait there holding their
//! slots. No lane holds more than [`LANE_SHARE`] slots. A request whose lane
//! is at its share waits behind that lane only; one to another lane is
//! admitted while a slot is free.
//!
//! The numbers: one silent station or network holds 4 slots and leaves 60,
//! eight leave 32, and it takes 16 to fill the pool. Even then a lane
//! holding none goes ahead of every lane holding some (below), so a request
//! to a live device waits for slots to come back, not for a silent
//! device's queue. Four at once is ample for one device, which often
//! handles one transaction at a time, and for one network, where a router
//! passes one at a time.
//!
//! The waiters of one lane go first come, first served. A slot that comes
//! back goes to a lane under its share with a waiter: the one holding the
//! fewest slots, and of those, the one that has waited longest since it
//! began to wait or last got a slot. So a lane holding none goes ahead of
//! every lane holding some, and each lane waiting ahead of it gets at most
//! one slot first: with k lanes ahead, its first waiter has a slot by the
//! (k + 1)th slot to come back. Every held slot comes back within its
//! operation's life: the APDU timeout times the retries plus one (24 s at
//! the session defaults), its turn at the destination, and for a routed one
//! its wait at the router behind the others of its lane. With the pool full
//! and fewer than 64 lanes ahead, that is within one slot's life.
//!
//! A slot that comes back is granted, under the lock, to the next waiter,
//! which is woken once the lock is let go: one wake per slot, and every
//! waiter at once when the slots close. A grant made before the waiter has
//! left its waker is found by its next poll. A waiter dropped before it
//! takes its grant gives the slot back at once, so a dropped wait holds
//! nothing; one never polled again but not dropped keeps its grant, as a
//! semaphore's waiter does, which is at most its lane's share. The lock is
//! a leaf: held only inside these methods, never across an await, and no
//! other lock is taken under it, so it orders after the database's and is
//! never held with the coordinator's or a client's transaction state. A
//! lane's entry goes with its last slot and waiter.
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
    /// Slots held, those granted to a waiter that has yet to take one too.
    held: usize,
    lanes: HashMap<Lane, LaneState>,
    /// Each lane with a waiter, by the slots it holds and then its turn:
    /// the first one under its share is served next.
    order: BTreeMap<(usize, u64), Lane>,
    /// The next turn and the next waiter's number; both only grow.
    next_turn: u64,
    next_waiter: u64,
    closed: bool,
}

#[derive(Default)]
struct LaneState {
    held: usize,
    /// Oldest first; their numbers rise front to back.
    waiters: VecDeque<Waiter>,
    /// The waiters granted a slot that they have yet to take.
    granted: Vec<u64>,
    /// Its turn in `order` while it has waiters: when it began to wait, or
    /// last got a slot with more waiting.
    turn: u64,
}

struct Waiter {
    number: u64,
    waker: Option<Waker>,
}

enum Arrival {
    Admitted,
    Queued(u64),
}

impl State {
    /// Admit a request to `lane` at once, or queue it; `None` once closed.
    fn arrive(&mut self, lane: &Lane) -> Option<Arrival> {
        if self.closed {
            return None;
        }
        let free = self.held < SLOTS;
        let entry = self.lanes.entry(lane.clone()).or_default();
        if free && entry.held < LANE_SHARE {
            // A lane with room and a waiter while a slot is free can't be:
            // the slot went to its first waiter as it came back.
            debug_assert!(entry.waiters.is_empty(), "a waiter was passed over");
            entry.held += 1;
            self.held += 1;
            return Some(Arrival::Admitted);
        }
        let number = self.next_waiter;
        self.next_waiter += 1;
        if entry.waiters.is_empty() {
            entry.turn = self.next_turn;
            self.next_turn += 1;
            self.order.insert((entry.held, entry.turn), lane.clone());
        }
        entry.waiters.push_back(Waiter {
            number,
            waker: None,
        });
        Some(Arrival::Queued(number))
    }

    /// Waiter `number` of `lane`: granted (`true`), closed (`false`), or
    /// still waiting, keeping `cx`'s waker.
    fn poll(&mut self, cx: &Context<'_>, lane: &Lane, number: u64) -> Poll<bool> {
        if self.closed {
            return Poll::Ready(false);
        }
        let Some(entry) = self.lanes.get_mut(lane) else {
            return Poll::Ready(false);
        };
        if let Some(at) = entry.granted.iter().position(|&n| n == number) {
            entry.granted.swap_remove(at);
            return Poll::Ready(true);
        }
        let Ok(at) = entry.waiters.binary_search_by_key(&number, |w| w.number) else {
            return Poll::Ready(false);
        };
        let waker = &mut entry.waiters[at].waker;
        if !waker.as_ref().is_some_and(|w| w.will_wake(cx.waker())) {
            *waker = Some(cx.waker().clone());
        }
        Poll::Pending
    }

    /// Waiter `number` of `lane` gives up: its place, or the slot it was
    /// granted, which goes on. What to wake.
    fn leave(&mut self, lane: &Lane, number: u64) -> Option<Waker> {
        let entry = self.lanes.get_mut(lane)?;
        if let Some(at) = entry.granted.iter().position(|&n| n == number) {
            entry.granted.swap_remove(at);
            return self.release(lane);
        }
        let at = (entry.waiters)
            .binary_search_by_key(&number, |w| w.number)
            .ok()?;
        entry.waiters.remove(at);
        // A lane whose head left was served no later than it would have
        // been: with a slot free and room in the lane, the head would have
        // had it already.
        if entry.waiters.is_empty() {
            self.order.remove(&(entry.held, entry.turn));
            self.forget_if_idle(lane);
        }
        None
    }

    /// A slot of `lane` comes back and goes on. What to wake.
    fn release(&mut self, lane: &Lane) -> Option<Waker> {
        let entry = self.lanes.get_mut(lane).expect("a held slot's lane");
        if !entry.waiters.is_empty() {
            self.order.remove(&(entry.held, entry.turn));
            self.order
                .insert((entry.held - 1, entry.turn), lane.clone());
        }
        entry.held -= 1;
        self.held -= 1;
        self.forget_if_idle(lane);
        self.grant()
    }

    /// Grant the slot that came back to the first waiter of the first lane
    /// in order, if that lane is under its share. What to wake.
    fn grant(&mut self) -> Option<Waker> {
        debug_assert!(self.held < SLOTS, "a slot came back");
        if self.closed {
            return None;
        }
        let (&(held, turn), _) = self.order.first_key_value()?;
        if held >= LANE_SHARE {
            return None;
        }
        let lane = self.order.remove(&(held, turn)).expect("the first lane");
        let entry = self.lanes.get_mut(&lane).expect("a waiting lane");
        let waiter = entry.waiters.pop_front().expect("a waiting lane's waiter");
        entry.held += 1;
        entry.granted.push(waiter.number);
        self.held += 1;
        if !entry.waiters.is_empty() {
            entry.turn = self.next_turn;
            self.next_turn += 1;
            self.order.insert((entry.held, entry.turn), lane);
        }
        waiter.waker
    }

    fn forget_if_idle(&mut self, lane: &Lane) {
        if (self.lanes.get(lane)).is_some_and(|entry| entry.held == 0 && entry.waiters.is_empty()) {
            self.lanes.remove(lane);
        }
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

    /// Wait for a slot in `lane`, as the module documentation describes: at
    /// once while one is free, the lane is under its share and none of its
    /// requests waits. `None` once the slots are closed. Dropping the wait
    /// gives up its place, and a slot granted to it.
    pub(crate) async fn admit(self: &Arc<Self>, lane: Lane) -> Option<Slot> {
        let arrival = self.lock().arrive(&lane)?;
        let number = match arrival {
            Arrival::Admitted => {
                return Some(Slot {
                    slots: Arc::clone(self),
                    lane,
                });
            }
            Arrival::Queued(number) => number,
        };
        let mut place = Place {
            slots: self,
            lane: &lane,
            number: Some(number),
        };
        let granted = poll_fn(|cx| self.lock().poll(cx, &lane, number)).await;
        if granted {
            place.number = None;
        }
        drop(place);
        granted.then(|| Slot {
            slots: Arc::clone(self),
            lane,
        })
    }

    /// End every wait, and admit and grant nothing more. Slots already
    /// held stay held until they drop.
    pub(crate) fn close(&self) {
        let wakers: Vec<Waker> = {
            let mut state = self.lock();
            state.closed = true;
            (state.lanes.values_mut())
                .flat_map(|entry| entry.waiters.iter_mut())
                .filter_map(|waiter| waiter.waker.take())
                .collect()
        };
        for waker in wakers {
            waker.wake();
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.lock().closed
    }

    /// How many slots are free.
    #[cfg(test)]
    pub(crate) fn available(&self) -> usize {
        SLOTS - self.lock().held
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
        let waker = self.slots.lock().release(&self.lane);
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// A waiter's place, given up as it drops before it has its slot.
struct Place<'a> {
    slots: &'a Slots,
    lane: &'a Lane,
    /// `None` once it has its slot.
    number: Option<u64>,
}

impl Drop for Place<'_> {
    fn drop(&mut self) {
        if let Some(number) = self.number.take() {
            let waker = self.slots.lock().leave(self.lane, number);
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}

#[cfg(test)]
#[path = "source_audit_slots_tests.rs"]
mod tests;
