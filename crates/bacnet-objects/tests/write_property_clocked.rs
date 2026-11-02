//! The WriteProperty proptest again, with clocks bound and state seeded
//! (#2262).
//!
//! `write_property.rs` runs the catalogue as `representatives()` builds it.
//! Here each object sits in a database that binds a Device clock and a
//! monotonic clock, with the seeds `support/write_clocks.rs` lists (log
//! records, Command actions, a BBMD Network Port, a raised event, Averaging
//! samples and Present_Value commands from a local origin). Three writes in
//! four go through `write_property_from` with that origin, the rest through
//! `write_property`. The clocks move one step before each write.
//!
//! The same contract holds: a refused write changes nothing, and an
//! accepted write of P changes only P, what `write_side_effects.rs` lists,
//! and what `write_side_effects_clocked.rs` adds. A stamp that table lists
//! may move only to the current step's date and time.
//!
//! [`every_writable_row_succeeds`] is the coverage half: a fixed,
//! deterministic pass over every writable row of every catalogue object,
//! held to the same tables, that names each row no write reached.

use bacnet_objects::catalogue::representatives;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, PropertyValue, Time};
use proptest::prelude::*;

#[path = "support/write_clocks.rs"]
mod write_clocks;
#[path = "support/write_proptest.rs"]
mod write_proptest;
#[path = "support/write_side_effects.rs"]
mod write_side_effects;
#[path = "support/write_side_effects_clocked.rs"]
mod write_side_effects_clocked;

use write_clocks::{Candidate, DATE, Fixture, origin};
use write_proptest::{
    Attempt, CATALOGUE_SPAN, Key, Write, check, pick, reads_change_nothing, type_of, value_of,
    vary, view, write,
};
use write_side_effects_clocked::{clocked_effects, may_also_change_clocked, stamps};

const TABLE: &str = "support/write_side_effects_clocked.rs";

/// Whether `value` holds `date` and `time`: as values, or as their
/// application-tagged encodings inside encoded data.
fn holds_stamp(value: &PropertyValue, date: Date, time: Time) -> bool {
    fn has(value: &PropertyValue, wanted: &PropertyValue, tagged: &[u8]) -> bool {
        match value {
            PropertyValue::List(items) => items.iter().any(|item| has(item, wanted, tagged)),
            PropertyValue::ApplicationData(octets) => {
                octets.windows(tagged.len()).any(|window| window == tagged)
            }
            other => other == wanted,
        }
    }
    let date_tagged = [0xA4, date.year, date.month, date.day, date.day_of_week];
    let time_tagged = [0xB4, time.hour, time.minute, time.second, time.hundredths];
    has(value, &PropertyValue::Date(date), &date_tagged)
        && has(value, &PropertyValue::Time(time), &time_tagged)
}

/// Whether the change of `key` that a write of `property` made is one the
/// tables allow: a listed side effect, or a listed stamp now reading the
/// current step.
fn allowed(object: &dyn BACnetObject, property: PropertyIdentifier, key: Key, time: Time) -> bool {
    let object_type = type_of(object);
    let other = key.property();
    if may_also_change_clocked(object_type, property, other) {
        return true;
    }
    let stamp = stamps(object_type)
        .iter()
        .any(|(written, moved)| *written == property && moved.contains(&other));
    stamp
        && matches!(key, Key::Whole(_))
        && object
            .read_property(other, None)
            .is_ok_and(|value| holds_stamp(&value, DATE, time))
}

/// Make one write against the fixture, through `write_property_from` with
/// [`origin`] when `from` is set, and check it.
fn attempt(
    fixture: &mut Fixture,
    name: &str,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    value: PropertyValue,
    priority: Option<u8>,
    from: bool,
) -> Result<Result<(), Error>, TestCaseError> {
    let time = fixture.time();
    let object = fixture.object();
    let before = reads_change_nothing(name, object)?;
    let outcome = if from {
        object.write_property_from(property, array_index, value.clone(), priority, &origin())
    } else {
        object.write_property(property, array_index, value.clone(), priority)
    };
    let after = view(object);
    let attempt = Attempt {
        name,
        property,
        array_index,
        priority,
        value: &value,
        outcome: &outcome,
    };
    check(&attempt, &before, &after, TABLE, |key| {
        allowed(object, property, key, time)
    })?;
    Ok(outcome)
}

/// Run `writes` against the seeded catalogue object at `index`.
fn run(index: usize, writes: &[(Write, bool)]) -> Result<(), TestCaseError> {
    let mut fixture = Fixture::new(index);
    let name = format!("{:?}", fixture.object().object_identifier());
    for (write, from) in writes {
        fixture.step();
        let object = fixture.object();
        let property = pick(object, &write.target);
        let value = value_of(object, property, write);
        // Refused or taken, `attempt` has checked it.
        let _outcome = attempt(
            &mut fixture,
            &name,
            property,
            write.array_index,
            value,
            write.priority,
            *from,
        )?;
    }
    Ok(())
}

proptest! {
    // About 64 runs per catalogue type; PROPTEST_CASES changes it.
    #![proptest_config(ProptestConfig::with_cases(4096))]

    #[test]
    fn clocked_writes_change_only_what_they_may(
        index in 0..CATALOGUE_SPAN,
        writes in proptest::collection::vec((write(), prop_oneof![3 => Just(true), 1 => Just(false)]), 1..8),
    ) {
        run(index, &writes)?;
    }
}

/// The writes the targeted pass tries for `property`, in order: what it
/// reads now (whole, at priorities 8 and 16, and once through
/// `write_property`), variations of that, the same for its first element,
/// and the row's [`write_clocks::extra_candidates`].
fn candidates(object: &dyn BACnetObject, property: PropertyIdentifier) -> Vec<Candidate> {
    let mut out = Vec::new();
    let current = object
        .read_property(property, None)
        .unwrap_or(PropertyValue::Null);
    for priority in [None, Some(8), Some(16)] {
        out.push((None, current.clone(), priority, true));
    }
    out.push((None, current.clone(), None, false));
    for seed in 0..48 {
        out.push((None, vary(current.clone(), seed), None, true));
    }
    if object.is_array_property(property)
        && let Ok(first) = object.read_property(property, Some(1))
    {
        out.push((Some(1), first.clone(), None, true));
        for seed in 0..16 {
            out.push((Some(1), vary(first.clone(), seed), None, true));
        }
    }
    out.extend(write_clocks::extra_candidates(type_of(object), property));
    out
}

/// Try every candidate for `property` on a fresh fixture of the object at
/// `index`, after the row's [`write_clocks::prerequisite`] when `prepared`
/// is set; whether one was accepted. Every attempt is held to the tables.
fn sweep_row(index: usize, property: PropertyIdentifier, prepared: bool) -> bool {
    let mut fixture = Fixture::new(index);
    let name = format!("{:?}", fixture.object().object_identifier());
    let write = |fixture: &mut Fixture, property, array_index, value, priority, from| {
        fixture.step();
        attempt(fixture, &name, property, array_index, value, priority, from)
            .unwrap_or_else(|failure| panic!("{failure}"))
            .is_ok()
    };
    let object_type = type_of(fixture.object());
    let (first, value) = write_clocks::prerequisite(object_type, property);
    if prepared && !write(&mut fixture, first, None, value, None, true) {
        return false;
    }
    let mut reached = false;
    for (array_index, value, priority, from) in candidates(fixture.object(), property) {
        reached |= write(&mut fixture, property, array_index, value, priority, from);
    }
    reached
}

/// Every row the catalogue marks writable takes at least one write in this
/// setup: a targeted pass tries [`candidates`] on a fresh fixture per row,
/// each held to the tables, and lists the rows none of them reached.
#[test]
fn every_writable_row_succeeds() {
    let mut unreached = Vec::new();
    for index in 0..representatives().len() {
        let rows: Vec<PropertyIdentifier> = {
            let mut fixture = Fixture::new(index);
            let object = fixture.object();
            object
                .property_metadata()
                .iter()
                .map(|row| row.property_identifier)
                .filter(|&property| object.is_writable_property(property))
                .collect()
        };
        for property in rows {
            // As seeded, then after the row's prerequisite write.
            let reached = sweep_row(index, property, false) || sweep_row(index, property, true);
            if !reached {
                let object_type = Fixture::new(index)
                    .object()
                    .object_identifier()
                    .object_type();
                unreached.push(format!("{object_type:?} {property:?}"));
            }
        }
    }
    assert!(
        unreached.is_empty(),
        "{} writable rows never took a write:\n{}",
        unreached.len(),
        unreached.join("\n")
    );
}

/// The clocked table names only rows each type's catalogue object has: a
/// written property that is writable, and other rows it may change.
#[test]
fn the_clocked_table_names_real_rows() {
    for index in 0..representatives().len() {
        let mut fixture = Fixture::new(index);
        let object = fixture.object();
        let object_type = type_of(object);
        let rows = object.property_metadata();
        let has = |property: PropertyIdentifier| {
            rows.iter().any(|row| row.property_identifier == property)
        };
        let entries = clocked_effects(object_type)
            .iter()
            .flat_map(|fragment| fragment.iter())
            .chain(stamps(object_type));
        for &(written, changed) in entries {
            assert!(
                has(written) && object.is_writable_property(written),
                "{object_type:?}: {TABLE} lists a write of {written:?}, which isn't a writable row"
            );
            for &other in changed {
                assert!(
                    other != written && has(other),
                    "{object_type:?}: a write of {written:?} lists {other:?}, which isn't another row"
                );
            }
        }
    }
}
