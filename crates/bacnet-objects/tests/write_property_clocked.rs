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
//! [`every_writable_row_and_table_entry_is_reached`] is the coverage half:
//! a fixed, deterministic pass over every writable row of every catalogue
//! object, and the scenarios in `support/write_scenarios.rs`, held to the
//! same tables. It fails on a writable row no write changed (apart from the
//! listed identity-only rows) and on a clocked table entry no write used.

use bacnet_objects::property_metadata::{is_array_property, is_writable_property};
use std::cell::RefCell;
use std::collections::BTreeSet;

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
#[path = "support/write_scenarios.rs"]
mod write_scenarios;
#[path = "support/write_side_effects.rs"]
mod write_side_effects;
#[path = "support/write_side_effects_clocked.rs"]
mod write_side_effects_clocked;
#[path = "support/write_values.rs"]
mod write_values;

use write_clocks::{Candidate, DATE, Fixture, origin};
use write_proptest::{
    Attempt, CATALOGUE_SPAN, Key, Write, check, pick, reads_change_nothing, type_of, value_of,
    vary, view, write,
};
use write_scenarios::{IDENTITY_ONLY, SCENARIOS, Step, UNREACHABLE};
use write_side_effects_clocked::{entries, listed, routed_effects, stamps};

const TABLE: &str = "support/write_side_effects_clocked.rs";

/// One entry of the clocked table: object type, written property and the
/// property it moved, as raw numbers.
type Used = (u32, u32, u32);

thread_local! {
    /// The clocked table's entries the writes on this thread have used.
    static USED: RefCell<BTreeSet<Used>> = const { RefCell::new(BTreeSet::new()) };
}

/// Whether `value` holds `date` and `time`, as values.
fn holds_stamp(value: &PropertyValue, date: Date, time: Time) -> bool {
    fn has(value: &PropertyValue, wanted: &PropertyValue) -> bool {
        match value {
            PropertyValue::List(items) => items.iter().any(|item| has(item, wanted)),
            other => other == wanted,
        }
    }
    has(value, &PropertyValue::Date(date)) && has(value, &PropertyValue::Time(time))
}

/// Whether the change of `key` that a write of `property` made is one the
/// tables allow: an entry of the clock-free table, an entry of the clocked
/// table (its routed entries only through `write_property_from`), or a
/// listed stamp now reading the current step. Each clocked entry used is
/// noted in [`USED`].
fn allowed(
    object: &dyn BACnetObject,
    property: PropertyIdentifier,
    key: Key,
    time: Time,
    from: bool,
) -> bool {
    let object_type = type_of(object);
    let other = key.property();
    if write_side_effects::may_also_change(object_type, property).any(|listed| listed == other) {
        return true;
    }
    let stamp = stamps(object_type)
        .iter()
        .any(|(written, moved)| *written == property && moved.contains(&other))
        && matches!(key, Key::Whole(_))
        && object
            .read_property(other, None)
            .is_ok_and(|value| holds_stamp(&value, DATE, time));
    let ok = stamp || listed(object_type, property, other, from);
    if ok {
        let used = (object_type.to_raw(), property.to_raw(), other.to_raw());
        USED.with(|set| set.borrow_mut().insert(used));
    }
    ok
}

/// Make one write against the fixture, through `write_property_from` with
/// [`origin`] when `from` is set, and check it. What the write returned,
/// and whether the view changed.
fn attempt(
    fixture: &mut Fixture,
    name: &str,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    value: PropertyValue,
    priority: Option<u8>,
    from: bool,
) -> Result<(Result<(), Error>, bool), TestCaseError> {
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
        allowed(object, property, key, time, from)
    })?;
    let changed = before != after;
    Ok((outcome, changed))
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
        let _checked = attempt(
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
    if is_array_property(object, property)
        && let Ok(first) = object.read_property(property, Some(1))
    {
        out.push((Some(1), first.clone(), None, true));
        for seed in 0..16 {
            out.push((Some(1), vary(first.clone(), seed), None, true));
        }
    }
    out.extend(write_clocks::extra_candidates(type_of(object), property));
    // Last, at 4, a slot nothing has commanded, so its source is new.
    if property == PropertyIdentifier::PRESENT_VALUE {
        out.push((None, current, Some(4), true));
    }
    // A row with routed entries is tried through plain `write_property`
    // too, which may move none of them.
    let object_type = type_of(object);
    let routed = routed_effects(object_type)
        .iter()
        .flat_map(|fragment| fragment.iter())
        .any(|(written, _)| *written == property);
    if routed {
        let plain: Vec<Candidate> = out
            .iter()
            .map(|(index, value, priority, _)| (*index, value.clone(), *priority, false))
            .collect();
        out.extend(plain);
    }
    out
}

/// How far a row's candidates got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Reach {
    /// No candidate was accepted.
    Refused,
    /// Some were accepted, but none changed the view.
    Identity,
    /// An accepted candidate changed the view.
    Changed,
}

/// Make one write on `fixture` after a step, held to the tables; whether it
/// was accepted, and whether the view changed.
fn step_and_write(fixture: &mut Fixture, name: &str, write: &Step) -> (bool, bool) {
    fixture.wait(1 + write.wait);
    let (outcome, changed) = attempt(
        fixture,
        name,
        write.property,
        write.array_index,
        write.value.clone(),
        write.priority,
        write.from,
    )
    .unwrap_or_else(|failure| panic!("{failure}"));
    (outcome.is_ok(), changed)
}

/// Try every candidate for `property` on a fresh fixture of the object at
/// `index`, after the row's [`write_clocks::prerequisite`] when `prepared`
/// is set. Every attempt is held to the tables.
fn sweep_row(index: usize, property: PropertyIdentifier, prepared: bool) -> Reach {
    let mut fixture = Fixture::new(index);
    let name = format!("{:?}", fixture.object().object_identifier());
    let object_type = type_of(fixture.object());
    if prepared {
        let (first, value) = write_clocks::prerequisite(object_type, property);
        if !step_and_write(&mut fixture, &name, &Step::new(first, value)).0 {
            return Reach::Refused;
        }
    }
    let mut reach = Reach::Refused;
    for (array_index, value, priority, from) in candidates(fixture.object(), property) {
        let step = Step {
            array_index,
            priority,
            from,
            ..Step::new(property, value)
        };
        reach = reach.max(match step_and_write(&mut fixture, &name, &step) {
            (true, true) => Reach::Changed,
            (true, false) => Reach::Identity,
            (false, _) => Reach::Refused,
        });
    }
    reach
}

/// The deterministic coverage pass, held to the same tables:
/// - every row the catalogue marks writable takes a write that changes the
///   object (as seeded, or after the row's prerequisite), except the rows
///   in [`IDENTITY_ONLY`], which take only the value they hold;
/// - with [`SCENARIOS`] run too, every entry of the clocked table is used,
///   except those in [`UNREACHABLE`].
#[test]
fn every_writable_row_and_table_entry_is_reached() {
    USED.with(|set| set.borrow_mut().clear());
    let mut failures = Vec::new();
    // The scenarios first: a row a scenario changes counts as changed.
    let mut changed_by_scenario = BTreeSet::new();
    for scenario in SCENARIOS() {
        let mut fixture = Fixture::of(scenario.object_type);
        let name = format!("{:?}", fixture.object().object_identifier());
        for (at, step) in scenario.steps.iter().enumerate() {
            let (taken, changed) = step_and_write(&mut fixture, &name, step);
            if !taken {
                failures.push(format!(
                    "scenario {:?} on {:?}: step {at} ({:?}) was refused",
                    scenario.name, scenario.object_type, step.property
                ));
            }
            if changed {
                changed_by_scenario.insert((scenario.object_type.to_raw(), step.property.to_raw()));
            }
        }
    }
    for index in 0..representatives().len() {
        let mut fixture = Fixture::new(index);
        let object_type = type_of(fixture.object());
        let rows: Vec<PropertyIdentifier> = {
            let object = fixture.object();
            object
                .property_metadata()
                .iter()
                .map(|row| row.property_identifier)
                .filter(|&property| is_writable_property(&*object, property))
                .collect()
        };
        for property in rows {
            let mut reach = sweep_row(index, property, false);
            if changed_by_scenario.contains(&(object_type.to_raw(), property.to_raw())) {
                reach = Reach::Changed;
            }
            if reach != Reach::Changed {
                reach = reach.max(sweep_row(index, property, true));
            }
            let identity = IDENTITY_ONLY
                .iter()
                .any(|&(t, p, _)| t == object_type && p == property);
            match (reach, identity) {
                (Reach::Changed, false) | (Reach::Identity, true) => {}
                (Reach::Changed, true) => failures.push(format!(
                    "{object_type:?} {property:?} changes now: drop it from IDENTITY_ONLY"
                )),
                (Reach::Identity, false) => failures.push(format!(
                    "{object_type:?} {property:?} took only writes that changed nothing"
                )),
                (Reach::Refused, _) => {
                    failures.push(format!("{object_type:?} {property:?} took no write"))
                }
            }
        }
    }
    let used = USED.with(|set| set.borrow().clone());
    for object in representatives() {
        let object_type = type_of(object.as_ref());
        for &(written, moved) in entries(object_type, true, true) {
            for &other in moved {
                let entry = (object_type.to_raw(), written.to_raw(), other.to_raw());
                let exempt = UNREACHABLE
                    .iter()
                    .any(|&(t, w, o, _)| t == object_type && w == written && o == other);
                match (used.contains(&entry), exempt) {
                    (false, false) => failures.push(format!(
                        "{object_type:?}: no write of {written:?} moved {other:?}"
                    )),
                    (true, true) => failures.push(format!(
                        "{object_type:?}: {written:?} -> {other:?} is used: drop it from UNREACHABLE"
                    )),
                    _ => {}
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} coverage failures:\n{}",
        failures.len(),
        failures.join("\n")
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
        for &(written, changed) in entries(object_type, true, true) {
            assert!(
                has(written) && is_writable_property(&*object, written),
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
