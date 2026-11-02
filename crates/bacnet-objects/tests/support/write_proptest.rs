//! The machinery the WriteProperty proptests share (#2234, #2262): views of
//! an object, the write strategies, and the check of one write against a
//! side-effect table.
//!
//! `tests/write_property.rs` runs it over the catalogue as it is built, and
//! `tests/write_property_clocked.rs` over the catalogue with clocks bound
//! and state seeded.

use bacnet_objects::property_metadata::{is_array_property, is_writable_property};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use proptest::prelude::*;

/// How many catalogue objects there are; the object index of a case is
/// taken modulo the real count.
pub const CATALOGUE_SPAN: usize = 256;

/// Which part of a view an entry shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    /// The metadata rows themselves.
    Rows,
    /// A property read whole (raw identifier).
    Whole(u32),
    /// An array property's count (raw identifier).
    Count(u32),
}

impl Key {
    /// The property this entry belongs to: the rows count as Property_List.
    pub fn property(self) -> PropertyIdentifier {
        match self {
            Key::Rows => PropertyIdentifier::PROPERTY_LIST,
            Key::Whole(raw) | Key::Count(raw) => PropertyIdentifier::from_raw(raw),
        }
    }
}

pub type View = std::collections::BTreeMap<Key, String>;

/// Everything the public read API shows of `object`: each row's whole
/// value, an array's count, and the rows themselves.
pub fn view(object: &dyn BACnetObject) -> View {
    let rows = object.property_metadata();
    let mut view = View::new();
    view.insert(Key::Rows, format!("{rows:?}"));
    for row in rows.iter() {
        let property = row.property_identifier;
        view.insert(
            Key::Whole(property.to_raw()),
            format!("{:?}", object.read_property(property, None)),
        );
        if is_array_property(object, property) {
            view.insert(
                Key::Count(property.to_raw()),
                format!("{:?}", object.read_property(property, Some(0))),
            );
        }
    }
    view
}

/// The keys whose entries differ between two views, including entries
/// only one of them has.
pub fn changed(before: &View, after: &View) -> Vec<Key> {
    let mut keys: Vec<Key> = before.keys().chain(after.keys()).copied().collect();
    keys.sort();
    keys.dedup();
    keys.retain(|key| before.get(key) != after.get(key));
    keys
}

/// The first entry where two views differ, for the failure message.
pub fn first_difference(before: &View, after: &View) -> String {
    match changed(before, after).first() {
        Some(key) => format!(
            "before {key:?} {:?}\n after {key:?} {:?}",
            before.get(key),
            after.get(key)
        ),
        None => "no difference".into(),
    }
}

/// Which property a write names.
#[derive(Debug, Clone)]
pub enum Target {
    /// The writable row at this index, modulo their count (any row when
    /// none is writable), so most writes can succeed.
    Writable(usize),
    /// The row at this index, modulo the row count.
    Row(usize),
    /// Any identifier, most likely one the object lacks.
    Any(u32),
}

/// What a write carries.
#[derive(Debug, Clone)]
pub enum Value {
    /// This value as it is.
    Fresh(PropertyValue),
    /// What the property reads now (the element, under an index), varied
    /// by this seed: see [`vary`].
    Vary(u64),
}

#[derive(Debug, Clone)]
pub struct Write {
    pub target: Target,
    pub array_index: Option<u32>,
    pub value: Value,
    pub priority: Option<u8>,
}

/// A small deterministic generator, so a variation is a function of its
/// seed and proptest can shrink the seed.
pub struct Mix(pub u64);

impl Mix {
    pub fn next(&mut self) -> u64 {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

/// Small, edge and arbitrary unsigned values.
pub fn mixed_unsigned(mix: &mut Mix) -> u64 {
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
pub fn vary(current: PropertyValue, seed: u64) -> PropertyValue {
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

pub fn date(bits: u64) -> Date {
    let [year, month, day, day_of_week, ..] = bits.to_le_bytes();
    Date {
        year,
        month: month % 16,
        day: day % 36,
        day_of_week: day_of_week % 9,
    }
}

pub fn time(bits: u64) -> Time {
    let [hour, minute, second, hundredths, ..] = bits.to_le_bytes();
    Time {
        hour: hour % 26,
        minute: minute % 62,
        second: second % 62,
        hundredths: hundredths % 102,
    }
}

/// Any primitive application value, edges drawn often.
pub fn primitive() -> impl Strategy<Value = PropertyValue> {
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
pub fn value() -> impl Strategy<Value = PropertyValue> {
    prop_oneof![
        4 => primitive(),
        1 => proptest::collection::vec(primitive(), 0..6).prop_map(PropertyValue::List),
    ]
}

pub fn write() -> impl Strategy<Value = Write> {
    (
        prop_oneof![
            7 => any::<usize>().prop_map(Target::Writable),
            2 => any::<usize>().prop_map(Target::Row),
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

/// The property `target` names on `object`.
pub fn pick(object: &dyn BACnetObject, target: &Target) -> PropertyIdentifier {
    let rows = object.property_metadata();
    match *target {
        Target::Writable(row) => {
            let writable: Vec<PropertyIdentifier> = rows
                .iter()
                .map(|row| row.property_identifier)
                .filter(|&property| is_writable_property(object, property))
                .collect();
            match writable.len() {
                0 => rows[row % rows.len()].property_identifier,
                count => writable[row % count],
            }
        }
        Target::Row(row) => rows[row % rows.len()].property_identifier,
        Target::Any(raw) => PropertyIdentifier::from_raw(raw),
    }
}

/// The value `write` carries for `property` on `object`.
pub fn value_of(
    object: &dyn BACnetObject,
    property: PropertyIdentifier,
    write: &Write,
) -> PropertyValue {
    match &write.value {
        Value::Fresh(value) => value.clone(),
        Value::Vary(seed) => {
            let current = object
                .read_property(property, write.array_index)
                .unwrap_or(PropertyValue::Null);
            vary(current, *seed)
        }
    }
}

/// One write of `value` to `property` and what came of it, for [`check`].
pub struct Attempt<'a> {
    /// The object's name, for messages.
    pub name: &'a str,
    /// The property written.
    pub property: PropertyIdentifier,
    /// The array index and priority the write carried.
    pub array_index: Option<u32>,
    /// The write's priority.
    pub priority: Option<u8>,
    /// The value written.
    pub value: &'a PropertyValue,
    /// What the write returned.
    pub outcome: &'a Result<(), Error>,
}

/// Hold one write to the `write_property` contract and the side-effect
/// table: a refused write changes nothing, and an accepted write of P
/// changes only P and what `allowed` admits of each other changed key;
/// `table` names the table file in the failure message.
pub fn check(
    attempt: &Attempt<'_>,
    before: &View,
    after: &View,
    table: &str,
    allowed: impl Fn(Key) -> bool,
) -> Result<(), TestCaseError> {
    let Attempt {
        name,
        property,
        array_index,
        priority,
        value,
        outcome,
    } = attempt;
    if let Err(error) = outcome {
        prop_assert!(
            before == after,
            "{name}: the refused write of {property:?}[{array_index:?}] = {value:?} at {priority:?} ({error:?}) changed the object: {}",
            first_difference(before, after)
        );
        return Ok(());
    }
    let mut undeclared = Vec::new();
    for key in changed(before, after) {
        let other = key.property();
        if other != *property && !allowed(key) {
            undeclared.push(format!(
                "{other:?}{}: {} -> {}",
                if matches!(key, Key::Count(_)) {
                    "[0]"
                } else {
                    ""
                },
                before.get(&key).map_or("absent", String::as_str),
                after.get(&key).map_or("absent", String::as_str),
            ));
        }
    }
    prop_assert!(
        undeclared.is_empty(),
        "{name}: the write of {property:?}[{array_index:?}] = {value:?} at {priority:?} also changed what {table} doesn't list:\n{}",
        undeclared.join("\n")
    );
    Ok(())
}

/// Two views taken back to back must match: reading changes nothing.
pub fn reads_change_nothing(name: &str, object: &dyn BACnetObject) -> Result<View, TestCaseError> {
    let before = view(object);
    let again = view(object);
    prop_assert!(
        before == again,
        "{name}: reading changed the object: {}",
        first_difference(&before, &again)
    );
    Ok(before)
}

/// The object's type, for table lookups.
pub fn type_of(object: &dyn BACnetObject) -> ObjectType {
    object.object_identifier().object_type()
}
