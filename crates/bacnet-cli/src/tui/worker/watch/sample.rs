//! Turn values as read or notified into watch samples.
//!
//! A sample from the hub first becomes a [`Reading`], the value still
//! encoded. The row's [`RowFormat`], made from its first read, then writes
//! it out twice: for the screen (floats rounded, the state texts, a unit
//! symbol) and exactly, for the CSV log (every digit, no state texts). A
//! row's [`Writer`] does both, holding back what comes before that read.

use std::collections::VecDeque;
use std::time::UNIX_EPOCH;

use bacnet_ops::format::{Floats, RawRow, StateTexts, ValueFormat, decode_all, object_name, units};
use bacnet_ops::label::kebab;
use bacnet_ops::watch::{self as hub, Via};
use bacnet_types::enums::{EngineeringUnits, PropertyIdentifier};
use bacnet_types::primitives::{PropertyValue, StatusFlags};
use tokio::time::Instant;

use super::PENDING_PER_ROW;
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

/// A sample written out: for the screen, in full for the log, how it came
/// (`cov` or `polled`), and when, in milliseconds of the wall clock since
/// the Unix epoch.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Written {
    pub(crate) shown: Sample,
    pub(crate) logged: Result<String, String>,
    pub(crate) source: &'static str,
    pub(crate) time: u128,
}

/// What a first read found besides the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Details {
    /// The object's Object_Name, when the device gave it.
    pub(crate) name: Option<String>,
    /// The Units as text, when the object has them.
    pub(crate) units: Option<String>,
}

/// What a row's samples gave in one tick, oldest first.
#[derive(Debug, Default)]
pub(crate) struct Taken {
    pub(crate) details: Vec<Details>,
    pub(crate) written: Vec<Written>,
    /// Notifications held back and lost to newer ones.
    pub(crate) dropped: u64,
}

/// Writes one row's samples out with the format of its first read.
///
/// A device usually notifies as soon as it takes the subscription, and the
/// hub hands the first read on only once its start is over, so a
/// notification can come before the read that says how to write it. It
/// waits for that read, for the whole start (the SubscribeCOV timeout and
/// its retries included), then is written after it, since the read ended
/// before the subscription was asked for (#2167). A first read that
/// failed, or a start that ended without one seen, lets what waits out
/// without units or state texts.
///
/// A later start (after a failed renewal, or a retry from polling) writes
/// with the format it has, so nothing waits; its read, handed on last,
/// can be older than a notification already written. It then still gives
/// the format and the details but not a value, so every value written is
/// at least as new as the one before.
#[derive(Debug)]
pub(crate) struct Writer {
    point: Point,
    /// `None` until a first read lands.
    format: Option<RowFormat>,
    /// Notifications that came before it.
    held: VecDeque<hub::Sample>,
    /// When the newest value written arrived.
    newest: Option<Instant>,
}

impl Writer {
    pub(crate) fn new(point: Point) -> Self {
        Self {
            point,
            format: None,
            held: VecDeque::new(),
            newest: None,
        }
    }

    /// Notifications waiting for the first read.
    pub(crate) fn held(&self) -> u64 {
        self.held.len() as u64
    }

    /// Take one sample; `polled` when the row is polled now.
    pub(crate) fn take(&mut self, sample: hub::Sample, polled: bool, out: &mut Taken) {
        match (sample.via, &sample.values) {
            (Via::Notification, _) if self.format.is_none() => {
                if self.held.len() >= PENDING_PER_ROW {
                    self.held.pop_front();
                    out.dropped += 1;
                }
                self.held.push_back(sample);
            }
            (Via::First, values) => {
                match values {
                    Ok(rows) => {
                        let format = RowFormat::from_rows(rows);
                        out.details.push(Details {
                            name: object_name(rows),
                            units: format.units_name(),
                        });
                        self.format = Some(format);
                    }
                    // Keeps what an earlier start's read found, or none.
                    Err(_) => {
                        self.format.get_or_insert_default();
                    }
                }
                // What waited came after the read.
                self.write(&sample, polled, out);
                self.flush(polled, out);
            }
            (Via::Notification | Via::Poll, _) => {
                self.release(polled, out);
                self.write(&sample, polled, out);
            }
        }
    }

    /// The start ended and no first read came (a reader that falls behind
    /// loses the oldest samples), or the session is ending: what waits goes
    /// out without the format.
    pub(crate) fn release(&mut self, polled: bool, out: &mut Taken) {
        if self.format.is_none() {
            self.format = Some(RowFormat::default());
            self.flush(polled, out);
        }
    }

    fn flush(&mut self, polled: bool, out: &mut Taken) {
        while let Some(sample) = self.held.pop_front() {
            self.write(&sample, polled, out);
        }
    }

    fn write(&mut self, sample: &hub::Sample, polled: bool, out: &mut Taken) {
        if sample.via == Via::First && self.newest.is_some_and(|newest| sample.at < newest) {
            // A later start's read, older than a value already written.
            return;
        }
        let property = self.point.property;
        let reading = match &sample.values {
            Ok(rows) => match sample.via {
                Via::Poll => Some(
                    from_rows(property, rows)
                        .unwrap_or_else(|| Reading::failed(hub::poll::MISSING.into())),
                ),
                Via::First | Via::Notification => from_rows(property, rows),
            },
            // A first read that failed leaves the row without a value
            // until the next.
            Err(_) if sample.via == Via::First => None,
            Err(error) => Some(Reading::failed(error.clone())),
        };
        let Some(mut reading) = reading else {
            return;
        };
        reading.at = sample.at.into_std();
        self.newest = self.newest.max(Some(sample.at));
        let source = match sample.via {
            Via::Notification => "cov",
            Via::Poll => "polled",
            Via::First if polled => "polled",
            Via::First => "cov",
        };
        let time = sample
            .time
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis());
        let none = RowFormat::default();
        let format = self.format.as_ref().unwrap_or(&none);
        let (shown, logged) = format.sample(&self.point, reading);
        out.written.push(Written {
            shown,
            logged,
            source,
            time,
        });
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
