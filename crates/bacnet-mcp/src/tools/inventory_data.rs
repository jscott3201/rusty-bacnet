//! An inventory's point record as `bacnet_inventory` returns it: the
//! fields and shapes of the JSON `bacnet inventory` prints
//! ([`PointRecord`]'s serde form), with the device's text in untrusted
//! envelopes.
//!
//! The fields that hold no device text (the device's address and
//! instance, the object, units, the state count, whether it takes
//! commands) are copied from the record's own JSON. Every other field is
//! built here, one by one, so a field the record gains later appears only
//! once someone decides how to show it (a test compares the keys):
//!
//! - `name`, `description`, `device_type`, `profile_name` and each text of
//!   `state_texts` are envelopes;
//! - each value is `{value, decoded}` as in the CLI's JSON: `value` the
//!   formatter's text, an envelope when it may quote the device (a
//!   character string, or a state named by the object's own text), and
//!   `decoded` the typed form with each device-text leaf enveloped;
//! - each problem's `error` is the read tool's error row: the device's
//!   error class and code by name and number, `no-value`, or `failed` with
//!   the inventory's words in an envelope, since they can quote what the
//!   device sent (`not a Real (got a character string, ...)`).

use bacnet_ops::PropertyError;
use bacnet_ops::inventory::{PointRecord, Problem};
use bacnet_ops::label::kebab;
use bacnet_types::enums::PropertyIdentifier as P;
use serde_json::{Map, Value, json};

use super::schema;
use crate::errors::row_error_json;
use crate::untrusted::envelope;
use crate::value::{object_text, value_json};

/// The record's fields with no device text, copied from its own JSON.
const PLAIN: [&str; 6] = [
    "device",
    "device_instance",
    "object",
    "units",
    "number_of_states",
    "commandable",
];

/// The record's device-text fields and the property each comes from.
const TEXTS: [(&str, P); 4] = [
    ("name", P::OBJECT_NAME),
    ("description", P::DESCRIPTION),
    ("device_type", P::DEVICE_TYPE),
    ("profile_name", P::PROFILE_NAME),
];

/// The record's values, by their JSON names, in the record's order.
const VALUES: [&str; 10] = [
    "relinquish_default",
    "cov_increment",
    "min_present_value",
    "max_present_value",
    "polarity",
    "present_value",
    "command_priority",
    "status_flags",
    "event_state",
    "reliability",
];

/// The record's values, in [`VALUES`]' order, each with the property its
/// envelopes name as their source: the priority in control by the property
/// an output gives it in.
fn values(record: &PointRecord) -> [(&Option<bacnet_ops::Value>, P); 10] {
    [
        (&record.relinquish_default, P::RELINQUISH_DEFAULT),
        (&record.cov_increment, P::COV_INCREMENT),
        (&record.min_present_value, P::MIN_PRES_VALUE),
        (&record.max_present_value, P::MAX_PRES_VALUE),
        (&record.polarity, P::POLARITY),
        (&record.present_value, P::PRESENT_VALUE),
        (&record.command_priority, P::CURRENT_COMMAND_PRIORITY),
        (&record.status_flags, P::STATUS_FLAGS),
        (&record.event_state, P::EVENT_STATE),
        (&record.reliability, P::RELIABILITY),
    ]
}

/// The object as the tools' arguments name it, `analog-input:1`.
fn object_of(record: &PointRecord) -> String {
    object_text(record.object.object_type(), record.object.instance_number())
}

/// Where a record's `property` came from, for an envelope:
/// `device 1201 analog-input:1 object-name`.
fn source(record: &PointRecord, property: &str) -> String {
    format!(
        "device {} {} {property}",
        record.device_instance,
        object_of(record)
    )
}

/// `record` as the tool returns it, device text capped at `cap`
/// characters.
pub(super) fn record_json(record: &PointRecord, cap: usize) -> Value {
    let cli = serde_json::to_value(record).expect("a point record serializes");
    let mut out = Map::new();
    for key in PLAIN {
        out.insert(key.into(), cli[key].clone());
    }
    let text = |text: &Option<String>, property: &str| {
        text.as_deref().map_or(Value::Null, |text| {
            envelope(text, source(record, property), cap)
        })
    };
    let fields = [
        &record.name,
        &record.description,
        &record.device_type,
        &record.profile_name,
    ];
    for ((key, property), field) in TEXTS.iter().zip(fields) {
        out.insert((*key).into(), text(field, &kebab(*property)));
    }
    let texts = &record.state_texts;
    let states: Vec<Value> = texts
        .states
        .iter()
        .enumerate()
        .map(|(at, state)| {
            envelope(
                state,
                source(record, &format!("state-text[{}]", at + 1)),
                cap,
            )
        })
        .collect();
    out.insert(
        "state_texts".into(),
        json!({
            "active": text(&texts.active, "active-text"),
            "inactive": text(&texts.inactive, "inactive-text"),
            "states": states,
        }),
    );
    for (key, (value, property)) in VALUES.iter().zip(values(record)) {
        let shown = value.as_ref().map_or(Value::Null, |value| {
            let both = value_json(value, &source(record, &kebab(property)), cap);
            json!({ "value": both["display"], "decoded": both["typed"] })
        });
        out.insert((*key).into(), shown);
    }
    let problems: Vec<Value> = record
        .problems
        .iter()
        .map(|problem| problem_json(record, problem, cap))
        .collect();
    out.insert("problems".into(), Value::Array(problems));
    Value::Object(out)
}

/// One problem: its property, and why it gave no usable value.
fn problem_json(record: &PointRecord, problem: &Problem, cap: usize) -> Value {
    let property = kebab(problem.property);
    let error = match &problem.error {
        PropertyError::Other(text) => json!({
            "result": "failed",
            "message": envelope(text, source(record, &property), cap),
        }),
        other => row_error_json(other),
    };
    json!({ "property": property, "error": error })
}

/// A page's problems in a summary's form: each with its object named, in
/// the order of the records, then of the properties asked.
pub(super) fn problems_of(record: &Value) -> Vec<Value> {
    let object = &record["object"];
    let named = format!(
        "{}:{}",
        object["type"].as_str().unwrap_or_default(),
        object["instance"]
    );
    record["problems"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|problem| {
            json!({
                "object": named,
                "property": problem["property"],
                "error": problem["error"],
            })
        })
        .collect()
}

/// The output schema of one record.
pub(super) fn point() -> Value {
    let string = json!({ "type": "string" });
    let integer = json!({ "type": "integer" });
    let mut required: Vec<&str> = PLAIN.to_vec();
    required.extend(TEXTS.iter().map(|(key, _)| *key));
    required.extend(["state_texts", "problems"]);
    required.extend(VALUES);
    json!({
        "type": "object",
        "description": format!(
            "One object's point record, as `bacnet inventory --json` writes it, device text in \
             envelopes. Each of {} is null or {{value, decoded}}: the formatter's text (an \
             envelope when it may quote the device) and the typed form",
            VALUES.join(", ")
        ),
        "properties": {
            "device": {
                "type": "string",
                "description": "Where the device answers, as a target (10.0.0.2:47808, 2:0x05)",
            },
            "device_instance": integer,
            "object": {
                "type": "object",
                "properties": {
                    "type": string,
                    "type_number": integer,
                    "instance": integer,
                },
            },
            "name": schema::envelope(),
            "description": schema::envelope(),
            "device_type": schema::envelope(),
            "profile_name": schema::envelope(),
            "units": {
                "type": ["object", "null"],
                "properties": { "value": integer, "name": string, "symbol": string },
            },
            "state_texts": {
                "type": "object",
                "description": "A binary object's active and inactive texts, or a multi-state \
                                object's state texts from state 1",
                "properties": {
                    "active": schema::envelope(),
                    "inactive": schema::envelope(),
                    "states": { "type": "array", "items": schema::envelope_object() },
                },
            },
            "number_of_states": { "type": ["integer", "null"] },
            "commandable": { "type": "boolean" },
            "problems": {
                "type": "array",
                "description": "Properties that gave no usable value, and why; never a failed \
                                call",
                "items": problem_schema(false),
            },
        },
        "required": required,
        "additionalProperties": {
            "type": ["object", "null"],
            "properties": {
                "value": {
                    "type": ["string", "object"],
                    "description": "The formatter's text; an untrusted envelope when it may \
                                    quote the device",
                },
                "decoded": schema::typed(),
            },
            "required": ["value", "decoded"],
        },
    })
}

/// A problem's schema, with the object it is on when `object`.
pub(super) fn problem_schema(object: bool) -> Value {
    let mut properties = json!({
        "property": { "type": "string" },
        "error": {
            "type": "object",
            "description": "The device's error class and code, no-value, or failed with a \
                            message in an envelope",
        },
    });
    let mut required = vec!["property", "error"];
    if object {
        properties["object"] = json!({ "type": "string" });
        required.push("object");
    }
    json!({ "type": "object", "properties": properties, "required": required })
}

#[cfg(test)]
mod tests {
    use bacnet_ops::DeviceAddress;
    use bacnet_ops::format::{Floats, StateTexts, ValueFormat};
    use bacnet_types::enums::{EngineeringUnits, ErrorClass, ErrorCode, ObjectType};
    use bacnet_types::error::RemoteError;
    use bacnet_types::primitives::ObjectIdentifier;

    use super::*;

    /// A multi-state value with hostile texts, a state named by its own
    /// text, and two problems: one the device's error, one the
    /// inventory's words quoting the device.
    fn record() -> PointRecord {
        let address: DeviceAddress = "10.0.0.2:47808".parse().unwrap();
        let object = ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 2).unwrap();
        let mut record = PointRecord::new(address, 1201, object);
        record.name = Some("Fan\u{202E} Mode".into());
        record.description = Some("see https://evil.example".into());
        record.units = Some(EngineeringUnits::PERCENT);
        record.state_texts = StateTexts {
            states: vec!["Off".into(), "Low\u{0007}".into()],
            ..StateTexts::default()
        };
        record.number_of_states = Some(2);
        record.commandable = true;
        let texts = record.state_texts.clone();
        let format = ValueFormat::new(ObjectType::MULTI_STATE_VALUE, &texts, Floats::Exact);
        // Unsigned 2: state 2, named by the object's own text.
        record.present_value = Some(bacnet_ops::Value::new(
            &format,
            P::PRESENT_VALUE,
            None,
            vec![0x21, 0x02],
            None,
        ));
        record.problems = vec![
            Problem::new(
                P::DESCRIPTION,
                PropertyError::Remote(RemoteError::Error {
                    class: ErrorClass::PROPERTY,
                    code: ErrorCode::READ_ACCESS_DENIED,
                }),
            ),
            Problem::new(
                P::RELINQUISH_DEFAULT,
                PropertyError::Other("not an Unsigned (got a character string, Ignore)".into()),
            ),
        ];
        record
    }

    /// The ReadProperty counts the tool's description quotes for a device
    /// without ReadPropertyMultiple, one request a property: 2 to 5 an
    /// object with no option, at most 14 with every one, over every type
    /// the standard assigns.
    #[test]
    fn the_description_quotes_the_readproperty_counts() {
        use bacnet_ops::InventoryOptions;
        use bacnet_ops::inventory::properties;
        let all = InventoryOptions::new()
            .with_values(true)
            .with_settings(true)
            .with_details(true);
        let counts = |options: &InventoryOptions| -> Vec<usize> {
            (0..128)
                .map(|raw| properties(ObjectType::from_raw(raw), options).len())
                .collect()
        };
        let plain = counts(&InventoryOptions::new());
        assert_eq!(plain.iter().min(), Some(&2));
        assert_eq!(plain.iter().max(), Some(&5));
        assert_eq!(counts(&all).iter().max(), Some(&14));
        let quoted = "2 to 5 an object, up to 14 with every option";
        let description = crate::tools::inventory::SPEC.description.replace("\n", " ");
        let description: String = description.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(description.contains(quoted), "{description}");
    }

    /// The tool's record has exactly the keys of the CLI's: a field the
    /// record gains fails here until it is shown on purpose.
    #[test]
    fn a_record_has_the_cli_jsons_keys() {
        let record = record();
        let cli = serde_json::to_value(&record).unwrap();
        let ours = record_json(&record, 256);
        let keys = |value: &Value| {
            let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        };
        assert_eq!(keys(&ours), keys(&cli));
        for key in PLAIN {
            assert_eq!(ours[key], cli[key], "{key}");
        }
        assert_eq!(
            ours["object"],
            json!({ "type": "multi-state-value", "type_number": 19, "instance": 2 })
        );
    }

    /// Device text is enveloped, cleaned and sourced; a value naming a
    /// state by the object's text is enveloped too, its decoded form not;
    /// the device's error keeps its names, the inventory's words are
    /// enveloped.
    #[test]
    fn device_text_is_enveloped() {
        let ours = record_json(&record(), 256);
        assert_eq!(
            ours["name"],
            json!({ "untrusted": "Fan Mode", "source": "device 1201 multi-state-value:2 object-name",
                    "truncated": false })
        );
        assert_eq!(
            ours["description"]["untrusted"],
            json!("see hxxps://evil.example")
        );
        let states = &ours["state_texts"]["states"];
        assert_eq!(states[1]["untrusted"], json!("Low "));
        assert_eq!(
            states[1]["source"],
            json!("device 1201 multi-state-value:2 state-text[2]")
        );
        assert_eq!(ours["state_texts"]["active"], Value::Null);
        let present = &ours["present_value"];
        assert!(
            present["value"]["untrusted"]
                .as_str()
                .unwrap()
                .contains("Low"),
            "{present}"
        );
        assert_eq!(present["decoded"], json!(2));
        assert_eq!(ours["reliability"], Value::Null);
        let problems = ours["problems"].as_array().unwrap();
        assert_eq!(problems[0]["property"], json!("description"));
        assert_eq!(problems[0]["error"]["code"], json!("read-access-denied"));
        assert_eq!(problems[1]["error"]["result"], json!("failed"));
        assert_eq!(
            problems[1]["error"]["message"]["source"],
            json!("device 1201 multi-state-value:2 relinquish-default")
        );
        let summary = problems_of(&ours);
        assert_eq!(summary[1]["object"], json!("multi-state-value:2"));
        assert_eq!(summary[1]["property"], json!("relinquish-default"));
    }
}
