//! Random WriteProperty calls against one object of every catalogue type
//! (#2180, #2234).
//!
//! Each case takes one object from [`representatives`] and runs a short
//! run of writes against it through the public `write_property` only: a
//! property picked from the object's metadata rows (mostly one that
//! `is_writable_property` names, now and then one the object lacks), an
//! array index, a priority, and a value that is either
//! any application value, wrong datatypes included, or a variation of what
//! the property reads now, so writes of the right shape get through too.
//!
//! What it holds the object to, comparing views (every row read whole, an
//! array's count, and the rows themselves):
//! - no write or read panics;
//! - reads change nothing: two views taken back to back are equal;
//! - a write that fails leaves the object as it was (the `write_property`
//!   contract);
//! - a write of P that succeeds changes only P and what
//!   [`may_also_change`] lists for P on that type (a change of the rows
//!   counts as Property_List). The table and why each entry is there are in
//!   `support/write_side_effects.rs`.
//!
//! Values are compared by their `Debug` text, so a NaN matches itself.

use bacnet_objects::averaging::AveragingObject;
use bacnet_objects::catalogue::representatives;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use proptest::prelude::*;

#[path = "support/write_proptest.rs"]
mod write_proptest;
#[path = "support/write_side_effects.rs"]
mod write_side_effects;

use write_proptest::{
    Attempt, CATALOGUE_SPAN, Write, check, pick, reads_change_nothing, type_of, value_of, view,
    write,
};
use write_side_effects::{effects, may_also_change};

/// The catalogue object at `index`, with the Averaging object holding two
/// samples so that the writes that reset its window show.
fn catalogue_object(index: usize) -> Box<dyn BACnetObject> {
    let mut objects = representatives();
    let object = objects.swap_remove(index % objects.len());
    if object.object_identifier().object_type() != ObjectType::AVERAGING {
        return object;
    }
    let mut seeded = AveragingObject::new(
        object.object_identifier().instance_number(),
        object.object_name(),
    )
    .expect("the catalogue's Averaging instance");
    for sample in [1.0, 3.0] {
        seeded.add_sample(sample).expect("a finite sample");
    }
    Box::new(seeded)
}

/// Run `writes` against the catalogue object at `index`, checking each.
fn run(index: usize, writes: &[Write]) -> Result<(), TestCaseError> {
    let mut object = catalogue_object(index);
    let name = format!("{:?}", object.object_identifier());
    let object_type = type_of(object.as_ref());
    for write in writes {
        let property = pick(object.as_ref(), &write.target);
        let value = value_of(object.as_ref(), property, write);
        let before = reads_change_nothing(&name, object.as_ref())?;
        let outcome =
            object.write_property(property, write.array_index, value.clone(), write.priority);
        let after = view(object.as_ref());
        let attempt = Attempt {
            name: &name,
            property,
            array_index: write.array_index,
            priority: write.priority,
            value: &value,
            outcome: &outcome,
        };
        check(
            &attempt,
            &before,
            &after,
            "support/write_side_effects.rs",
            |key| may_also_change(object_type, property).any(|listed| listed == key.property()),
        )?;
    }
    Ok(())
}

proptest! {
    // About 64 runs per catalogue type; PROPTEST_CASES changes it.
    #![proptest_config(ProptestConfig::with_cases(4096))]

    #[test]
    fn writes_change_only_what_they_may(
        index in 0..CATALOGUE_SPAN,
        writes in proptest::collection::vec(write(), 1..8),
    ) {
        run(index, &writes)?;
    }
}

/// Every catalogue object is reachable from a case's index.
#[test]
fn every_catalogue_type_is_reachable() {
    assert!(representatives().len() <= CATALOGUE_SPAN);
}

/// The side-effect table has a row for every catalogue type, and names only
/// properties the type's object has (Property_List stands for the rows).
#[test]
fn the_side_effect_table_covers_the_catalogue() {
    for object in representatives() {
        let object_type = object.object_identifier().object_type();
        let fragments = effects(object_type).unwrap_or_else(|| {
            panic!("{object_type:?} has no row in support/write_side_effects.rs")
        });
        let rows = object.property_metadata();
        let has = |property: PropertyIdentifier| {
            rows.iter().any(|row| row.property_identifier == property)
        };
        for &(written, changed) in fragments.iter().flat_map(|fragment| fragment.iter()) {
            assert!(
                has(written) && object.is_writable_property(written),
                "{object_type:?}: the table lists a write of {written:?}, which isn't a writable row"
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
