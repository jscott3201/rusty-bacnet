//! Random WriteProperty calls against one object of every catalogue type
//! (#2180).
//!
//! Each case takes one object from [`representatives`] and runs a short
//! run of writes against it through the public `write_property` only: a
//! property picked from the object's metadata rows (or now and then one the
//! object lacks), an array index, a priority, and a value that is either
//! any application value, wrong datatypes included, or a variation of what
//! the property reads now, so writes of the right shape get through too.
//!
//! What it holds the object to:
//! - no write or read panics;
//! - reads change nothing: two views taken back to back are equal;
//! - a write that fails leaves the object as it was: every row read whole
//!   (and as a count, for an array), and the rows themselves, compare equal
//!   before and after (the `write_property` contract).
//!
//! A write that succeeds isn't checked further. Which other properties a
//! write may move (Status_Flags with Out_Of_Service, Number_Of_States with
//! State_Text, the priority array with Present_Value, a log's counts with
//! Enable, ...) differs by type and is the object tests' business.
//!
//! Values are compared by their `Debug` text, so a NaN matches itself.

use bacnet_objects::catalogue::representatives;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use proptest::prelude::*;

/// How many catalogue objects there are; the object index of a case is
/// taken modulo the real count.
const CATALOGUE_SPAN: usize = 256;

/// Everything the public read API shows of `object`: each row's whole
/// value, an array's count, and the rows themselves.
fn view(object: &dyn BACnetObject) -> Vec<String> {
    let rows = object.property_metadata();
    let mut view = vec![format!("{rows:?}")];
    for row in rows.iter() {
        let property = row.property_identifier;
        view.push(format!(
            "{property:?}: {:?}",
            object.read_property(property, None)
        ));
        if object.is_array_property(property) {
            view.push(format!(
                "{property:?}[0]: {:?}",
                object.read_property(property, Some(0))
            ));
        }
    }
    view
}

/// The first entry where two views differ, for the failure message.
fn first_difference(before: &[String], after: &[String]) -> String {
    before
        .iter()
        .zip(after)
        .find(|(before, after)| before != after)
        .map(|(before, after)| format!("before {before}\n after {after}"))
        .unwrap_or_else(|| format!("{} entries before, {} after", before.len(), after.len()))
}

/// Which property a write names.
#[derive(Debug, Clone)]
enum Target {
    /// The row at this index, modulo the row count.
    Row(usize),
    /// Any identifier, most likely one the object lacks.
    Any(u32),
}

/// What a write carries.
#[derive(Debug, Clone)]
enum Value {
    /// This value as it is.
    Fresh(PropertyValue),
    /// What the property reads now (the element, under an index), varied
    /// by this seed: see [`vary`].
    Vary(u64),
}

#[derive(Debug, Clone)]
struct Write {
    target: Target,
    array_index: Option<u32>,
    value: Value,
    priority: Option<u8>,
}

/// A small deterministic generator, so a variation is a function of its
/// seed and proptest can shrink the seed.
struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

/// Small, edge and arbitrary unsigned values.
fn mixed_unsigned(mix: &mut Mix) -> u64 {
    match mix.below(4) {
        0 => mix.below(4),
        1 => [255, 256, 65_535, 65_536, u64::from(u32::MAX), u64::MAX][mix.below(6) as usize],
        2 => mix.below(1_000),
        _ => mix.next(),
    }
}

/// `current` varied by `seed`: the same datatype with another value, an
/// element dropped from or added to a list, or now and then another
/// datatype altogether.
fn vary(current: PropertyValue, seed: u64) -> PropertyValue {
    let mut mix = Mix(seed);
    if mix.below(8) == 0 {
        return PropertyValue::Null;
    }
    match current {
        PropertyValue::Boolean(value) => PropertyValue::Boolean(!value),
        PropertyValue::Unsigned(_) => PropertyValue::Unsigned(mixed_unsigned(&mut mix)),
        PropertyValue::Signed(_) => PropertyValue::Signed(mixed_unsigned(&mut mix) as i32),
        PropertyValue::Enumerated(_) => PropertyValue::Enumerated(mixed_unsigned(&mut mix) as u32),
        PropertyValue::Real(value) => PropertyValue::Real(
            [
                value + 1.0,
                -value,
                0.0,
                -0.0,
                f32::NAN,
                f32::INFINITY,
                f32::MIN_POSITIVE / 2.0,
                f32::MAX,
            ][mix.below(8) as usize],
        ),
        PropertyValue::Double(value) => PropertyValue::Double(
            [value + 1.0, f64::NAN, f64::NEG_INFINITY, -0.0][mix.below(4) as usize],
        ),
        PropertyValue::CharacterString(text) => {
            PropertyValue::CharacterString(match mix.below(3) {
                0 => String::new(),
                1 => format!("{text}-{}", mix.below(100)),
                _ => "x".repeat(mix.below(600) as usize),
            })
        }
        PropertyValue::OctetString(mut octets) => {
            octets.push(mix.next() as u8);
            PropertyValue::OctetString(octets)
        }
        PropertyValue::BitString { data, .. } => PropertyValue::BitString {
            unused_bits: mix.below(9) as u8,
            data: data.iter().map(|_| mix.next() as u8).collect(),
        },
        PropertyValue::Date(_) => PropertyValue::Date(date(mix.next())),
        PropertyValue::Time(_) => PropertyValue::Time(time(mix.next())),
        PropertyValue::ObjectIdentifier(_) => PropertyValue::ObjectIdentifier(
            ObjectIdentifier::new(
                ObjectType::from_raw(mix.below(70) as u32),
                mix.below(4) as u32,
            )
            .expect("a small type and instance"),
        ),
        PropertyValue::List(mut items) => {
            match mix.below(4) {
                0 => {
                    items.pop();
                }
                1 if !items.is_empty() => {
                    let at = mix.below(items.len() as u64) as usize;
                    let item = items[at].clone();
                    items[at] = vary(item, mix.next());
                }
                2 => {
                    let item = items.last().cloned().unwrap_or(PropertyValue::Null);
                    items.push(vary(item, mix.next()));
                }
                _ => items.clear(),
            }
            PropertyValue::List(items)
        }
        PropertyValue::ApplicationData(mut octets) => {
            if !octets.is_empty() && mix.below(2) == 0 {
                let at = mix.below(octets.len() as u64) as usize;
                octets[at] ^= 1 << mix.below(8);
            } else {
                octets.truncate(mix.below(octets.len() as u64 + 1) as usize);
            }
            PropertyValue::ApplicationData(octets)
        }
        PropertyValue::Null => PropertyValue::Unsigned(mixed_unsigned(&mut mix)),
    }
}

fn date(bits: u64) -> Date {
    let [year, month, day, day_of_week, ..] = bits.to_le_bytes();
    Date {
        year,
        month: month % 16,
        day: day % 36,
        day_of_week: day_of_week % 9,
    }
}

fn time(bits: u64) -> Time {
    let [hour, minute, second, hundredths, ..] = bits.to_le_bytes();
    Time {
        hour: hour % 26,
        minute: minute % 62,
        second: second % 62,
        hundredths: hundredths % 102,
    }
}

/// Any primitive application value, edges drawn often.
fn primitive() -> impl Strategy<Value = PropertyValue> {
    prop_oneof![
        Just(PropertyValue::Null),
        any::<bool>().prop_map(PropertyValue::Boolean),
        prop_oneof![
            Just(0u64),
            Just(1),
            Just(u64::from(u32::MAX)),
            Just(u64::MAX),
            any::<u64>()
        ]
        .prop_map(PropertyValue::Unsigned),
        prop_oneof![
            Just(i32::MIN),
            Just(-1),
            Just(0),
            Just(i32::MAX),
            any::<i32>()
        ]
        .prop_map(PropertyValue::Signed),
        prop_oneof![
            Just(f32::NAN),
            Just(f32::NEG_INFINITY),
            Just(-0.0f32),
            Just(1.0e-40f32),
            any::<f32>()
        ]
        .prop_map(PropertyValue::Real),
        any::<f64>().prop_map(PropertyValue::Double),
        proptest::collection::vec(any::<u8>(), 0..24).prop_map(PropertyValue::OctetString),
        "[ -~]{0,40}".prop_map(PropertyValue::CharacterString),
        (0u8..10, proptest::collection::vec(any::<u8>(), 0..6))
            .prop_map(|(unused_bits, data)| PropertyValue::BitString { unused_bits, data }),
        prop_oneof![0u32..300, any::<u32>()].prop_map(PropertyValue::Enumerated),
        any::<u64>().prop_map(|bits| PropertyValue::Date(date(bits))),
        any::<u64>().prop_map(|bits| PropertyValue::Time(time(bits))),
        (0u32..1024, 0u32..=ObjectIdentifier::MAX_INSTANCE).prop_map(|(kind, instance)| {
            PropertyValue::ObjectIdentifier(
                ObjectIdentifier::new(ObjectType::from_raw(kind), instance)
                    .expect("a type under 1,024 and a 22-bit instance"),
            )
        }),
        proptest::collection::vec(any::<u8>(), 0..16).prop_map(PropertyValue::ApplicationData),
    ]
}

/// Any value: a primitive, or a list of up to six of them.
fn value() -> impl Strategy<Value = PropertyValue> {
    prop_oneof![
        4 => primitive(),
        1 => proptest::collection::vec(primitive(), 0..6).prop_map(PropertyValue::List),
    ]
}

fn write() -> impl Strategy<Value = Write> {
    (
        prop_oneof![
            9 => any::<usize>().prop_map(Target::Row),
            1 => prop_oneof![0u32..600, any::<u32>()].prop_map(Target::Any),
        ],
        prop_oneof![
            6 => Just(None),
            2 => Just(Some(0)),
            3 => (1u32..6).prop_map(Some),
            1 => Just(Some(u32::MAX)),
            1 => any::<u32>().prop_map(Some),
        ],
        prop_oneof![
            1 => value().prop_map(Value::Fresh),
            2 => any::<u64>().prop_map(Value::Vary),
        ],
        prop_oneof![
            4 => Just(None),
            3 => (1u8..=16).prop_map(Some),
            1 => any::<u8>().prop_map(Some),
        ],
    )
        .prop_map(|(target, array_index, value, priority)| Write {
            target,
            array_index,
            value,
            priority,
        })
}

/// Run `writes` against the catalogue object at `index`, checking each.
fn run(index: usize, writes: &[Write]) -> Result<(), TestCaseError> {
    let mut objects = representatives();
    let mut object = objects.swap_remove(index % objects.len());
    drop(objects);
    let name = format!("{:?}", object.object_identifier());
    for write in writes {
        let rows = object.property_metadata();
        let property = match write.target {
            Target::Row(row) => rows[row % rows.len()].property_identifier,
            Target::Any(raw) => PropertyIdentifier::from_raw(raw),
        };
        drop(rows);
        let value = match &write.value {
            Value::Fresh(value) => value.clone(),
            Value::Vary(seed) => {
                let current = object
                    .read_property(property, write.array_index)
                    .unwrap_or(PropertyValue::Null);
                vary(current, *seed)
            }
        };
        let before = view(object.as_ref());
        let again = view(object.as_ref());
        prop_assert!(
            before == again,
            "{name}: reading changed the object: {}",
            first_difference(&before, &again)
        );
        let outcome =
            object.write_property(property, write.array_index, value.clone(), write.priority);
        if let Err(error) = outcome {
            let after = view(object.as_ref());
            prop_assert!(
                before == after,
                "{name}: the refused write of {property:?}[{:?}] = {value:?} at {:?} ({error:?}) changed the object: {}",
                write.array_index,
                write.priority,
                first_difference(&before, &after)
            );
        }
    }
    Ok(())
}

proptest! {
    // About 16 runs per catalogue type; PROPTEST_CASES raises it.
    #![proptest_config(ProptestConfig::with_cases(1024))]

    #[test]
    fn refused_writes_leave_the_object_unchanged(
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
