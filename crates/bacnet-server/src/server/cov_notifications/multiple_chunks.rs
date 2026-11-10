//! Splitting one COV-multiple report into notifications that each fit the
//! subscriber's maximum APDU (135-2020 §13.1, §13.18.1.1; #986, #1008,
//! #1038, #1090).
//!
//! The report's timestamped changes go out strictly in capture order, each
//! reference's latest change included. The last notification carries the
//! untimestamped values with as many of the newest changes as still fit, and
//! the older changes go out first, oldest first, in as few notifications as
//! fit. A reference whose latest change goes out early conveys no change in
//! the last notification, where a sibling carrying its field times it as for
//! any reference that conveys no change (#987). A change that does not fit a
//! notification even alone, latest or not, goes out one value per
//! notification instead, its values in the order they were captured, each
//! with the change's Time_Of_Change and an envelope naming the change (#1090).
//! A reference whose latest change goes out so is finished by the part with
//! its last value. A value that does not fit even alone is dropped and
//! counted, since every attempt to send it would fail.
//!
//! When the untimestamped values alone do not fit one notification, every
//! timestamped change goes out first, and the untimestamped values follow in
//! as few notifications as fit: runs of whole object items, and one object's
//! references apart where its item alone does not fit (#1038). Each of those
//! notifications completes only the references it carries, and any value a
//! sibling's time stamps there belongs to a change already conveyed. A
//! reference whose values fit no notification alone is left out and handed
//! back in [`Split::forgone`] for the host to count and log (#1066); it is
//! evaluated again at its next fanout.
//!
//! The split holds no histories. It takes the claim as plain data and hands
//! back, beside the parts, what the histories have to be told: the changes
//! that lost a value ([`Split::lost`]), the references left out
//! ([`Split::forgone`]) and whatever no part carries ([`Split::rest`]).
use std::collections::BTreeSet;

use bacnet_services::cov_multiple::{
    COVNotificationItem, COVNotificationMultipleRequest, COVNotificationValue,
};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::multiple_items::{Coordinate, History, Latest, Retained, Stamp, build_items};
use bacnet_device_core::cov::CovSubscriptionKey;
use bacnet_device_core::cov::CovSubscriptionSnapshot;
use bacnet_device_core::cov::multiple_reads::MultipleReads;
use bacnet_device_core::cov::timed::{
    ITEM_FRAMING, TimedChange, TimedClaim, ValueFit, request_header, value_len,
};

/// Project the request-level and per-value COV timestamps from one sample.
pub(in crate::server) fn cov_multiple_datetime(
    frame: bacnet_objects::clock::ClockFrame,
) -> (
    bacnet_types::primitives::Date,
    bacnet_types::primitives::Time,
) {
    (frame.local_date, frame.local_time)
}

/// Envelope fields every notification of one report shares.
pub(super) struct Envelope {
    pub(super) subscriber_process_identifier: u32,
    pub(super) initiating_device_identifier: ObjectIdentifier,
    pub(super) time_remaining: u32,
}

impl Envelope {
    fn notification(
        &self,
        timestamp: Option<bacnet_objects::clock::ClockFrame>,
        items: Vec<COVNotificationItem>,
    ) -> COVNotificationMultipleRequest {
        COVNotificationMultipleRequest {
            subscriber_process_identifier: self.subscriber_process_identifier,
            initiating_device_identifier: self.initiating_device_identifier,
            time_remaining: self.time_remaining,
            timestamp: timestamp.map(cov_multiple_datetime),
            list_of_cov_notifications: items,
        }
    }
}

/// A report split into the notifications that convey it, with what the split
/// gave up or left over.
pub(super) struct Split {
    /// The notifications, in the order they go out.
    pub(super) parts: Vec<Part>,
    /// The reference of each change that lost a value too large for any
    /// notification to the subscriber, once per change and in capture order
    /// (#1090): `TimedHistories::lost_values` counts them.
    pub(super) lost: Vec<CovSubscriptionKey>,
    /// The untimestamped references whose values fit no notification on
    /// their own (#1066): `TimedHistories::forgo_untimed` counts them, and
    /// they are not owed.
    pub(super) forgone: TimedClaim,
    /// What no part carries: the changes of a last notification that would
    /// convey nothing. `TimedHistories::give_back` returns them to their
    /// references.
    pub(super) rest: TimedClaim,
}

/// One notification of a report, with the changes and values it retires once
/// delivered.
pub(super) struct Part {
    pub(super) notification: COVNotificationMultipleRequest,
    /// What the part's delivery retires: its holder commits it once the
    /// notification is delivered, and gives it back otherwise.
    pub(super) claim: TimedClaim,
    /// References whose observation this part's delivery completes: each
    /// timestamped reference whose latest drained change it carries, and each
    /// untimestamped reference whose values it carries.
    pub(super) finishes: BTreeSet<CovSubscriptionKey>,
}

/// Service-request octets one notification may take: the smaller of the local
/// maximum APDU and the subscriber's, less the request header.
pub(super) fn request_limit(local: u32, subscriber: Option<u16>, confirmed: bool) -> usize {
    let local = usize::try_from(local).unwrap_or(usize::MAX);
    let apdu = subscriber.map_or(local, |subscriber| local.min(usize::from(subscriber)));
    apdu.saturating_sub(request_header(confirmed))
}

/// Octets the untimestamped values among `retained` take, with one item's
/// framing per object.
pub(super) fn untimed_octets(retained: &Retained<'_>) -> usize {
    let mut objects = BTreeSet::new();
    retained
        .iter()
        .filter(|(sub, _)| !sub.timestamped)
        .map(|(sub, values)| {
            let framing = if objects.insert(sub.monitored_object_identifier) {
                ITEM_FRAMING
            } else {
                0
            };
            framing + values.iter().map(value_len).sum::<usize>()
        })
        .sum()
}

/// What one report's notifications carry besides the claimed changes.
pub(super) struct ReportContent<'a> {
    pub(super) envelope: &'a Envelope,
    pub(super) retained: &'a Retained<'a>,
    pub(super) reads: &'a MultipleReads,
    pub(super) untimed: &'a BTreeSet<Coordinate>,
    /// Times a field of a timestamped reference that conveys no change in the
    /// last notification, carried by a sibling (#987). Only the last
    /// notification has such current values.
    pub(super) stamp: &'a Stamp<'a>,
}

impl ReportContent<'_> {
    /// Notification of `changes` as history only, named after its last.
    fn history(&self, changes: &History<'_>) -> COVNotificationMultipleRequest {
        let timestamp = changes.last().map(|(_, change)| change.frame());
        let (items, _) = build_items(
            changes,
            None,
            &Latest::new(),
            self.reads,
            self.untimed,
            self.stamp,
        );
        self.envelope.notification(timestamp, items)
    }

    /// The report's last notification, carrying every retained reference.
    fn last(&self, changes: &History<'_>) -> COVNotificationMultipleRequest {
        self.current(changes, self.retained)
    }

    /// A notification of current state. Of `changes`, in capture order, each
    /// reference's last is its current state and the others are history; the
    /// untimestamped values of `retained` follow. Its header names the newest
    /// change whose time it carries, claimed now or kept by a reference that
    /// conveys no change.
    fn current(
        &self,
        changes: &History<'_>,
        retained: &Retained<'_>,
    ) -> COVNotificationMultipleRequest {
        let latest: Latest<'_> = changes.iter().copied().collect();
        let history: Vec<_> = changes
            .iter()
            .copied()
            .filter(|(key, change)| latest[key].seq() != change.seq())
            .collect();
        let (items, stamped) = build_items(
            &history,
            Some(retained),
            &latest,
            self.reads,
            self.untimed,
            self.stamp,
        );
        let newest = changes
            .last()
            .map(|(_, change)| (change.seq(), change.frame()))
            .into_iter()
            .chain(stamped)
            .max_by_key(|(seq, _)| *seq)
            .map(|(_, frame)| frame);
        self.envelope.notification(newest, items)
    }
}

/// Encoded length of a notification's service request; `None` if it does not
/// encode.
fn encoded_len(notification: &COVNotificationMultipleRequest) -> Option<usize> {
    let mut encoded = BytesMut::new();
    notification.encode(&mut encoded).ok()?;
    Some(encoded.len())
}

fn fits(notification: &COVNotificationMultipleRequest, limit: usize) -> bool {
    encoded_len(notification).is_some_and(|len| len <= limit)
}

/// Split the report `claim` conveys into notifications of at most `limit`
/// service-request octets, in the order they go out: the oldest changes
/// first, each part with the claim it retires, and last the notification
/// with the untimestamped values and the newest changes, left out when it
/// would carry nothing. A change too large for a notification of its own
/// goes out one value per notification (#1090). Untimestamped values that do
/// not fit one notification alone go out after every change, in as many
/// notifications as they need.
pub(super) fn split(content: &ReportContent<'_>, mut claim: TimedClaim, limit: usize) -> Split {
    // Each earlier part as its count of the oldest remaining changes, and
    // whether that one change cannot fit a notification on its own.
    let plan = {
        let changes = claim.in_order();
        let whole = content.last(&changes);
        // A notification that does not encode is reported by its send.
        if encoded_len(&whole).is_none_or(|len| len <= limit) {
            return finish(content, Vec::new(), whole, claim, Vec::new());
        }
        // Moving more of the oldest changes out never grows the rest, so the
        // fewest to move out can be found by bisection. Moving all of them
        // leaves the untimestamped values alone, or nothing, which is left
        // out and so always fits.
        let moved = smallest(changes.len(), |count| {
            let last = content.last(&changes[count..]);
            last.list_of_cov_notifications.is_empty() || fits(&last, limit)
        });
        let mut plan = Vec::new();
        let mut start = 0;
        while start < moved {
            let alone = |end: usize| fits(&content.history(&changes[start..end]), limit);
            let end = largest(start + 1, moved, alone);
            plan.push((end - start, end == start + 1 && !alone(end)));
            start = end;
        }
        plan
    };
    let mut parts = Vec::new();
    let mut lost = Vec::new();
    // How a notification of one value of a change too large for a
    // notification of its own would go out.
    let fit = |key: &CovSubscriptionKey, value: &TimedChange| {
        let notification = content.history(&[(key, value)]);
        if notification.list_of_cov_notifications.is_empty() {
            ValueFit::Empty
        } else if fits(&notification, limit) {
            ValueFit::Fits
        } else {
            ValueFit::TooLarge
        }
    };
    for (count, too_large) in plan {
        let part = claim.split_oldest(count);
        if !too_large {
            parts.push((content.history(&part.in_order()), part));
            continue;
        }
        let (values, gave_up) = part.split_values(fit);
        lost.extend(gave_up);
        for value in values {
            parts.push((content.history(&value.in_order()), value));
        }
    }
    let last = content.last(&claim.in_order());
    if encoded_len(&last).is_some_and(|len| len > limit) {
        // The untimestamped values alone do not fit, so every change went
        // into the parts above.
        let mut parts = history_parts(parts);
        let (untimed, forgone) = untimed_parts(content, &mut claim, limit);
        parts.extend(untimed);
        finish_timed(&mut parts);
        return Split {
            parts,
            lost,
            forgone,
            rest: claim,
        };
    }
    finish(content, parts, last, claim, lost)
}

/// The report's parts: the earlier `parts`, then `last` with the rest of the
/// claim, finishing every untimestamped reference, unless it carries nothing;
/// the rest of the claim is then left over.
fn finish(
    content: &ReportContent<'_>,
    parts: Vec<(COVNotificationMultipleRequest, TimedClaim)>,
    last: COVNotificationMultipleRequest,
    claim: TimedClaim,
    lost: Vec<CovSubscriptionKey>,
) -> Split {
    let mut parts = history_parts(parts);
    let mut rest = TimedClaim::new();
    if last.list_of_cov_notifications.is_empty() {
        rest = claim;
    } else {
        let untimed = content
            .retained
            .iter()
            .filter(|(sub, _)| !sub.timestamped)
            .map(|(sub, _)| sub.key().clone());
        parts.push(Part {
            notification: last,
            claim,
            finishes: untimed.collect(),
        });
    }
    finish_timed(&mut parts);
    Split {
        parts,
        lost,
        forgone: TimedClaim::new(),
        rest,
    }
}

/// Parts of history only, finishing nothing yet.
fn history_parts(parts: Vec<(COVNotificationMultipleRequest, TimedClaim)>) -> Vec<Part> {
    parts
        .into_iter()
        .map(|(notification, claim)| Part {
            notification,
            claim,
            finishes: BTreeSet::new(),
        })
        .collect()
}

/// Of the report's `parts`, in send order, a timestamped reference is
/// finished by the last that carries it, which carries its latest drained
/// change, or the last value of it sent when that change went out one value
/// per notification (#1090).
fn finish_timed(parts: &mut [Part]) {
    let mut later = BTreeSet::new();
    for part in parts.iter_mut().rev() {
        for (key, _) in part.claim.last_changes() {
            if later.insert(key.clone()) {
                part.finishes.insert(key.clone());
            }
        }
    }
}

/// One retained untimestamped reference with its prepared values.
type Untimed<'a> = (&'a CovSubscriptionSnapshot, &'a [COVNotificationValue]);

/// The report's untimestamped values, which `claim` carries with no change
/// left, in as few notifications of current state as fit `limit`: runs of
/// whole object items, and one object's references apart where its item does
/// not fit alone. A reference whose values fit no notification alone is left
/// out, not owed: the second claim holds those, for the host to count
/// (#1038, #1066).
fn untimed_parts(
    content: &ReportContent<'_>,
    claim: &mut TimedClaim,
    limit: usize,
) -> (Vec<Part>, TimedClaim) {
    // Untimestamped references by object, in the order they were retained.
    let mut objects: Vec<Vec<Untimed<'_>>> = Vec::new();
    for &(sub, values) in content.retained.iter().filter(|(sub, _)| !sub.timestamped) {
        let oid = sub.monitored_object_identifier;
        match objects
            .iter_mut()
            .find(|refs| refs[0].0.monitored_object_identifier == oid)
        {
            Some(refs) => refs.push((sub, values)),
            None => objects.push(vec![(sub, values)]),
        }
    }
    let alone = |refs: &[Untimed<'_>]| fits(&content.current(&[], refs), limit);
    // Adding an object or a reference to a run never shrinks the rest, so
    // each run is the longest that fits.
    let mut groups: Vec<Vec<Untimed<'_>>> = Vec::new();
    let mut start = 0;
    while start < objects.len() {
        let run = |end: usize| alone(&objects[start..end].concat());
        let end = largest(start + 1, objects.len(), run);
        if end > start + 1 || run(end) {
            groups.push(objects[start..end].concat());
            start = end;
            continue;
        }
        // This object's item alone does not fit: its references go apart.
        let refs = &objects[start];
        let mut first = 0;
        while first < refs.len() {
            let end = largest(first + 1, refs.len(), |end| alone(&refs[first..end]));
            // A reference that fits no notification even alone joins no
            // group, and so stays in the claim.
            if end > first + 1 || alone(&refs[first..end]) {
                groups.push(refs[first..end].to_vec());
            }
            first = end;
        }
        start += 1;
    }
    let parts = groups
        .into_iter()
        .map(|group| {
            let keys: BTreeSet<_> = group.iter().map(|(sub, _)| sub.key().clone()).collect();
            Part {
                notification: content.current(&[], &group),
                claim: claim.split_untimed(|key| keys.contains(key)),
                finishes: keys,
            }
        })
        .collect();
    // What is left fits no notification: owing it would retry it for good,
    // so it is counted and given up instead.
    let forgone = claim.split_untimed(|_| true);
    (parts, forgone)
}

/// Smallest `count` in `1..=max` for which `fits` holds, knowing it fails at
/// zero and only flips once; `max` when it never holds.
fn smallest(max: usize, fits: impl Fn(usize) -> bool) -> usize {
    let (mut failing, mut fitting) = (0, max);
    if !fits(max) {
        return max;
    }
    while fitting - failing > 1 {
        let mid = failing + (fitting - failing) / 2;
        if fits(mid) {
            fitting = mid;
        } else {
            failing = mid;
        }
    }
    fitting
}

/// Largest `end` in `first..=max` for which `fits` holds, knowing it only
/// flips once; `first` when even that fails, so every chunk carries at least
/// one change. Gallops from `first` so the cost follows the chunk's size.
fn largest(first: usize, max: usize, fits: impl Fn(usize) -> bool) -> usize {
    if !fits(first) {
        return first;
    }
    let (mut fitting, mut step) = (first, 1);
    let mut failing = loop {
        let next = fitting + step;
        if next > max {
            break max + 1;
        }
        if !fits(next) {
            break next;
        }
        fitting = next;
        step *= 2;
    };
    while failing - fitting > 1 {
        let mid = fitting + (failing - fitting) / 2;
        if fits(mid) {
            fitting = mid;
        } else {
            failing = mid;
        }
    }
    fitting
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bisection_finds_the_flip_point() {
        for flip in 1..=9 {
            assert_eq!(smallest(9, |count| count >= flip), flip);
            assert_eq!(largest(1, 9, |end| end <= flip), flip);
        }
        assert_eq!(smallest(9, |_| false), 9, "never fits: move everything");
        assert_eq!(largest(3, 9, |_| false), 3, "at least one change");
        assert_eq!(largest(3, 9, |_| true), 9);
    }

    #[test]
    fn the_limit_is_the_smaller_maximum_less_the_header() {
        assert_eq!(request_limit(1476, None, false), 1474);
        assert_eq!(request_limit(1476, Some(206), false), 204);
        assert_eq!(request_limit(480, Some(1476), true), 476);
    }
}
