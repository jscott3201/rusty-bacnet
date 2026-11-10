//! Retained timestamped COV-multiple changes (135-2020 §13.16.3.1.2.3, §13.17.1.1).
//!
//! A reference subscribed with `Timestamped=TRUE` must report every qualifying
//! change with the local time it happened, and keep those changes until a
//! notification carrying them is sent. Producers capture each change under
//! the database write guard; the notification builder drains the queue into a
//! [`TimedClaim`], which is committed once the notification is delivered
//! (transmitted when unconfirmed, acknowledged when confirmed) and given
//! back otherwise, its entries requeued. Keeping a confirmed report's changes
//! until its Ack is local policy (#896).
//!
//! Changes are normally reported as soon as they are captured. A change stays
//! queued when its notification fails or is held back: a failed send, an
//! unanswered confirmed report, DISABLE_INITIATION, an exhausted budget. The
//! context's Max_Notification_Delay, an upper bound, then limits the wait,
//! measured from its earliest queued change (§13.1, §13.16.1.1.4):
//! [`TimedHistories::take_due`] hands the host each context whose deadline
//! has passed, with the next deadline still ahead, and the host fans it out
//! again.
//!
//! Local policy for that backstop:
//! - it acts no sooner than one second after the earliest change;
//! - after an attempt it waits the delay (one second at least) before the
//!   next one, so a persistent failure cannot spin;
//! - a confirmed hold-off moves the next attempt to the end of the hold-off,
//!   and an outstanding confirmed report leaves the follow-up to its outcome;
//! - [`TimedHistories::rearm`] drops every wait when a block lifts
//!   (communication re-enabled, a shorter delay admitted), so overdue changes
//!   go out at once.
//!
//! Untimestamped references have no queue: their current values are read
//! again for each report. One whose values a report began going out without
//! (parts held back after an earlier part went, or deferred behind a
//! confirmed report's outstanding part) is owed instead, and the backstop
//! treats it as a pending change of its context, from the moment it was
//! first owed (#1038). The report that next evaluates it takes the mark.
//!
//! Delays and waits live only with timestamped histories and owed
//! references, so a context with neither costs nothing here.
//!
//! Changes that one notification cannot carry go out in several (§13.1,
//! §13.18.1.1), strictly in capture order: `TimedClaim::split_oldest` moves
//! the oldest changes, a reference's latest included, into earlier
//! notifications (#1008). A change too large for a notification even alone
//! goes out one value per notification instead, in its captured order
//! (`TimedClaim::split_values`); only the part with its last value delivers
//! the change, and parts that come back undelivered rejoin as one change
//! (#1090). An unconfirmed context sends one such report at a time
//! ([`TimedHistories::begin_send`]), so a later report cannot overtake the
//! parts of an earlier one. Untimestamped values that one notification cannot
//! carry go out after every timestamped change, split by object item (#1038).
//!
//! Local bound policy: the pending changes of one COV-multiple context are
//! limited to an estimate of what `HISTORY_NOTIFICATIONS` notifications can
//! carry, each sized to the lesser of the local and the subscriber's maximum
//! APDU, less the envelope the encoder puts around a notification's items for
//! that context (`envelope_len`, #1197). The envelope is sized from the
//! context itself: its confirmed or unconfirmed header, its process
//! identifier, and the lifetime it had left when last admitted, which later
//! notifications report as less, never in more octets. This room for items
//! counts each change's values and item framing, as if every change started
//! its own item. It keeps a reserve, per context and at most one
//! notification's worth, for the room the context's untimestamped values took
//! in its reports since it was last admitted or lost a reference; they travel
//! with the last notification.
//!
//! Memory has a ceiling of its own (#1287), counted in the bytes a pending
//! change really takes (#1357): `CHANGE_MEMORY` for the change itself, in
//! its slot of the history's queue, and for each of its values
//! `VALUE_MEMORY`, the value's slots in the change's vectors, plus its
//! encoded octets on the heap. A context's changes take no more than
//! `CEILING_BYTES_PER_OCTET` bytes for each octet `HISTORY_NOTIFICATIONS`
//! notifications of the local maximum APDU have for items, whatever its
//! subscriber's size, so many tiny changes cannot outgrow it: 23,216 bytes at
//! a 1476-octet local maximum, about 116 of the smallest changes (one empty
//! value each) or 87 REAL Present_Value and Status_Flags changes. A small
//! change takes eight to thirteen times the bytes in memory that it takes
//! octets in a notification, and the ratio falls toward one as its values
//! grow; four bytes an octet keeps about the history #1287 kept when it
//! charged a fixed 32 octets per change, a quarter of what the smallest
//! change takes. Near the local maximum the ceiling binds before the room
//! does for small changes, the room for large ones; a small subscriber's
//! room binds first. The room counts only octets
//! that travel in a notification: charged with memory, it would let a
//! 50-octet subscriber keep one REAL Present_Value change where four
//! notifications carry four.
//!
//! No ceiling spans the table; the subscription caps bound it. Each
//! COV-multiple reference counts as one subscription, so under the default
//! global cap of 1,024 there are at most 1,024 contexts, and their pending
//! changes take at most 1,024 times the ceiling, about 24 MB at a 1476-octet
//! local maximum, besides the changes eviction may not take, two per
//! reference at most (below).
//!
//! Only on overflow of either limit, the last resort, is a change dropped:
//! the oldest of the same reference first, then the oldest in the context.
//! Two changes of a reference are never evicted:
//! - its newest, so the bound never hides the reference's current state;
//! - its change in delivery (#1163): one sent value by value of which a part
//!   was delivered, or went out as a confirmed report's first part with the
//!   rest deferred, kept until its last value is delivered.
//!
//! The second is for small subscribers. At a 50-octet maximum APDU the room
//! comes to 68 to 100 octets of items, by the envelope's size: two or three
//! Present_Value and Status_Flags changes of 33 octets, so a value held back
//! by a failed send or a deferral would be evicted after a few newer changes,
//! and the value-by-value delivery of #1090 might never finish. Counting the
//! bound in values instead would still need an octet limit, as nothing limits
//! a value's size when it is captured, and would change eviction for every
//! change of such a context; exempting only the change already partly sent
//! leaves every other eviction as it was. Each reference marks one change in
//! delivery, the one a part last went out ahead of, so what a context holds
//! stays bounded: after eviction, no more than its room and its ceiling allow
//! or, beyond them, only changes eviction may not take, two per reference at
//! most.
//!
//! Each discarded change is counted ([`TimedCounts::changes_dropped`]) and
//! reported ([`Dropped`]); a report says whether it is its context's first
//! for that cause since the context was admitted afresh (#1039), so a host
//! can log one warning for a subscriber too small for any timestamped value
//! instead of flooding its log.
//!
//! The histories are plain state: no lock, clock, wake-up or log. A host
//! keeps them under its own guard and passes `now`, on one monotonic
//! timeline, to every call that stores or compares an instant. After a call
//! it takes what the call counted ([`TimedHistories::take_counts`]) and
//! dropped ([`TimedHistories::take_drops`]), and asks whether a deadline may
//! have moved earlier ([`TimedHistories::take_moved`]); a host that sleeps
//! until [`TimedHistories::take_due`]'s next deadline wakes and asks again
//! when one has.
//!
//! Two things have a deadline, and [`TimedHistories::next_deadline`] is the
//! earlier: a context's backstop, above, and a lodged claim
//! ([`TimedHistories::lodge`]), which is given back at its deadline if its
//! holder never settled it. [`TimedHistories::take_due`] acts on both.
//!
//! A host does not string these calls together itself. Its fanout runs on
//! [`multiple_report`](super::multiple_report), which drains, adopts and
//! times through this module and splits each report
//! (`multiple_chunks`), and its producers on
//! [`timed_capture`](super::timed_capture). What a host calls here is what
//! those steps leave to it: the deadline ([`TimedHistories::take_due`]),
//! the counts and drops, and the settling of what a report hands it:
//! [`TimedHistories::commit`], [`TimedHistories::give_back`],
//! [`TimedHistories::lodge`] and [`TimedHistories::settle`] for a claim,
//! [`TimedHistories::end_send`] for a send turn. The calls only the steps
//! need (a claim's splits, the field times, the reserve) are private to the
//! crate.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec;
use alloc::vec::Vec;

use bacnet_objects::clock::ClockFrame;
use bacnet_services::cov_multiple::COVNotificationValue;
use bacnet_time::{Duration, Instant};

use super::{CovObservation, CovSubscriptionKey, MultipleContextKey};

/// Notifications' worth of pending history one context may hold.
pub(crate) const HISTORY_NOTIFICATIONS: usize = 4;
/// Octets of one item's framing: its context-tagged object identifier (5) and
/// the opening and closing tags of its value list (2).
pub(crate) const ITEM_FRAMING: usize = 7;
/// Bytes each pending change counts against its context's memory ceiling for
/// itself, in its slot of the history's queue (#1357), and
/// [`CAPTURE_ALLOWANCE`]. The room for notification items does not count it
/// (#1287).
pub(crate) const CHANGE_MEMORY: usize = core::mem::size_of::<TimedChange>() + CAPTURE_ALLOWANCE;
/// Bytes a pending change still counts for the capture instant it held on
/// the host's clock before it took `bacnet-time`'s 8-octet one (#2320).
/// That instant took 16 octets on the 64-bit hosts the histories ran on,
/// the only ones there were, so the allowance is the 8 they saved, on every
/// target. Without it a change would count 8 bytes less on such a host, and
/// a context's ceiling would hold more changes than it did (120 of the
/// smallest for 116 at a 1476-octet local maximum): which change overflow
/// drops is part of what a subscriber sees, so it stays as it was. The
/// sizes behind it are pinned for 64-bit hosts only
/// (`the_memory_terms_are_the_measured_sizes`); a 32-bit build's changes are
/// smaller and it has no earlier behaviour to keep.
pub(crate) const CAPTURE_ALLOWANCE: usize = 8;
/// Bytes each value of a pending change counts against the memory ceiling
/// besides its encoded octets: the value in the change's vector of values,
/// and its slot in the vector of positions (#1357).
pub(crate) const VALUE_MEMORY: usize =
    core::mem::size_of::<COVNotificationValue>() + core::mem::size_of::<usize>();
/// Bytes of memory a context's pending changes may take for each octet of
/// items [`HISTORY_NOTIFICATIONS`] notifications of the local maximum APDU
/// have (#1357); see the module documentation.
pub(crate) const CEILING_BYTES_PER_OCTET: usize = 4;
/// Where [`TimedHistories::next_start`]'s sequence begins: any value but
/// zero, the one state xorshift never leaves.
const TURN_SEED: u32 = 0x9E37_79B9;
/// Earliest the deadline backstop acts after a change, and the least spacing
/// between its attempts on one blocked context.
pub(crate) const DEADLINE_FLOOR: Duration = Duration::from_secs(1);

/// One captured change of a timestamped reference, ready to be conveyed.
#[derive(Debug, Clone)]
pub struct TimedChange {
    seq: u64,
    frame: ClockFrame,
    /// Instant of the capture on the host's monotonic timeline: the
    /// Max_Notification_Delay origin, unaffected by later Device clock
    /// adjustments.
    captured_at: Instant,
    values: Vec<COVNotificationValue>,
    observation: CovObservation,
    /// Octets the change counts against its context's room for items: its
    /// encoding and one item's framing. The memory ceiling counts
    /// [`memory`](Self::memory) instead.
    octets: usize,
    /// Position of each of `values`, in the same order, among those the change
    /// was captured with: `0..n` at capture. A part of a change sent one value
    /// per notification keeps its value's position, so parts that come back in
    /// any order rejoin in captured order (#1090).
    positions: Vec<usize>,
    /// Whether later values of this change go out in later notifications of
    /// the same report, so delivering this part alone does not deliver the
    /// change (#1090).
    continues: bool,
}

/// Octets of a context-tagged unsigned value: its tag and the minimal
/// big-endian content, one octet at least.
fn unsigned_len(value: u64) -> usize {
    1 + (1..8)
        .find(|octets| value >> (8 * octets) == 0)
        .unwrap_or(8)
}

/// Encoded octets of one entry of a COV-multiple value list: its property
/// identifier, optional array index, the value between its opening and closing
/// tags, and its Time_Of_Change when present.
pub(crate) fn value_len(value: &COVNotificationValue) -> usize {
    unsigned_len(u64::from(value.property_identifier.to_raw()))
        + value
            .property_array_index
            .map_or(0, |index| unsigned_len(u64::from(index)))
        + 2
        + value.value.len()
        + if value.time_of_change.is_some() { 5 } else { 0 }
}

/// Octets an item carrying `values` takes in a notification: their encoding
/// and the item's framing.
fn item_octets(values: &[COVNotificationValue]) -> usize {
    ITEM_FRAMING + values.iter().map(value_len).sum::<usize>()
}

impl TimedChange {
    /// Pair prepared values with the Device clock frame of the change,
    /// captured at `now`. Every value carries the frame's local time as its
    /// Time_Of_Change.
    pub fn new(
        frame: ClockFrame,
        mut values: Vec<COVNotificationValue>,
        observation: CovObservation,
        now: Instant,
    ) -> Self {
        for value in &mut values {
            value.time_of_change = Some(frame.local_time);
        }
        Self {
            seq: 0,
            frame,
            captured_at: now,
            octets: item_octets(&values),
            positions: (0..values.len()).collect(),
            values,
            observation,
            continues: false,
        }
    }

    /// The change as one part per value, in order, each a change of its own
    /// with this one's sequence, time and observation, and each marked as
    /// continued: the caller unmarks the last part it keeps (#1090).
    fn into_values(self) -> impl Iterator<Item = TimedChange> {
        self.values
            .into_iter()
            .zip(self.positions)
            .map(move |(value, position)| TimedChange {
                seq: self.seq,
                frame: self.frame,
                captured_at: self.captured_at,
                octets: item_octets(core::slice::from_ref(&value)),
                values: vec![value],
                observation: self.observation.clone(),
                positions: vec![position],
                continues: true,
            })
    }

    /// Take back `other`, another part of this change, and order the values
    /// by their captured positions again, whichever parts came back before.
    /// Returns the octets this adds to the context's room for items.
    fn rejoin(&mut self, other: TimedChange) -> usize {
        let mut merged: Vec<_> = core::mem::take(&mut self.positions)
            .into_iter()
            .zip(core::mem::take(&mut self.values))
            .chain(other.positions.into_iter().zip(other.values))
            .collect();
        merged.sort_by_key(|(position, _)| *position);
        (self.positions, self.values) = merged.into_iter().unzip();
        let before = self.octets;
        self.octets = item_octets(&self.values);
        self.octets - before
    }

    /// Bytes the change counts against its context's memory ceiling:
    /// [`CHANGE_MEMORY`], and [`VALUE_MEMORY`] and the encoded octets for
    /// each of its values (#1357).
    pub(crate) fn memory(&self) -> usize {
        CHANGE_MEMORY
            + self
                .values
                .iter()
                .map(|value| VALUE_MEMORY + value.value.len())
                .sum::<usize>()
    }

    /// Capture sequence of the change: its place in capture order across the
    /// whole table.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Device clock frame of the change.
    pub fn frame(&self) -> ClockFrame {
        self.frame
    }

    /// Conveyed values, each stamped with this change's local time.
    pub fn values(&self) -> &[COVNotificationValue] {
        &self.values
    }

    /// Observation this change establishes as the reference's baseline.
    pub fn observation(&self) -> &CovObservation {
        &self.observation
    }
}

/// Encoded value of a reference's own coordinate, with the sequence and clock
/// frame of the change that set it.
#[derive(Debug, Clone)]
struct FieldRecord {
    value: Vec<u8>,
    seq: u64,
    frame: ClockFrame,
}

/// An untimestamped reference of a COV-multiple context, kept so a report
/// that did not deliver its values can owe them (#1038).
#[derive(Debug)]
struct UntimedReference {
    generation: u64,
    /// The context's Max_Notification_Delay, as last admitted.
    delay: Duration,
    /// When a report first prepared values of it that are still undelivered.
    owed_since: Option<Instant>,
}

#[derive(Debug)]
struct TimedHistory {
    /// Identity of this history across renewals. A cancelled and recreated
    /// reference gets a new one, so late returns cannot reach it.
    incarnation: u64,
    generation: u64,
    /// Latest observation already captured or conveyed; the next change
    /// qualifies against it rather than the last completed delivery.
    baseline: Option<CovObservation>,
    /// The newest known value of the reference's own coordinate and when it
    /// changed, kept once delivered: what an explicit timestamped selector
    /// reports for its field in a round it conveys no change in (#987).
    field: Option<FieldRecord>,
    /// Sequence of the newest captured or conveyed change.
    latest: u64,
    /// Sequence of the newest delivered change. An older change returned by
    /// a failed notification would contradict what was already delivered.
    committed: u64,
    /// Sequence of the change in delivery: the one a part carrying only some
    /// of its values last went out ahead of. The bound never evicts it
    /// (#1163); once it is delivered no pending change has its sequence.
    in_delivery: u64,
    /// The context's Max_Notification_Delay, as last admitted.
    delay: Duration,
    entries: VecDeque<TimedChange>,
}

/// Sizing terms of a context with timestamped histories.
#[derive(Debug, Clone, Copy)]
struct ContextTerms {
    /// Octets one notification to the context may take: the smaller of the
    /// local maximum APDU and the subscriber's, as last admitted.
    apdu: usize,
    /// Octets each notification to the context spends outside its items, as
    /// last admitted: see [`envelope_len`].
    envelope: usize,
    /// Most octets the context's untimestamped values have taken in one
    /// report since the context was last admitted or lost a reference, at
    /// most one notification's worth.
    reserve: usize,
    /// Causes of dropped changes the context has warned about (#1039).
    warned: DropWarnings,
}

impl ContextTerms {
    /// Octets of items one notification can carry.
    fn notification(self) -> usize {
        self.apdu.saturating_sub(self.envelope)
    }

    /// Octets of items the context's pending changes may take: what
    /// [`HISTORY_NOTIFICATIONS`] notifications carry, less the reserve.
    fn room(self) -> usize {
        (HISTORY_NOTIFICATIONS * self.notification()).saturating_sub(self.reserve)
    }

    /// Bytes the context's pending changes may take in memory, as
    /// [`Held::memory`] counts them: [`CEILING_BYTES_PER_OCTET`] for each
    /// octet [`HISTORY_NOTIFICATIONS`] notifications of the local maximum
    /// APDU `local` have for items, whatever the subscriber's.
    fn ceiling(self, local: usize) -> usize {
        CEILING_BYTES_PER_OCTET * HISTORY_NOTIFICATIONS * local.saturating_sub(self.envelope)
    }

    /// Whether the context may hold `held`: within both its room and its
    /// memory ceiling.
    fn holds(self, held: Held, local: usize) -> bool {
        held.octets <= self.room() && held.memory <= self.ceiling(local)
    }
}

/// The encoded value `values` carry for the reference's own coordinate.
fn own_value(key: &CovSubscriptionKey, values: &[COVNotificationValue]) -> Option<Vec<u8>> {
    let CovSubscriptionKey::Multiple {
        property, index, ..
    } = key
    else {
        return None;
    };
    values
        .iter()
        .find(|value| {
            value.property_identifier == *property && value.property_array_index == *index
        })
        .map(|value| value.value.clone())
}

/// How [`TimedHistories::field_time`] times a carried field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldTiming {
    /// The reference is no longer live: the field is an ordinary value.
    NotLive,
    /// No time belongs to this value: leave the field out.
    NoTime,
    /// The capture sequence and clock frame of the change that set it.
    At(u64, ClockFrame),
}

/// Pending timestamped changes of every live timestamped Multiple reference.
#[derive(Debug)]
pub struct TimedHistories {
    histories: BTreeMap<CovSubscriptionKey, TimedHistory>,
    /// Every live untimestamped COV-multiple reference (#1038).
    untimed: BTreeMap<CovSubscriptionKey, UntimedReference>,
    /// Sizing terms of every context with a timestamped history.
    terms: BTreeMap<MultipleContextKey, ContextTerms>,
    /// Unconfirmed contexts with a report going out, and whether another
    /// fanout stood back meanwhile and is owed a follow-up.
    sending: BTreeMap<MultipleContextKey, bool>,
    /// Pending changes of every context with any, as its bound counts them.
    context_held: BTreeMap<MultipleContextKey, Held>,
    /// Earliest the deadline backstop may hand a context out again. Only a
    /// context with a timestamped history has an entry.
    not_before: BTreeMap<MultipleContextKey, Instant>,
    /// Where the next fanout over several contexts starts its turns: the
    /// state of [`Self::next_start`]'s sequence.
    turn: u32,
    /// Claims kept until they are settled or their deadline passes, oldest
    /// first.
    lodged: Vec<claim::Lodged>,
    next_claim: u64,
    /// Whether a deadline may have moved earlier since the host last asked
    /// ([`Self::take_moved`]).
    moved: bool,
    next_seq: u64,
    next_incarnation: u64,
    /// Local maximum APDU length.
    local_apdu: usize,
    /// What has been counted since the host last took the counts.
    counts: TimedCounts,
    /// Drops not yet taken by the host.
    drops: Vec<Dropped>,
}

/// What the histories counted since their host last took the counts
/// ([`TimedHistories::take_counts`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TimedCounts {
    /// Pending timestamped changes discarded while their reference stayed
    /// subscribed: evicted on overflow of the context's bound, superseded by
    /// a newer delivered change, or, once per change, losing a value too
    /// large for any notification to the subscriber.
    pub changes_dropped: u64,
    /// Untimestamped references a report left out because their values alone
    /// exceed one notification to the subscriber (#1066).
    pub untimed_oversized: u64,
}

impl TimedHistories {
    /// Histories of a device whose local maximum APDU is `local_apdu`
    /// octets, which bounds each context's pending changes.
    pub fn new(local_apdu: usize) -> Self {
        Self {
            histories: BTreeMap::new(),
            untimed: BTreeMap::new(),
            terms: BTreeMap::new(),
            sending: BTreeMap::new(),
            context_held: BTreeMap::new(),
            not_before: BTreeMap::new(),
            turn: TURN_SEED,
            lodged: Vec::new(),
            next_claim: 1,
            moved: false,
            next_seq: 1,
            next_incarnation: 1,
            local_apdu,
            counts: TimedCounts::default(),
            drops: Vec::new(),
        }
    }

    /// What was counted since the last call.
    pub fn take_counts(&mut self) -> TimedCounts {
        core::mem::take(&mut self.counts)
    }

    /// Whether a deadline may have moved earlier since the last call: a
    /// change queued ahead of the others, a reference owed from earlier, a
    /// shorter delay, a hold-off, a lifted block, a claim lodged. A host
    /// sleeping until the next deadline [`Self::take_due`] gave it asks
    /// again.
    pub fn take_moved(&mut self) -> bool {
        core::mem::take(&mut self.moved)
    }

    /// Apply a context's admitted Max_Notification_Delay to its timestamped
    /// histories. Every admission of the context, renewals included, calls
    /// it; a context without one keeps nothing.
    pub fn set_delay(&mut self, context: &MultipleContextKey, seconds: u32) {
        let delay = Duration::from_secs(u64::from(seconds));
        let mut shorter = false;
        for (key, history) in &mut self.histories {
            if key.multiple_context() == Some(context) {
                shorter |= delay < history.delay;
                history.delay = delay;
            }
        }
        for (key, reference) in &mut self.untimed {
            if key.multiple_context() == Some(context) {
                shorter |= reference.owed_since.is_some() && delay < reference.delay;
                reference.delay = delay;
            }
        }
        if shorter {
            self.rearm_context(context);
        }
    }

    /// Apply an admission of the context, renewals included: the maximum
    /// APDU its subscriber last advertised, if known, sizes notifications and
    /// the history bound together with the local maximum, and the seconds of
    /// lifetime it has left size the envelope of each (#1197). The admission
    /// may have changed the context's references, so the reserve for its
    /// untimestamped values starts over and its next report notes it again.
    pub(crate) fn set_sizing(
        &mut self,
        context: &MultipleContextKey,
        subscriber: Option<u16>,
        time_remaining: u32,
    ) {
        let apdu = subscriber.map_or(self.local_apdu, |subscriber| {
            self.local_apdu.min(usize::from(subscriber))
        });
        if let Some(terms) = self.terms.get_mut(context) {
            // Changes that fit no notification before may fit now, or the
            // other way round: warn afresh about drops (#1039).
            if terms.apdu != apdu {
                terms.warned = DropWarnings::default();
            }
            terms.apdu = apdu;
            terms.envelope = envelope_len(context, time_remaining);
            terms.reserve = 0;
        }
    }

    /// Note the octets the context's untimestamped values took in a report.
    /// The history bound keeps room for the most they have taken, at most one
    /// notification's worth, since they travel with the last notification of
    /// the next report too.
    pub(crate) fn note_reserve(&mut self, context: &MultipleContextKey, octets: usize) {
        if let Some(terms) = self.terms.get_mut(context) {
            terms.reserve = terms.reserve.max(octets.min(terms.notification()));
        }
    }

    /// Start sending a report to an unconfirmed `context`, unless one is
    /// still going out; then note that this fanout stood back and return
    /// `false`.
    ///
    /// The parts of a report go out one after another, and an unconfirmed
    /// context has no outstanding-report mark, so another fanout could send
    /// a newer change or value between them: the subscriber would see a
    /// reference move back to older history, or to the older value a later
    /// part carries. Between this and [`Self::end_send`] other fanouts of
    /// the context stand back and leave their changes queued (#986, #1038).
    pub fn begin_send(&mut self, context: &MultipleContextKey) -> bool {
        match self.sending.get_mut(context) {
            Some(owed) => {
                *owed = true;
                false
            }
            None => {
                self.sending.insert(context.clone(), false);
                true
            }
        }
    }

    /// Finish sending to `context`, after the last part or when the report
    /// stops early; `true` if a fanout stood back meanwhile, which the host
    /// then owes one follow-up over the context's references. The follow-up
    /// reads the untimestamped values afresh.
    pub fn end_send(&mut self, context: &MultipleContextKey) -> bool {
        self.sending.remove(context).unwrap_or(false)
    }

    /// Where a fanout over `contexts` contexts starts its turns: an index
    /// below `contexts`, from a sequence that moves on at every call.
    ///
    /// One input may send only so many notifications, and the contexts
    /// whose turn comes after its budget is spent report nothing. From a
    /// fixed start the same ones would lose every time. A counter would
    /// share the loss out only until changes fall into step with it: with
    /// two contexts and two objects changing in alternation, each context
    /// would always lose the same object. So the start comes from a small
    /// generator (xorshift32) whose steps follow no such pattern. It
    /// promises an even spread over many fanouts, not a turn for any one
    /// context within a given number of them. The seed is fixed, so two
    /// hosts given the same fanouts take the same turns.
    pub(crate) fn next_start(&mut self, contexts: usize) -> usize {
        let mut x = self.turn;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.turn = x;
        // The upper half scaled by the count: no division, and no bias
        // that matters for the few contexts one fanout has.
        let count = contexts.min(0xFFFF) as u32;
        (((x >> 16) * count) >> 16) as usize
    }

    /// Drop every backstop wait: a block lifted, so overdue changes go out
    /// at the next scan.
    pub fn rearm(&mut self) {
        self.not_before.clear();
        self.moved = true;
    }

    /// A shorter delay, or a hold-off that went with a replaced flight,
    /// moves the context's deadline earlier: drop its wait, so the host
    /// works the deadline out again.
    fn rearm_context(&mut self, context: &MultipleContextKey) {
        self.not_before.remove(context);
        self.moved = true;
    }

    /// A confirmed hold-off blocks `context` until `until`: the backstop's
    /// next attempt waits for exactly that, not for its usual spacing.
    pub fn hold_until(&mut self, context: &MultipleContextKey, until: Instant) {
        if self
            .histories
            .keys()
            .any(|key| key.multiple_context() == Some(context))
            || self.owes(context)
        {
            self.not_before.insert(context.clone(), until);
            self.moved = true;
        }
    }

    /// Earliest pending change or owed reference, and shortest delay, of
    /// each context with either.
    fn pending<'a>(
        histories: &'a BTreeMap<CovSubscriptionKey, TimedHistory>,
        untimed: &'a BTreeMap<CovSubscriptionKey, UntimedReference>,
    ) -> BTreeMap<&'a MultipleContextKey, (Instant, Duration)> {
        let mut pending: BTreeMap<&MultipleContextKey, (Instant, Duration)> = BTreeMap::new();
        let timed = histories.iter().filter_map(|(key, history)| {
            Some((key, history.entries.front()?.captured_at, history.delay))
        });
        let owed = untimed
            .iter()
            .filter_map(|(key, reference)| Some((key, reference.owed_since?, reference.delay)));
        for (key, since, delay) in timed.chain(owed) {
            let Some(context) = key.multiple_context() else {
                continue;
            };
            pending
                .entry(context)
                .and_modify(|(at, shortest)| {
                    *at = (*at).min(since);
                    *shortest = (*shortest).min(delay);
                })
                .or_insert((since, delay));
        }
        pending
    }

    /// When a context whose earliest pending change or owed reference is
    /// from `first` is due, and the wait an attempt at it starts: its delay
    /// after `first`, but no sooner than [`DEADLINE_FLOOR`] after it, nor
    /// before `wait`, the end of its wait from a previous attempt or
    /// hold-off.
    fn due_at(first: Instant, delay: Duration, wait: Option<Instant>) -> (Instant, Duration) {
        let spacing = delay.max(DEADLINE_FLOOR);
        let deadline = first + spacing;
        (wait.map_or(deadline, |wait| deadline.max(wait)), spacing)
    }

    /// The earliest instant [`Self::take_due`] has something to do: a
    /// context's Max_Notification_Delay deadline (Clause 13.16.1.1.4), or a
    /// lodged claim's ([`Self::lodge`]). It may have passed already. `None`
    /// while nothing is pending, owed or lodged.
    pub fn next_deadline(&self) -> Option<Instant> {
        let backstop = Self::pending(&self.histories, &self.untimed)
            .into_iter()
            .map(|(context, (first, delay))| {
                Self::due_at(first, delay, self.not_before.get(context).copied()).0
            })
            .min();
        bacnet_time::earliest([backstop, self.next_unsettled()])
    }

    /// Give back every lodged claim whose deadline has passed at `now`,
    /// then take the references with pending changes, or owed values, of
    /// every context whose deadline has passed at `now`, and the next
    /// deadline still ahead, a context's or a lodged claim's.
    ///
    /// A context is due at its earliest pending change or owed reference plus
    /// its delay, but no sooner than one second after it (`DEADLINE_FLOOR`),
    /// nor before its wait from a previous attempt or hold-off. Handing it out starts a
    /// wait of its delay (the floor at least), in case the attempt is blocked
    /// again.
    pub fn take_due(&mut self, now: Instant) -> (Vec<CovSubscriptionKey>, Option<Instant>) {
        // First, so a claim nobody settled is pending again for its context.
        self.requeue_unsettled(now);
        let pending = Self::pending(&self.histories, &self.untimed);
        let mut due = Vec::new();
        let mut next = self.next_unsettled();
        for (&context, &(first, delay)) in &pending {
            let (deadline, spacing) =
                Self::due_at(first, delay, self.not_before.get(context).copied());
            if deadline <= now {
                due.push((context.clone(), spacing));
            } else {
                next = Some(next.map_or(deadline, |at| at.min(deadline)));
            }
        }
        self.not_before
            .retain(|context, _| pending.contains_key(context));
        for (context, spacing) in &due {
            self.not_before.insert(context.clone(), now + *spacing);
        }
        let due: Vec<_> = due.into_iter().map(|(context, _)| context).collect();
        let is_due = |key: &CovSubscriptionKey| {
            key.multiple_context()
                .is_some_and(|context| due.contains(context))
        };
        let timed = self
            .histories
            .iter()
            .filter(|(key, history)| !history.entries.is_empty() && is_due(key))
            .map(|(key, _)| key.clone());
        let owed = self
            .untimed
            .iter()
            .filter(|(key, reference)| reference.owed_since.is_some() && is_due(key))
            .map(|(key, _)| key.clone());
        (timed.chain(owed).collect(), next)
    }

    /// Bind a (re)published reference generation with its context's admitted
    /// Max_Notification_Delay in seconds. A renewal keeps changes not yet
    /// conveyed; its initial report is captured afresh.
    pub fn reset(&mut self, key: &CovSubscriptionKey, generation: u64, delay: u32) {
        let delay = Duration::from_secs(u64::from(delay));
        let incarnation = self.next_incarnation;
        if let Some(context) = key.multiple_context() {
            let apdu = self.local_apdu;
            // Until the admission's sizing, the envelope of the longest
            // lifetime.
            let envelope = envelope_len(context, u32::MAX);
            let terms = self.terms.entry(context.clone()).or_insert(ContextTerms {
                apdu,
                envelope,
                reserve: 0,
                warned: DropWarnings::default(),
            });
            // A (re)admitted reference warns afresh about drops (#1039).
            terms.warned = DropWarnings::default();
        }
        let history = self
            .histories
            .entry(key.clone())
            .or_insert_with(|| TimedHistory {
                incarnation,
                generation,
                baseline: None,
                field: None,
                latest: 0,
                committed: 0,
                in_delivery: 0,
                delay,
                entries: VecDeque::new(),
            });
        if history.incarnation == incarnation {
            self.next_incarnation += 1;
        }
        let shorter = delay < history.delay;
        history.generation = generation;
        history.baseline = None;
        history.delay = delay;
        if let Some(context) = key.multiple_context().filter(|_| shorter) {
            self.rearm_context(context);
        }
    }

    /// Drop the history, or owed mark, of a removed reference, and its
    /// context's backstop wait once no timestamped reference or owed one of
    /// the context remains.
    pub fn remove(&mut self, key: &CovSubscriptionKey) {
        self.untimed.remove(key);
        if let Some(history) = self.histories.remove(key) {
            self.release(key, Held::of(&history.entries));
        }
        if let Some(context) = key.multiple_context() {
            if self
                .histories
                .keys()
                .any(|other| other.multiple_context() == Some(context))
            {
                // The context lost a reference: its reserve starts over.
                if let Some(terms) = self.terms.get_mut(context) {
                    terms.reserve = 0;
                }
            } else {
                if !self.owes(context) {
                    self.not_before.remove(context);
                }
                self.terms.remove(context);
            }
        }
    }

    /// Timestamped histories (or context terms or bound counts, should any
    /// outlive them) plus untimestamped references, and backstop waits held,
    /// for leak tests. A test hook, behind the `test-support` feature for a
    /// host's own tests.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn held(&self) -> (usize, usize) {
        let timed = self
            .histories
            .len()
            .max(self.terms.len())
            .max(self.context_held.len());
        (timed + self.untimed.len(), self.not_before.len())
    }

    /// The octets a notification to `context` spends outside its items, and
    /// those it has for items, as last admitted. A hook for a host's tests of
    /// what its admissions size, behind the `test-support` feature.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn sizing_for_test(&self, context: &MultipleContextKey) -> Option<(usize, usize)> {
        self.terms
            .get(context)
            .map(|terms| (terms.envelope, terms.notification()))
    }

    /// Latest captured or conveyed observation of a live generation.
    pub fn baseline(&self, key: &CovSubscriptionKey, generation: u64) -> Option<&CovObservation> {
        self.history(key, generation)?.baseline.as_ref()
    }

    /// How to time the own field of a timestamped reference that conveys no
    /// change now, carried by a sibling with the encoded `value` (#987).
    ///
    /// A reference that is no longer live (cancelled or renewed since its
    /// fanout looked) owns nothing: the field goes out as an ordinary sibling
    /// value. The value the reference last saw keeps that change's time: a
    /// capture records the own value whether or not it qualified, an admission
    /// or renewal capture included. Any other value changed without a capture
    /// (no producer recorded it): it takes `now()`, the preparation time,
    /// which is remembered for that value so a later notification carrying it
    /// reports the same time; the reference's baseline, and so its increment,
    /// is untouched. Without a time, because the clock is invalid or because a
    /// producer snapshot may be older than the record, the field has none to
    /// give.
    pub(crate) fn field_time(
        &mut self,
        key: &CovSubscriptionKey,
        generation: u64,
        value: &[u8],
        now: impl FnOnce() -> Option<ClockFrame>,
    ) -> FieldTiming {
        let seq = self.next_seq;
        let Some(history) = self.history_mut(key, generation) else {
            return FieldTiming::NotLive;
        };
        if let Some(field) = history.field.as_ref().filter(|field| field.value == value) {
            return FieldTiming::At(field.seq, field.frame);
        }
        let Some(frame) = now() else {
            return FieldTiming::NoTime;
        };
        history.field = Some(FieldRecord {
            value: value.to_vec(),
            seq,
            frame,
        });
        self.next_seq += 1;
        FieldTiming::At(seq, frame)
    }

    /// Record the own-field value a capture saw although it did not qualify
    /// (it moved less than the reference's increment), at `frame`, the time of
    /// the commit, so a sibling carrying it reports when it really changed.
    pub(crate) fn note_field(
        &mut self,
        key: &CovSubscriptionKey,
        generation: u64,
        values: &[COVNotificationValue],
        frame: ClockFrame,
    ) {
        let seq = self.next_seq;
        let (Some(history), Some(value)) =
            (self.history_mut(key, generation), own_value(key, values))
        else {
            return;
        };
        if history
            .field
            .as_ref()
            .is_some_and(|field| field.value == value)
        {
            return;
        }
        history.field = Some(FieldRecord { value, seq, frame });
        self.next_seq += 1;
    }

    /// Queue a captured change and make it the reference's baseline, evicting
    /// older pending changes if the context bound is exceeded. Returns `false`
    /// when the generation is no longer live and nothing was queued.
    pub fn push(&mut self, key: &CovSubscriptionKey, generation: u64, change: TimedChange) -> bool {
        if self.history(key, generation).is_none() {
            return false;
        }
        let change = self.adopt(key, generation, change);
        self.insert_ordered(key, generation, vec![change], true);
        true
    }

    /// Sequence a change and make it the reference's baseline without
    /// queueing it (the builder's current-state fallback).
    pub(crate) fn adopt(
        &mut self,
        key: &CovSubscriptionKey,
        generation: u64,
        mut change: TimedChange,
    ) -> TimedChange {
        change.seq = self.next_seq;
        self.next_seq += 1;
        if let Some(history) = self.history_mut(key, generation) {
            history.baseline = Some(change.observation.clone());
            history.field = own_value(key, &change.values).map(|value| FieldRecord {
                value,
                seq: change.seq,
                frame: change.frame,
            });
            history.latest = change.seq;
        }
        change
    }

    /// Take every pending change of a live generation, oldest first, with the
    /// history incarnation that later commits or returns them. Older changes
    /// returned by a failed notification stay queued while the reference's
    /// newest change is still in flight elsewhere, so they are never conveyed
    /// as its latest state; that change's outcome settles them.
    pub fn drain(&mut self, key: &CovSubscriptionKey, generation: u64) -> (u64, Vec<TimedChange>) {
        let Some(history) = self.history_mut(key, generation) else {
            return (0, Vec::new());
        };
        let incarnation = history.incarnation;
        if history
            .entries
            .back()
            .is_some_and(|newest| newest.seq < history.latest)
        {
            return (incarnation, Vec::new());
        }
        let drained: Vec<_> = history.entries.drain(..).collect();
        self.release(key, Held::of(&drained));
        (incarnation, drained)
    }

    /// Record that changes up to `seq` were delivered; queued older changes
    /// are superseded by it.
    fn commit_through(&mut self, key: &CovSubscriptionKey, incarnation: u64, seq: u64) {
        let Some(history) = self.incarnation_mut(key, incarnation) else {
            return;
        };
        history.committed = history.committed.max(seq);
        let committed = history.committed;
        let stale = history
            .entries
            .partition_point(|change| change.seq < committed);
        let removed: Vec<_> = history.entries.drain(..stale).collect();
        if !removed.is_empty() {
            self.release(key, Held::of(&removed));
            self.dropped(key, removed.len(), DropReason::Superseded);
        }
    }

    /// Return undelivered changes to their reference in capture order, also
    /// across a renewal. A cancelled reference discards them, and a newer
    /// delivered change supersedes them: delivering them now would regress
    /// the subscriber. `evict` applies the context bound to the result.
    fn requeue(
        &mut self,
        key: &CovSubscriptionKey,
        incarnation: u64,
        changes: Vec<TimedChange>,
        evict: bool,
    ) {
        let Some(history) = self.incarnation_mut(key, incarnation) else {
            return;
        };
        let generation = history.generation;
        let committed = history.committed;
        let (keep, stale): (Vec<_>, Vec<_>) = changes
            .into_iter()
            .partition(|change| change.seq > committed);
        if !stale.is_empty() {
            self.dropped(key, stale.len(), DropReason::Superseded);
        }
        self.insert_ordered(key, generation, keep, evict);
    }

    fn insert_ordered(
        &mut self,
        key: &CovSubscriptionKey,
        generation: u64,
        changes: Vec<TimedChange>,
        evict: bool,
    ) {
        let Some(history) = self.history_mut(key, generation) else {
            return;
        };
        let mut added = Held::default();
        // A new oldest pending change can bring its context's deadline forward.
        let mut new_front = false;
        for mut change in changes {
            change.continues = false;
            let at = history.entries.partition_point(|e| e.seq < change.seq);
            match history.entries.get_mut(at).filter(|e| e.seq == change.seq) {
                // A change sent one value per notification comes back in
                // parts, which rejoin as one change (#1090).
                Some(entry) => {
                    let before = entry.memory();
                    added.octets += entry.rejoin(change);
                    added.memory += entry.memory() - before;
                }
                None => {
                    added.octets += change.octets;
                    added.memory += change.memory();
                    new_front |= at == 0;
                    history.entries.insert(at, change);
                }
            }
        }
        if let Some(context) = key.multiple_context().filter(|_| added != Held::default()) {
            self.context_held
                .entry(context.clone())
                .or_default()
                .grow(added);
        }
        if evict {
            self.enforce_bound(key);
        }
        if new_front {
            self.moved = true;
        }
    }

    fn history(&self, key: &CovSubscriptionKey, generation: u64) -> Option<&TimedHistory> {
        self.histories
            .get(key)
            .filter(|h| h.generation == generation)
    }

    fn history_mut(
        &mut self,
        key: &CovSubscriptionKey,
        generation: u64,
    ) -> Option<&mut TimedHistory> {
        self.histories
            .get_mut(key)
            .filter(|h| h.generation == generation)
    }

    fn incarnation_mut(
        &mut self,
        key: &CovSubscriptionKey,
        incarnation: u64,
    ) -> Option<&mut TimedHistory> {
        self.histories
            .get_mut(key)
            .filter(|h| h.incarnation == incarnation)
    }
}

mod bound;
mod claim;
mod drops;
mod follow;
mod owed;
use bound::Held;
pub(crate) use bound::envelope_len;
pub(crate) use bound::request_header;
pub(crate) use claim::ValueFit;
pub use claim::{ClaimId, Settlement, TimedClaim};
use drops::DropWarnings;
pub use drops::{DropReason, Dropped};
pub use follow::follow_table;

#[cfg(test)]
#[path = "timed_test_host.rs"]
mod host;
#[cfg(test)]
#[path = "timed_settle_tests.rs"]
mod settle_tests;
#[cfg(test)]
#[path = "timed_split_tests.rs"]
mod split_tests;
#[cfg(test)]
#[path = "timed_tests.rs"]
mod tests;
