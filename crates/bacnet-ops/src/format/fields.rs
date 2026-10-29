//! A value decoded field by field, written once as text and once as typed
//! data: Event_Parameters (#2090) and an event notification's values
//! (#2073). Each field has a name, which keys it in the JSON (`low_limit`),
//! and a label, which leads it in the text (`low 10`).

use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetPropertyStates};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::{PropertyValue, StatusFlags};

use super::constructed::{on_device, reference};
use super::primitive::{hex, plain_bits};
use super::recipients::property_state_text;
use super::tagged::fields_text;
use super::{Datum, Fields, ValueFormat, decode_all};

/// A property with no enumeration, named bits or state: the values here
/// are named by the field, not by a property.
const ANY: PropertyIdentifier = PropertyIdentifier::LOG_BUFFER;

/// An enumerated value as typed data: its number, and its name when the
/// enumeration has one (a number it doesn't name prints as itself).
pub(super) fn named(value: u32, name: String) -> Datum {
    Datum::Enumerated {
        value,
        name: Some(name).filter(|name| name.parse::<u64>().is_err()),
    }
}

/// Status flags as the property names them: `in-alarm | fault`, or `none`.
pub(super) fn status_flags_value(format: &ValueFormat<'_>, flags: StatusFlags) -> (String, Datum) {
    let value = PropertyValue::BitString {
        unused_bits: 4,
        data: vec![flags.bits() << 4],
    };
    (
        format.element(PropertyIdentifier::STATUS_FLAGS, &value),
        format.element_data(PropertyIdentifier::STATUS_FLAGS, &value),
    )
}

/// A device object property reference, as the element table writes one.
pub(super) fn reference_value(r: &BACnetDeviceObjectPropertyReference) -> (String, Datum) {
    let text = on_device(
        reference(
            r.object_identifier,
            r.property_identifier,
            r.property_array_index,
        ),
        r.device_identifier,
    );
    let datum = Datum::Constructed {
        element: "device-object-property-reference".into(),
        text: text.clone(),
        fields: None,
    };
    (text, datum)
}

/// Encoded application values (an ABSTRACT-SYNTAX value, an extended
/// parameter list): one value as itself, several as a list. Octets that
/// aren't application values, or hold a context-tagged one, show their
/// fields by tag and stay hex in the data.
pub(super) fn encoded_value(format: &ValueFormat<'_>, octets: &[u8]) -> (String, Datum) {
    match decode_all(octets).as_deref() {
        Some([one]) if !matches!(one, PropertyValue::ApplicationData(_)) => {
            (format.element(ANY, one), format.element_data(ANY, one))
        }
        Some(many)
            if !many.is_empty()
                && !many
                    .iter()
                    .any(|value| matches!(value, PropertyValue::ApplicationData(_))) =>
        {
            let texts: Vec<String> = many
                .iter()
                .map(|value| format.element(ANY, value))
                .collect();
            let data = many
                .iter()
                .map(|value| format.element_data(ANY, value))
                .collect();
            (format!("[{}]", texts.join(", ")), Datum::List(data))
        }
        _ => (
            fields_text(octets, format.floats),
            Datum::Raw {
                raw: hex(octets, ""),
            },
        ),
    }
}

/// The fields of one value, gathered in order.
pub(super) struct FieldList<'a> {
    format: ValueFormat<'a>,
    texts: Vec<String>,
    fields: Vec<(String, Datum)>,
}

impl<'a> FieldList<'a> {
    pub(super) fn new(format: &ValueFormat<'a>) -> Self {
        Self {
            format: *format,
            texts: Vec::new(),
            fields: Vec::new(),
        }
    }

    pub(super) fn format(&self) -> &ValueFormat<'a> {
        &self.format
    }

    /// A field that adds no text: the value's kind, which the text names
    /// before its fields.
    pub(super) fn quiet(&mut self, name: &str, datum: Datum) {
        self.fields.push((name.into(), datum));
    }

    /// A field whose text is written out whole.
    pub(super) fn field(&mut self, name: &str, text: String, datum: Datum) {
        self.texts.push(text);
        self.fields.push((name.into(), datum));
    }

    /// A field as its label and value text: `low 10`.
    pub(super) fn labelled(&mut self, name: &str, label: &str, (text, datum): (String, Datum)) {
        self.field(name, format!("{label} {text}"), datum);
    }

    /// A time delay in seconds, `delay 30 s`, the field `time_delay`.
    pub(super) fn delay(&mut self, seconds: u32) {
        self.field(
            "time_delay",
            format!("delay {seconds} s"),
            Datum::Unsigned(u64::from(seconds)),
        );
    }

    /// An application value under `label`.
    pub(super) fn value(&mut self, name: &str, label: &str, value: &PropertyValue) {
        let pair = (
            self.format.element(ANY, value),
            self.format.element_data(ANY, value),
        );
        self.labelled(name, label, pair);
    }

    pub(super) fn unsigned(&mut self, name: &str, label: &str, value: u64) {
        self.value(name, label, &PropertyValue::Unsigned(value));
    }

    pub(super) fn signed(&mut self, name: &str, label: &str, value: i32) {
        self.value(name, label, &PropertyValue::Signed(value));
    }

    pub(super) fn real(&mut self, name: &str, label: &str, value: f32) {
        self.value(name, label, &PropertyValue::Real(value));
    }

    pub(super) fn double(&mut self, name: &str, label: &str, value: f64) {
        self.value(name, label, &PropertyValue::Double(value));
    }

    /// A bit string with no names: `bits(0110)`.
    pub(super) fn bits(&mut self, name: &str, label: &str, (unused_bits, data): (u8, &[u8])) {
        let pair = bits_value(unused_bits, data);
        self.labelled(name, label, pair);
    }

    pub(super) fn status_flags(&mut self, name: &str, label: &str, flags: StatusFlags) {
        let pair = status_flags_value(&self.format, flags);
        self.labelled(name, label, pair);
    }

    pub(super) fn reference(
        &mut self,
        name: &str,
        label: &str,
        r: &BACnetDeviceObjectPropertyReference,
    ) {
        self.labelled(name, label, reference_value(r));
    }

    /// Enumerated values by name, `[active, alarm]`.
    pub(super) fn enumerations(
        &mut self,
        name: &str,
        label: &str,
        values: impl Iterator<Item = (u32, String)>,
    ) {
        let items = values.map(|(raw, name)| (name.clone(), named(raw, name)));
        self.list(name, label, items.collect());
    }

    /// Items as a list, `[a, b]`.
    pub(super) fn list(&mut self, name: &str, label: &str, items: Vec<(String, Datum)>) {
        let (texts, data): (Vec<String>, Vec<Datum>) = items.into_iter().unzip();
        let pair = (format!("[{}]", texts.join(", ")), Datum::List(data));
        self.labelled(name, label, pair);
    }

    /// The fields' texts, comma separated, and the fields.
    pub(super) fn finish(self) -> (String, Fields) {
        (self.texts.join(", "), Fields::new(self.fields))
    }
}

/// A property state by name, as text and as a `property-state` element.
pub(super) fn property_state_value(state: &BACnetPropertyStates) -> (String, Datum) {
    let text = property_state_text(state);
    let datum = Datum::Constructed {
        element: "property-state".into(),
        text: text.clone(),
        fields: None,
    };
    (text, datum)
}

/// A bit string with no names, as text and data.
pub(super) fn bits_value(unused_bits: u8, data: &[u8]) -> (String, Datum) {
    let value = PropertyValue::BitString {
        unused_bits,
        data: data.to_vec(),
    };
    let bits = plain_bits(&value);
    let digits = bits
        .trim_start_matches("bits(")
        .trim_end_matches(')')
        .to_string();
    (
        bits,
        Datum::Bits {
            bits: digits,
            names: None,
        },
    )
}
