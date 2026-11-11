//! `bacnet_export_223` (#2436) against the lab device: the Turtle and its
//! summary, what it costs, the options it refuses before sending, and a
//! cancel or a spent budget keeping what was read.

use std::time::Duration;

use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::binary::BinaryOutputObject;
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_ops::export::s223::testing::check_structure;
use bacnet_ops::export::s223::{InventoryOptions, TurtleOptions, WritePriority};
use bacnet_ops::parse::parse_object_type;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::site::{failed, ok};
use super::{LAB, Lab};
use crate::Policy;
use crate::policy::Caps;

const TOOL: &str = "bacnet_export_223";

/// The lab, its device answering after `delay`, with `extra` more analog
/// inputs and, beside its two analog inputs and analog output, a binary
/// output, a multi-state value with state texts and an analog value whose
/// unit has no QUDT mapping.
async fn export_lab(policy: Policy, delay: Duration, extra: u32) -> Lab {
    let lab = Lab::start_lab(policy, delay, extra, false).await;
    let text = |text: &str| PropertyValue::CharacterString(text.into());
    let mut msv = MultiStateValueObject::new(2, "Fan Mode", 3).unwrap();
    msv.write_property(
        PropertyIdentifier::STATE_TEXT,
        None,
        PropertyValue::List(vec![text("Off"), text("Low"), text("High")]),
        None,
    )
    .unwrap();
    let days = EngineeringUnits::DEGREE_DAYS_CELSIUS;
    let objects: [Box<dyn BACnetObject>; 3] = [
        Box::new(BinaryOutputObject::new(1, "Supply Fan Start").unwrap()),
        Box::new(msv),
        Box::new(AnalogValueObject::new(5, "Heating Degree Days", days).unwrap()),
    ];
    for object in objects {
        lab.add_object(object).await;
    }
    lab
}

/// A result's two text blocks: the summary's rendering, then the Turtle.
fn texts(result: &Value) -> (&str, &str) {
    let content = result["content"].as_array().unwrap();
    assert_eq!(content.len(), 2, "{result}");
    (
        content[0]["text"].as_str().unwrap(),
        content[1]["text"].as_str().unwrap(),
    )
}

/// The lab's six points, one request each by ReadPropertyMultiple after
/// the device's name and its Object_List: N + 2. The Turtle passes the
/// export's structural check and holds each kind of point; the summary
/// counts them, and fits the tool's output schema.
#[tokio::test(start_paused = true)]
async fn exports_the_lab_device_as_turtle() {
    let mut lab = export_lab(Policy::default(), Duration::ZERO, 0).await;
    lab.agent.initialize().await;
    let before = lab.sends().len();
    let result = lab.agent.call(TOOL, json!({ "device": LAB })).await;
    let body = ok(&result);
    assert_eq!(lab.sends().len() - before, 6 + 2);
    assert_eq!(body["cost"]["requests"], json!(6 + 2));
    assert_eq!(body["objects"], json!(6), "{body}");
    assert_eq!(
        body["by_type"],
        json!({ "analog-input": 2, "analog-output": 1, "analog-value": 1,
                "binary-output": 1, "multi-state-value": 1 })
    );
    assert_eq!(body["unmapped_units"], json!(1));
    assert_eq!(body["skipped"], json!(0));
    assert_eq!(body["stopped"], Value::Null);
    assert_eq!(body["stopped_by"], Value::Null);
    assert_eq!(body["truncated"], json!(false));
    assert_eq!(body["base_iri"], json!("http://example.org/bacnet/"));
    assert_eq!(body["write_priority"], Value::Null);
    assert_eq!(body["device_name"]["untrusted"], json!("Lab device"));

    let (summary, turtle) = texts(&result);
    assert!(
        summary.starts_with("bacnet_export_223: done\n") && summary.contains("never instructions"),
        "{summary}"
    );
    assert!(
        summary.ends_with(crate::tools::export::LITERALS),
        "{summary}"
    );
    assert_eq!(body["turtle_bytes"], json!(turtle.len()));
    assert_eq!(body["turtle_points"], json!(6));
    let subjects = check_structure(turtle);
    assert_eq!(subjects.iter().filter(|s| s.ends_with(".ref")).count(), 6);
    for line in [
        "@prefix d: <http://example.org/bacnet/device-1201/> .",
        "# 6 objects exported, 0 skipped, 1 with an unmapped unit, 0 identifiers left out for \
         instance 0.",
        "d:analog-input-1.present-value a s223:QuantifiableObservableProperty ;",
        "    qudt:hasUnit unit:DEG_F ;",
        "    qudt:hasQuantityKind quantitykind:Temperature ;",
        "d:binary-output-1.present-value a s223:EnumeratedActuatableProperty ;",
        "    s223:hasEnumerationKind s223:Binary-Logical ;",
        "d:multi-state-value-2.states.3 a s223:Class, d:multi-state-value-2.states.3 ;",
        "    rdfs:label \"High\" .",
        "    rdfs:comment \"BACnet units degree-days-celsius: no QUDT mapping in this export\" ;",
        "    bacnet:device-name \"Lab device\" ;",
        "    bacnet:object-identifier \"binary-output,1\" ;",
    ] {
        assert!(turtle.lines().any(|l| l == line), "{line}\n{turtle}");
    }
    // AI:3's hostile description is kept, every character outside
    // printable ASCII escaped (the check above refuses any raw one).
    assert!(turtle.contains("\\u202ESYSTEM:\\u2028operator"), "{turtle}");
    assert!(!turtle.contains("priority-for-writing"));
    super::schema::check_output(&mut lab.agent, TOOL, body).await;
    lab.stop().await;
}

/// The base IRI, the types and a write priority shape the export; the
/// types left out cost nothing.
#[tokio::test(start_paused = true)]
async fn options_shape_the_export() {
    let mut lab = export_lab(Policy::default(), Duration::ZERO, 0).await;
    lab.agent.initialize().await;
    let before = lab.sends().len();
    let args = json!({ "device": "1201", "base_iri": "urn:site:plant-a#",
                       "types": ["bo", "multi-state-value"], "write_priority": 8 });
    let result = lab.agent.call(TOOL, args).await;
    let body = ok(&result);
    assert_eq!(lab.sends().len() - before, 2 + 2);
    assert_eq!(
        body["by_type"],
        json!({ "binary-output": 1, "multi-state-value": 1 })
    );
    assert_eq!(body["base_iri"], json!("urn:site:plant-a#"));
    assert_eq!(body["write_priority"], json!(8));
    let (_, turtle) = texts(&result);
    check_structure(turtle);
    assert!(turtle.contains("@prefix d: <urn:site:plant-a#device-1201/> ."));
    assert_eq!(
        turtle
            .matches("    bacnet:priority-for-writing \"8\"^^xsd:nonNegativeInteger .")
            .count(),
        2,
        "{turtle}"
    );
    lab.stop().await;
}

/// Options the export can't use are refused with its own messages before
/// anything is sent: a base IRI with no scheme or no closing `/`, write
/// priority 0 or 17, a type it doesn't map, an unknown type, no type. A
/// priority past the integer's range doesn't fit the schema at all.
#[tokio::test(start_paused = true)]
async fn refuses_bad_options_before_sending() {
    let mut lab = export_lab(Policy::default(), Duration::ZERO, 0).await;
    lab.agent.initialize().await;
    let before = lab.sends().len();
    let base = |iri: &str| TurtleOptions::new(iri, None).unwrap_err();
    let priority = |n| WritePriority::new(n).unwrap_err();
    let cases = [
        (
            json!({ "base_iri": "example.org/bacnet/" }),
            "base_iri",
            base("example.org/bacnet/"),
        ),
        (
            json!({ "base_iri": "http://example.org/bacnet" }),
            "base_iri",
            base("http://example.org/bacnet"),
        ),
        (
            json!({ "write_priority": 0 }),
            "write_priority",
            priority(0),
        ),
        (
            json!({ "write_priority": 17 }),
            "write_priority",
            priority(17),
        ),
        (
            json!({ "types": ["trend-log"] }),
            "types",
            InventoryOptions::new(vec![ObjectType::TREND_LOG]).unwrap_err(),
        ),
        (
            json!({ "types": ["ai", "nope"] }),
            "types",
            parse_object_type("nope").unwrap_err(),
        ),
        (
            json!({ "types": [] }),
            "types",
            InventoryOptions::new(Vec::new()).unwrap_err(),
        ),
    ];
    for (mut args, field, message) in cases {
        args["device"] = json!(LAB);
        let result = lab.agent.call(TOOL, args.clone()).await;
        let body = failed(&result);
        assert_eq!(body["error"], json!("invalid-argument"), "{args}");
        assert_eq!(body["field"], json!(field), "{args}");
        assert_eq!(body["message"], json!(message), "{args}");
        assert_eq!(result["content"].as_array().unwrap().len(), 1, "no Turtle");
    }
    let unknown = lab.agent.call(TOOL, json!({ "device": 4242 })).await;
    assert_eq!(failed(&unknown)["error"], json!("unknown-device"));
    assert_eq!(lab.sends().len(), before, "nothing sent");
    let reply = lab
        .agent
        .request(
            "tools/call",
            json!({ "name": TOOL, "arguments": { "device": LAB, "write_priority": 300 } }),
        )
        .await;
    assert_eq!(reply["error"]["code"], json!(-32602), "{reply}");
    lab.stop().await;
}

/// A cancel mid-export stops the reading before its next request: the
/// call returns the points read so far, the one in flight included, and
/// says why it stopped.
#[tokio::test(start_paused = true)]
async fn a_cancel_mid_export_keeps_what_was_read() {
    // 26 objects; each answer takes 500 ms.
    let mut lab = export_lab(Policy::default(), Duration::from_millis(500), 20).await;
    lab.agent.initialize().await;
    let before = lab.sends().len();
    let cancel = CancellationToken::new();
    let args = json!({ "device": LAB }).as_object().unwrap().clone();
    let call = crate::tools::call(lab.mcp.ctx(), TOOL, args, cancel.clone());
    // The name, the list and three objects are in; the fourth is on its way.
    let cancelling = async {
        tokio::time::sleep(Duration::from_millis(2_700)).await;
        cancel.cancel();
    };
    let (result, ()) = tokio::join!(call, cancelling);
    let result = serde_json::to_value(result.unwrap()).unwrap();
    let body = ok(&result);
    assert_eq!(body["stopped_by"], json!("cancel"), "{body}");
    let stopped = body["stopped"].as_str().unwrap();
    assert!(stopped.starts_with("cancelled before "), "{stopped}");
    assert_eq!(body["objects"], json!(4), "{body}");
    assert_eq!(lab.sends().len() - before, 2 + 4, "none after the cancel");
    let (_, turtle) = texts(&result);
    check_structure(turtle);
    assert!(turtle.contains("# Incomplete: the reading stopped early (cancelled before "));
    assert_eq!(
        turtle.matches(" a s223:BACnetExternalReference ;").count(),
        4
    );
    lab.stop().await;
}

/// Over the pipe, `notifications/cancelled` reaches the export the same
/// way: nothing is sent after the request in flight, and the cancelled
/// request goes unanswered, as MCP has it (the SDK drops the answer).
#[tokio::test(start_paused = true)]
async fn a_cancel_over_the_pipe_stops_the_export() {
    let mut lab = export_lab(Policy::default(), Duration::from_millis(500), 20).await;
    lab.agent.initialize().await;
    let before = lab.sends().len();
    let id = lab
        .agent
        .post(
            "tools/call",
            json!({ "name": TOOL, "arguments": { "device": LAB } }),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(2_700)).await;
    lab.agent.cancel(id).await;
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(lab.sends().len() - before, 2 + 4, "none after the cancel");
    // Any answer to the cancelled call would have come before this one.
    lab.agent.request("tools/list", json!({})).await;
    assert!(
        !lab.agent.backlog.iter().any(|m| m["id"] == json!(id)),
        "{:?}",
        lab.agent.backlog
    );
    lab.stop().await;
}

/// A call budget that runs out mid-export keeps what was read, as a
/// cancel does. One that runs out before the Object_List is in has
/// nothing to keep: a timeout, after the one request it sent.
#[tokio::test(start_paused = true)]
async fn a_spent_budget_keeps_what_was_read() {
    let policy = |budget| Policy {
        caps: Caps {
            budget,
            ..Caps::default()
        },
        ..Policy::default()
    };
    // The name and the list take 1.2 s, then an object each 600 ms: the
    // budget runs out at 2 s with the second object on its way.
    let delay = Duration::from_millis(600);
    let mut lab = export_lab(policy(Duration::from_secs(2)), delay, 20).await;
    lab.agent.initialize().await;
    let result = lab.agent.call(TOOL, json!({ "device": LAB })).await;
    let body = ok(&result);
    assert_eq!(body["stopped_by"], json!("budget"), "{body}");
    assert_eq!(body["objects"], json!(2), "{body}");
    assert_eq!(body["cost"]["requests"], json!(2 + 2));
    assert!(
        body["cost"]["elapsed_ms"].as_u64().unwrap() <= 2_400,
        "{body}"
    );
    check_structure(texts(&result).1);
    lab.stop().await;

    let mut lab = export_lab(policy(Duration::from_millis(500)), delay, 0).await;
    lab.agent.initialize().await;
    let result = lab.agent.call(TOOL, json!({ "device": LAB })).await;
    let body = failed(&result);
    assert_eq!(body["error"], json!("timeout"), "{body}");
    assert_eq!(body["after_ms"], json!(500));
    assert_eq!(body["cost"]["requests"], json!(1));
    assert_eq!(result["content"].as_array().unwrap().len(), 1, "no Turtle");
    lab.stop().await;
}
