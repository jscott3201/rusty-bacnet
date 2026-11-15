//! A [`PointRecord`] from one object's rows as read.

use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::primitives::ObjectIdentifier;

use super::plan::required;
use super::{PointRecord, Problem};
use crate::device::DeviceAddress;
use crate::format::{PropertyError, RawRow, StateTexts, Value, units};
use crate::reading::readings;
use crate::write::{Commandability, unknown_property};

/// `object`'s record from `rows`, the properties its read asked for, each
/// its value or its error. The first row of a property counts.
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
    let mut has_priorities = false;
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
                has_priorities = true;
                None
            }
            P::RELINQUISH_DEFAULT => keep(value, &mut record.relinquish_default),
            P::COV_INCREMENT => keep(value, &mut record.cov_increment),
            P::MIN_PRES_VALUE => keep(value, &mut record.min_present_value),
            P::MAX_PRES_VALUE => keep(value, &mut record.max_present_value),
            P::PRESENT_VALUE => keep(value, &mut record.present_value),
            P::STATUS_FLAGS => keep(value, &mut record.status_flags),
            // State_Text is read from the rows above.
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
        _ => has_priorities,
    };
    record
}

/// Put `value` in `field`.
fn keep(value: Value, field: &mut Option<Value>) -> Option<PropertyError> {
    *field = Some(value);
    None
}

/// Put `value`'s character string in `field`, exactly as the device sent
/// it; a value that isn't one is the problem returned.
fn text(value: &Value, field: &mut Option<String>) -> Option<PropertyError> {
    *field = value.as_str().map(str::to_string);
    field
        .is_none()
        .then(|| PropertyError::Other("not a character string".into()))
}
