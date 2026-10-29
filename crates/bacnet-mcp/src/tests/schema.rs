//! Every tool's structured content, success and error alike, against the
//! output schema the tool declares.
//!
//! The check covers the JSON Schema keywords the output schemas use:
//! `type` (one or a list), `properties`, `required`, `items`, `enum`,
//! `anyOf` and `additionalProperties` (`false`, or a schema every other
//! key's value must fit).
//! A test asserts the schemas use no other keyword that constrains a
//! value, so a new one can't pass unchecked.

use serde_json::{Value, json};

use super::Lab;
use crate::Policy;

/// The keywords this check understands; descriptions are notes.
const KNOWN: [&str; 8] = [
    "type",
    "properties",
    "required",
    "items",
    "enum",
    "anyOf",
    "additionalProperties",
    "description",
];

/// Why `value` doesn't fit `schema`, at `path`.
fn violations(schema: &Value, value: &Value, path: &str, out: &mut Vec<String>) {
    let Some(schema) = schema.as_object() else {
        return;
    };
    for key in schema.keys() {
        assert!(
            KNOWN.contains(&key.as_str()),
            "{path}: the check doesn't know `{key}`"
        );
    }
    if let Some(types) = schema.get("type") {
        let types: Vec<&str> = match types {
            Value::String(one) => vec![one.as_str()],
            Value::Array(many) => many.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        let fits = types.iter().any(|t| match *t {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => false,
        });
        if !fits {
            out.push(format!("{path}: {value} is not {types:?}"));
            return;
        }
    }
    if let Some(Value::Array(choices)) = schema.get("anyOf") {
        let fits_one = choices.iter().any(|choice| {
            let mut inner = Vec::new();
            violations(choice, value, path, &mut inner);
            inner.is_empty()
        });
        if !fits_one {
            out.push(format!("{path}: {value} fits none of anyOf"));
            return;
        }
    }
    if schema.get("additionalProperties") == Some(&Value::Bool(false))
        && let (Some(object), properties) = (value.as_object(), schema.get("properties"))
    {
        for key in object.keys() {
            if properties.and_then(|p| p.get(key)).is_none() {
                out.push(format!("{path}: unexpected {key}"));
            }
        }
    }
    if let (Some(extra @ Value::Object(_)), Some(object)) =
        (schema.get("additionalProperties"), value.as_object())
    {
        let properties = schema.get("properties");
        for (key, field) in object {
            if properties.and_then(|p| p.get(key)).is_none() {
                violations(extra, field, &format!("{path}.{key}"), out);
            }
        }
    }
    if let Some(Value::Array(allowed)) = schema.get("enum")
        && !allowed.contains(value)
    {
        out.push(format!("{path}: {value} is not one of {allowed:?}"));
    }
    if let (Some(Value::Object(properties)), Some(object)) =
        (schema.get("properties"), value.as_object())
    {
        for (key, inner) in properties {
            if let Some(field) = object.get(key) {
                violations(inner, field, &format!("{path}.{key}"), out);
            }
        }
    }
    if let (Some(Value::Array(required)), Some(object)) =
        (schema.get("required"), value.as_object())
    {
        for key in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(key) {
                out.push(format!("{path}: missing {key}"));
            }
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (i, item) in array.iter().enumerate() {
            violations(items, item, &format!("{path}[{i}]"), out);
        }
    }
}

/// Check `body` against `tool`'s output schema, as the server lists it.
pub(super) async fn check_output(agent: &mut super::Agent, tool: &str, body: &Value) {
    let tools = agent.request("tools/list", json!({})).await;
    let schema = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == json!(tool))
        .map(|t| t["outputSchema"].clone())
        .unwrap();
    let mut out = Vec::new();
    violations(&schema, body, "$", &mut out);
    assert!(out.is_empty(), "{tool}: {out:?}");
}

/// The validator itself catches a wrong type, a missing field and a value
/// outside an enum.
#[test]
fn the_check_finds_violations() {
    let schema = json!({
        "type": "object",
        "properties": { "n": { "type": "integer" }, "e": { "enum": ["a"] } },
        "required": ["n"],
    });
    let mut out = Vec::new();
    violations(&schema, &json!({ "n": "1", "e": "b" }), "$", &mut out);
    violations(&schema, &json!({}), "$", &mut out);
    assert_eq!(out.len(), 3, "{out:?}");
    let either = json!({ "anyOf": [
        { "type": "integer" },
        { "type": "object", "properties": { "a": {} }, "additionalProperties": false },
    ] });
    let mut out = Vec::new();
    violations(&either, &json!(3), "$", &mut out);
    violations(&either, &json!({ "a": 1 }), "$", &mut out);
    assert!(out.is_empty(), "{out:?}");
    violations(&either, &json!({ "b": 1 }), "$", &mut out);
    violations(&either, &json!("x"), "$", &mut out);
    assert_eq!(out.len(), 2, "{out:?}");
    // `additionalProperties` as a schema: every other key's value fits it.
    let map = json!({ "type": "object", "additionalProperties": { "type": "integer" } });
    let mut out = Vec::new();
    violations(&map, &json!({ "x": 1 }), "$", &mut out);
    assert!(out.is_empty(), "{out:?}");
    violations(&map, &json!({ "x": "s" }), "$", &mut out);
    assert_eq!(out.len(), 1, "{out:?}");
}

/// Calls that succeed and calls that fail, for every tool. A watched
/// point's values come in the same shapes as a read's.
#[tokio::test(start_paused = true)]
async fn results_fit_their_output_schemas() {
    let mut lab = Lab::start_site(Policy::default(), std::time::Duration::ZERO).await;
    lab.add_logs().await;
    lab.agent.initialize().await;
    let tools = lab.agent.request("tools/list", json!({})).await;
    let schemas: Vec<(String, Value)> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_string(),
                t["outputSchema"].clone(),
            )
        })
        .collect();
    let calls = [
        ("bacnet_discover", json!({ "range": "1201", "names": true })),
        ("bacnet_discover", json!({ "send": false })),
        ("bacnet_discover", json!({ "range": "1-4194303" })),
        (
            "bacnet_discover",
            json!({ "scope": "network", "network": 0 }),
        ),
        (
            "bacnet_read",
            json!({ "device": "1201", "reads": [{ "object": "ai:3" }], "preset": "point" }),
        ),
        (
            "bacnet_read",
            json!({ "device": "1201", "reads": [{ "object": "ai:1", "properties": ["all"] }] }),
        ),
        (
            "bacnet_read",
            json!({ "device": "4242", "reads": [{ "object": "ai:1" }] }),
        ),
        (
            "bacnet_read",
            json!({ "device": "2001:300", "reads": [{ "object": "ai:1" }] }),
        ),
        (
            "bacnet_read",
            json!({ "device": "1201", "reads": [{ "object": "nope:1" }] }),
        ),
        (
            "bacnet_list_objects",
            json!({ "device": "1201", "names": true }),
        ),
        ("bacnet_list_objects", json!({ "device": "4242" })),
        (
            "bacnet_list_objects",
            json!({ "device": "1201", "cursor": "00.00" }),
        ),
        (
            "bacnet_describe_device",
            json!({ "device": "1201", "count_objects": true }),
        ),
        ("bacnet_describe_device", json!({ "device": "2001:300" })),
        (
            "bacnet_find_object",
            json!({ "name": "zone", "description": "", "range": "1201" }),
        ),
        (
            "bacnet_find_object",
            json!({ "name": "zone", "range": "1201" }),
        ),
        (
            "bacnet_find_object",
            json!({ "types": ["ai"], "range": "1201-1202" }),
        ),
        ("bacnet_find_object", json!({ "range": "1201" })),
        ("bacnet_network_map", json!({})),
        ("bacnet_network_map", json!({ "send": true })),
        // The lab's link is no B/IP: refused (tests/bbmd.rs checks a survey).
        ("bacnet_network_map", json!({ "bbmd": true })),
        (
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "trend-log:1", "max_records": 60 }),
        ),
        (
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "trend-log:2", "summary": true }),
        ),
        // State columns labelled with enveloped state texts (#2122).
        (
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "trend-log-multiple:1", "max_records": 8 }),
        ),
        (
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "ai:1" }),
        ),
        (
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "trend-log:9" }),
        ),
        // Event and audit records, device text enveloped (#2103).
        (
            "bacnet_read_log_records",
            json!({ "device": 1201, "object": "event-log:1", "max_records": 12 }),
        ),
        (
            "bacnet_read_log_records",
            json!({ "device": 1201, "object": "audit-log:1", "max_records": 12 }),
        ),
        (
            "bacnet_read_log_records",
            json!({ "device": 1201, "object": "audit-log:1", "summary": true }),
        ),
        (
            "bacnet_read_log_records",
            json!({ "device": 1201, "object": "trend-log:1" }),
        ),
        (
            "bacnet_alarms",
            json!({ "device": 1201, "summaries": true }),
        ),
        ("bacnet_alarms", json!({ "device": 1201, "max": 1 })),
        ("bacnet_alarms", json!({ "device": 4242 })),
        (
            "bacnet_decode",
            json!({ "hex": super::site::ack_with_text("Fan\u{202E}") }),
        ),
        ("bacnet_decode", json!({ "hex": "10 08", "link": "apdu" })),
        ("bacnet_decode", json!({ "hex": "zz" })),
        (
            "bacnet_watch",
            json!({ "points": [{ "device": 1201, "object": "ai:1" },
                               { "device": 1201, "object": "ai:3", "property": "description" },
                               { "device": 4242, "object": "ai:1" }] }),
        ),
        (
            "bacnet_watch",
            json!({ "points": [{ "device": 1201, "object": "nope:1" }] }),
        ),
    ];
    // Each tool's (successes, errors) checked.
    let mut checked: std::collections::BTreeMap<&str, (u32, u32)> = Default::default();
    for (tool, args) in calls {
        let result = lab.agent.call(tool, args.clone()).await;
        let tally = checked.entry(tool).or_default();
        if result["isError"] == json!(true) {
            tally.1 += 1;
        } else {
            tally.0 += 1;
        }
        let schema = &schemas.iter().find(|(name, _)| name == tool).unwrap().1;
        let mut out = Vec::new();
        violations(schema, &result["structuredContent"], "$", &mut out);
        assert!(out.is_empty(), "{tool} {args}: {out:?}");
    }
    assert_eq!(checked.len(), 11, "every tool: {checked:?}");
    for (tool, (successes, errors)) in &checked {
        assert!(
            *successes > 0 && *errors > 0,
            "{tool}: {successes} successes, {errors} errors"
        );
    }
    lab.stop().await;
}

/// An Event_Parameters value decoded field by field (#2090) fits the read
/// tool's schema, its element kind stays plain, and the device's alarm
/// strings among its fields come back enveloped.
#[tokio::test(start_paused = true)]
async fn event_parameters_fields_fit_the_schema_and_envelope_device_text() {
    use bacnet_objects::event_enrollment::EventEnrollmentObject;
    use bacnet_types::constructed::{
        BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
    };
    use bacnet_types::enums::{EventType, ObjectType, PropertyIdentifier};
    use bacnet_types::primitives::ObjectIdentifier;

    let mut lab = Lab::start(Policy::default()).await;
    let mut enrollment =
        EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_CHARACTERSTRING).unwrap();
    enrollment.set_event_parameters(BACnetEventParameter::ChangeOfCharacterstring {
        time_delay: 5,
        list_of_alarm_values: vec!["FIRE".into(), super::hostile()],
    });
    let mut states = EventEnrollmentObject::new(2, "EE-2", EventType::CHANGE_OF_STATE).unwrap();
    states.set_event_parameters(BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: vec![BACnetPropertyStates::State(3)],
    });
    let mut feedback = EventEnrollmentObject::new(3, "EE-3", EventType::COMMAND_FAILURE).unwrap();
    feedback.set_event_parameters(BACnetEventParameter::CommandFailure {
        time_delay: 1,
        feedback_property_reference: BACnetDeviceObjectPropertyReference::new_local(
            ObjectIdentifier::new(ObjectType::BINARY_INPUT, 4).unwrap(),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ),
    });
    {
        let mut db = lab.server.database().write().await;
        for object in [enrollment, states, feedback] {
            db.add(Box::new(object)).unwrap();
        }
    }
    lab.agent.initialize().await;
    let tools = lab.agent.request("tools/list", json!({})).await;
    let schema = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "bacnet_read")
        .unwrap()["outputSchema"]
        .clone();
    let read = lab
        .agent
        .call(
            "bacnet_read",
            json!({ "device": "1201",
                    "reads": [{ "object": "event-enrollment:1",
                                "properties": ["event-parameters"] },
                              { "object": "event-enrollment:2",
                                "properties": ["event-parameters"] },
                              { "object": "event-enrollment:3",
                                "properties": ["event-parameters"] }] }),
        )
        .await;
    let mut out = Vec::new();
    violations(&schema, &read["structuredContent"], "$", &mut out);
    assert!(out.is_empty(), "{out:?}");

    let typed = &read["structuredContent"]["results"][0]["value"]["typed"];
    assert_eq!(typed["element"], json!("event-parameter"));
    assert!(typed["text"]["untrusted"].is_string(), "{typed}");
    let fields = &typed["fields"];
    assert_eq!(
        fields["algorithm"],
        json!({"value": 17, "name": "change-of-characterstring"})
    );
    assert_eq!(fields["time_delay"], json!(5));
    let values = fields["list_of_alarm_values"].as_array().unwrap();
    assert_eq!(values[0]["untrusted"], json!("FIRE"));
    let hostile = values[1]["untrusted"].as_str().unwrap();
    assert!(hostile.contains("hxxps://evil.example/x"), "{hostile}");
    assert!(!hostile.contains('\u{202E}'), "{hostile}");

    // A nested element is a value too: a property state's text (a kind
    // that may hold a string) is enveloped, a reference's left plain.
    let results = &read["structuredContent"]["results"];
    let state = &results[1]["value"]["typed"]["fields"]["list_of_values"][0];
    assert_eq!(state["element"], json!("property-state"));
    assert_eq!(state["text"]["untrusted"], json!("high-limit"), "{state}");
    let reference = &results[2]["value"]["typed"]["fields"]["feedback_property_reference"];
    assert_eq!(
        *reference,
        json!({"element": "device-object-property-reference",
               "text": "binary-input 4 present-value"})
    );
    lab.stop().await;
}
