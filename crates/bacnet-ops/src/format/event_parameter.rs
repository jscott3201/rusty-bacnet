//! An Event Enrollment's Event_Parameters, every algorithm by its fields
//! (#2079, #2090): `out-of-range: delay 5 s, low 10, high 90, deadband 1`
//! as text, and in the typed data the algorithm and each field by name.

use bacnet_encoding::constructed::{decode_event_parameter, split_elements};
use bacnet_types::constructed::{BACnetEventParameter, ChangeOfValueCriteria};
use bacnet_types::enums::{EventType, LifeSafetyState, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::fields::{FieldList, encoded_value, named, property_state_value};
use super::primitive::hex;
use super::tagged::fields_text;
use super::{Datum, Fields, ValueFormat};
use crate::label::kebab;

/// The choice tag `decode_event_parameter` gives the forms that aren't the
/// wire CHOICE (an application octet string, and a legacy wrapper); those
/// are left to the generic view.
const NOT_A_CHOICE: u8 = u8::MAX;

/// The element kind the JSON names an Event_Parameters value by.
const ELEMENT: &str = "event-parameter";

/// The algorithm of an Event_Parameters choice, named as the event type
/// with the same number.
fn algorithm(tag: u8) -> String {
    kebab(EventType::from_raw(u32::from(tag)))
}

/// An Event_Parameters value as text, `out-of-range: delay 5 s, ...`, and
/// its fields, the first of them `algorithm`.
pub(super) fn render(
    format: &ValueFormat<'_>,
    parameter: &BACnetEventParameter,
) -> (String, Fields) {
    use BACnetEventParameter as EP;
    let name = algorithm(parameter.tag());
    let mut fields = FieldList::new(format);
    fields.quiet("algorithm", named(u32::from(parameter.tag()), name.clone()));
    match parameter {
        EP::ChangeOfBitstring {
            time_delay,
            bitmask,
            list_of_values,
        } => {
            fields.delay(*time_delay);
            fields.bits("bitmask", "mask", (bitmask.0, &bitmask.1));
            let values = list_of_values
                .iter()
                .map(|(unused, data)| super::fields::bits_value(*unused, data))
                .collect();
            fields.list("list_of_bitstring_values", "alarm values", values);
        }
        EP::ChangeOfState {
            time_delay,
            list_of_values,
        } => {
            fields.delay(*time_delay);
            let values = list_of_values.iter().map(property_state_value).collect();
            fields.list("list_of_values", "alarm values", values);
        }
        EP::ChangeOfValue {
            time_delay,
            criteria,
        } => {
            fields.delay(*time_delay);
            match criteria {
                ChangeOfValueCriteria::Bitmask { unused_bits, data } => {
                    fields.bits("bitmask", "mask", (*unused_bits, data));
                }
                ChangeOfValueCriteria::ReferencedPropertyIncrement(increment) => {
                    fields.real("referenced_property_increment", "increment", *increment);
                }
            }
        }
        EP::CommandFailure {
            time_delay,
            feedback_property_reference,
        } => {
            fields.delay(*time_delay);
            fields.reference(
                "feedback_property_reference",
                "feedback",
                feedback_property_reference,
            );
        }
        EP::FloatingLimit {
            time_delay,
            setpoint_reference,
            low_diff_limit,
            high_diff_limit,
            deadband,
        } => {
            fields.delay(*time_delay);
            fields.reference("setpoint_reference", "setpoint", setpoint_reference);
            fields.real("low_diff_limit", "low diff", *low_diff_limit);
            fields.real("high_diff_limit", "high diff", *high_diff_limit);
            fields.real("deadband", "deadband", *deadband);
        }
        EP::OutOfRange {
            time_delay,
            low_limit,
            high_limit,
            deadband,
        } => {
            fields.delay(*time_delay);
            fields.real("low_limit", "low", *low_limit);
            fields.real("high_limit", "high", *high_limit);
            fields.real("deadband", "deadband", *deadband);
        }
        EP::ChangeOfLifeSafety {
            time_delay,
            list_of_life_safety_alarm_values,
            list_of_alarm_values,
            mode_property_reference,
        } => {
            fields.delay(*time_delay);
            fields.enumerations(
                "list_of_life_safety_alarm_values",
                "life-safety alarm values",
                life_safety(list_of_life_safety_alarm_values),
            );
            fields.enumerations(
                "list_of_alarm_values",
                "alarm values",
                life_safety(list_of_alarm_values),
            );
            fields.reference("mode_property_reference", "mode", mode_property_reference);
        }
        EP::Extended {
            vendor_id,
            extended_event_type,
            parameters,
        } => {
            let (text, datum) = (
                fields_text(parameters, format.floats),
                encoded_value(format, parameters).1,
            );
            fields.quiet("vendor_id", Datum::Unsigned(u64::from(*vendor_id)));
            fields.field(
                "extended_event_type",
                format!("vendor {vendor_id} type {extended_event_type}"),
                Datum::Unsigned(u64::from(*extended_event_type)),
            );
            fields.field("parameters", format!("parameters ({text})"), datum);
        }
        EP::BufferReady {
            notification_threshold,
            previous_notification_count,
        } => {
            fields.unsigned(
                "notification_threshold",
                "threshold",
                u64::from(*notification_threshold),
            );
            fields.unsigned(
                "previous_notification_count",
                "previous count",
                u64::from(*previous_notification_count),
            );
        }
        EP::UnsignedRange {
            time_delay,
            low_limit,
            high_limit,
        } => {
            fields.delay(*time_delay);
            fields.unsigned("low_limit", "low", *low_limit);
            fields.unsigned("high_limit", "high", *high_limit);
        }
        EP::AccessEvent {
            list_of_access_events,
            access_event_time_reference,
        } => {
            let events = list_of_access_events
                .iter()
                .map(|event| (event.to_raw(), kebab(event)));
            fields.enumerations("list_of_access_events", "access events", events);
            fields.reference(
                "access_event_time_reference",
                "time reference",
                access_event_time_reference,
            );
        }
        EP::DoubleOutOfRange {
            time_delay,
            low_limit,
            high_limit,
            deadband,
        } => {
            fields.delay(*time_delay);
            fields.double("low_limit", "low", *low_limit);
            fields.double("high_limit", "high", *high_limit);
            fields.double("deadband", "deadband", *deadband);
        }
        EP::SignedOutOfRange {
            time_delay,
            low_limit,
            high_limit,
            deadband,
        } => {
            fields.delay(*time_delay);
            fields.signed("low_limit", "low", *low_limit);
            fields.signed("high_limit", "high", *high_limit);
            fields.unsigned("deadband", "deadband", *deadband);
        }
        EP::UnsignedOutOfRange {
            time_delay,
            low_limit,
            high_limit,
            deadband,
        } => {
            fields.delay(*time_delay);
            fields.unsigned("low_limit", "low", *low_limit);
            fields.unsigned("high_limit", "high", *high_limit);
            fields.unsigned("deadband", "deadband", *deadband);
        }
        EP::ChangeOfCharacterstring {
            time_delay,
            list_of_alarm_values,
        } => {
            fields.delay(*time_delay);
            let values = list_of_alarm_values
                .iter()
                .map(|text| {
                    let value = PropertyValue::CharacterString(text.clone());
                    let shown = fields
                        .format()
                        .element(PropertyIdentifier::DESCRIPTION, &value);
                    (shown, Datum::Text(text.clone()))
                })
                .collect();
            fields.list("list_of_alarm_values", "alarm values", values);
        }
        EP::ChangeOfStatusFlags {
            time_delay,
            selected_flags,
        } => {
            fields.delay(*time_delay);
            fields.status_flags("selected_flags", "flags", *selected_flags);
        }
        EP::None => return ("none".into(), fields.finish().1),
        EP::ChangeOfDiscreteValue { time_delay } => fields.delay(*time_delay),
        EP::ChangeOfTimer {
            time_delay,
            alarm_values,
            update_time_reference,
        } => {
            fields.delay(*time_delay);
            let states = alarm_values
                .iter()
                .map(|state| (state.to_raw(), kebab(state)));
            fields.enumerations("alarm_values", "alarm values", states);
            fields.reference(
                "update_time_reference",
                "update time",
                update_time_reference,
            );
        }
        // A tag the standard doesn't assign: its fields by context tag.
        EP::Opaque { data, .. } => fields.field(
            "data",
            fields_text(data, format.floats),
            Datum::Raw { raw: hex(data, "") },
        ),
    }
    let (details, fields) = fields.finish();
    (format!("{name}: {details}"), fields)
}

/// Life safety states by name.
fn life_safety(states: &[LifeSafetyState]) -> impl Iterator<Item = (u32, String)> + '_ {
    states.iter().map(|state| (state.to_raw(), kebab(state)))
}

/// The Event_Parameters value at `offset`; the forms that aren't the
/// CHOICE are refused, for the generic view to show.
fn decode(octets: &[u8], offset: usize) -> Result<(BACnetEventParameter, usize), Error> {
    let (parameter, end) = decode_event_parameter(octets, offset)?;
    if let BACnetEventParameter::Opaque {
        tag: NOT_A_CHOICE, ..
    } = parameter
    {
        return Err(Error::decoding(offset, "not an Event_Parameters choice"));
    }
    Ok((parameter, end))
}

/// The element table's renderer.
pub(super) fn event_parameter(
    format: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    let (parameter, end) = decode(octets, offset)?;
    Ok((render(format, &parameter).0, end))
}

/// Each Event_Parameters value in `data` as typed data with its fields,
/// or `None` when `data` is empty or one doesn't decode.
pub(super) fn data(format: &ValueFormat<'_>, data: &[u8]) -> Option<Vec<Datum>> {
    if data.is_empty() {
        return None;
    }
    let mut items = Vec::new();
    split_elements(data, |octets, offset| {
        decode(octets, offset).map(|(parameter, end)| {
            let (text, fields) = render(format, &parameter);
            items.push(Datum::Constructed {
                element: ELEMENT.into(),
                text,
                fields: Some(fields),
            });
            end
        })
    })?;
    Some(items)
}

#[cfg(test)]
#[path = "event_parameter_tests.rs"]
mod tests;
