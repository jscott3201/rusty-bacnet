//! Property values in the two forms the tools return (design §5): the
//! display text of the shared formatter (`bacnet_ops::format`), which
//! decodes constructed values such as timestamps and schedules, and the
//! formatter's typed form ([`Datum`]: enumerations with their number and
//! name, bit strings with their set bits named, objects by type and
//! instance, constructed elements as text), the same shape as the CLI's
//! JSON. Raw hex is only the fallback for octets that don't decode.
//!
//! Device text goes in untrusted envelopes, leaf by leaf: a character
//! string, and the text of a constructed element whose kind can hold one.
//! State texts are never spliced into the display text, and the display
//! text is enveloped whole only when the value holds device text.

use bacnet_ops::format::{Datum, Floats, PLAIN_ELEMENTS, StateTexts, ValueFormat, decode_all};
use bacnet_ops::label::kebab;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;
use serde_json::{Value, json};

use crate::untrusted::envelope;

/// A property's value as `{display, typed}`, read at `index` (an array
/// element) or whole: `source` names it for an envelope, `cap` caps any
/// device text, and `units` (the object's, when read) follow a number in
/// the display text.
pub(crate) fn property_value(
    object_type: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
    data: &[u8],
    units: Option<EngineeringUnits>,
    source: &str,
    cap: usize,
) -> Value {
    let texts = StateTexts::default();
    let format = ValueFormat::new(object_type, &texts, Floats::Display);
    let text = match index {
        None => format.with_units(property, data, units),
        Some(_) => format.value_at(property, index, data),
    };
    let mut typed = datum_json(&format.data_at(property, index, data));
    let display = if envelope_leaves(&mut typed, source, cap) {
        envelope(&text, source, cap)
    } else {
        json!(text)
    };
    json!({ "display": display, "typed": typed })
}

fn datum_json(datum: &Datum) -> Value {
    serde_json::to_value(datum).expect("a datum serializes")
}

/// Any serializable value built from [`Datum`]s (a single datum, or a
/// summary such as an alarm's) as JSON, with each device-text leaf in an
/// envelope from `source`, capped at `cap`.
pub(crate) fn enveloped(value: &impl serde::Serialize, source: &str, cap: usize) -> Value {
    let mut json = serde_json::to_value(value).expect("decoded data serializes");
    envelope_leaves(&mut json, source, cap);
    json
}

/// The object forms of [`Datum`] whose strings are the formatter's own
/// (names, digits, dates), by their keys; a key in the second list may be
/// left out.
const SAFE_SHAPES: [(&[&str], &[&str]); 8] = [
    (&["float"], &[]),
    (&["octets"], &[]),
    (&["value"], &["name"]),
    (&["bits"], &["names"]),
    (&["date"], &[]),
    (&["time"], &[]),
    (&["type", "type_number", "instance"], &[]),
    (&["raw"], &[]),
];

/// Whether `map` is one of [`SAFE_SHAPES`]: every required key there, and
/// no key the shape doesn't have.
fn safe_shape(map: &serde_json::Map<String, Value>) -> bool {
    SAFE_SHAPES.iter().any(|(required, optional)| {
        required.iter().all(|key| map.contains_key(*key))
            && map
                .keys()
                .all(|key| required.contains(&key.as_str()) || optional.contains(&key.as_str()))
    })
}

/// Put each device-text leaf of `typed` in an envelope; true when there
/// was one. A bare JSON string in a datum is a character string; an
/// object with `element` and `text` is a constructed element, whose text
/// is enveloped unless its kind is plain and whose `fields`, when it has
/// them, are values in turn; a known object form is left as
/// it is; and any other object (a form the formatter adds later) has
/// every string in it enveloped, so nothing new slips past.
fn envelope_leaves(typed: &mut Value, source: &str, cap: usize) -> bool {
    match typed {
        Value::String(text) => {
            *typed = envelope(text, source, cap);
            true
        }
        Value::Array(items) => {
            let mut any = false;
            for item in items {
                any |= envelope_leaves(item, source, cap);
            }
            any
        }
        Value::Object(map) if safe_shape(map) => false,
        Value::Object(map) if is_element(map) => {
            let plain = map
                .get("element")
                .and_then(Value::as_str)
                .is_some_and(|kind| PLAIN_ELEMENTS.contains(&kind));
            let mut any = match map.get_mut("text") {
                Some(text) if !plain => envelope_leaves(text, source, cap),
                _ => false,
            };
            // Each member by name, as any other value: a character string
            // among them is device text.
            if let Some(Value::Object(fields)) = map.get_mut("fields") {
                any |= envelope_every(fields, source, cap);
            }
            any
        }
        Value::Object(map) => envelope_every(map, source, cap),
        _ => false,
    }
}

/// Whether `map` is a constructed element: its kind and text, and the
/// fields of an element decoded field by field (an object), nothing else.
fn is_element(map: &serde_json::Map<String, Value>) -> bool {
    map.get("element").is_some_and(Value::is_string)
        && map.contains_key("text")
        && map.get("fields").is_none_or(Value::is_object)
        && map
            .keys()
            .all(|key| ["element", "text", "fields"].contains(&key.as_str()))
}

/// Envelope every string in an object of a form not known to be safe.
fn envelope_every(map: &mut serde_json::Map<String, Value>, source: &str, cap: usize) -> bool {
    let mut any = false;
    for value in map.values_mut() {
        any |= envelope_leaves(value, source, cap);
    }
    any
}

/// The names of a bit string's set bits, for a property whose bits have
/// names (services, object types, status flags).
pub(crate) fn bit_names(property: PropertyIdentifier, value: &PropertyValue) -> Vec<String> {
    let texts = StateTexts::default();
    let format = ValueFormat::new(ObjectType::DEVICE, &texts, Floats::Display);
    match format.element_data(property, value) {
        Datum::Bits {
            names: Some(names), ..
        } => names,
        _ => Vec::new(),
    }
}

/// The text of an encoded character string, as the device sent it (to be
/// enveloped); `None` unless the octets are one character string.
pub(crate) fn text_of(data: &[u8]) -> Option<String> {
    match decode_all(data)?.as_slice() {
        [PropertyValue::CharacterString(text)] => Some(text.clone()),
        _ => None,
    }
}

/// The text of `property` among rows as read, as [`text_of`].
pub(crate) fn row_text(
    rows: &[bacnet_ops::format::RawRow],
    property: PropertyIdentifier,
) -> Option<String> {
    let (_, value) = rows.iter().find(|(p, _)| *p == property)?;
    text_of(value.as_deref().ok()?)
}

/// An object in the tools' argument form, `analog-input:1`.
pub(crate) fn object_text(object_type: ObjectType, instance: u32) -> String {
    format!("{}:{instance}", kebab(object_type))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_encoding::primitives::encode_property_value;
    use bytes::BytesMut;

    fn encoded(value: &PropertyValue) -> Vec<u8> {
        let mut buf = BytesMut::new();
        encode_property_value(&mut buf, value).unwrap();
        buf.to_vec()
    }

    #[test]
    fn numbers_carry_units_and_enumerations_their_names() {
        let pv = encoded(&PropertyValue::Real(72.5));
        let value = property_value(
            ObjectType::ANALOG_INPUT,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            &pv,
            Some(EngineeringUnits::DEGREES_FAHRENHEIT),
            "device 1 analog-input:1 present-value",
            256,
        );
        assert_eq!(value["display"], json!("72.5 °F"));
        assert_eq!(value["typed"], json!(72.5));
        let units = encoded(&PropertyValue::Enumerated(64));
        let value = property_value(
            ObjectType::ANALOG_INPUT,
            PropertyIdentifier::UNITS,
            None,
            &units,
            None,
            "s",
            256,
        );
        assert_eq!(
            value["typed"],
            json!({ "value": 64, "name": "degrees-fahrenheit" })
        );
    }

    #[test]
    fn set_bits_are_named() {
        let flags = PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0b1001_0000],
        };
        assert_eq!(
            bit_names(PropertyIdentifier::STATUS_FLAGS, &flags),
            ["in-alarm", "out-of-service"]
        );
    }

    #[test]
    fn device_text_is_enveloped_in_both_forms() {
        let name = encoded(&PropertyValue::CharacterString(
            "Fan\u{202E} https://x".into(),
        ));
        let value = property_value(
            ObjectType::ANALOG_INPUT,
            PropertyIdentifier::DESCRIPTION,
            None,
            &name,
            None,
            "device 1 analog-input:1 description",
            256,
        );
        assert_eq!(value["display"]["untrusted"], json!("\"Fan hxxps://x\""));
        assert_eq!(value["typed"]["untrusted"], json!("Fan hxxps://x"));
        assert_eq!(
            value["typed"]["source"],
            json!("device 1 analog-input:1 description")
        );
    }

    /// A constructed element that holds no device string isn't enveloped;
    /// one that can is, leaf by leaf, and its display text with it.
    #[test]
    fn only_device_text_elements_are_enveloped() {
        let mut stamp = json!([{ "element": "time-stamp", "text": "2026-10-25 07:00" }]);
        assert!(!envelope_leaves(&mut stamp, "s", 256));
        assert_eq!(stamp[0]["text"], json!("2026-10-25 07:00"));
        // The shared list (#2111) has the notification formatter's
        // date-time, which holds no device string either.
        let mut when = json!({ "element": "date-time", "text": "2026-10-25 07:00:00.00" });
        assert!(!envelope_leaves(&mut when, "s", 256));
        assert_eq!(when["text"], json!("2026-10-25 07:00:00.00"));
        let mut named = json!([{ "element": "name-value", "text": "mode = \"Fan\u{202E}\"" }, 7]);
        assert!(envelope_leaves(&mut named, "s", 256));
        assert_eq!(named[0]["text"]["untrusted"], json!("mode = \"Fan\""));
        assert_eq!(named[1], json!(7));
        let mut unknown = json!({ "element": "element", "text": "x" });
        assert!(envelope_leaves(&mut unknown, "s", 256));
        // The formatter's own strings stay; a form it doesn't have yet has
        // its strings enveloped.
        let mut known = json!([{ "value": 1, "name": "active" }, { "bits": "01", "names": ["a"] }]);
        assert!(!envelope_leaves(&mut known, "s", 256));
        let mut future = json!({ "label": "Fan\u{202E}", "count": 2 });
        assert!(envelope_leaves(&mut future, "s", 256));
        assert_eq!(future["label"]["untrusted"], json!("Fan"));
    }
}
