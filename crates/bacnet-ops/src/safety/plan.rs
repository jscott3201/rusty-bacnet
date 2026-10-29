//! A requested write turned into a preview, from the properties read just
//! before it: the value to send, the pre-image, the slot's occupancy, what
//! Present_Value will do, and the confirmation tier. Pure: no I/O.
//!
//! Tiers (`docs/design/tui.md` section 5):
//!
//! | Change | Tier |
//! |---|---|
//! | priority 9 to 16, or a non-commandable property | T1 |
//! | priorities 3, 4, 7 and 8; Out_Of_Service; relinquishing another commander's slot; a batch | T2 |
//! | priorities 1, 2 and 5 (life safety and critical equipment, Table 19-1) | T3 |
//! | priority 6, kept for minimum on and off times (Clause 19.2.3) | blocked, or T3 with `dangerous = true` |

use crate::format::{Floats, RawRow, StateTexts, ValueFormat, decode_all};
use crate::label::PRIORITIES;
use bacnet_encoding::primitives::{decode_priority_date_time, encode_property_value};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use super::{Effect, Gate, Occupancy, Planned, Preview, Stored, Tier, WriteSpec};

/// What is read before a write, besides the target property.
pub const PRE_READ: [PropertyIdentifier; 4] = [
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::PRIORITY_ARRAY,
    PropertyIdentifier::RELINQUISH_DEFAULT,
    PropertyIdentifier::OUT_OF_SERVICE,
];

/// What was read before one write: the [`PRE_READ`] properties (and the
/// whole target property, for a write without an array index) as rows,
/// and for an indexed write the element itself, or why it couldn't be.
#[derive(Debug, Clone, Default)]
pub struct PreRead {
    /// The properties read by name.
    pub rows: Vec<RawRow>,
    /// The element an indexed write targets, as read just before it.
    /// `None` for a write without an index.
    pub element: Option<Result<Vec<u8>, String>>,
}

impl From<Vec<RawRow>> for PreRead {
    fn from(rows: Vec<RawRow>) -> Self {
        Self {
            rows,
            element: None,
        }
    }
}

/// Why a write to an array's length (index 0) has no pre-image.
pub const RESIZE: &str = "a resize can't be reverted";

/// The priority a commandable write without one gets (Clause 19.2.1).
pub const DEFAULT_PRIORITY: u8 = 16;

/// Why priority 6 is refused.
pub const PRIORITY_6: &str = "priority 6 is reserved for minimum on and off times \
     (Clause 19.2.3); a profile with dangerous = true allows it";

/// The tier of one write.
pub fn gate(
    property: PropertyIdentifier,
    priority: Option<u8>,
    relinquish: bool,
    occupancy: Occupancy,
    dangerous: bool,
) -> Gate {
    Gate::Allowed(match priority {
        Some(6) if !dangerous => return Gate::Blocked(PRIORITY_6.into()),
        Some(1 | 2 | 5 | 6) => Tier::T3,
        Some(3 | 4 | 7 | 8) => Tier::T2,
        Some(_) if relinquish && occupancy == Occupancy::Other => Tier::T2,
        Some(_) => Tier::T1,
        None if property == PropertyIdentifier::OUT_OF_SERVICE => Tier::T2,
        None => Tier::T1,
    })
}

/// The tier of a whole change: its strongest write's, at least T2 for a
/// batch; blocked when any write is.
pub fn overall(items: &[Planned], batch: bool) -> Gate {
    let mut tier = if batch { Tier::T2 } else { Tier::T1 };
    for item in items {
        match &item.gate {
            Gate::Blocked(why) => return Gate::Blocked(format!("{}: {why}", item.label())),
            Gate::Allowed(t) => tier = tier.max(*t),
        }
    }
    Gate::Allowed(tier)
}

/// The first (object, property, index) a batch names twice. A
/// WritePropertyMultiple error names its first failed write by that
/// triple alone, with no priority, so a repeat would make the failed write
/// ambiguous and could leave an applied one untracked.
pub fn repeated_target(specs: &[&WriteSpec]) -> Option<String> {
    specs.iter().enumerate().find_map(|(i, spec)| {
        specs[..i]
            .iter()
            .any(|earlier| {
                (earlier.object, earlier.property, earlier.index)
                    == (spec.object, spec.property, spec.index)
            })
            .then(|| {
                format!(
                    "a batch writes each property once; {} {} appears twice",
                    crate::label::object_label(spec.object),
                    crate::label::kebab(spec.property)
                )
            })
    })
}

/// Build the preview of `specs` from `reads`, one per spec. `ours` says
/// whether the ledger wrote what a slot holds now (its encoded value).
pub fn preview(
    specs: &[WriteSpec],
    reads: &[PreRead],
    ours: impl Fn(&WriteSpec, Option<u8>, &[u8]) -> bool,
    dangerous: bool,
    batch: bool,
) -> Result<Preview, String> {
    let device = specs.first().ok_or("nothing to write")?.device;
    if let Some(why) = repeated_target(&specs.iter().collect::<Vec<_>>()) {
        return Err(why);
    }
    let items = specs
        .iter()
        .zip(reads)
        .map(|(spec, read)| plan(spec, read, &ours, dangerous))
        .collect::<Result<Vec<_>, _>>()?;
    let gate = overall(&items, batch);
    Ok(Preview {
        device,
        items,
        batch,
        gate,
    })
}

fn raw(rows: &[RawRow], property: PropertyIdentifier) -> Option<&[u8]> {
    rows.iter()
        .find(|(p, _)| *p == property)
        .and_then(|(_, value)| value.as_deref().ok())
}

fn first(data: &[u8]) -> Option<PropertyValue> {
    decode_all(data)?.into_iter().next()
}

/// A priority slot as the value a Present_Value write carries. A DateTime
/// Value's set slot is the BACnetPriorityValue date-time choice under
/// context tag 1 (#2098); its Present_Value is the bare Date and Time, which
/// is what a revert writes back and what the ledger compares.
fn slot_value(slot: PropertyValue) -> PropertyValue {
    if let PropertyValue::ApplicationData(octets) = &slot
        && let Ok(((date, time), end)) = decode_priority_date_time(octets, 0)
        && end == octets.len()
    {
        return PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)]);
    }
    slot
}

/// The value a write's octets carry, as a priority slot holds it: one
/// application value, or a BACnetDateTime's Date and Time together.
fn written(data: &[u8]) -> PropertyValue {
    match decode_all(data) {
        Some(values) if values.len() == 1 => values.into_iter().next().expect("one value"),
        Some(values) if !values.is_empty() => PropertyValue::List(values),
        _ => PropertyValue::Null,
    }
}

/// Give a typed number the datatype the property already holds:
/// `parse_value_for` reads `42` as Unsigned, which an analog Present_Value
/// would refuse as the wrong datatype.
pub fn coerce(value: PropertyValue, like: Option<&PropertyValue>) -> PropertyValue {
    use PropertyValue as V;
    match (like, value) {
        (Some(V::Real(_)), V::Unsigned(n)) => V::Real(n as f32),
        (Some(V::Real(_)), V::Signed(n)) => V::Real(n as f32),
        (Some(V::Double(_)), V::Unsigned(n)) => V::Double(n as f64),
        (Some(V::Double(_)), V::Signed(n)) => V::Double(f64::from(n)),
        (Some(V::Double(_)), V::Real(n)) => V::Double(f64::from(n)),
        (Some(V::Enumerated(_)), V::Unsigned(n)) if n <= u64::from(u32::MAX) => {
            V::Enumerated(n as u32)
        }
        (Some(V::Signed(_)), V::Unsigned(n)) if n <= i32::MAX as u64 => V::Signed(n as i32),
        (_, value) => value,
    }
}

/// `value` encoded, with its text. Previews keep every digit of a float
/// ([`Floats::Exact`]): what is shown is what is sent.
fn stored(
    format: &ValueFormat<'_>,
    property: PropertyIdentifier,
    value: &PropertyValue,
) -> Result<Stored, String> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, value).map_err(|e| format!("cannot encode the value: {e}"))?;
    Ok(Stored {
        text: format.value(property, &buf),
        bytes: buf.to_vec(),
    })
}

/// The slot in control and Present_Value's text for `slots`, falling back
/// to Relinquish_Default when every slot is NULL (Clause 19.2).
fn effective(
    format: &ValueFormat<'_>,
    slots: &[PropertyValue],
    default: Option<&str>,
) -> (Option<u8>, String) {
    match slots.iter().position(|v| *v != PropertyValue::Null) {
        Some(i) => {
            let text = stored(format, PropertyIdentifier::PRESENT_VALUE, &slots[i])
                .map_or_else(|e| e, |s| s.text);
            (Some(i as u8 + 1), text)
        }
        None => (None, default.unwrap_or("?").to_string()),
    }
}

/// One write's preview.
fn plan(
    spec: &WriteSpec,
    read: &PreRead,
    ours: &impl Fn(&WriteSpec, Option<u8>, &[u8]) -> bool,
    dangerous: bool,
) -> Result<Planned, String> {
    let rows = read.rows.as_slice();
    let texts = StateTexts::from_rows(rows);
    let format = ValueFormat::new(spec.object.object_type(), &texts, Floats::Exact);
    let present_value = spec.property == PropertyIdentifier::PRESENT_VALUE && spec.index.is_none();
    // No Priority_Array (or an error reading it) means not commandable. One
    // that was read but isn't 16 values can't be previewed honestly.
    let slots = match raw(rows, PropertyIdentifier::PRIORITY_ARRAY) {
        None => None,
        Some(bytes) => match decode_all(bytes) {
            Some(values) if values.len() == PRIORITIES => {
                Some(values.into_iter().map(slot_value).collect::<Vec<_>>())
            }
            _ if present_value => {
                return Err(
                    "the Priority_Array was read but isn't 16 values; refusing to write".into(),
                );
            }
            _ => None,
        },
    };
    let commandable = present_value && slots.is_some();
    // An indexed write's pre-image is the element, read on its own; when
    // that read failed the pre-image is unknown, and the note says why.
    // Index 0 is the array's length: writing it resizes the array, which
    // putting the old length back would not undo.
    let (current, unknown) = match (spec.index, &read.element) {
        (None, _) => (raw(rows, spec.property), None),
        (Some(0), _) => (None, Some(RESIZE.to_string())),
        (Some(_), Some(Ok(bytes))) => (Some(bytes.as_slice()), None),
        (Some(_), Some(Err(why))) => (
            None,
            Some(format!(
                "the element's old value could not be read ({why}), so this can't be reverted"
            )),
        ),
        (Some(_), None) => (
            None,
            Some("the element's old value was not read, so this can't be reverted".into()),
        ),
    };
    let default = raw(rows, PropertyIdentifier::RELINQUISH_DEFAULT);
    let mut notes = Vec::new();
    let value = match &spec.value {
        None => PropertyValue::Null,
        Some(text) => {
            let like = current.or(default).and_then(first);
            let typed = crate::parse::parse_value_for(
                spec.object.object_type(),
                spec.property,
                spec.index,
                text,
            )?;
            coerce(typed, like.as_ref())
        }
    };
    // A typed `null` is a relinquish too, and is treated as one.
    let relinquish = value == PropertyValue::Null;
    if relinquish && !commandable {
        return Err(
            "relinquishing (NULL) needs a commandable Present_Value (one with a Priority_Array)"
                .into(),
        );
    }
    let value = stored(&format, spec.property, &value)?;
    let out_of_service = match raw(rows, PropertyIdentifier::OUT_OF_SERVICE).and_then(first) {
        Some(PropertyValue::Boolean(on)) => Some(on),
        _ => None,
    };
    let default_text = default.map(|d| format.value(PropertyIdentifier::RELINQUISH_DEFAULT, d));
    let (priority, before, occupancy, effect) = match slots {
        Some(slots) if commandable => {
            let priority = spec.priority.unwrap_or(DEFAULT_PRIORITY);
            if !(1..=16).contains(&priority) {
                return Err(format!("priority must be 1-16, got {priority}"));
            }
            let slot = &slots[usize::from(priority) - 1];
            let before = match slot {
                PropertyValue::Null => None,
                held => Some(stored(&format, PropertyIdentifier::PRESENT_VALUE, held)?),
            };
            let occupancy = match &before {
                None => Occupancy::Empty,
                Some(held) if ours(spec, Some(priority), &held.bytes) => Occupancy::Ours,
                Some(_) => Occupancy::Other,
            };
            let (held_by, now) = effective(&format, &slots, default_text.as_deref());
            let mut after = slots.clone();
            let new = written(&value.bytes);
            after[usize::from(priority) - 1] = new;
            let (_, then) = effective(&format, &after, default_text.as_deref());
            let effect = if now == then {
                Effect::Unchanged {
                    held_by: held_by.filter(|slot| *slot < priority),
                }
            } else {
                Effect::Changes {
                    from: now,
                    to: then,
                }
            };
            (Some(priority), before, occupancy, effect)
        }
        _ => {
            if spec.priority.is_some() {
                notes.push("not commandable: the priority is not sent (Clause 19.2.1)".into());
            }
            let before = match current {
                Some(bytes) => Some(Stored {
                    text: format.value(spec.property, bytes),
                    bytes: bytes.to_vec(),
                }),
                None => {
                    notes.push(unknown.clone().unwrap_or_else(|| {
                        "the old value could not be read, so this can't be reverted".into()
                    }));
                    None
                }
            };
            let effect = if spec.property == PropertyIdentifier::PRESENT_VALUE {
                if out_of_service == Some(false) {
                    notes.push(
                        "Out_Of_Service is FALSE: a device usually refuses this write".into(),
                    );
                }
                Effect::Changes {
                    from: before.as_ref().map_or("?".into(), |b| b.text.clone()),
                    to: value.text.clone(),
                }
            } else {
                Effect::NotPresentValue
            };
            (None, before, Occupancy::NotCommandable, effect)
        }
    };
    if occupancy == Occupancy::Other {
        notes.push(
            "the slot holds another commander's value; the write replaces it and they are not told"
                .into(),
        );
    }
    let gate = gate(spec.property, priority, relinquish, occupancy, dangerous);
    Ok(Planned {
        spec: spec.clone(),
        priority,
        value,
        commandable,
        before,
        occupancy,
        effect,
        out_of_service,
        notes,
        gate,
    })
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
