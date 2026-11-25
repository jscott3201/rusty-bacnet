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
//! slot is free and its lane is under its share takes it, newcomer or not,
//! if its condition (its turn at the destination) holds then. So a waiter
//! that is never polled again holds nothing, and within a lane the first
//! waiter polled goes first. A waiter is asleep until woken, and woken
//! until it next polls. A slot that comes back wakes, for each free slot,
//! one asleep waiter of a lane under its share, as many from a lane as it
//! has room for, oldest first. Lanes go in an order: fewest slots held,
//! then longest since the lane began to wait or was last woken, and a lane
//! goes behind those holding as many once woken. A woken waiter is not
//! woken again until it polls, so the next slot to come back wakes the
//! next asleep waiter, of the same lane or past it, and a lane whose
//! waiters are all woken is passed over. A woken waiter that leaves
//! without taking a free slot (dropped, or its turn not due) wakes the
//! next. The last slot to come back, with nothing held, wakes every asleep
//! waiter: no later one would.
//!
//! No wake is lost: a waiter falls asleep, under the lock, only when it can
//! take no slot, and whatever frees a slot or gives up a wake wakes the
//! next asleep waiter under the same lock. A woken waiter that never polls
//! keeps a free slot from the others only until the next slot comes back,
//! which wakes past it; while slots are held one will, since each is held
//! by a task the session owns, which is always polled (#2755), and with
//! none held none is asleep.
//!
//! So a lane holding none is woken by the (k + 1)th slot to come back,
//! where k counts the lanes that have waited longer since they were last
//! woken, whatever they hold now: each of those is woken at most once
//! first. That bounds when a lane is woken, not when one of its requests is
//! admitted: a newcomer polled first may take the slot. Every held slot
//! comes back within its operation's life: the APDU timeout times the
//! retries plus one (24 s at the session defaults), and for a routed one
//! its wait at the router behind the others of its lane.
//!
//! A slot that comes back wakes at most one waiter per free slot, and
//! finding whom to wake costs a lookup per lane woken. So draining a queue
//! costs a wake and a poll per waiter, as long as each woken waiter polls
//! before the next slot comes back. The last held slot to come back wakes
//! every asleep waiter: a lane's slots coming back one after another with
//! nothing else held wake all its waiters. Wakers are collected under the
//! lock and woken once it is let go, and those replaced or removed are
//! dropped then too; closing wakes every waiter once. The lock is a
//! leaf apart from the condition a waiter checks as it takes a slot, which
//! takes the client's pacer lock (itself a leaf). It is never held across
//! an await, it orders after the database's, and it is never held with the
//! coordinator's or a client's transaction state. A lane's entry goes with
//! its last slot and waiter.
//!
//! A request goes to one station (`SourceAudit::one_station`), checked once
//! it holds its slot: a broadcast or group address takes the lane its form
//! names, its MAC on this link or its network, and is then refused. A
//! routed station whose router isn't known yet is in its network's lane.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::poll_fn;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::sync::MutexExt;

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

/// How a wait for a slot ended.
pub(crate) enum Admission {
    /// With a slot, its condition holding.
    Admitted(Slot),
    /// A slot was free but its condition did not hold: nothing is held.
    NotReady,
    /// The slots are closed.
    Closed,
}

/// How a poll under the lock ended, before `admit` makes the slot.
enum Polled {
    Admitted,
    NotReady,
    Closed,
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
    /// Each lane with an asleep waiter, by the slots it holds and then its
    /// turn: the order lanes are woken in.
    order: BTreeMap<(usize, u64), Lane>,
    /// The next turn and the next waiter's number; both only grow.
    next_turn: u64,
    next_waiter: u64,
    closed: bool,
}

#[derive(Default)]
struct LaneState {
    held: usize,
    /// Its waiters by number, each with the waker its last poll left.
    waiters: BTreeMap<u64, Waker>,
    /// Those not woken since they last polled, oldest first.
    asleep: BTreeSet<u64>,
    /// Its turn in `order`: when it began to wait, or was last woken.
    turn: u64,
}

impl LaneState {
    /// Its key in `order`, while it is there.
    fn key(&self) -> Option<(usize, u64)> {
        (!self.asleep.is_empty()).then_some((self.held, self.turn))
    }
}

/// How many asleep waiters to wake.
#[derive(Clone, Copy)]
enum Wake {
    /// This many, at most as many from a lane as it has room for.
    Up(usize),
    /// Every one, of every lane under its share.
    All,
}

impl State {
    fn free(&self) -> usize {
        SLOTS - self.held
    }

    /// Move `lane` in `order` from `before` to where its entry puts it.
    fn place(&mut self, lane: &Lane, before: Option<(usize, u64)>) {
        if let Some(key) = before {
            self.order.remove(&key);
        }
        if let Some(key) = self.lanes.get(lane).and_then(LaneState::key) {
            self.order.insert(key, lane.clone());
        }
    }

    /// Waiter `number` of `lane`, polled. With a slot free and room in the
    /// lane it takes the slot if `ready()`, and otherwise leaves, passing
    /// on its wake. Else it waits, asleep, with `cx`'s waker.
    fn poll(
        &mut self,
        cx: &Context<'_>,
        lane: &Lane,
        number: u64,
        ready: &mut dyn FnMut() -> bool,
        after: &mut After,
    ) -> Poll<Polled> {
        if self.closed {
            self.remove(lane, number, after);
            return Poll::Ready(Polled::Closed);
        }
        let free = self.held < SLOTS;
        let entry = self.lanes.entry(lane.clone()).or_default();
        let before = entry.key();
        if free && entry.held < LANE_SHARE {
            let admitted = ready();
            if admitted {
                entry.held += 1;
                self.held += 1;
            }
            self.place(lane, before);
            let woken = self.remove(lane, number, after);
            self.forget_if_idle(lane);
            if admitted {
                return Poll::Ready(Polled::Admitted);
            }
            // Woken for the slot it leaves free: the next asleep waiter is.
            if woken {
                self.pass_on(after);
            }
            return Poll::Ready(Polled::NotReady);
        }
        if entry.waiters.is_empty() {
            entry.turn = self.next_turn;
            self.next_turn += 1;
        }
        // Its age is its number, kept from its first poll.
        if let Some(replaced) = entry.waiters.insert(number, cx.waker().clone()) {
            after.spent.push(replaced);
        }
        entry.asleep.insert(number);
        self.place(lane, before);
        Poll::Pending
    }

    /// Waiter `number` of `lane` leaves, if it waits. Whether it was woken
    /// and had not polled since.
    fn remove(&mut self, lane: &Lane, number: u64, after: &mut After) -> bool {
        let Some(entry) = self.lanes.get_mut(lane) else {
            return false;
        };
        let before = entry.key();
        let Some(removed) = entry.waiters.remove(&number) else {
            return false;
        };
        after.spent.push(removed);
        let woken = !entry.asleep.remove(&number);
        self.place(lane, before);
        self.forget_if_idle(lane);
        woken
    }

    /// Waiter `number` of `lane` gives up waiting. One woken for a free
    /// slot passes the wake on.
    fn leave(&mut self, lane: &Lane, number: u64, after: &mut After) {
        if self.remove(lane, number, after) && self.free() > 0 {
            self.pass_on(after);
        }
    }

    /// A free slot's wake goes to the next asleep waiter. With nothing held
    /// none is asleep: the last slot back woke them all, and none can fall
    /// asleep with every slot free.
    fn pass_on(&mut self, after: &mut After) {
        self.wake(Wake::Up(1), after);
    }

    /// A slot of `lane` comes back: one asleep waiter is woken per free
    /// slot, or every one when nothing is held.
    fn release(&mut self, lane: &Lane, after: &mut After) {
        let entry = self.lanes.get_mut(lane).expect("a held slot's lane");
        let before = entry.key();
        entry.held -= 1;
        self.held -= 1;
        self.place(lane, before);
        self.forget_if_idle(lane);
        let wake = if self.held == 0 {
            Wake::All
        } else {
            Wake::Up(self.free())
        };
        self.wake(wake, after);
    }

    /// Wake asleep waiters, oldest first, of the lanes under their share in
    /// order, and move each lane woken behind those holding as many.
    fn wake(&mut self, wake: Wake, after: &mut After) {
        if self.closed {
            return;
        }
        let mut budget = match wake {
            Wake::Up(budget) => budget,
            Wake::All => usize::MAX,
        };
        // Each lane taken wakes at least one waiter.
        let lanes: Vec<(usize, u64)> = (self.order.keys())
            .take_while(|(held, _)| *held < LANE_SHARE)
            .take(budget)
            .copied()
            .collect();
        for key in lanes {
            if budget == 0 {
                break;
            }
            let lane = self.order.remove(&key).expect("a lane in order");
            let entry = self.lanes.get_mut(&lane).expect("a waiting lane");
            let room = match wake {
                Wake::Up(_) => LANE_SHARE - entry.held,
                Wake::All => usize::MAX,
            };
            for _ in 0..budget.min(room) {
                let Some(number) = entry.asleep.pop_first() else {
                    break;
                };
                after.wake.push(entry.waiters[&number].clone());
                budget -= 1;
            }
            entry.turn = self.next_turn;
            self.next_turn += 1;
            if let Some(key) = entry.key() {
                self.order.insert(key, lane);
            }
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

/// What a change under the lock leaves for after it is let go: wakers to
/// wake, and wakers it replaced or removed, dropped there too, since
/// dropping one may run code that takes this lock.
#[derive(Default)]
struct After {
    wake: Vec<Waker>,
    spent: Vec<Waker>,
}

impl After {
    /// Run it, the lock let go.
    fn run(self) {
        for waker in self.wake {
            waker.wake();
        }
        drop(self.spent);
    }
}

impl Slots {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.locked()
    }

    /// Wait for a slot in `lane`, as the module documentation describes.
    /// `ready` is checked under the lock as a slot is taken, and must not
    /// take this lock: when it is false the wait ends with nothing held.
    /// Dropping the wait gives up its place; it holds nothing until it has
    /// its slot.
    pub(crate) async fn admit(
        self: &Arc<Self>,
        lane: Lane,
        mut ready: impl FnMut() -> bool,
    ) -> Admission {
        let mut place = Place {
            slots: self,
            lane: &lane,
            number: None,
        };
        let admission = poll_fn(|cx| {
            let mut after = After::default();
            let polled = {
                let mut state = self.lock();
                let number = *place.number.get_or_insert_with(|| {
                    state.next_waiter += 1;
                    state.next_waiter
                });
                state.poll(cx, &lane, number, &mut ready, &mut after)
            };
            after.run();
            polled
        })
        .await;
        // Whatever the end, it waits no more.
        place.number = None;
        drop(place);
        match admission {
            Polled::Admitted => Admission::Admitted(Slot {
                slots: Arc::clone(self),
                lane,
            }),
            Polled::NotReady => Admission::NotReady,
            Polled::Closed => Admission::Closed,
        }
    }

    /// End every wait, and admit nothing more. Slots already held stay held
    /// until they drop.
    pub(crate) fn close(&self) {
        let wake = {
            let mut state = self.lock();
            state.closed = true;
            (state.lanes.values())
                .flat_map(|entry| entry.waiters.values())
                .cloned()
                .collect()
        };
        After {
            wake,
            spent: Vec::new(),
        }
        .run();
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
        let mut after = After::default();
        self.slots.lock().release(&self.lane, &mut after);
        after.run();
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
            let mut after = After::default();
            self.slots.lock().leave(self.lane, number, &mut after);
            after.run();
        }
    }
}

#[cfg(test)]
#[path = "source_audit_slots_tests.rs"]
mod tests;
