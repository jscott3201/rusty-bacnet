//! A [`PointRecord`] from one object's rows as read.

use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::ObjectIdentifier;

use super::control;
use super::plan::required;
use super::settings::{mistyped, refused, state_text};
use super::{PointRecord, Problem};
use crate::device::DeviceAddress;
use crate::format::{PropertyError, RawRow, StateTexts, Value, units};
use crate::reading::readings;
use crate::write::{Commandability, unknown_property};

/// `object`'s record from `rows`, the properties its read asked for, each
/// its value or its error. The first row of a property counts. A value of
/// the wrong type is a problem and stays out of the record; a setting a
/// point list builder would refuse is kept and is a problem too
/// ([`refused`]). Read with
/// values (the rows hold a Present_Value), a value object with a
/// Priority_Array gets its priority in control from that array.
pub(crate) fn record(
    device: &DeviceAddress,
    device_instance: u32,
    object: ObjectIdentifier,
    rows: Vec<RawRow>,
) -> PointRecord {
    let object_type = object.object_type();
    let mut record = PointRecord::new(device.clone(), device_instance, object);
    // Device text exactly, an empty text included: the values' own text
    // names states by the cleaned form.
    record.state_texts.states = StateTexts::from_rows(&rows).states;
    record.units = units(&rows);
    let mut priorities: Option<Value> = None;
    let mut seen: Vec<P> = Vec::with_capacity(rows.len());
    for reading in readings(rows.into_iter().map(|row| (object, None, row)).collect()) {
        let property = reading.property;
        if seen.contains(&property) {
            continue;
        }
        seen.push(property);
        let value = match reading.value {
            Ok(value) => value,
            // An optional property the object doesn't have.
            Err(error) if unknown_property(&error) && !required(object_type, property) => continue,
            Err(error) => {
                record.problems.push(Problem::new(property, error));
                continue;
            }
        };
        let wrong = |what: &str| PropertyError::Other(format!("not {what}"));
        let problem = match property {
            P::OBJECT_NAME => text(&value, &mut record.name),
            P::DESCRIPTION => text(&value, &mut record.description),
            P::DEVICE_TYPE => typed_text(object_type, property, &value, &mut record.device_type),
            P::PROFILE_NAME => typed_text(object_type, property, &value, &mut record.profile_name),
            P::ACTIVE_TEXT => text(&value, &mut record.state_texts.active),
            P::INACTIVE_TEXT => text(&value, &mut record.state_texts.inactive),
            P::UNITS => record.units.is_none().then(|| wrong("an engineering unit")),
            P::NUMBER_OF_STATES => {
                record.number_of_states = value.as_u64().and_then(|n| u32::try_from(n).ok());
                record
                    .number_of_states
                    .is_none()
                    .then(|| wrong("an Unsigned"))
            }
            P::PRIORITY_ARRAY => {
                priorities = Some(value);
                None
            }
            P::RELINQUISH_DEFAULT => {
                typed(object_type, property, value, &mut record.relinquish_default)
            }
            P::COV_INCREMENT => typed(object_type, property, value, &mut record.cov_increment),
            P::MIN_PRES_VALUE => typed(object_type, property, value, &mut record.min_present_value),
            P::MAX_PRES_VALUE => typed(object_type, property, value, &mut record.max_present_value),
            P::POLARITY => typed(object_type, property, value, &mut record.polarity),
            P::PRESENT_VALUE => keep(value, &mut record.present_value),
            // As its siblings: one that isn't a bit string is a problem
            // and stays out, so `PointRecord::flags` sees only bit strings.
            P::STATUS_FLAGS => typed(object_type, property, value, &mut record.status_flags),
            P::EVENT_STATE => typed(object_type, property, value, &mut record.event_state),
            P::RELIABILITY => typed(object_type, property, value, &mut record.reliability),
            P::CURRENT_COMMAND_PRIORITY => {
                let (kept, problem) = control::read(object_type, value);
                record.command_priority = kept;
                problem
            }
            // State_Text is read from the rows above, an entry that isn't
            // a string as an empty text.
            P::STATE_TEXT => state_text(object_type, &value),
            _ => None,
        };
        if let Some(error) = problem {
            record.problems.push(Problem::new(property, error));
        }
    }
    record.commandable = match Commandability::of(object_type, P::PRESENT_VALUE) {
        Commandability::Always => true,
        Commandability::Never => false,
        // Only a Priority_Array says so.
        _ => priorities.is_some(),
    };
    // A value object's priority in control, with its values: what its
    // Priority_Array shows, as no Current_Command_Priority is asked of it.
    let values = seen.contains(&P::PRESENT_VALUE);
    if values
        && !seen.contains(&P::CURRENT_COMMAND_PRIORITY)
        && let Some(array) = &priorities
    {
        match control::from_array(object_type, array) {
            Ok(value) => record.command_priority = Some(value),
            Err(error) => record.problems.push(Problem::new(P::PRIORITY_ARRAY, error)),
        }
    }
    // What a point list builder would refuse; a relinquish default only
    // where the point holds one, so after `commandable`.
    record.problems.extend(refused(&record));
    // In the order asked, the builder's refusals among the rest.
    record
        .problems
        .sort_by_key(|problem| seen.iter().position(|&asked| asked == problem.property));
    record
}

/// Put `value` in `field`.
fn keep(value: Value, field: &mut Option<Value>) -> Option<PropertyError> {
    *field = Some(value);
    None
}

/// Put `value` in `field` when it is of the datatype `property` has on an
/// object of `object_type`; a value that isn't is the problem returned,
/// and `field` stays empty, so the export finds nothing there either.
fn typed(
    object_type: ObjectType,
    property: P,
    value: Value,
    field: &mut Option<Value>,
) -> Option<PropertyError> {
    let problem = mistyped(object_type, property, &value);
    if problem.is_none() {
        *field = Some(value);
    }
    problem
}

/// Put `value`'s character string in `field`, exactly as the device sent
/// it, when it is of the datatype `property` has on an object of
/// `object_type`; a value that isn't is the problem returned, saying what
/// came back.
fn typed_text(
    object_type: ObjectType,
    property: P,
    value: &Value,
    field: &mut Option<String>,
) -> Option<PropertyError> {
    mistyped(object_type, property, value).or_else(|| text(value, field))
}

/// Put `value`'s character string in `field`, exactly as the device sent
/// it; a value that isn't one is the problem returned.
fn text(value: &Value, field: &mut Option<String>) -> Option<PropertyError> {
    *field = value.as_str().map(str::to_string);
    field
        .is_none()
        .then(|| PropertyError::Other("not a character string".into()))
}
