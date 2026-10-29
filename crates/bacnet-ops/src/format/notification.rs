//! An event notification's values (its `event_values`, the
//! BACnetNotificationParameters choice) for every event type the stack
//! decodes (#2073): as text, `exceeding value 101.5, flags in-alarm,
//! deadband 1, limit 100`, and as typed data with each field by name. The
//! CLI's Event Log records, the terminal UI and the MCP server share them.

use bacnet_encoding::constructed::{decode_authentication_factor, decode_bacnet_property_value};
use bacnet_types::constructed::{
    BACnetDeviceObjectReference, BACnetPropertyValue, ChangeOfValueChoice, NotificationParameters,
};
use bacnet_types::enums::{EventType, ObjectType};
use bacnet_types::primitives::{Date, PropertyValue, Time};

use super::commands::factor_text;
use super::constructed::object_reference;
use super::fields::{
    FieldList, bits_value, encoded_value, named, property_state_value, reference_value,
};
use super::primitive::{date_time, hex};
use super::{Datum, Fields, Floats, StateTexts, ValueFormat};
use crate::label::kebab;

/// The element kind the JSON names the values by.
const ELEMENT: &str = "notification-parameters";

/// Property values read from a change-of-reliability notification before
/// the rest is left as hex.
const MAX_PROPERTY_VALUES: usize = 256;

/// An event notification's values, decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct EventValues {
    /// The event type they belong to, kebab case: `out-of-range`.
    pub event_type: String,
    /// The fields as text, `exceeding value 101.5, flags in-alarm, ...`;
    /// empty for a type with none.
    pub details: String,
    /// The values as typed data: a `notification-parameters`
    /// [`Datum::Constructed`] whose text is `event_type: details` and whose
    /// fields start with `event_type` (an enumerated), then each value by
    /// its snake-case name.
    pub data: Datum,
}

impl EventValues {
    /// The values of one notification, every digit kept.
    pub fn new(parameters: &NotificationParameters) -> Self {
        Self::with_floats(parameters, Floats::Exact)
    }

    /// [`Self::new`] with floats written `floats`.
    pub fn with_floats(parameters: &NotificationParameters, floats: Floats) -> Self {
        let texts = StateTexts::default();
        let format = ValueFormat::new(ObjectType::DEVICE, &texts, floats).whole_lists();
        let (event_type, details, fields) = render(&format, parameters);
        let text = if details.is_empty() {
            event_type.clone()
        } else {
            format!("{event_type}: {details}")
        };
        Self {
            event_type,
            details,
            data: Datum::Constructed {
                element: ELEMENT.into(),
                text,
                fields: Some(fields),
            },
        }
    }

    /// The text of [`Self::data`], `out-of-range: exceeding value 101.5, ...`.
    pub fn text(&self) -> &str {
        match &self.data {
            Datum::Constructed { text, .. } => text,
            _ => &self.details,
        }
    }
}

/// The event type a choice of the notification parameters reports.
fn event_type(parameters: &NotificationParameters) -> u32 {
    use NotificationParameters as N;
    match parameters {
        N::ChangeOfBitstring { .. } => 0,
        N::ChangeOfState { .. } => 1,
        N::ChangeOfValue { .. } => 2,
        N::CommandFailure { .. } => 3,
        N::FloatingLimit { .. } => 4,
        N::OutOfRange { .. } => 5,
        N::ComplexEventType { .. } => 6,
        N::ChangeOfLifeSafety { .. } => 8,
        N::Extended { .. } => 9,
        N::BufferReady { .. } => 10,
        N::UnsignedRange { .. } => 11,
        N::AccessEvent { .. } => 13,
        N::DoubleOutOfRange { .. } => 14,
        N::SignedOutOfRange { .. } => 15,
        N::UnsignedOutOfRange { .. } => 16,
        N::ChangeOfCharacterstring { .. } => 17,
        N::ChangeOfStatusFlags { .. } => 18,
        N::ChangeOfReliability { .. } => 19,
        N::ChangeOfDiscreteValue { .. } => 21,
        N::ChangeOfTimer { .. } => 22,
    }
}

/// An enumerated value by name.
fn enumerated(value: u32, name: impl std::fmt::Display) -> (String, Datum) {
    let name = kebab(name);
    (name.clone(), named(value, name))
}

/// A date and time, as text and as a `date-time` element.
fn date_and_time(date: &Date, time: &Time) -> (String, Datum) {
    let text = date_time(date, time);
    let datum = Datum::Constructed {
        element: "date-time".into(),
        text: text.clone(),
        fields: None,
    };
    (text, datum)
}

/// A device object reference, as text and as its element.
fn object(reference: &BACnetDeviceObjectReference) -> (String, Datum) {
    let text = object_reference(reference);
    let datum = Datum::Constructed {
        element: "device-object-reference".into(),
        text: text.clone(),
        fields: None,
    };
    (text, datum)
}

/// One reported property value: `present-value[2] = 72.5 @8`, and the
/// same by field.
fn property_value(format: &ValueFormat<'_>, value: &BACnetPropertyValue) -> (String, Datum) {
    let (shown, datum) = encoded_value(format, &value.value);
    let mut property = kebab(value.property_identifier);
    if let Some(index) = value.property_array_index {
        property.push_str(&format!("[{index}]"));
    }
    let mut text = format!("{property} = {shown}");
    if let Some(priority) = value.priority {
        text.push_str(&format!(" @{priority}"));
    }
    let fields = Fields::new(vec![
        (
            "property_identifier".into(),
            named(
                value.property_identifier.to_raw(),
                kebab(value.property_identifier),
            ),
        ),
        (
            "property_array_index".into(),
            value
                .property_array_index
                .map_or(Datum::Null, |index| Datum::Unsigned(u64::from(index))),
        ),
        ("value".into(), datum),
        (
            "priority".into(),
            value
                .priority
                .map_or(Datum::Null, |priority| Datum::Unsigned(u64::from(priority))),
        ),
    ]);
    let datum = Datum::Constructed {
        element: "property-value".into(),
        text: text.clone(),
        fields: Some(fields),
    };
    (text, datum)
}

/// The property values of a change-of-reliability notification, each
/// decoded, or the octets by tag when they don't decode.
fn property_values(format: &ValueFormat<'_>, octets: &[u8]) -> Option<Vec<(String, Datum)>> {
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < octets.len() {
        if values.len() >= MAX_PROPERTY_VALUES {
            return None;
        }
        let (value, next) = decode_bacnet_property_value(octets, offset).ok()?;
        values.push(property_value(format, &value));
        offset = next;
    }
    Some(values)
}

/// The event type's name, the fields' text, and the fields.
fn render(
    format: &ValueFormat<'_>,
    parameters: &NotificationParameters,
) -> (String, String, Fields) {
    use NotificationParameters as N;
    let raw = event_type(parameters);
    // Event type 6 has no name of its own: the standard leaves the number
    // to proprietary algorithms, which report through this choice.
    let name = match parameters {
        N::ComplexEventType { .. } => "complex-event-type".to_string(),
        _ => kebab(EventType::from_raw(raw)),
    };
    let mut f = FieldList::new(format);
    f.quiet("event_type", named(raw, name.clone()));
    match parameters {
        N::ChangeOfBitstring {
            referenced_bitstring,
            status_flags,
        } => {
            let (unused, data) = referenced_bitstring;
            f.labelled("referenced_bitstring", "bits", bits_value(*unused, data));
            f.status_flags("status_flags", "flags", *status_flags);
        }
        N::ChangeOfState {
            new_state,
            status_flags,
        } => {
            f.labelled("new_state", "state", property_state_value(new_state));
            f.status_flags("status_flags", "flags", *status_flags);
        }
        N::ChangeOfValue {
            new_value,
            status_flags,
        } => {
            match new_value {
                ChangeOfValueChoice::ChangedBits { unused_bits, data } => {
                    f.labelled(
                        "changed_bits",
                        "changed bits",
                        bits_value(*unused_bits, data),
                    );
                }
                ChangeOfValueChoice::ChangedValue(value) => {
                    f.real("changed_value", "value", *value);
                }
            }
            f.status_flags("status_flags", "flags", *status_flags);
        }
        N::CommandFailure {
            command_value,
            status_flags,
            feedback_value,
        } => {
            f.labelled(
                "command_value",
                "commanded",
                encoded_value(format, command_value),
            );
            f.status_flags("status_flags", "flags", *status_flags);
            f.labelled(
                "feedback_value",
                "feedback",
                encoded_value(format, feedback_value),
            );
        }
        N::FloatingLimit {
            reference_value,
            status_flags,
            setpoint_value,
            error_limit,
        } => {
            f.real("reference_value", "value", *reference_value);
            f.status_flags("status_flags", "flags", *status_flags);
            f.real("setpoint_value", "setpoint", *setpoint_value);
            f.real("error_limit", "error limit", *error_limit);
        }
        N::OutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        } => {
            f.real("exceeding_value", "exceeding value", *exceeding_value);
            f.status_flags("status_flags", "flags", *status_flags);
            f.real("deadband", "deadband", *deadband);
            f.real("exceeded_limit", "limit", *exceeded_limit);
        }
        N::ComplexEventType { property_values } => {
            let values = property_values
                .iter()
                .map(|value| property_value(format, value))
                .collect();
            f.list("property_values", "values", values);
        }
        N::ChangeOfLifeSafety {
            new_state,
            new_mode,
            status_flags,
            operation_expected,
        } => {
            f.labelled(
                "new_state",
                "state",
                enumerated(new_state.to_raw(), new_state),
            );
            f.labelled("new_mode", "mode", enumerated(new_mode.to_raw(), new_mode));
            f.status_flags("status_flags", "flags", *status_flags);
            f.labelled(
                "operation_expected",
                "operation",
                enumerated(operation_expected.to_raw(), operation_expected),
            );
        }
        N::Extended {
            vendor_id,
            extended_event_type,
            parameters,
        } => {
            f.quiet("vendor_id", Datum::Unsigned(u64::from(*vendor_id)));
            f.field(
                "extended_event_type",
                format!("vendor {vendor_id} type {extended_event_type}"),
                Datum::Unsigned(u64::from(*extended_event_type)),
            );
            f.labelled(
                "parameters",
                "parameters",
                encoded_value(format, parameters),
            );
        }
        N::BufferReady {
            buffer_property,
            previous_notification,
            current_notification,
        } => {
            f.labelled(
                "buffer_property",
                "buffer",
                reference_value(buffer_property),
            );
            f.unsigned(
                "previous_notification",
                "previous",
                u64::from(*previous_notification),
            );
            f.unsigned(
                "current_notification",
                "current",
                u64::from(*current_notification),
            );
        }
        N::UnsignedRange {
            exceeding_value,
            status_flags,
            exceeded_limit,
        } => {
            f.unsigned("exceeding_value", "exceeding value", *exceeding_value);
            f.status_flags("status_flags", "flags", *status_flags);
            f.unsigned("exceeded_limit", "limit", *exceeded_limit);
        }
        N::AccessEvent {
            access_event,
            status_flags,
            access_event_tag,
            access_event_time,
            access_credential,
            authentication_factor,
        } => {
            f.labelled(
                "access_event",
                "event",
                enumerated(access_event.to_raw(), access_event),
            );
            f.status_flags("status_flags", "flags", *status_flags);
            f.unsigned("access_event_tag", "tag", u64::from(*access_event_tag));
            let (date, time) = access_event_time;
            f.labelled("access_event_time", "time", date_and_time(date, time));
            f.labelled("access_credential", "credential", object(access_credential));
            match authentication_factor {
                Some(octets) => {
                    let pair = match decode_authentication_factor(octets, 0) {
                        Ok((factor, end)) if end == octets.len() => {
                            let text = factor_text(&factor);
                            let datum = Datum::Constructed {
                                element: "authentication-factor".into(),
                                text: text.clone(),
                                fields: None,
                            };
                            (text, datum)
                        }
                        _ => encoded_value(format, octets),
                    };
                    f.labelled("authentication_factor", "factor", pair);
                }
                None => f.quiet("authentication_factor", Datum::Null),
            }
        }
        N::DoubleOutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        } => {
            f.double("exceeding_value", "exceeding value", *exceeding_value);
            f.status_flags("status_flags", "flags", *status_flags);
            f.double("deadband", "deadband", *deadband);
            f.double("exceeded_limit", "limit", *exceeded_limit);
        }
        N::SignedOutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        } => {
            f.signed("exceeding_value", "exceeding value", *exceeding_value);
            f.status_flags("status_flags", "flags", *status_flags);
            f.unsigned("deadband", "deadband", *deadband);
            f.signed("exceeded_limit", "limit", *exceeded_limit);
        }
        N::UnsignedOutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        } => {
            f.unsigned("exceeding_value", "exceeding value", *exceeding_value);
            f.status_flags("status_flags", "flags", *status_flags);
            f.unsigned("deadband", "deadband", *deadband);
            f.unsigned("exceeded_limit", "limit", *exceeded_limit);
        }
        N::ChangeOfCharacterstring {
            changed_value,
            status_flags,
            alarm_value,
        } => {
            f.value(
                "changed_value",
                "value",
                &PropertyValue::CharacterString(changed_value.clone()),
            );
            f.status_flags("status_flags", "flags", *status_flags);
            f.value(
                "alarm_value",
                "alarm value",
                &PropertyValue::CharacterString(alarm_value.clone()),
            );
        }
        N::ChangeOfStatusFlags {
            present_value,
            referenced_flags,
        } => {
            match present_value {
                Some(octets) => f.labelled("present_value", "value", encoded_value(format, octets)),
                None => f.quiet("present_value", Datum::Null),
            }
            f.status_flags("referenced_flags", "flags", *referenced_flags);
        }
        N::ChangeOfReliability {
            reliability,
            status_flags,
            property_values: octets,
        } => {
            f.labelled(
                "reliability",
                "reliability",
                enumerated(reliability.to_raw(), reliability),
            );
            f.status_flags("status_flags", "flags", *status_flags);
            match property_values(format, octets) {
                Some(values) => f.list("property_values", "values", values),
                None => f.field(
                    "property_values",
                    format!("values {}", super::raw_text(octets)),
                    Datum::Raw {
                        raw: hex(octets, ""),
                    },
                ),
            }
        }
        N::ChangeOfDiscreteValue {
            new_value,
            status_flags,
        } => {
            f.labelled("new_value", "value", encoded_value(format, new_value));
            f.status_flags("status_flags", "flags", *status_flags);
        }
        N::ChangeOfTimer {
            new_state,
            status_flags,
            update_time,
            last_state_change,
            initial_timeout,
            expiration_time,
        } => {
            f.labelled(
                "new_state",
                "state",
                enumerated(new_state.to_raw(), new_state),
            );
            f.status_flags("status_flags", "flags", *status_flags);
            let (date, time) = update_time;
            f.labelled("update_time", "updated", date_and_time(date, time));
            match last_state_change {
                Some(change) => f.labelled(
                    "last_state_change",
                    "last change",
                    enumerated(change.to_raw(), change),
                ),
                None => f.quiet("last_state_change", Datum::Null),
            }
            match initial_timeout {
                Some(seconds) => f.field(
                    "initial_timeout",
                    format!("timeout {seconds} s"),
                    Datum::Unsigned(u64::from(*seconds)),
                ),
                None => f.quiet("initial_timeout", Datum::Null),
            }
            match expiration_time {
                Some((date, time)) => {
                    f.labelled("expiration_time", "expires", date_and_time(date, time))
                }
                None => f.quiet("expiration_time", Datum::Null),
            }
        }
    }
    let (details, fields) = f.finish();
    (name, details, fields)
}

#[cfg(test)]
#[path = "notification_tests.rs"]
mod tests;
