//! The time-ordered event queue.
//!
//! Events at the same instant run in a fixed class order, then by station
//! address, then in the order they were scheduled, so a run never depends on
//! heap internals:
//!
//! 1. receiver windows ending (characters that finished arrive first);
//! 2. transmissions ending (final data stop bit);
//! 3. physical driver releases after post-drive hold;
//! 4. scheduled scenario and application events;
//! 5. local echo becoming available to host buffering;
//! 6. ready host reads, completion and echo timers;
//! 7. core deadlines;
//! 8. adapters starting transmissions;
//! 9. later octets of a transmission starting.
//!
//! Putting arrivals first means a node always hears an octet that ended at
//! the instant it decides to transmit, the conservative reading of a tie.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use bacnet_mstp_core::Instant;

/// Same-instant ordering class; see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Class {
    WindowEnd,
    TxEnd,
    DriverRelease,
    Scheduled,
    LocalEcho,
    HostRead,
    Deadline,
    AdapterStart,
    OctetStart,
}

/// The total order of events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Key {
    pub at: Instant,
    pub class: Class,
    /// Station address, or a value above 255 for the line and noise.
    pub tie: u16,
    pub seq: u64,
}

#[derive(Debug)]
struct Entry<E> {
    key: Key,
    event: E,
}

impl<E> PartialEq for Entry<E> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl<E> Eq for Entry<E> {}

impl<E> PartialOrd for Entry<E> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<E> Ord for Entry<E> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key.cmp(&other.key)
    }
}

/// A min-queue of events by [`Key`].
#[derive(Debug)]
pub(crate) struct Queue<E> {
    heap: BinaryHeap<Reverse<Entry<E>>>,
}

impl<E> Queue<E> {
    pub(crate) fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
        }
    }

    pub(crate) fn push(&mut self, key: Key, event: E) {
        self.heap.push(Reverse(Entry { key, event }));
    }

    pub(crate) fn peek_key(&self) -> Option<Key> {
        self.heap.peek().map(|entry| entry.0.key)
    }

    pub(crate) fn pop(&mut self) -> Option<(Key, E)> {
        self.heap.pop().map(|entry| (entry.0.key, entry.0.event))
    }

    pub(crate) fn len(&self) -> usize {
        self.heap.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{Class, Key, Queue};
    use bacnet_mstp_core::Instant;

    #[test]
    fn orders_by_time_then_class_then_station_then_sequence() {
        let mut queue = Queue::new();
        let key = |at, class, tie, seq| Key {
            at: Instant::from_micros(at),
            class,
            tie,
            seq,
        };
        queue.push(key(5, Class::OctetStart, 0, 0), "late");
        queue.push(key(1, Class::Deadline, 3, 1), "deadline st3");
        queue.push(key(1, Class::Deadline, 0, 2), "deadline st0");
        queue.push(key(1, Class::WindowEnd, 300, 3), "window");
        queue.push(key(1, Class::Deadline, 0, 4), "deadline st0 again");
        let order: Vec<&str> = std::iter::from_fn(|| queue.pop().map(|(_, event)| event)).collect();
        assert_eq!(
            order,
            [
                "window",
                "deadline st0",
                "deadline st0 again",
                "deadline st3",
                "late"
            ]
        );
    }
}
