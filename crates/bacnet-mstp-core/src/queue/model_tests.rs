//! Both queue backends against a reference model of the queues as they
//! were before their items stayed in fixed slots (#1691): two vectors in
//! arrival order, an item taken out with `Vec::remove` and copied out by
//! value. Random operations run on both; after each one, its result and
//! every item each holds must match, with its wait, and so must the peeks,
//! depths and counters.

use std::vec::Vec;

use bacnet_mstp_codec::BROADCAST_MAC;
use bacnet_time::{Duration, Instant};
use proptest::prelude::*;

use super::{HeaplessQueues, NetworkPriority, QueueItemInfo, QueueKind, QueueRefusal};
use super::{QueueCounters, QueueStorage};

/// The slot length under test: short, so NPDUs at, under and over it come
/// up often.
const N: usize = 6;
const UNICAST: usize = 3;
const BROADCAST: usize = 2;

/// An item taken off a queue, owned so the model and a queue can be
/// compared: its destination, class, priority, sequence and NPDU.
type Taken = (u8, QueueKind, NetworkPriority, u64, Vec<u8>);

#[derive(Clone, Debug)]
struct Item {
    destination: u8,
    priority: NetworkPriority,
    enqueued_at: Instant,
    expires_at: Option<Instant>,
    sequence: u64,
    service_wait: u8,
    npdu: Vec<u8>,
}

impl Item {
    fn info(&self, kind: QueueKind) -> QueueItemInfo {
        QueueItemInfo {
            destination: self.destination,
            kind,
            priority: self.priority,
            len: self.npdu.len(),
            enqueued_at: self.enqueued_at,
            expires_at: self.expires_at,
            sequence: self.sequence,
            service_wait: self.service_wait,
        }
    }
}

const KINDS: [QueueKind; 2] = [QueueKind::Unicast, QueueKind::Broadcast];

fn lane(kind: QueueKind) -> usize {
    match kind {
        QueueKind::Unicast => 0,
        QueueKind::Broadcast => 1,
    }
}

fn expired(expires_at: Option<Instant>, now: Instant) -> bool {
    expires_at.is_some_and(|deadline| now >= deadline)
}

/// The selection policy, written out again: a starved item first, the
/// oldest of them, else the highest priority, the oldest within it.
fn serves_first(candidate: &Item, current: &Item, limit: u8) -> bool {
    let older = |left: u64, right: u64| left != right && left.wrapping_sub(right) > u64::MAX / 2;
    let starved = (
        candidate.service_wait >= limit,
        current.service_wait >= limit,
    );
    match starved {
        (true, false) => true,
        (false, true) => false,
        (true, true) => older(candidate.sequence, current.sequence),
        (false, false) => {
            candidate.priority > current.priority
                || (candidate.priority == current.priority
                    && older(candidate.sequence, current.sequence))
        }
    }
}

/// The queues as they were. `HeaplessQueues` took an item's sequence number
/// before it found the queue full, so a full refusal used one up;
/// `AllocQueues` didn't.
struct Model {
    lanes: [Vec<Item>; 2],
    capacity: [usize; 2],
    counters: [QueueCounters; 2],
    sequence: u64,
    limit: u8,
    numbers_full_refusals: bool,
}

impl Model {
    fn new(limit: u8, numbers_full_refusals: bool) -> Self {
        Self {
            lanes: [Vec::new(), Vec::new()],
            capacity: [UNICAST, BROADCAST],
            counters: [QueueCounters::default(); 2],
            sequence: 0,
            limit,
            numbers_full_refusals,
        }
    }

    fn enqueue(
        &mut self,
        destination: u8,
        npdu: &[u8],
        priority: NetworkPriority,
        now: Instant,
        expires_at: Option<Instant>,
    ) -> Result<(), QueueRefusal> {
        let k = lane(QueueKind::from_destination(destination));
        if npdu.len() > N {
            self.counters[k].record_drop_oversize();
            return Err(QueueRefusal::Oversize);
        }
        if expired(expires_at, now) {
            self.counters[k].record_drop_expired();
            return Err(QueueRefusal::Expired);
        }
        let sequence = self.sequence;
        if self.numbers_full_refusals {
            self.sequence = self.sequence.wrapping_add(1);
        }
        if self.lanes[k].len() >= self.capacity[k] {
            self.counters[k].record_drop_full();
            return Err(QueueRefusal::Full);
        }
        if !self.numbers_full_refusals {
            self.sequence = self.sequence.wrapping_add(1);
        }
        self.lanes[k].push(Item {
            destination,
            priority,
            enqueued_at: now,
            expires_at,
            sequence,
            service_wait: 0,
            npdu: npdu.to_vec(),
        });
        let depth = self.lanes[k].len();
        self.counters[k].record_enqueue(depth);
        Ok(())
    }

    fn expire(&mut self, now: Instant) -> usize {
        let mut removed = 0;
        for k in 0..2 {
            let mut index = 0;
            while index < self.lanes[k].len() {
                if expired(self.lanes[k][index].expires_at, now) {
                    self.lanes[k].remove(index);
                    self.counters[k].record_drop_expired();
                    let depth = self.lanes[k].len();
                    self.counters[k].record_dequeue(depth);
                    removed += 1;
                } else {
                    index += 1;
                }
            }
        }
        removed
    }

    /// The live item policy serves first among `kinds`, with its index.
    fn select(&self, kinds: &[QueueKind], now: Instant) -> Option<(QueueKind, usize)> {
        let mut best: Option<(QueueKind, usize)> = None;
        for &kind in kinds {
            for (index, item) in self.lanes[lane(kind)].iter().enumerate() {
                if expired(item.expires_at, now) {
                    continue;
                }
                let wins = best.is_none_or(|(best_kind, best_index)| {
                    serves_first(item, &self.lanes[lane(best_kind)][best_index], self.limit)
                });
                if wins {
                    best = Some((kind, index));
                }
            }
        }
        best
    }

    fn peek(&self, kinds: &[QueueKind], now: Instant) -> Option<QueueItemInfo> {
        self.select(kinds, now)
            .map(|(kind, index)| self.lanes[lane(kind)][index].info(kind))
    }

    fn dequeue_kind(
        &mut self,
        kind: QueueKind,
        now: Instant,
        max_len: usize,
    ) -> Result<Option<Taken>, QueueRefusal> {
        self.expire(now);
        let Some((_, index)) = self.select(&[kind], now) else {
            return Ok(None);
        };
        let k = lane(kind);
        let item = self.lanes[k].remove(index);
        let depth = self.lanes[k].len();
        self.counters[k].record_dequeue(depth);
        if item.npdu.len() > max_len {
            self.counters[k].record_drop_oversize();
            return Err(QueueRefusal::Oversize);
        }
        for waiting in self.lanes.iter_mut().flatten() {
            waiting.service_wait = waiting.service_wait.saturating_add(1);
        }
        Ok(Some((
            item.destination,
            kind,
            item.priority,
            item.sequence,
            item.npdu,
        )))
    }

    fn dequeue(&mut self, now: Instant, max_len: usize) -> Result<Option<Taken>, QueueRefusal> {
        let Some(info) = self.peek(&KINDS, now) else {
            return Ok(None);
        };
        self.dequeue_kind(info.kind, now, max_len)
    }

    /// Every item, ordered by sequence number.
    fn items(&self) -> Vec<(QueueItemInfo, Vec<u8>)> {
        let mut items: Vec<_> = KINDS
            .iter()
            .flat_map(|&kind| {
                self.lanes[lane(kind)]
                    .iter()
                    .map(move |item| (item.info(kind), item.npdu.clone()))
            })
            .collect();
        items.sort_by_key(|(info, _)| info.sequence);
        items
    }
}

/// A backend under test, and what the model gives it to compare.
trait Backend: QueueStorage {
    /// Every item it holds, with its octets, ordered by sequence number.
    fn items(&self) -> Vec<(QueueItemInfo, Vec<u8>)>;
}

impl Backend for HeaplessQueues<UNICAST, BROADCAST, N> {
    fn items(&self) -> Vec<(QueueItemInfo, Vec<u8>)> {
        let mut items = self.snapshot();
        items.sort_by_key(|(info, _)| info.sequence);
        items
    }
}

#[cfg(feature = "alloc")]
impl Backend for super::AllocQueues<N> {
    fn items(&self) -> Vec<(QueueItemInfo, Vec<u8>)> {
        let mut items = self.snapshot();
        items.sort_by_key(|(info, _)| info.sequence);
        items
    }
}

#[derive(Clone, Debug)]
enum Op {
    Enqueue {
        destination: u8,
        len: usize,
        priority: NetworkPriority,
        /// Nanoseconds to its expiry, if it has one; 0 expires it at once.
        lifetime: Option<u64>,
    },
    Dequeue {
        max_len: usize,
    },
    DequeueKind {
        kind: QueueKind,
        max_len: usize,
    },
    Expire,
    Wait {
        nanos: u64,
    },
}

fn priority() -> impl Strategy<Value = NetworkPriority> {
    prop_oneof![
        Just(NetworkPriority::Normal),
        Just(NetworkPriority::Urgent),
        Just(NetworkPriority::CriticalEquipment),
        Just(NetworkPriority::LifeSafety),
    ]
}

fn op() -> impl Strategy<Value = Op> {
    let destination = prop_oneof![Just(1u8), Just(2), Just(BROADCAST_MAC)];
    let kind = prop_oneof![Just(QueueKind::Unicast), Just(QueueKind::Broadcast)];
    prop_oneof![
        5 => (destination, 0..=N + 1, priority(), proptest::option::of(0u64..12)).prop_map(
            |(destination, len, priority, lifetime)| Op::Enqueue {
                destination,
                len,
                priority,
                lifetime,
            }
        ),
        2 => (N - 2..=N + 1).prop_map(|max_len| Op::Dequeue { max_len }),
        2 => (kind, N - 2..=N + 1).prop_map(|(kind, max_len)| Op::DequeueKind { kind, max_len }),
        1 => Just(Op::Expire),
        2 => (0u64..6).prop_map(|nanos| Op::Wait { nanos }),
    ]
}

fn owned(item: Option<super::DequeuedNpdu<'_>>) -> Option<Taken> {
    item.map(|item| {
        (
            item.destination,
            item.kind,
            item.priority,
            item.sequence,
            item.npdu.to_vec(),
        )
    })
}

/// Everything a queue shows, against the model, at `now` and a little
/// later, when more items have expired.
fn same_views<Q: Backend>(
    queue: &Q,
    model: &Model,
    now: u64,
    step: usize,
) -> Result<(), TestCaseError> {
    prop_assert_eq!(queue.items(), model.items(), "items after step {}", step);
    for at in [now, now + 3, now + 12] {
        let at = Instant::from_nanos(at);
        prop_assert_eq!(
            queue.peek(at),
            model.peek(&KINDS, at),
            "peek after step {}",
            step
        );
        for kind in KINDS {
            prop_assert_eq!(
                queue.peek_kind(kind, at),
                model.peek(&[kind], at),
                "peek_kind after step {}",
                step
            );
        }
    }
    for kind in KINDS {
        prop_assert_eq!(queue.depth(kind), model.lanes[lane(kind)].len());
        prop_assert_eq!(
            queue.counters(kind),
            model.counters[lane(kind)],
            "counters after step {}",
            step
        );
    }
    prop_assert_eq!(
        queue.total_depth(),
        model.lanes[0].len() + model.lanes[1].len()
    );
    Ok(())
}

fn run<Q: Backend>(
    mut queue: Q,
    numbers_full_refusals: bool,
    limit: u8,
    ops: &[Op],
) -> Result<(), TestCaseError> {
    queue.set_starvation_limit(limit);
    let mut model = Model::new(limit, numbers_full_refusals);
    let mut now = 0u64;
    let mut stamp = 0u8;
    for (step, op) in ops.iter().enumerate() {
        let at = Instant::from_nanos(now);
        match *op {
            Op::Enqueue {
                destination,
                len,
                priority,
                lifetime,
            } => {
                let npdu: Vec<u8> = (0..len).map(|i| stamp.wrapping_add(i as u8)).collect();
                stamp = stamp.wrapping_add(17);
                let expires_at = lifetime.map(|nanos| at + Duration::from_nanos(nanos));
                prop_assert_eq!(
                    queue.enqueue_npdu(destination, &npdu, priority, at, expires_at),
                    model.enqueue(destination, &npdu, priority, at, expires_at),
                    "enqueue at step {}",
                    step
                );
            }
            Op::Dequeue { max_len } => {
                let got = queue.dequeue(at, max_len).map(owned);
                prop_assert_eq!(got, model.dequeue(at, max_len), "dequeue at step {}", step);
            }
            Op::DequeueKind { kind, max_len } => {
                let got = queue.dequeue_kind(kind, at, max_len).map(owned);
                let want = model.dequeue_kind(kind, at, max_len);
                prop_assert_eq!(got, want, "dequeue_kind at step {}", step);
            }
            Op::Expire => {
                prop_assert_eq!(
                    queue.expire(at),
                    model.expire(at),
                    "expire at step {}",
                    step
                );
            }
            Op::Wait { nanos } => now += nanos,
        }
        same_views(&queue, &model, now, step)?;
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn heapless_queues_match_the_model(
        limit in 1u8..=4,
        ops in proptest::collection::vec(op(), 1..80),
    ) {
        run(HeaplessQueues::<UNICAST, BROADCAST, N>::new(), true, limit, &ops)?;
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn alloc_queues_match_the_model(
        limit in 1u8..=4,
        ops in proptest::collection::vec(op(), 1..80),
    ) {
        run(super::AllocQueues::<N>::new(UNICAST, BROADCAST), false, limit, &ops)?;
    }
}

/// `AllocQueues` adds a slot only when every slot so far holds an item, so
/// however many items come and go, its slots stay within the room reserved
/// for them when it was built, and the reservation never moves.
#[cfg(feature = "alloc")]
#[test]
fn alloc_queue_slots_stay_within_their_reservation() {
    let mut queue = super::AllocQueues::<N>::new(UNICAST, BROADCAST);
    let reserved = queue.slot_room().map(|(_, capacity)| capacity);
    let now = Instant::ZERO;
    for round in 0..200u8 {
        for destination in [1, BROADCAST_MAC] {
            for _ in 0..1 + usize::from(round % 3) {
                let _ =
                    queue.enqueue_npdu(destination, &[round], NetworkPriority::Normal, now, None);
            }
        }
        for _ in 0..1 + usize::from(round % 2) {
            let _ = queue.dequeue(now, N);
        }
        let room = queue.slot_room();
        for (k, (slots, capacity)) in room.into_iter().enumerate() {
            let limit = [UNICAST, BROADCAST][k];
            assert!(
                slots <= limit,
                "round {round}: {slots} slots for {limit} items"
            );
            assert_eq!(
                capacity, reserved[k],
                "round {round}: the reservation moved"
            );
        }
    }
}
