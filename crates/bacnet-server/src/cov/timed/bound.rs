//! The local history bound of a COV-multiple context (#986, #1039, #1163):
//! the envelope that sizes its room for items (#1197), what its pending
//! changes count against that room and its memory ceiling (#1287), and
//! eviction under both. The policy is in the parent module's docs.
//!
//! Eviction may take any pending change except, per reference, its newest
//! and its change in delivery: the change a part went out ahead of, sent
//! one value per notification (#1090). Parts go out in capture order, one
//! report at a time, so the change a part last went out ahead of is the only
//! one of the reference whose later values can still be waiting; each
//! reference records just that one.

use super::{
    CovSubscriptionKey, DropReason, MultipleContextKey, TimedChange, TimedHistories, TimedHistory,
    unsigned_len,
};

/// Octets of an unsegmented confirmed request's header: type and flags,
/// maximum segments and APDU, invoke ID and service choice.
const CONFIRMED_HEADER: usize = 4;
/// Octets of an unconfirmed request's header: type and service choice.
const UNCONFIRMED_HEADER: usize = 2;
/// Octets of the initiating device identifier: a context tag and the four
/// octets of the identifier, whatever the instance.
const DEVICE_IDENTIFIER: usize = 5;
/// Octets of the timestamp a notification of timestamped changes carries: an
/// application-tagged Date and Time, five octets each, between an opening and
/// a closing tag.
const TIMESTAMP: usize = 12;
/// Octets of the two tags that delimit the list of items.
const LIST_TAGS: usize = 2;

/// Octets of the APDU header before a COV-multiple notification's service
/// request, confirmed or not; a notification is never segmented.
pub(crate) fn request_header(confirmed: bool) -> usize {
    if confirmed {
        CONFIRMED_HEADER
    } else {
        UNCONFIRMED_HEADER
    }
}

/// Octets a COV-multiple notification to `context` spends outside its items,
/// counted the way the encoder lays the request out (Clauses 13.17.1 and
/// 13.18.1, and the Clause 21 production): the APDU header, then the
/// process identifier and time remaining as context-tagged unsigned values in
/// their fewest octets, the device identifier, the timestamp, and the tags
/// around the list. From 25 octets, unconfirmed with both numbers below 256,
/// to 33, confirmed with both at or above 2^24.
///
/// `time_remaining` is the lifetime in seconds the context had left when it
/// was admitted. Its notifications report less as time passes, which never
/// takes more octets, so the envelope never outgrows this size.
pub(crate) fn envelope_len(context: &MultipleContextKey, time_remaining: u32) -> usize {
    request_header(context.confirmed)
        + unsigned_len(u64::from(context.process_id))
        + DEVICE_IDENTIFIER
        + unsigned_len(u64::from(time_remaining))
        + TIMESTAMP
        + LIST_TAGS
}

/// Pending changes of one context, as its bound counts them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Held {
    /// Octets of their items, each as [`TimedChange::octets`] counts it.
    pub(super) octets: usize,
    /// Bytes they take in memory, each as [`TimedChange::memory`] counts it
    /// (#1357).
    pub(super) memory: usize,
}

impl Held {
    /// What `changes` count against their context's bound.
    pub(super) fn of<'a>(changes: impl IntoIterator<Item = &'a TimedChange>) -> Self {
        changes
            .into_iter()
            .fold(Self::default(), |held, change| Self {
                octets: held.octets + change.octets,
                memory: held.memory + change.memory(),
            })
    }

    /// Count `more` changes as well.
    pub(super) fn grow(&mut self, more: Held) {
        self.octets += more.octets;
        self.memory += more.memory;
    }
}

/// Position of the oldest change of `history` that eviction may take: one
/// that is neither the reference's newest nor its change in delivery.
fn evictable(history: &TimedHistory) -> Option<usize> {
    let newest = history.entries.len().checked_sub(1)?;
    (0..newest).find(|&at| history.entries[at].seq != history.in_delivery)
}

impl TimedHistories {
    /// A part carrying some values of change `seq` of `key` went out ahead of
    /// the rest: that change is in delivery until its last value is, and the
    /// bound keeps it meanwhile (#1163).
    pub(super) fn deliver_by_value(
        &mut self,
        key: &CovSubscriptionKey,
        incarnation: u64,
        seq: u64,
    ) {
        if let Some(history) = self.incarnation_mut(key, incarnation) {
            history.in_delivery = seq;
        }
    }

    /// Evict pending changes of `key`'s context while it is over its room for
    /// items or its memory ceiling: this reference's oldest evictable change
    /// first, then the oldest in the context. Stops when only changes eviction
    /// may not take are left.
    pub(super) fn enforce_bound(&mut self, key: &CovSubscriptionKey) {
        let Some(context) = key.multiple_context().cloned() else {
            return;
        };
        let Some(terms) = self.terms.get(&context).copied() else {
            return;
        };
        let held = |histories: &Self| {
            histories
                .context_held
                .get(&context)
                .copied()
                .unwrap_or_default()
        };
        while !terms.holds(held(self), self.local_apdu) {
            let victim = match self.histories.get(key).and_then(evictable) {
                Some(at) => Some((key.clone(), at)),
                None => self
                    .histories
                    .iter()
                    .filter(|(k, _)| k.multiple_context() == Some(&context))
                    .filter_map(|(k, h)| evictable(h).map(|at| (k, at, h.entries[at].seq)))
                    .min_by_key(|&(_, _, seq)| seq)
                    .map(|(k, at, _)| (k.clone(), at)),
            };
            let Some((victim, at)) = victim else {
                return;
            };
            let evicted = self
                .histories
                .get_mut(&victim)
                .and_then(|h| h.entries.remove(at))
                .expect("victim has an evictable change");
            self.release(&victim, Held::of([&evicted]));
            self.dropped(&victim, 1, DropReason::HistoryFull);
        }
    }

    /// Return `released` changes of `key` to its context's bound.
    pub(super) fn release(&mut self, key: &CovSubscriptionKey, released: Held) {
        let Some(context) = key.multiple_context() else {
            return;
        };
        let Some(held) = self.context_held.get_mut(context) else {
            debug_assert_eq!(released, Held::default(), "released more than held");
            return;
        };
        debug_assert!(
            held.octets >= released.octets && held.memory >= released.memory,
            "released {released:?} of {held:?}"
        );
        held.octets = held.octets.saturating_sub(released.octets);
        held.memory = held.memory.saturating_sub(released.memory);
        if *held == Held::default() {
            self.context_held.remove(context);
        }
    }
}

#[cfg(test)]
mod tests {
    use bacnet_services::cov_multiple::{
        COVNotificationItem, COVNotificationMultipleRequest, COVNotificationValue,
    };
    use bacnet_types::enums::{ObjectType, PropertyIdentifier};
    use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
    use bytes::BytesMut;

    use super::super::tests::{
        change, context, dropped, frame, histories, key, seconds, timed_reference,
    };
    use super::super::{
        CEILING_BYTES_PER_OCTET, CHANGE_MEMORY, HISTORY_NOTIFICATIONS, ITEM_FRAMING, TimedChange,
        VALUE_MEMORY, value_len,
    };
    use super::*;
    use crate::cov::{
        AtomicCovCounters, CovObservation, CovSample, CovSubscriptionTable, SubscriberEndpoint,
    };
    use std::sync::Arc;

    /// The local maximum APDU these tests' devices take: the largest B/IP one.
    const LOCAL: usize = 1476;

    /// The service request of a notification carrying `change` of Analog
    /// Value 1 alone to `context`, laid out as a report's history part.
    fn notification(
        context: &MultipleContextKey,
        time_remaining: u32,
        device: u32,
        change: &TimedChange,
    ) -> Vec<u8> {
        let frame = change.frame();
        let request = COVNotificationMultipleRequest {
            subscriber_process_identifier: context.process_id,
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, device)
                .unwrap(),
            time_remaining,
            timestamp: Some((frame.local_date, frame.local_time)),
            list_of_cov_notifications: vec![COVNotificationItem {
                monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1)
                    .unwrap(),
                list_of_values: change.values().to_vec(),
            }],
        };
        let mut encoded = BytesMut::new();
        request.encode(&mut encoded).unwrap();
        encoded.to_vec()
    }

    /// A timestamped change of a REAL Present_Value of 22.5.
    fn real_present_value(second: u8) -> TimedChange {
        let sample = CovSample::new(&PropertyValue::Real(22.5)).unwrap();
        TimedChange::new(
            frame(second),
            vec![COVNotificationValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x44, 0x41, 0xB4, 0x00, 0x00],
                time_of_change: None,
            }],
            CovObservation::new(sample, None).unwrap(),
        )
    }

    /// Octets the items of a notification carrying `change` alone take.
    fn item_len(change: &TimedChange) -> usize {
        ITEM_FRAMING + change.values().iter().map(value_len).sum::<usize>()
    }

    #[test]
    fn the_envelope_is_what_the_encoder_lays_around_the_items() {
        let change = real_present_value(30);
        let widths = [0, 255, 256, 65_535, 65_536, 16_777_216, u32::MAX];
        for confirmed in [false, true] {
            for process_id in widths {
                for time_remaining in widths {
                    for device in [0, 4_194_302] {
                        let context = MultipleContextKey {
                            process_id,
                            confirmed,
                            ..context(1)
                        };
                        let wire = request_header(confirmed)
                            + notification(&context, time_remaining, device, &change).len();
                        assert_eq!(
                            wire,
                            envelope_len(&context, time_remaining) + item_len(&change),
                            "confirmed={confirmed} process={process_id} \
                             remaining={time_remaining} device={device}"
                        );
                    }
                }
            }
        }
    }

    /// #1197: at the smallest maximum APDU a subscriber can advertise, a
    /// change that one notification carries on the wire fits one
    /// notification's room in the bound too, and the bound keeps what four
    /// such notifications carry.
    #[test]
    fn at_a_50_octet_subscriber_a_change_that_fits_the_wire_fits_the_bound() {
        let (mut h, counters) = histories(8, 4);
        let k = key(1, 1);
        let small = context(1);
        h.reset(&k, 1, 0);
        h.set_sizing(&small, Some(50), 120);

        // One REAL Present_Value goes out in one 50-octet notification...
        let present_value = real_present_value(1);
        let wire = request_header(false) + notification(&small, 120, 8, &present_value).len();
        assert!(wire <= 50, "{wire} octets on the wire");
        // ...and the bound's room for one notification holds it, which the
        // largest envelope, the one every context used to be sized with,
        // does not.
        let largest = MultipleContextKey {
            process_id: u32::MAX,
            confirmed: true,
            ..context(1)
        };
        assert_eq!(envelope_len(&largest, u32::MAX), 33);
        assert!(item_len(&present_value) > 50 - 33);
        assert!(item_len(&present_value) <= h.terms[&small].notification());

        // The room of four such notifications is 100 octets of items: five
        // changes of a four-octet value, 20 octets each, fill it.
        for second in 1..=5 {
            h.push(&k, 1, change(second, 4));
        }
        assert_eq!(seconds(&h.drain(&k, 1).1), [1, 2, 3, 4, 5]);
        assert_eq!(dropped(&counters), 0);
        // A longer lifetime takes another octet in every notification, and
        // the room of four no longer holds the oldest change.
        h.set_sizing(&small, Some(50), 300);
        for second in 6..=10 {
            h.push(&k, 1, change(second, 4));
        }
        assert_eq!(seconds(&h.drain(&k, 1).1), [7, 8, 9, 10]);
        assert_eq!(dropped(&counters), 1);
    }

    /// Changes of `payload` octets: the octets one takes as an item, and
    /// the bytes it counts against the memory ceiling.
    fn sizes(payload: usize) -> (usize, usize) {
        let items = item_len(&change(0, payload));
        (items, change(0, payload).memory())
    }

    /// #1357: the memory terms are the sizes of what a pending change holds,
    /// as measured on the 64-bit targets CI builds. A field added to either
    /// structure makes this fail, so the ceiling's sums get looked at again.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn the_memory_terms_are_the_measured_sizes() {
        assert_eq!(std::mem::size_of::<TimedChange>(), 144);
        assert_eq!(std::mem::size_of::<COVNotificationValue>(), 48);
        assert_eq!((CHANGE_MEMORY, VALUE_MEMORY), (144, 56));
        // The smallest change, one empty value, and a REAL Present_Value,
        // whose value takes five octets.
        assert_eq!(change(0, 0).memory(), 200);
        assert_eq!(real_present_value(0).memory(), 205);
    }

    /// #1287: at the smallest maximum APDU a subscriber can advertise, the
    /// bound keeps what four notifications carry. The memory a held change
    /// takes besides its items counts against the memory ceiling, which a
    /// context this small never reaches, not against the room for items.
    #[test]
    fn at_a_50_octet_subscriber_the_bound_keeps_four_notifications_of_changes() {
        let counters = Arc::new(AtomicCovCounters::default());
        let mut h = TimedHistories::new(LOCAL, Arc::clone(&counters));
        let k = key(1, 1);
        let small = context(1);
        h.reset(&k, 1, 0);
        h.set_sizing(&small, Some(50), 120);
        let room = 50 - envelope_len(&small, 120);

        // A REAL Present_Value change takes a notification of its own, as
        // two do not fit one, and the bound keeps four of them.
        let present_value = item_len(&real_present_value(0));
        assert!(present_value <= room && 2 * present_value > room);
        for second in 1..=10 {
            h.push(&k, 1, real_present_value(second));
        }
        assert_eq!(seconds(&h.drain(&k, 1).1), [7, 8, 9, 10]);
        assert_eq!(dropped(&counters), 6);

        // The estimate counts octets, not whole notifications: five Binary
        // Present_Value changes, 18 octets each, fit the 100 octets of four.
        let (binary, _) = sizes(2);
        assert_eq!((binary, HISTORY_NOTIFICATIONS * room), (18, 100));
        for second in 11..=20 {
            h.push(&k, 1, change(second, 2));
        }
        assert_eq!(seconds(&h.drain(&k, 1).1), [16, 17, 18, 19, 20]);
        assert_eq!(dropped(&counters), 11);
    }

    /// #1287: across subscriber and change sizes, a context keeps as many
    /// changes as fit both its room for items, four notifications of its
    /// size less its envelope, and its memory ceiling, four bytes for each
    /// octet four notifications of the local maximum APDU have for items,
    /// each change counted as [`TimedChange::memory`] counts it (#1357);
    /// never fewer than one, its newest. A subscriber as large as this
    /// device is held by the ceiling while its changes are small, and by its
    /// room once a value is large enough to take fewer bytes in memory per
    /// octet it takes in a notification. Where that happens depends on the
    /// sizes `the_memory_terms_are_the_measured_sizes` pins.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn the_bound_keeps_what_fits_both_its_notifications_and_its_memory_ceiling() {
        let (k, small) = (key(1, 1), context(1));
        let envelope = envelope_len(&small, 120);
        let ceiling = CEILING_BYTES_PER_OCTET * HISTORY_NOTIFICATIONS * (LOCAL - envelope);
        for subscriber in [50, 128, 206, 480, 1024, 1476] {
            let room = HISTORY_NOTIFICATIONS * (usize::from(subscriber) - envelope);
            for payload in [0, 2, 5, 40, 200, 2000] {
                let (items, memory) = sizes(payload);
                let fit = (room / items).min(ceiling / memory).max(1);
                if usize::from(subscriber) == LOCAL {
                    let ceiling_binds = ceiling / memory < room / items;
                    assert_eq!(ceiling_binds, payload <= 40, "payload {payload}");
                }
                let counters = Arc::new(AtomicCovCounters::default());
                let mut h = TimedHistories::new(LOCAL, Arc::clone(&counters));
                h.reset(&k, 1, 0);
                h.set_sizing(&small, Some(subscriber), 120);
                for n in 0..fit + 2 {
                    h.push(&k, 1, change(u8::try_from(n % 60).unwrap(), payload));
                }
                let kept = h.drain(&k, 1).1.len();
                assert_eq!(kept, fit, "subscriber {subscriber}, payload {payload}");
                assert_eq!(
                    dropped(&counters),
                    2,
                    "subscriber {subscriber}, payload {payload}"
                );
            }
        }
    }

    /// #1287: the ceiling caps the history of every context however many
    /// subscribers push however many of the smallest changes. Small
    /// contexts keep what four of their notifications carry, far below it;
    /// contexts as large as this device keep what it allows; and none holds
    /// more bytes, counted as each change really takes them (#1357), than
    /// four for each octet four notifications of the local maximum APDU
    /// have for items.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn the_memory_ceiling_caps_every_context_under_many_subscribers() {
        let counters = Arc::new(AtomicCovCounters::default());
        let mut h = TimedHistories::new(LOCAL, Arc::clone(&counters));
        // The smallest timestamped change: one empty value, 16 octets as an
        // item and 200 bytes in memory.
        let (items, memory) = sizes(0);
        assert_eq!((items, memory), (16, CHANGE_MEMORY + VALUE_MEMORY));
        // 64 contexts of two references each, every other one at 50 octets.
        let contexts: Vec<_> = (1..=64u32)
            .map(|process| (process, (process % 2 == 0).then_some(50u16)))
            .collect();
        for &(process, subscriber) in &contexts {
            for instance in 1..=2 {
                h.reset(&key(process, instance), 1, 0);
            }
            h.set_sizing(&context(process), subscriber, 120);
        }
        for second in 0..200u8 {
            for &(process, _) in &contexts {
                for instance in 1..=2 {
                    h.push(&key(process, instance), 1, change(second % 60, 0));
                }
            }
        }
        for &(process, subscriber) in &contexts {
            let envelope = envelope_len(&context(process), 120);
            let ceiling = CEILING_BYTES_PER_OCTET * HISTORY_NOTIFICATIONS * (LOCAL - envelope);
            let held: usize = (1..=2)
                .map(|instance| h.drain(&key(process, instance), 1).1.len())
                .sum();
            assert!(held * memory <= ceiling, "process {process}");
            // A small context fills its 100 octets of items; one as large as
            // this device, its 23,216 bytes of memory: 116 changes, where a
            // fixed 32 octets counted per change for bookkeeping kept 120.
            let (expected, fit) = match subscriber {
                Some(apdu) => (
                    6,
                    HISTORY_NOTIFICATIONS * (usize::from(apdu) - envelope) / items,
                ),
                None => (116, ceiling / memory),
            };
            assert_eq!((held, fit), (expected, expected), "process {process}");
        }
    }

    #[test]
    fn admission_sizes_the_envelope_from_the_lifetime_left() {
        let route = SubscriberEndpoint::new(&[10, 0, 0, 1, 0xBA, 0xC0], None);
        for (lifetime, envelope) in [(120, 25), (300, 26), (70_000, 27)] {
            let mut table = CovSubscriptionTable::new();
            let expires = std::time::Instant::now() + std::time::Duration::from_secs(lifetime);
            let reference = timed_reference(1, expires);
            table
                .subscribe_multiple(&context(1), &route, expires, 10, Some(50), vec![reference])
                .unwrap();
            let terms = table.timed().lock().terms[&context(1)];
            assert_eq!(
                (terms.envelope, terms.notification()),
                (envelope, 50 - envelope),
                "lifetime {lifetime}"
            );
        }
    }
}
