//! Item assembly for one COV-multiple notification: queued timestamped history
//! followed by one current value per coordinate.
use std::collections::{HashMap, HashSet};

use bacnet_objects::clock::ClockFrame;
use bacnet_services::cov_multiple::{COVNotificationItem, COVNotificationValue};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

use bacnet_device_core::cov::CovSubscriptionKey;
use bacnet_device_core::cov::CovSubscriptionSnapshot;
use bacnet_device_core::cov::multiple_reads::MultipleReads;
use bacnet_device_core::cov::timed::TimedChange;

/// Object, property and index of a subscribed coordinate.
pub(super) type Coordinate = (ObjectIdentifier, Option<PropertyIdentifier>, Option<u32>);

/// Each retained reference with its prepared current values (empty for a
/// timestamped reference, whose latest change in [`Latest`] supplies them).
pub(super) type Retained<'a> = [(&'a CovSubscriptionSnapshot, &'a [COVNotificationValue])];

/// Queued history conveyed as distinct timestamped values, in capture order.
pub(super) type History<'a> = [(&'a CovSubscriptionKey, &'a TimedChange)];

/// The latest change a notification carries of each timestamped reference,
/// conveyed as its current state. A reference without one conveys no change
/// in that notification: its changes went out in an earlier one (#1008).
pub(super) type Latest<'a> = HashMap<&'a CovSubscriptionKey, &'a TimedChange>;

/// Capture sequence and clock frame of a change whose time a value carries.
pub(super) type LastChange = (u64, ClockFrame);

/// How to time a current value that would otherwise go out untimed.
pub(super) enum FieldStamp {
    /// No timestamped reference that conveys no change now owns its coordinate.
    Unowned,
    /// Its owner's field takes the time of this change.
    At(LastChange),
    /// Its owner has no time to give it: leave the value out.
    Withhold,
}

/// Times a current value of a coordinate whose timestamped reference conveys
/// no change now, given the value's encoding.
pub(super) type Stamp<'a> = dyn Fn(&Coordinate, &[u8]) -> FieldStamp + 'a;

/// Build the items of one notification: `history`, then, unless `retained` is
/// `None` (a notification carrying history only), the current state of the
/// `retained` references. Untimestamped references supply their prepared
/// current values; timestamped references their change in `latest`, and those
/// without one add nothing. `untimed` holds coordinates the context explicitly
/// subscribes without timestamps, and `stamp` times the current values
/// described above. Returns the items and the newest change whose time `stamp`
/// gave them.
pub(super) fn build_items(
    history: &History<'_>,
    retained: Option<&Retained<'_>>,
    latest: &Latest<'_>,
    reads: &MultipleReads,
    untimed: &HashSet<Coordinate>,
    stamp: &Stamp<'_>,
) -> (Vec<COVNotificationItem>, Option<LastChange>) {
    let mut items: Vec<COVNotificationItem> = Vec::new();
    let item_for = |items: &mut Vec<COVNotificationItem>, oid: ObjectIdentifier| {
        items
            .iter()
            .position(|item| item.monitored_object_identifier == oid)
            .unwrap_or_else(|| {
                items.push(COVNotificationItem {
                    monitored_object_identifier: oid,
                    list_of_values: Vec::new(),
                });
                items.len() - 1
            })
    };
    // Queued history first: every earlier timestamped change as distinct
    // values with its own time (repeated coordinates are permitted). A
    // coordinate explicitly subscribed without timestamps is never repeated
    // or timestamped (§13.17.3.1.2.4).
    for &(key, change) in history {
        let index = item_for(&mut items, key.object());
        let list = &mut items[index].list_of_values;
        for value in change.values() {
            let coordinate = (
                key.object(),
                Some(value.property_identifier),
                value.property_array_index,
            );
            if !untimed.contains(&coordinate) {
                list.push(value.clone());
            }
        }
    }
    let Some(retained) = retained else {
        // Each history value keeps its own time; no current state follows.
        items.retain(|item| !item.list_of_values.is_empty());
        for item in &mut items {
            let values = std::mem::take(&mut item.list_of_values);
            let rows = values.len();
            item.list_of_values = collapse_repeats(values, rows);
        }
        return (items, None);
    };
    let history_rows: Vec<usize> = items.iter().map(|item| item.list_of_values.len()).collect();
    let start = |index: usize| history_rows.get(index).copied().unwrap_or(0);
    // Current state: one value per coordinate. A timestamped reference
    // contributes its latest change stamped with that change's own time; one
    // whose changes all went out earlier contributes nothing, not even an
    // item (#1008).
    for (sub, values) in retained {
        let values = if sub.timestamped {
            match latest.get(sub.key()) {
                Some(change) => change.values().to_vec(),
                None => continue,
            }
        } else {
            values.to_vec()
        };
        let index = item_for(&mut items, sub.monitored_object_identifier);
        let from = start(index);
        for value in values {
            let current = &mut items[index].list_of_values[from..];
            if let Some(existing) = current.iter_mut().find(|v| {
                v.property_identifier == value.property_identifier
                    && v.property_array_index == value.property_array_index
            }) {
                existing.time_of_change = existing.time_of_change.or(value.time_of_change);
            } else {
                items[index].list_of_values.push(value);
            }
        }
    }
    for (index, item) in items.iter_mut().enumerate() {
        if item.list_of_values[start(index)..]
            .iter()
            .any(|v| v.property_identifier == PropertyIdentifier::STATUS_FLAGS)
        {
            continue;
        }
        if let Some(encoded) = reads.encoded_flags(&item.monitored_object_identifier) {
            item.list_of_values.push(COVNotificationValue {
                property_identifier: PropertyIdentifier::STATUS_FLAGS,
                property_array_index: None,
                value: encoded.to_vec(),
                time_of_change: None,
            });
        }
    }
    // Qualified explicit selectors control their own current coordinate. OR
    // above combines only implicit companion intent; an explicit false remains
    // false. Unqualified references have no entry in this list. A timestamped
    // selector that carries no change here is left to `stamp` below, like any
    // other that conveys no change now (#987, #1008).
    for (sub, _) in retained {
        let current = latest.get(sub.key());
        if sub.timestamped && current.is_none() {
            continue;
        }
        let Some(index) = items
            .iter()
            .position(|item| item.monitored_object_identifier == sub.monitored_object_identifier)
        else {
            continue;
        };
        let from = start(index);
        if let Some(value) = items[index].list_of_values[from..]
            .iter_mut()
            .find(|value| {
                Some(value.property_identifier) == sub.monitored_property
                    && value.property_array_index == sub.monitored_property_array_index
            })
        {
            value.time_of_change = current.map(|change| change.frame().local_time);
        }
    }
    // An explicit timestamped selector that conveys no change this round only
    // fills in a missing time when a sibling carries its field: a value its
    // subscription asked to timestamp never goes out untimed (§13.17.3.1.2.4,
    // §13.18.3.1.3; #987). A companion that already carries a time keeps it.
    // Unlike an explicit untimestamped selector below, which governs its
    // coordinate outright, it overrides nothing.
    let mut stamped: Option<LastChange> = None;
    for (index, item) in items.iter_mut().enumerate() {
        let oid = item.monitored_object_identifier;
        let from = start(index);
        let mut kept = Vec::with_capacity(item.list_of_values.len());
        for (at, mut value) in std::mem::take(&mut item.list_of_values)
            .into_iter()
            .enumerate()
        {
            if at >= from && value.time_of_change.is_none() {
                let coordinate = (
                    oid,
                    Some(value.property_identifier),
                    value.property_array_index,
                );
                match stamp(&coordinate, &value.value) {
                    FieldStamp::Unowned => {}
                    FieldStamp::At((seq, frame)) => {
                        value.time_of_change = Some(frame.local_time);
                        stamped = Some(match stamped {
                            Some(newest) if newest.0 > seq => newest,
                            _ => (seq, frame),
                        });
                    }
                    FieldStamp::Withhold => continue,
                }
            }
            kept.push(value);
        }
        item.list_of_values = kept;
    }
    // An explicit untimestamped selector governs its coordinate whether or not
    // it qualified this round, over any timestamped companion (§13.17.3.1.2.4).
    for (index, item) in items.iter_mut().enumerate() {
        let oid = item.monitored_object_identifier;
        for value in &mut item.list_of_values[start(index)..] {
            if untimed.contains(&(
                oid,
                Some(value.property_identifier),
                value.property_array_index,
            )) {
                value.time_of_change = None;
            }
        }
    }
    for (index, item) in items.iter_mut().enumerate() {
        item.list_of_values =
            collapse_repeats(std::mem::take(&mut item.list_of_values), start(index));
    }
    // An object whose history rows were all explicitly untimestamped, or
    // whose only current value was withheld.
    items.retain(|item| !item.list_of_values.is_empty());
    (items, stamped)
}

/// Drop each history row (the first `history` values) whose next row for the
/// same coordinate repeats it exactly, value and time: overlapping selectors
/// of one change, or a companion that did not change. A value that returns
/// after a different one is a distinct change and stays (A-B-A).
fn collapse_repeats(
    values: Vec<COVNotificationValue>,
    history: usize,
) -> Vec<COVNotificationValue> {
    let repeated: Vec<bool> = (0..values.len())
        .map(|at| {
            at < history
                && values[at + 1..]
                    .iter()
                    .find(|later| {
                        later.property_identifier == values[at].property_identifier
                            && later.property_array_index == values[at].property_array_index
                    })
                    .is_some_and(|next| *next == values[at])
        })
        .collect();
    values
        .into_iter()
        .zip(repeated)
        .filter_map(|(value, repeated)| (!repeated).then_some(value))
        .collect()
}
