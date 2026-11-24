//! The priority in control of a record's Present_Value (#2620): an
//! output's Current_Command_Priority as read, or a value object's worked
//! out from the Priority_Array the inventory reads of it anyway, both
//! written as the property would be, with the priority named.

use bacnet_encoding::primitives::{encode_app_null, encode_app_unsigned};
use bacnet_encoding::tags::{TagClass, app_tag, decode_tag, extract_context_value};
use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::PropertyValue as V;
use bytes::BytesMut;

use super::settings::{mistyped, undecodable};
use crate::Priority;
use crate::format::{Floats, PropertyError, StateTexts, Value, ValueFormat};

/// An object's Current_Command_Priority as read, its text naming the
/// priority, and what is wrong with it: one that is neither an Unsigned
/// nor NULL is a problem and no value; one outside 1 to 16 is kept with a
/// problem.
pub(super) fn read(
    object_type: ObjectType,
    mut value: Value,
) -> (Option<Value>, Option<PropertyError>) {
    if let Some(problem) = mistyped(object_type, P::CURRENT_COMMAND_PRIORITY, &value) {
        return (None, Some(problem));
    }
    let level = match value.decode() {
        Ok(V::Unsigned(level)) => Some(level),
        _ => None,
    };
    value.text = text(level);
    let outside = level.is_some_and(|level| !(1..=Priority::COUNT as u64).contains(&level));
    let problem = outside.then(|| PropertyError::Other("not a priority (1 to 16)".into()));
    (Some(value), problem)
}

/// The priority in control a value object's Priority_Array shows, as its
/// Current_Command_Priority would say it: the first of the sixteen slots
/// that holds a command, or NULL when none does. Only whether a slot is
/// NULL matters, so a slot is told by its tags alone, and a command this
/// stack can't convert (a string in a character set it doesn't read)
/// still counts. An array whose tags don't make sixteen values is the
/// error, saying so.
pub(super) fn from_array(object_type: ObjectType, array: &Value) -> Result<Value, PropertyError> {
    let slots = held(&array.raw).map_err(|(index, at)| {
        let why = undecodable(&array.raw, at);
        PropertyError::Other(format!("slot {} {why}", index + 1))
    })?;
    if slots.len() != Priority::COUNT {
        let held = match slots.len() {
            1 => "one value".to_string(),
            count => format!("{count} values"),
        };
        let slots = Priority::COUNT;
        return Err(PropertyError::Other(format!("{held}, not {slots}")));
    }
    let level = slots
        .iter()
        .position(|&holds| holds)
        .map(|at| at as u64 + 1);
    let mut raw = BytesMut::new();
    match level {
        Some(level) => encode_app_unsigned(&mut raw, level),
        None => encode_app_null(&mut raw),
    }
    let texts = StateTexts::default();
    let format = ValueFormat::new(object_type, &texts, Floats::Exact);
    let mut value = Value::new(
        &format,
        P::CURRENT_COMMAND_PRIORITY,
        None,
        raw.to_vec(),
        None,
    );
    value.text = text(level);
    Ok(value)
}

/// The text of a priority in control: its number and what the project
/// calls it (`8 (operator, manual)`, `10 (site defined)`), `relinquish
/// default` when no priority holds a command, and the number alone when
/// it is no priority at all.
fn text(level: Option<u64>) -> String {
    let Some(level) = level else {
        return "relinquish default".into();
    };
    match u8::try_from(level)
        .ok()
        .and_then(|level| Priority::new(level).ok())
    {
        Some(priority) => format!("{level} ({})", priority.purpose()),
        None => level.to_string(),
    }
}

/// Whether each value in `data`, a Priority_Array's octets, holds a
/// command (is anything but NULL), told by its tags: an application value
/// is skipped by its length, a context-tagged one by its closing tag. The
/// error is the place of the value whose tags are broken and where it
/// starts.
fn held(data: &[u8]) -> Result<Vec<bool>, (usize, usize)> {
    let mut slots = Vec::with_capacity(Priority::COUNT);
    let mut offset = 0;
    while offset < data.len() {
        let broken = (slots.len(), offset);
        let (tag, start) = decode_tag(data, offset).map_err(|_| broken)?;
        let next = if tag.is_opening {
            let (_, after) = extract_context_value(data, start, tag.number).map_err(|_| broken)?;
            after
        } else if tag.is_closing {
            return Err(broken);
        } else if tag.class == TagClass::Application && tag.number == app_tag::BOOLEAN {
            // Its value is in the tag.
            start
        } else {
            let length = usize::try_from(tag.length).map_err(|_| broken)?;
            start
                .checked_add(length)
                .filter(|&end| end <= data.len())
                .ok_or(broken)?
        };
        let null = tag.class == TagClass::Application && tag.number == app_tag::NULL;
        slots.push(!null);
        offset = next;
    }
    Ok(slots)
}
