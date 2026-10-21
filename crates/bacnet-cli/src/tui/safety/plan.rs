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

use bacnet_encoding::primitives::encode_property_value;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use super::{Effect, Gate, Occupancy, Planned, Preview, Stored, Tier, WriteSpec};
use crate::tui::format::{decode_all, Floats, RawRow, StateTexts, ValueFormat};
use crate::tui::point::PRIORITIES;

/// What is read before a write, besides the target property.
pub(crate) const PRE_READ: [PropertyIdentifier; 4] = [
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::PRIORITY_ARRAY,
    PropertyIdentifier::RELINQUISH_DEFAULT,
    PropertyIdentifier::OUT_OF_SERVICE,
];

/// What was read before one write: the [`PRE_READ`] properties (and the
/// whole target property, for a write without an array index) as rows,
/// and for an indexed write the element itself, or why it couldn't be.
#[derive(Debug, Clone, Default)]
pub(crate) struct PreRead {
    /// The properties read by name.
    pub(crate) rows: Vec<RawRow>,
    /// The element an indexed write targets, as read just before it.
    /// `None` for a write without an index.
    pub(crate) element: Option<Result<Vec<u8>, String>>,
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
pub(crate) const RESIZE: &str = "a resize can't be reverted";

/// The priority a commandable write without one gets (Clause 19.2.1).
pub(crate) const DEFAULT_PRIORITY: u8 = 16;

/// Why priority 6 is refused.
pub(crate) const PRIORITY_6: &str = "priority 6 is reserved for minimum on and off times \
     (Clause 19.2.3); a profile with dangerous = true allows it";

/// The tier of one write.
pub(crate) fn gate(
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
pub(crate) fn overall(items: &[Planned], batch: bool) -> Gate {
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
pub(crate) fn repeated_target(specs: &[&WriteSpec]) -> Option<String> {
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
                    crate::tui::app::objects::object_label(spec.object),
                    crate::tui::app::objects::kebab(spec.property)
                )
            })
    })
}

/// Build the preview of `specs` from `reads`, one per spec. `ours` says
/// whether the ledger wrote what a slot holds now (its encoded value).
pub(crate) fn preview(
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

/// Give a typed number the datatype the property already holds:
/// `parse_value` reads `42` as Unsigned, which an analog Present_Value
/// would refuse as the wrong datatype.
pub(crate) fn coerce(value: PropertyValue, like: Option<&PropertyValue>) -> PropertyValue {
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
            Some(values) if values.len() == PRIORITIES => Some(values),
            _ if present_value => {
                return Err(
                    "the Priority_Array was read but isn't 16 values; refusing to write".into(),
                )
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
            coerce(crate::parse::parse_value(text)?, like.as_ref())
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
            let new = first(&value.bytes).unwrap_or(PropertyValue::Null);
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
mod tests {
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::ObjectIdentifier;

    use super::*;

    fn encoded(values: &[PropertyValue]) -> Vec<u8> {
        let mut buf = BytesMut::new();
        for value in values {
            encode_property_value(&mut buf, value).unwrap();
        }
        buf.to_vec()
    }

    /// AO:1 of the lab: 55 at priority 10, Relinquish_Default 0.
    fn ao1() -> Vec<RawRow> {
        let mut slots = vec![PropertyValue::Null; 16];
        slots[9] = PropertyValue::Real(55.0);
        vec![
            (
                PropertyIdentifier::PRESENT_VALUE,
                Ok(encoded(&[PropertyValue::Real(55.0)])),
            ),
            (PropertyIdentifier::PRIORITY_ARRAY, Ok(encoded(&slots))),
            (
                PropertyIdentifier::RELINQUISH_DEFAULT,
                Ok(encoded(&[PropertyValue::Real(0.0)])),
            ),
            (
                PropertyIdentifier::OUT_OF_SERVICE,
                Ok(encoded(&[PropertyValue::Boolean(false)])),
            ),
        ]
    }

    fn spec(value: Option<&str>, priority: u8) -> WriteSpec {
        WriteSpec {
            device: 100,
            object: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property: PropertyIdentifier::PRESENT_VALUE,
            index: None,
            value: value.map(str::to_string),
            priority: Some(priority),
        }
    }

    fn one(spec: WriteSpec) -> Planned {
        let preview = preview(&[spec], &[ao1().into()], |_, _, _| false, false, false).unwrap();
        preview.items.into_iter().next().unwrap()
    }

    #[test]
    fn a_weaker_priority_leaves_present_value_alone() {
        let planned = one(spec(Some("42"), 12));
        assert_eq!(planned.effect, Effect::Unchanged { held_by: Some(10) });
        assert_eq!(planned.occupancy, Occupancy::Empty);
        assert_eq!(planned.gate, Gate::Allowed(Tier::T1));
        // The integer was sent as the REAL the object holds.
        assert_eq!(planned.value.bytes, encoded(&[PropertyValue::Real(42.0)]));
    }

    #[test]
    fn a_preview_keeps_every_digit_of_a_float() {
        // The screens round to six digits; a preview shows what is sent.
        let planned = one(spec(Some("72.12345"), 8));
        assert_eq!(planned.value.text, "72.12345");
        assert_eq!(
            planned.effect,
            Effect::Changes {
                from: "55".into(),
                to: "72.12345".into()
            }
        );
    }

    #[test]
    fn a_binary_preview_names_the_states() {
        let mut slots = vec![PropertyValue::Null; 16];
        slots[7] = PropertyValue::Enumerated(1);
        let rows = vec![
            (
                PropertyIdentifier::PRESENT_VALUE,
                Ok(encoded(&[PropertyValue::Enumerated(1)])),
            ),
            (PropertyIdentifier::PRIORITY_ARRAY, Ok(encoded(&slots))),
            (
                PropertyIdentifier::RELINQUISH_DEFAULT,
                Ok(encoded(&[PropertyValue::Enumerated(0)])),
            ),
        ];
        let spec = WriteSpec {
            object: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1).unwrap(),
            value: None,
            ..spec(None, 8)
        };
        let preview = preview(&[spec], &[rows.into()], |_, _, _| true, false, false).unwrap();
        assert_eq!(
            preview.items[0].effect,
            Effect::Changes {
                from: "active".into(),
                to: "inactive".into()
            }
        );
    }

    #[test]
    fn priority_8_changes_present_value_and_needs_t2() {
        let planned = one(spec(Some("42"), 8));
        assert_eq!(
            planned.effect,
            Effect::Changes {
                from: "55".into(),
                to: "42".into()
            }
        );
        assert_eq!(planned.gate, Gate::Allowed(Tier::T2));
        assert_eq!(planned.before, None, "slot 8 was empty");
    }

    #[test]
    fn tiers_follow_the_priority() {
        let tier = |p| {
            gate(
                PropertyIdentifier::PRESENT_VALUE,
                Some(p),
                false,
                Occupancy::Empty,
                false,
            )
        };
        for p in [1, 2, 5] {
            assert_eq!(tier(p), Gate::Allowed(Tier::T3), "priority {p}");
        }
        for p in [3, 4, 7, 8] {
            assert_eq!(tier(p), Gate::Allowed(Tier::T2), "priority {p}");
        }
        for p in 9..=16 {
            assert_eq!(tier(p), Gate::Allowed(Tier::T1), "priority {p}");
        }
        assert!(matches!(tier(6), Gate::Blocked(_)));
        assert_eq!(
            gate(
                PropertyIdentifier::PRESENT_VALUE,
                Some(6),
                false,
                Occupancy::Empty,
                true
            ),
            Gate::Allowed(Tier::T3),
            "dangerous allows it at T3"
        );
        assert_eq!(
            gate(
                PropertyIdentifier::PRESENT_VALUE,
                Some(12),
                true,
                Occupancy::Other,
                false
            ),
            Gate::Allowed(Tier::T2),
            "relinquishing someone else's slot"
        );
        assert_eq!(
            gate(
                PropertyIdentifier::OUT_OF_SERVICE,
                None,
                false,
                Occupancy::NotCommandable,
                false
            ),
            Gate::Allowed(Tier::T2)
        );
    }

    #[test]
    fn relinquishing_the_slot_in_control_falls_back_and_marks_another_writer() {
        let planned = one(spec(None, 10));
        assert_eq!(planned.occupancy, Occupancy::Other);
        assert_eq!(planned.gate, Gate::Allowed(Tier::T2));
        assert_eq!(
            planned.effect,
            Effect::Changes {
                from: "55".into(),
                to: "0".into()
            }
        );
        assert_eq!(planned.before.unwrap().text, "55");
        let ours = preview(
            &[spec(None, 10)],
            &[ao1().into()],
            |_, _, _| true,
            false,
            false,
        )
        .unwrap();
        assert_eq!(ours.items[0].occupancy, Occupancy::Ours);
        assert_eq!(ours.gate, Gate::Allowed(Tier::T1));
    }

    /// `spec` for AO:2, a second object with the same rows.
    fn other(value: Option<&str>, priority: u8) -> WriteSpec {
        WriteSpec {
            object: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 2).unwrap(),
            ..spec(value, priority)
        }
    }

    #[test]
    fn a_batch_takes_its_strongest_tier_and_any_block() {
        let batch = preview(
            &[spec(Some("1"), 12), other(Some("2"), 13)],
            &[ao1().into(), ao1().into()],
            |_, _, _| false,
            false,
            true,
        )
        .unwrap();
        assert_eq!(batch.gate, Gate::Allowed(Tier::T2));
        let blocked = preview(
            &[spec(Some("1"), 12), other(Some("2"), 6)],
            &[ao1().into(), ao1().into()],
            |_, _, _| false,
            false,
            true,
        )
        .unwrap();
        assert!(matches!(blocked.gate, Gate::Blocked(_)));
    }

    /// A WPM error names the failed write without its priority, so one
    /// property twice (at two priorities) would be ambiguous.
    #[test]
    fn a_batch_may_not_name_one_property_twice() {
        let error = preview(
            &[spec(Some("1"), 8), spec(Some("2"), 9)],
            &[ao1().into(), ao1().into()],
            |_, _, _| false,
            false,
            true,
        )
        .unwrap_err();
        assert!(error.contains("appears twice"), "{error}");
    }

    #[test]
    fn a_typed_null_is_a_relinquish() {
        // Slot 10 holds another commander's 55: relinquishing it is T2.
        let planned = one(spec(Some("null"), 10));
        assert_eq!(planned.value.bytes, super::super::NULL);
        assert_eq!(planned.gate, Gate::Allowed(Tier::T2));
        let mut description = spec(Some("null"), 12);
        description.property = PropertyIdentifier::DESCRIPTION;
        assert!(preview(
            &[description],
            &[ao1().into()],
            |_, _, _| false,
            false,
            false
        )
        .is_err());
    }

    #[test]
    fn a_slot_is_ours_only_by_its_current_value() {
        let fifty_five = encoded(&[PropertyValue::Real(55.0)]);
        let ours = |_: &WriteSpec, _: Option<u8>, now: &[u8]| now == fifty_five.as_slice();
        let planned = preview(&[spec(None, 10)], &[ao1().into()], ours, false, false).unwrap();
        assert_eq!(planned.items[0].occupancy, Occupancy::Ours);
        let wrote_other = |_: &WriteSpec, _: Option<u8>, now: &[u8]| now == [0x44, 0, 0, 0, 0];
        let planned = preview(
            &[spec(None, 10)],
            &[ao1().into()],
            wrote_other,
            false,
            false,
        )
        .unwrap();
        assert_eq!(planned.items[0].occupancy, Occupancy::Other);
        assert_eq!(planned.gate, Gate::Allowed(Tier::T2));
    }

    #[test]
    fn a_priority_array_that_does_not_decode_refuses_the_write() {
        let mut rows = ao1();
        rows[1].1 = Ok(encoded(&vec![PropertyValue::Null; 4]));
        let error = preview(
            &[spec(Some("1"), 12)],
            &[rows.clone().into()],
            |_, _, _| false,
            false,
            false,
        )
        .unwrap_err();
        assert!(error.contains("Priority_Array"), "{error}");
        rows[1].1 = Ok(vec![0xFF]);
        assert!(preview(
            &[spec(Some("1"), 12)],
            &[rows.into()],
            |_, _, _| false,
            false,
            false
        )
        .is_err());
        // An error reading it means not commandable.
        let mut rows = ao1();
        rows[1].1 = Err("PROPERTY:UNKNOWN_PROPERTY".into());
        let planned = preview(
            &[spec(Some("1"), 12)],
            &[rows.into()],
            |_, _, _| false,
            false,
            false,
        )
        .unwrap();
        assert!(!planned.items[0].commandable);
    }

    #[test]
    fn a_non_commandable_property_keeps_its_old_value_as_the_pre_image() {
        let mut rows = ao1();
        rows.push((
            PropertyIdentifier::DESCRIPTION,
            Ok(encoded(&[PropertyValue::CharacterString("old".into())])),
        ));
        let mut description = spec(Some("\"new\""), 12);
        description.property = PropertyIdentifier::DESCRIPTION;
        let planned = preview(
            &[description],
            &[rows.into()],
            |_, _, _| false,
            false,
            false,
        )
        .unwrap()
        .items
        .remove(0);
        assert_eq!(planned.priority, None);
        assert_eq!(planned.before.unwrap().text, "\"old\"");
        assert_eq!(planned.effect, Effect::NotPresentValue);
        let mut relinquish = spec(None, 12);
        relinquish.property = PropertyIdentifier::DESCRIPTION;
        assert!(preview(
            &[relinquish],
            &[ao1().into()],
            |_, _, _| false,
            false,
            false
        )
        .is_err());
    }

    /// `state-text[2]` of MSV:1, with `element` as the element's read.
    fn state_text(element: Option<Result<Vec<u8>, String>>) -> Planned {
        let spec = WriteSpec {
            object: ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 1).unwrap(),
            property: PropertyIdentifier::STATE_TEXT,
            index: Some(2),
            priority: None,
            ..spec(Some("\"Low\""), 12)
        };
        // The rows hold no State_Text: an indexed write reads its element alone.
        let read = PreRead {
            rows: Vec::new(),
            element,
        };
        preview(&[spec], &[read], |_, _, _| false, false, false)
            .unwrap()
            .items
            .remove(0)
    }

    #[test]
    fn an_indexed_write_keeps_the_element_as_its_pre_image() {
        let old = encoded(&[PropertyValue::CharacterString("State 2".into())]);
        let planned = state_text(Some(Ok(old.clone())));
        let before = planned.before.expect("the element's old value");
        assert_eq!(before.bytes, old);
        assert_eq!(before.text, "\"State 2\"");
        assert!(planned.notes.is_empty(), "{:?}", planned.notes);
    }

    #[test]
    fn an_unreadable_element_leaves_the_pre_image_unknown_and_says_why() {
        let planned = state_text(Some(Err("PROPERTY:INVALID_ARRAY_INDEX".into())));
        assert_eq!(planned.before, None);
        assert!(
            planned
                .notes
                .iter()
                .any(|n| n.contains("INVALID_ARRAY_INDEX") && n.contains("can't be reverted")),
            "{:?}",
            planned.notes
        );
        assert_eq!(state_text(None).before, None, "never read");
    }

    /// Index 0 is the array's length: a resize, never reverted, even with
    /// the old length in hand.
    #[test]
    fn writing_index_0_is_a_resize_with_no_pre_image() {
        let spec = WriteSpec {
            object: ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 1).unwrap(),
            property: PropertyIdentifier::STATE_TEXT,
            index: Some(0),
            priority: None,
            ..spec(Some("4"), 12)
        };
        let read = PreRead {
            rows: Vec::new(),
            element: Some(Ok(encoded(&[PropertyValue::Unsigned(3)]))),
        };
        let planned = preview(&[spec], &[read], |_, _, _| false, false, false)
            .unwrap()
            .items
            .remove(0);
        assert_eq!(planned.before, None);
        assert!(
            planned.notes.iter().any(|n| n == RESIZE),
            "{:?}",
            planned.notes
        );
    }
}
