//! BACnet value parsing and formatting utilities.
//!
//! Shared between the HTTP API and MCP modules. No HTTP or MCP dependencies —
//! only bacnet-types, bacnet-encoding, and serde_json.

use bacnet_encoding::primitives::decode_application_value;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;

/// Parse an object specifier like "analog-input:1" into (ObjectType, instance).
pub fn parse_object_specifier(s: &str) -> Result<(ObjectType, u32), String> {
    let (type_str, inst_str) = s
        .rsplit_once(':')
        .ok_or_else(|| format!("expected 'type:instance' format, got '{s}'"))?;

    let obj_type = parse_object_type(type_str)?;
    let instance = inst_str
        .parse::<u32>()
        .map_err(|_| format!("invalid instance number: '{inst_str}'"))?;

    Ok((obj_type, instance))
}

/// Parse an object type name like "analog-input" into ObjectType.
pub fn parse_object_type(s: &str) -> Result<ObjectType, String> {
    let s = s.trim();
    if let Ok(n) = s.parse::<u32>() {
        return Ok(ObjectType::from_raw(n));
    }
    let normalized = s.to_ascii_lowercase().replace('-', "_");
    for &(name, val) in ObjectType::ALL_NAMED {
        if name.eq_ignore_ascii_case(&normalized) {
            return Ok(val);
        }
    }
    Err(format!("unknown object type: '{s}'"))
}

/// Parse a property name like "present-value" into PropertyIdentifier.
pub fn parse_property_name(s: &str) -> Result<PropertyIdentifier, String> {
    let s = s.trim();
    if let Ok(n) = s.parse::<u32>() {
        return Ok(PropertyIdentifier::from_raw(n));
    }
    let normalized = s.to_ascii_lowercase().replace('-', "_");
    for &(name, val) in PropertyIdentifier::ALL_NAMED {
        if name.eq_ignore_ascii_case(&normalized) {
            return Ok(val);
        }
    }
    Err(format!("unknown property: '{s}'"))
}

/// Serialize a PropertyValue to a JSON-friendly representation.
pub fn property_value_to_json(value: &PropertyValue) -> serde_json::Value {
    match value {
        PropertyValue::Null => serde_json::Value::Null,
        PropertyValue::Boolean(b) => serde_json::json!({ "type": "boolean", "value": b }),
        PropertyValue::Unsigned(n) => serde_json::json!({ "type": "unsigned", "value": n }),
        PropertyValue::Signed(n) => serde_json::json!({ "type": "signed", "value": n }),
        PropertyValue::Real(f) => serde_json::json!({ "type": "real", "value": f }),
        PropertyValue::Double(f) => serde_json::json!({ "type": "double", "value": f }),
        PropertyValue::CharacterString(s) => {
            serde_json::json!({ "type": "string", "value": s })
        }
        PropertyValue::Enumerated(e) => serde_json::json!({ "type": "enumerated", "value": e }),
        PropertyValue::ObjectIdentifier(oid) => {
            serde_json::json!({
                "type": "object-identifier",
                "value": format!("{}:{}", object_type_name(oid.object_type()), oid.instance_number())
            })
        }
        PropertyValue::OctetString(bytes) => {
            serde_json::json!({ "type": "octet-string", "value": bytes.iter().map(|b| format!("{b:02x}")).collect::<String>() })
        }
        PropertyValue::BitString { unused_bits, data } => {
            let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
            serde_json::json!({
                "type": "bit-string",
                "unused_bits": unused_bits,
                "value": hex
            })
        }
        PropertyValue::Date(d) => {
            // BACnet year is offset from 1900; 0xFF = unspecified
            let year = if d.year == 0xFF {
                "*".to_string()
            } else {
                format!("{}", 1900u16 + d.year as u16)
            };
            let month = if d.month == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", d.month)
            };
            let day = if d.day == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", d.day)
            };
            serde_json::json!({ "type": "date", "value": format!("{year}-{month}-{day}") })
        }
        PropertyValue::Time(t) => {
            let hour = if t.hour == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", t.hour)
            };
            let min = if t.minute == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", t.minute)
            };
            let sec = if t.second == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", t.second)
            };
            let hun = if t.hundredths == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", t.hundredths)
            };
            serde_json::json!({ "type": "time", "value": format!("{hour}:{min}:{sec}.{hun}") })
        }
        PropertyValue::List(items) => {
            let arr: Vec<serde_json::Value> = items.iter().map(property_value_to_json).collect();
            serde_json::json!({ "type": "list", "value": arr })
        }
    }
}

/// Decode raw BACnet-encoded bytes into JSON.
pub fn decode_raw_property_to_json(data: &[u8]) -> serde_json::Value {
    let mut offset = 0;
    let mut values = Vec::new();
    while offset < data.len() {
        match decode_application_value(data, offset) {
            Ok((value, next)) => {
                values.push(property_value_to_json(&value));
                offset = next;
            }
            Err(_) => {
                let hex: String = data[offset..]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                values.push(serde_json::json!({ "type": "raw", "value": hex }));
                break;
            }
        }
    }
    if values.len() == 1 {
        values.into_iter().next().unwrap()
    } else {
        serde_json::json!({ "type": "list", "value": values })
    }
}

/// Get a human-readable name for an ObjectType (lowercase with hyphens).
pub fn object_type_name(ot: ObjectType) -> String {
    for &(name, val) in ObjectType::ALL_NAMED {
        if val == ot {
            return name.replace('_', "-").to_lowercase();
        }
    }
    format!("vendor-{}", ot.to_raw())
}

/// Get a human-readable name for a PropertyIdentifier (lowercase with hyphens).
pub fn property_name(pi: PropertyIdentifier) -> String {
    for &(name, val) in PropertyIdentifier::ALL_NAMED {
        if val == pi {
            return name.replace('_', "-").to_lowercase();
        }
    }
    format!("proprietary-{}", pi.to_raw())
}

/// Look up a name for an enumerated value given the property context.
///
/// Returns a human-readable name when the property type is known (e.g., units,
/// event-state, reliability), or None for unknown properties/values.
fn enumerated_name_for_property(value: u32, property: PropertyIdentifier) -> Option<String> {
    use bacnet_types::enums::*;

    macro_rules! lookup {
        ($enum_ty:ty) => {
            <$enum_ty>::ALL_NAMED
                .iter()
                .find(|(_, v)| v.to_raw() as u32 == value)
                .map(|(n, _)| n.replace('_', "-").to_lowercase())
        };
    }

    match property {
        PropertyIdentifier::OBJECT_TYPE => lookup!(ObjectType),
        PropertyIdentifier::UNITS => lookup!(EngineeringUnits),
        PropertyIdentifier::EVENT_STATE => lookup!(EventState),
        PropertyIdentifier::RELIABILITY => lookup!(Reliability),
        PropertyIdentifier::SYSTEM_STATUS => lookup!(DeviceStatus),
        PropertyIdentifier::SEGMENTATION_SUPPORTED => lookup!(Segmentation),
        PropertyIdentifier::NOTIFY_TYPE => lookup!(NotifyType),
        _ => None,
    }
}

/// Serialize a PropertyValue to JSON with property-context-aware enum decoding.
///
/// When the property is known (e.g., units, event-state, reliability), the
/// enumerated value is decoded to the correct enum name.
pub fn property_value_to_json_with_context(
    value: &PropertyValue,
    property: PropertyIdentifier,
) -> serde_json::Value {
    match value {
        PropertyValue::Enumerated(e) => {
            let mut obj = serde_json::json!({ "type": "enumerated", "value": e });
            if let Some(name) = enumerated_name_for_property(*e, property) {
                obj["name"] = serde_json::Value::String(name);
            }
            obj
        }
        _ => property_value_to_json(value),
    }
}

/// Decode raw BACnet-encoded bytes into JSON with property-context-aware enum decoding.
pub fn decode_raw_property_to_json_with_context(
    data: &[u8],
    property: PropertyIdentifier,
) -> serde_json::Value {
    let mut offset = 0;
    let mut values = Vec::new();
    while offset < data.len() {
        match decode_application_value(data, offset) {
            Ok((value, next)) => {
                values.push(property_value_to_json_with_context(&value, property));
                offset = next;
            }
            Err(_) => {
                let hex: String = data[offset..]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                values.push(serde_json::json!({ "type": "raw", "value": hex }));
                break;
            }
        }
    }
    if values.len() == 1 {
        values.into_iter().next().unwrap()
    } else {
        serde_json::json!({ "type": "list", "value": values })
    }
}

/// Convert a SocketAddrV4 to a 6-byte BACnet/IP MAC (4 bytes IP + 2 bytes port big-endian).
pub fn socket_addr_to_mac(addr: std::net::SocketAddrV4) -> Vec<u8> {
    let ip = addr.ip().octets();
    let port = addr.port().to_be_bytes();
    vec![ip[0], ip[1], ip[2], ip[3], port[0], port[1]]
}

/// Parse a JSON value into a PropertyValue.
pub fn json_to_property_value(v: &serde_json::Value) -> Result<PropertyValue, String> {
    match v {
        serde_json::Value::Null => Ok(PropertyValue::Null),
        serde_json::Value::Bool(b) => Ok(PropertyValue::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if n.is_f64() && f.fract() != 0.0 {
                    Ok(PropertyValue::Real(f as f32))
                } else if let Some(u) = n.as_u64() {
                    Ok(PropertyValue::Unsigned(u))
                } else if let Some(i) = n.as_i64() {
                    i32::try_from(i)
                        .map(PropertyValue::Signed)
                        .map_err(|_| format!("signed value {i} out of BACnet i32 range"))
                } else {
                    Ok(PropertyValue::Real(f as f32))
                }
            } else {
                Err("invalid number".to_string())
            }
        }
        serde_json::Value::String(s) => Ok(PropertyValue::CharacterString(s.clone())),
        _ => Err("unsupported JSON value type; use an object with 'type' and 'value' fields for complex types".to_string()),
    }
}

/// Construct a BACnet object by type. Shared between REST and MCP create handlers.
pub fn construct_object(
    obj_type: ObjectType,
    instance: u32,
    name: &str,
    number_of_states: Option<u32>,
) -> Result<Box<dyn bacnet_objects::traits::BACnetObject>, String> {
    let num_states = number_of_states.unwrap_or(2);

    match obj_type {
        ObjectType::ANALOG_INPUT => {
            bacnet_objects::analog::AnalogInputObject::new(instance, name, 95)
                .map(|o| Box::new(o) as Box<dyn bacnet_objects::traits::BACnetObject>)
        }
        ObjectType::ANALOG_OUTPUT => {
            bacnet_objects::analog::AnalogOutputObject::new(instance, name, 95)
                .map(|o| Box::new(o) as _)
        }
        ObjectType::ANALOG_VALUE => {
            bacnet_objects::analog::AnalogValueObject::new(instance, name, 95)
                .map(|o| Box::new(o) as _)
        }
        ObjectType::BINARY_INPUT => {
            bacnet_objects::binary::BinaryInputObject::new(instance, name).map(|o| Box::new(o) as _)
        }
        ObjectType::BINARY_OUTPUT => {
            bacnet_objects::binary::BinaryOutputObject::new(instance, name)
                .map(|o| Box::new(o) as _)
        }
        ObjectType::BINARY_VALUE => {
            bacnet_objects::binary::BinaryValueObject::new(instance, name).map(|o| Box::new(o) as _)
        }
        ObjectType::MULTI_STATE_INPUT => {
            bacnet_objects::multistate::MultiStateInputObject::new(instance, name, num_states)
                .map(|o| Box::new(o) as _)
        }
        ObjectType::MULTI_STATE_OUTPUT => {
            bacnet_objects::multistate::MultiStateOutputObject::new(instance, name, num_states)
                .map(|o| Box::new(o) as _)
        }
        ObjectType::MULTI_STATE_VALUE => {
            bacnet_objects::multistate::MultiStateValueObject::new(instance, name, num_states)
                .map(|o| Box::new(o) as _)
        }
        ObjectType::INTEGER_VALUE => {
            bacnet_objects::value_types::IntegerValueObject::new(instance, name)
                .map(|o| Box::new(o) as _)
        }
        ObjectType::POSITIVE_INTEGER_VALUE => {
            bacnet_objects::value_types::PositiveIntegerValueObject::new(instance, name)
                .map(|o| Box::new(o) as _)
        }
        ObjectType::LARGE_ANALOG_VALUE => {
            bacnet_objects::value_types::LargeAnalogValueObject::new(instance, name)
                .map(|o| Box::new(o) as _)
        }
        ObjectType::CHARACTERSTRING_VALUE => {
            bacnet_objects::value_types::CharacterStringValueObject::new(instance, name)
                .map(|o| Box::new(o) as _)
        }
        _ => {
            return Err(format!(
                "object type '{}' is not supported for creation via the API",
                object_type_name(obj_type),
            ))
        }
    }
    .map_err(|e| format!("{e}"))
}
