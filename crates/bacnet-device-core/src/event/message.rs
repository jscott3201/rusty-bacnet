//! The message text an intrinsic transition commits.

use alloc::string::String;
use bacnet_objects::event::EventStateChange;
use bacnet_types::primitives::ObjectIdentifier;

/// Select the server-owned message for one intrinsic transition: the
/// object, then the two states, as `ANALOG_INPUT,1: NORMAL -> HIGH_LIMIT`.
/// A type or state without a name reads as its number.
///
/// Built from literal fragments rather than formatted, so a firmware image
/// links no `core::fmt` for it (#2187).
pub fn intrinsic_event_message_text(
    object_identifier: &ObjectIdentifier,
    change: &EventStateChange,
) -> String {
    let object_type = object_identifier.object_type();
    let mut text = String::new();
    push_name(&mut text, object_type.name(), object_type.to_raw());
    text.push(',');
    push_decimal(&mut text, object_identifier.instance_number());
    text.push_str(": ");
    push_name(&mut text, change.from.name(), change.from.to_raw());
    text.push_str(" -> ");
    push_name(&mut text, change.to.name(), change.to.to_raw());
    text
}

/// An enumeration's name, or its number when it has none, as its `Display`
/// writes it.
fn push_name(text: &mut String, name: Option<&str>, raw: u32) {
    match name {
        Some(name) => text.push_str(name),
        None => push_decimal(text, raw),
    }
}

/// `value` in decimal, most significant digit first.
fn push_decimal(text: &mut String, mut value: u32) {
    let mut digits = [0u8; 10];
    let mut len = 0;
    loop {
        digits[len] = b'0' + (value % 10) as u8;
        len += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    for &digit in digits[..len].iter().rev() {
        text.push(char::from(digit));
    }
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::{EventState, ObjectType};

    use super::*;

    fn formatted(oid: &ObjectIdentifier, change: &EventStateChange) -> String {
        format!("{oid}: {} -> {}", change.from, change.to)
    }

    /// The text is the octets the server formatted before the move, for
    /// every named object type and event state and for unnamed values.
    #[test]
    fn literal_fragments_match_the_formatted_text() {
        let mut types: Vec<ObjectType> = ObjectType::ALL_NAMED.iter().map(|(_, t)| *t).collect();
        types.extend([ObjectType::from_raw(128), ObjectType::from_raw(1023)]);
        let mut states: Vec<EventState> = EventState::ALL_NAMED.iter().map(|(_, s)| *s).collect();
        states.extend([EventState::from_raw(64), EventState::from_raw(u32::MAX)]);
        for object_type in types {
            for instance in [0, 1, 9, 10, 4_194_302, 4_194_303] {
                let Ok(oid) = ObjectIdentifier::new(object_type, instance) else {
                    continue;
                };
                for from in &states {
                    for to in &states {
                        let change = EventStateChange {
                            from: *from,
                            to: *to,
                        };
                        assert_eq!(
                            intrinsic_event_message_text(&oid, &change),
                            formatted(&oid, &change)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn decimal_covers_the_whole_range() {
        for value in [0, 1, 9, 10, 99, 100, 65_535, 4_194_303, u32::MAX] {
            let mut text = String::new();
            push_decimal(&mut text, value);
            assert_eq!(text, value.to_string());
        }
    }
}
