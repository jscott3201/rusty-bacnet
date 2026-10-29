//! The browser's display data from property values as read: the device
//! card, an object's properties, its health badges and its priority pane,
//! each value written by the shared formatter ([`bacnet_ops::format`]).

use bacnet_ops::engine::ReadVia;
use bacnet_ops::format::{Floats, RawRow, StateTexts, ValueFormat, decode_all, object_name, units};
use bacnet_ops::label::PRIORITIES;
use bacnet_types::enums::{ObjectType, PropertyIdentifier, ResolvedBits, resolve_bits};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use crate::tui::point::{DeviceCard, Health, ObjectView, PriorityView, PropertyRow};
use crate::tui::row_error::row_error_text;

/// The single value of `property` among `rows`.
fn single(rows: &[RawRow], property: PropertyIdentifier) -> Option<PropertyValue> {
    let (_, value) = rows.iter().find(|(p, _)| *p == property)?;
    let mut values = decode_all(value.as_ref().ok()?)?;
    (values.len() == 1).then(|| values.remove(0))
}

/// How many bits of a supported-services or object-types bit string are set.
fn supported(data: &[u8], property: PropertyIdentifier) -> Option<String> {
    let value = decode_all(data)?.into_iter().next()?;
    let count = match resolve_bits(property, &value)? {
        ResolvedBits::ServicesSupported(bits) => bits.iter().count(),
        ResolvedBits::ObjectTypesSupported(bits) => bits.iter().count(),
        _ => return None,
    };
    Some(format!("{count} supported"))
}

/// The device card from the Device object's properties. Strings show
/// without their quotes.
pub(crate) fn card(rows: &[RawRow], via: ReadVia) -> DeviceCard {
    const LABELS: [(PropertyIdentifier, &str); 13] = [
        (PropertyIdentifier::OBJECT_NAME, "Name"),
        (PropertyIdentifier::VENDOR_NAME, "Vendor"),
        (PropertyIdentifier::VENDOR_IDENTIFIER, "Vendor ID"),
        (PropertyIdentifier::MODEL_NAME, "Model"),
        (PropertyIdentifier::FIRMWARE_REVISION, "Firmware"),
        (PropertyIdentifier::APPLICATION_SOFTWARE_VERSION, "Software"),
        (PropertyIdentifier::PROTOCOL_VERSION, "Protocol version"),
        (PropertyIdentifier::PROTOCOL_REVISION, "Protocol revision"),
        (PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED, "Services"),
        (
            PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED,
            "Object types",
        ),
        (PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED, "Max APDU"),
        (PropertyIdentifier::SEGMENTATION_SUPPORTED, "Segmentation"),
        (PropertyIdentifier::DATABASE_REVISION, "Database revision"),
    ];
    let texts = StateTexts::default();
    let format = ValueFormat::new(ObjectType::DEVICE, &texts, Floats::Display);
    let name = object_name(rows);
    let rows = LABELS
        .iter()
        .map(|&(property, label)| {
            let text = match rows.iter().find(|(p, _)| *p == property) {
                Some((_, Ok(data))) => {
                    supported(data, property).unwrap_or_else(|| format.plain(property, data))
                }
                Some((_, Err(error))) => format!("({})", row_error_text(error)),
                None => "-".into(),
            };
            (label, text)
        })
        .collect();
    DeviceCard {
        rows,
        via: Some(via),
        name,
    }
}

/// The health badges: Status_Flags as the device reports them, or, for an
/// object without it, what Event_State, Reliability and Out_Of_Service say
/// (Clause 12 defines each flag from those properties).
fn health(rows: &[RawRow]) -> Health {
    if let Some(PropertyValue::BitString { data, .. }) =
        single(rows, PropertyIdentifier::STATUS_FLAGS)
    {
        let flags = bacnet_types::bitstring::status_flags_from_bacnet(&data);
        return Health {
            alarm: flags.contains(StatusFlags::IN_ALARM),
            fault: flags.contains(StatusFlags::FAULT),
            overridden: flags.contains(StatusFlags::OVERRIDDEN),
            out_of_service: flags.contains(StatusFlags::OUT_OF_SERVICE),
        };
    }
    let nonzero =
        |property| matches!(single(rows, property), Some(PropertyValue::Enumerated(n)) if n != 0);
    Health {
        alarm: nonzero(PropertyIdentifier::EVENT_STATE),
        fault: nonzero(PropertyIdentifier::RELIABILITY),
        overridden: false,
        out_of_service: matches!(
            single(rows, PropertyIdentifier::OUT_OF_SERVICE),
            Some(PropertyValue::Boolean(true))
        ),
    }
}

/// The priority pane, for an object with a Priority_Array of 16 values.
fn priority(rows: &[RawRow], format: &ValueFormat<'_>) -> Option<PriorityView> {
    let (_, data) = rows
        .iter()
        .find(|(p, _)| *p == PropertyIdentifier::PRIORITY_ARRAY)?;
    let values = decode_all(data.as_ref().ok()?)?;
    if values.len() != PRIORITIES {
        return None;
    }
    let mut view = PriorityView::default();
    for (slot, value) in view.slots.iter_mut().zip(&values) {
        if *value != PropertyValue::Null {
            *slot = Some(format.element(PropertyIdentifier::PRIORITY_ARRAY, value));
        }
    }
    let data = |property| {
        rows.iter()
            .find(|(p, _)| *p == property)
            .and_then(|(_, value)| value.as_deref().ok())
    };
    view.relinquish_default = data(PropertyIdentifier::RELINQUISH_DEFAULT)
        .map(|data| format.value(PropertyIdentifier::RELINQUISH_DEFAULT, data));
    view.present_value = data(PropertyIdentifier::PRESENT_VALUE)
        .map(|data| format.with_units(PropertyIdentifier::PRESENT_VALUE, data, units(rows)));
    Some(view)
}

/// One object's view from its properties: values for the screen, with
/// floats rounded, the object's state texts, and units on Present_Value.
pub(crate) fn object_view(object: ObjectIdentifier, rows: &[RawRow], via: ReadVia) -> ObjectView {
    let texts = StateTexts::from_rows(rows);
    let format = ValueFormat::new(object.object_type(), &texts, Floats::Display);
    let units = units(rows);
    let priority = priority(rows, &format);
    let properties = rows
        .iter()
        .map(|(property, value)| PropertyRow {
            property: *property,
            value: match value {
                Ok(_) if *property == PropertyIdentifier::PRIORITY_ARRAY && priority.is_some() => {
                    Ok(match priority.as_ref().and_then(PriorityView::winner) {
                        Some(slot) => format!("priority {slot} in control"),
                        None => "all relinquished".into(),
                    })
                }
                Ok(data) if *property == PropertyIdentifier::PRESENT_VALUE => {
                    Ok(format.with_units(*property, data, units))
                }
                Ok(data) => Ok(format.value(*property, data)),
                Err(error) => Err(row_error_text(error)),
            },
        })
        .collect();
    ObjectView {
        object,
        properties,
        health: health(rows),
        priority,
        via,
        name: object_name(rows),
    }
}

#[cfg(test)]
mod tests {
    use bacnet_encoding::primitives::encode_property_value;
    use bytes::BytesMut;

    use super::*;

    fn encoded(values: &[PropertyValue]) -> Vec<u8> {
        let mut buf = BytesMut::new();
        for value in values {
            encode_property_value(&mut buf, value).unwrap();
        }
        buf.to_vec()
    }

    #[test]
    fn health_comes_from_status_flags_or_the_properties_behind_them() {
        let flags = PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0b0101_0000],
        };
        let rows = vec![(PropertyIdentifier::STATUS_FLAGS, Ok(encoded(&[flags])))];
        let flagged = health(&rows);
        assert!(flagged.fault && flagged.out_of_service && !flagged.alarm && !flagged.overridden);
        // Without Status_Flags, the properties behind them.
        let rows = vec![
            (
                PropertyIdentifier::EVENT_STATE,
                Ok(encoded(&[PropertyValue::Enumerated(2)])),
            ),
            (
                PropertyIdentifier::OUT_OF_SERVICE,
                Ok(encoded(&[PropertyValue::Boolean(true)])),
            ),
        ];
        assert_eq!(health(&rows).badges(), ["[ALM]", "[OOS]"]);
    }

    #[test]
    fn a_priority_array_needs_sixteen_values() {
        let mut slots = vec![PropertyValue::Null; 16];
        slots[9] = PropertyValue::Real(55.0);
        let rows = vec![
            (PropertyIdentifier::PRIORITY_ARRAY, Ok(encoded(&slots))),
            (
                PropertyIdentifier::RELINQUISH_DEFAULT,
                Ok(encoded(&[PropertyValue::Real(0.0)])),
            ),
        ];
        let texts = StateTexts::default();
        let format = ValueFormat::new(ObjectType::ANALOG_OUTPUT, &texts, Floats::Display);
        let view = priority(&rows, &format).unwrap();
        assert_eq!(view.winner(), Some(10));
        assert_eq!(view.slots[9].as_deref(), Some("55"));
        assert_eq!(view.relinquish_default.as_deref(), Some("0"));
        let short = vec![(PropertyIdentifier::PRIORITY_ARRAY, Ok(encoded(&slots[..4])))];
        assert!(priority(&short, &format).is_none());
    }
}

#[cfg(test)]
mod card_tests {
    use bacnet_encoding::primitives::encode_property_value;
    use bytes::BytesMut;

    use super::*;

    fn encoded(value: &PropertyValue) -> Vec<u8> {
        let mut buf = BytesMut::new();
        encode_property_value(&mut buf, value).unwrap();
        buf.to_vec()
    }

    #[test]
    fn the_card_shows_strings_without_quotes() {
        let rows = vec![
            (
                PropertyIdentifier::OBJECT_NAME,
                Ok(encoded(&PropertyValue::CharacterString("AHU-1".into()))),
            ),
            (
                PropertyIdentifier::SEGMENTATION_SUPPORTED,
                Ok(encoded(&PropertyValue::Enumerated(3))),
            ),
        ];
        let card = card(&rows, ReadVia::Rpm);
        assert!(card.rows.contains(&("Name", "AHU-1".into())), "{card:?}");
        assert!(
            card.rows.contains(&("Segmentation", "none".into())),
            "{card:?}"
        );
    }

    #[test]
    fn a_binary_priority_array_names_its_states_with_the_object_texts() {
        let mut slots = vec![PropertyValue::Null; 16];
        slots[7] = PropertyValue::Enumerated(1);
        let text = |s: &str| Ok(encoded(&PropertyValue::CharacterString(s.into())));
        let rows = vec![
            (
                PropertyIdentifier::PRESENT_VALUE,
                Ok(encoded(&PropertyValue::Enumerated(1))),
            ),
            (
                PropertyIdentifier::PRIORITY_ARRAY,
                Ok({
                    let mut buf = BytesMut::new();
                    for slot in &slots {
                        encode_property_value(&mut buf, slot).unwrap();
                    }
                    buf.to_vec()
                }),
            ),
            (
                PropertyIdentifier::RELINQUISH_DEFAULT,
                Ok(encoded(&PropertyValue::Enumerated(0))),
            ),
            (PropertyIdentifier::ACTIVE_TEXT, text("On")),
            (PropertyIdentifier::INACTIVE_TEXT, text("Off")),
        ];
        let object = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1).unwrap();
        let view = object_view(object, &rows, ReadVia::Rpm);
        let priority = view.priority.as_ref().unwrap();
        assert_eq!(priority.slots[7].as_deref(), Some("active (On)"));
        assert_eq!(
            priority.relinquish_default.as_deref(),
            Some("inactive (Off)")
        );
        assert_eq!(
            view.value(PropertyIdentifier::PRESENT_VALUE),
            Some("active (On)")
        );
    }
}
