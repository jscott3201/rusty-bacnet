//! Turn values as read or notified into watch samples.
//!
//! A sample from the hub first becomes a [`Reading`], the value still
//! encoded. The row's [`RowFormat`], made from its first read, then writes
//! it out twice: for the screen (floats rounded, the state texts, a unit
//! symbol) and exactly, for the CSV log (every digit, no state texts).

use bacnet_ops::format::{Floats, RawRow, StateTexts, ValueFormat, decode_all, units};
use bacnet_ops::label::kebab;
use bacnet_types::enums::{EngineeringUnits, PropertyIdentifier};
use bacnet_types::primitives::{PropertyValue, StatusFlags};
use tokio::time::Instant;

use crate::tui::point::Health;
use crate::tui::row_error::row_error_text;
use crate::tui::watched::{Point, Sample};

/// One value of a point, as read or notified, not yet written out.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Reading {
    /// The encoded value, or why it could not be read.
    pub(crate) value: Result<Vec<u8>, String>,
    /// Status_Flags, when the device sent them.
    pub(crate) flags: Option<Health>,
    /// When it arrived.
    pub(crate) at: std::time::Instant,
}

impl Reading {
    /// A reading that failed with `error`.
    pub(crate) fn failed(error: String) -> Self {
        Self {
            value: Err(error),
            flags: None,
            at: Instant::now().into_std(),
        }
    }
}

/// How one row writes its readings: the object's state texts and Units, as
/// its first read found them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RowFormat {
    texts: StateTexts,
    units: Option<EngineeringUnits>,
}

impl RowFormat {
    /// The format the properties of a first read give.
    pub(crate) fn from_rows(rows: &[RawRow]) -> Self {
        Self {
            texts: StateTexts::from_rows(rows),
            units: units(rows),
        }
    }

    /// The Units as text, such as `degrees-fahrenheit`.
    pub(crate) fn units_name(&self) -> Option<String> {
        self.units.map(kebab)
    }

    /// `reading` of `point` as a sample for the screen, and its value in
    /// full for the CSV log. The log names a state by its standard name or
    /// number only (`active`, `2`): the object's own texts are for the
    /// screen.
    pub(crate) fn sample(
        &self,
        point: &Point,
        reading: Reading,
    ) -> (Sample, Result<String, String>) {
        let object_type = point.object.object_type();
        let shown = ValueFormat::new(object_type, &self.texts, Floats::Display);
        let no_texts = StateTexts::default();
        let exact = ValueFormat::new(object_type, &no_texts, Floats::Exact);
        let property = point.property;
        let value = reading.value.as_deref().map(|data| {
            if property == PropertyIdentifier::PRESENT_VALUE {
                shown.with_units(property, data, self.units)
            } else {
                shown.value(property, data)
            }
        });
        let logged = reading
            .value
            .as_deref()
            .map(|data| exact.value(property, data))
            .map_err(Clone::clone);
        let sample = Sample {
            value: value.map_err(Clone::clone),
            number: reading.value.as_deref().ok().and_then(number),
            flags: reading.flags,
            at: reading.at,
        };
        (sample, logged)
    }
}

/// The value as a number for the sparkline: numbers, enumerations (a
/// multi-state or binary point's state) and booleans.
fn number(data: &[u8]) -> Option<f64> {
    let values = decode_all(data)?;
    let [value] = values.as_slice() else {
        return None;
    };
    Some(match value {
        PropertyValue::Real(v) => f64::from(*v),
        PropertyValue::Double(v) => *v,
        PropertyValue::Unsigned(v) => *v as f64,
        PropertyValue::Signed(v) => f64::from(*v),
        PropertyValue::Enumerated(v) => f64::from(*v),
        PropertyValue::Boolean(v) => f64::from(u8::from(*v)),
        _ => return None,
    })
    .filter(|v| v.is_finite())
}

/// Status_Flags as the health badges.
fn flags(data: &[u8]) -> Option<Health> {
    let values = decode_all(data)?;
    let [PropertyValue::BitString { data, .. }] = values.as_slice() else {
        return None;
    };
    let flags = bacnet_types::bitstring::status_flags_from_bacnet(data);
    Some(Health {
        alarm: flags.contains(StatusFlags::IN_ALARM),
        fault: flags.contains(StatusFlags::FAULT),
        overridden: flags.contains(StatusFlags::OVERRIDDEN),
        out_of_service: flags.contains(StatusFlags::OUT_OF_SERVICE),
    })
}

/// A reading of `property` from properties as read. `None` when the read
/// didn't return it at all.
pub(crate) fn from_rows(property: PropertyIdentifier, rows: &[RawRow]) -> Option<Reading> {
    let (_, value) = rows.iter().find(|(p, _)| *p == property)?;
    let status = rows
        .iter()
        .find(|(p, _)| *p == PropertyIdentifier::STATUS_FLAGS)
        .and_then(|(_, value)| value.as_deref().ok())
        .and_then(flags);
    Some(Reading {
        value: value.clone().map_err(|error| row_error_text(&error)),
        flags: status,
        at: Instant::now().into_std(),
    })
}

#[cfg(test)]
mod tests {
    use bacnet_encoding::primitives::encode_property_value;
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::ObjectIdentifier;
    use bytes::BytesMut;

    use super::*;

    fn encoded(value: &PropertyValue) -> Vec<u8> {
        let mut buf = BytesMut::new();
        encode_property_value(&mut buf, value).unwrap();
        buf.to_vec()
    }

    fn point(object_type: ObjectType, property: PropertyIdentifier) -> Point {
        Point {
            device: 1,
            network: None,
            object: ObjectIdentifier::new(object_type, 1).unwrap(),
            property,
        }
    }

    #[test]
    fn a_sample_has_the_value_its_number_and_the_flags() {
        let rows = vec![
            (
                PropertyIdentifier::PRESENT_VALUE,
                Ok(encoded(&PropertyValue::Real(72.5))),
            ),
            (
                PropertyIdentifier::STATUS_FLAGS,
                Ok(encoded(&PropertyValue::BitString {
                    unused_bits: 4,
                    data: vec![0b0101_0000],
                })),
            ),
        ];
        let reading = from_rows(PropertyIdentifier::PRESENT_VALUE, &rows).unwrap();
        let ai = point(ObjectType::ANALOG_INPUT, PropertyIdentifier::PRESENT_VALUE);
        let (sample, _) = RowFormat::default().sample(&ai, reading);
        assert_eq!(sample.value.as_deref(), Ok("72.5"));
        assert_eq!(sample.number, Some(72.5));
        assert_eq!(sample.flags.unwrap().badges(), ["[FLT]", "[OOS]"]);
        assert!(from_rows(PropertyIdentifier::DESCRIPTION, &rows).is_none());
        let text = vec![(
            PropertyIdentifier::DESCRIPTION,
            Ok(encoded(&PropertyValue::CharacterString("x".into()))),
        )];
        let reading = from_rows(PropertyIdentifier::DESCRIPTION, &text).unwrap();
        let description = point(ObjectType::ANALOG_INPUT, PropertyIdentifier::DESCRIPTION);
        let (sample, _) = RowFormat::default().sample(&description, reading);
        assert_eq!((sample.number, sample.flags), (None, None));
    }

    #[test]
    fn the_screen_rounds_and_adds_units_while_the_log_keeps_every_digit() {
        let rows = vec![
            (
                PropertyIdentifier::PRESENT_VALUE,
                Ok(encoded(&PropertyValue::Real(48.447_575))),
            ),
            (
                PropertyIdentifier::UNITS,
                Ok(encoded(&PropertyValue::Enumerated(
                    EngineeringUnits::DEGREES_FAHRENHEIT.to_raw(),
                ))),
            ),
        ];
        let format = RowFormat::from_rows(&rows);
        assert_eq!(format.units_name().as_deref(), Some("degrees-fahrenheit"));
        let ai = point(ObjectType::ANALOG_INPUT, PropertyIdentifier::PRESENT_VALUE);
        let reading = from_rows(PropertyIdentifier::PRESENT_VALUE, &rows).unwrap();
        let (sample, logged) = format.sample(&ai, reading);
        assert_eq!(sample.value.as_deref(), Ok("48.4476 °F"));
        assert_eq!(logged.as_deref(), Ok("48.447575"));
    }

    #[test]
    fn a_binary_point_shows_its_state_with_the_texts_of_its_first_read() {
        let text = |s: &str| Ok(encoded(&PropertyValue::CharacterString(s.into())));
        let rows = vec![
            (
                PropertyIdentifier::PRESENT_VALUE,
                Ok(encoded(&PropertyValue::Enumerated(1))),
            ),
            (PropertyIdentifier::ACTIVE_TEXT, text("Running")),
            (PropertyIdentifier::INACTIVE_TEXT, text("Stopped")),
        ];
        let format = RowFormat::from_rows(&rows);
        let bo = point(ObjectType::BINARY_OUTPUT, PropertyIdentifier::PRESENT_VALUE);
        // A later value, notified without the texts.
        let later = Reading {
            value: Ok(encoded(&PropertyValue::Enumerated(0))),
            flags: None,
            at: Instant::now().into_std(),
        };
        let (sample, logged) = format.sample(&bo, later);
        assert_eq!(sample.value.as_deref(), Ok("inactive (Stopped)"));
        assert_eq!(logged.as_deref(), Ok("inactive"));
        assert_eq!(sample.number, Some(0.0));
    }
}
