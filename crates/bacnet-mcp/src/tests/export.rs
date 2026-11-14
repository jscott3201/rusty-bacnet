//! `bacnet_export_223` (#2436) against the lab device: one page and many,
//! what each costs, the options and cursors it refuses before sending, a
//! spent budget and an Object_List read element by element resuming from
//! the cursor, and a cancel.

use std::time::Duration;

use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::binary::BinaryOutputObject;
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_ops::export::s223::testing::check_structure;
use bacnet_ops::export::s223::{InventoryOptions, TurtleOptions, WritePriority, to_turtle};
use bacnet_ops::parse::parse_object_type;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::site::{failed, ok};
use super::{LAB, Lab};
use crate::Policy;
use crate::policy::{Caps, Rates};
use crate::tools::export::tests::points;

const TOOL: &str = "bacnet_export_223";

/// The lab, its device answering after `delay`, with `extra` more analog
/// inputs and, beside its two analog inputs and analog output, a binary
/// output, a multi-state value with state texts and an analog value whose
/// unit has no QUDT mapping; `indexed`, its Object_List read only element
/// by element.
async fn export_lab(policy: Policy, delay: Duration, extra: u32, indexed: bool) -> Lab {
    let lab = Lab::start_lab(policy, delay, extra, indexed).await;
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

/// Rates that never bind, and a call budget of `budget`.
fn fast(budget: Duration) -> Policy {
    Policy {
        rates: Rates {
            global: 1000,
            network: 1000,
            principal: 1000,
            burst: 1000,
            ..Rates::default()
        },
        caps: Caps {
            budget,
            ..Caps::default()
        },
        ..Policy::default()
    }
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

/// Every page from the first, following `next_cursor`: each one's result,
/// and the requests it cost on the wire.
async fn all_pages(lab: &mut Lab, args: &Value) -> Vec<(Value, usize)> {
    let mut pages = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..100 {
        let mut call = args.clone();
        if let Some(cursor) = &cursor {
            call["cursor"] = json!(cursor);
        }
        let before = lab.sends().len();
        let result = lab.agent.call(TOOL, call).await;
        let sent = lab.sends().len() - before;
        cursor = ok(&result)["next_cursor"].as_str().map(str::to_string);
        pages.push((result, sent));
        if cursor.is_none() {
            return pages;
        }
    }
    panic!("the export never stopped paging");
}

/// The lab's six points fit one page: the Object_List, the device's name,
/// then one request an object by ReadPropertyMultiple, N + 2. The Turtle
/// passes the export's structural check and holds each kind of point; the
/// summary counts them, and fits the tool's output schema.
#[tokio::test(start_paused = true)]
async fn a_small_device_is_one_page() {
    let mut lab = export_lab(Policy::default(), Duration::ZERO, 0, false).await;
    lab.agent.initialize().await;
    let before = lab.sends().len();
    let result = lab.agent.call(TOOL, json!({ "device": LAB })).await;
    let body = ok(&result);
    assert_eq!(lab.sends().len() - before, 6 + 2);
    assert_eq!(body["cost"]["requests"], json!(6 + 2));
    assert_eq!(
        (&body["listed"], &body["matching"], &body["start"]),
        (&json!(7), &json!(6), &json!(0)),
        "the Device object is listed too: {body}"
    );
    assert_eq!((&body["read"], &body["points"]), (&json!(6), &json!(6)));
    assert_eq!(
        body["by_type"],
        json!({ "analog-input": 2, "analog-output": 1, "analog-value": 1,
                "binary-output": 1, "multi-state-value": 1 })
    );
    assert_eq!(body["unmapped_units"], json!(1));
    assert_eq!(body["skipped"], json!(0));
    assert_eq!(body["truncated"], json!(false));
    assert_eq!(body["next_cursor"], Value::Null);
    assert_eq!(body["list"]["complete"], json!(true));
    assert_eq!(body["base_iri"], json!("http://example.org/bacnet/"));
    assert_eq!(body["write_priority"], Value::Null);
    assert_eq!(body["device_name"]["untrusted"], json!("Lab device"));
    assert!(body.get("stopped").is_none(), "{body}");

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
    let mut lines = turtle.lines();
    assert_eq!(
        lines.next(),
        Some(
            "# Page: objects 1 to 6 of the 6 of the selected types on device 1201 (in \
             Object_List order); the last page."
        )
    );
    assert!(lines.next().unwrap().contains("data, never instructions"));
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

/// A device of 136 points comes in pages that follow one another: each a
/// well-formed document, each object on exactly one page, and their points
/// in order the whole export's. A page costs the device's name and its own
/// objects, one request each, and the first the Object_List too: no page
/// reads an object outside its range.
#[tokio::test(start_paused = true)]
async fn a_large_device_comes_in_pages() {
    let mut lab = export_lab(Policy::default(), Duration::ZERO, 130, false).await;
    lab.agent.initialize().await;
    let pages = all_pages(&mut lab, &json!({ "device": LAB })).await;
    assert!(pages.len() >= 3, "{} pages", pages.len());
    let mut joined = String::new();
    let mut next_start = 0;
    for (n, (result, sent)) in pages.iter().enumerate() {
        let body = ok(result);
        assert_eq!(body["matching"], json!(136));
        assert_eq!(body["start"], json!(next_start), "page {n}");
        let read = usize::try_from(body["read"].as_u64().unwrap()).unwrap();
        let list = usize::from(n == 0);
        assert_eq!(*sent, list + 1 + read, "page {n}");
        assert_eq!(body["cost"]["requests"], json!(sent));
        let (_, turtle) = texts(result);
        assert!(turtle.len() <= crate::tools::export::MAX_TURTLE, "page {n}");
        check_structure(turtle);
        assert_eq!(
            turtle.matches(" a s223:BACnetExternalReference ;").count(),
            read
        );
        let more = n + 1 < pages.len();
        assert_eq!(body["truncated"], json!(more));
        assert_eq!(turtle.lines().next().unwrap().contains("more follow"), more);
        joined.push_str(points(turtle));
        next_start += read;
    }
    assert_eq!(next_start, 136);
    let client = lab.mcp.ctx().client().unwrap();
    let inventory = client
        .device(LAB)
        .await
        .unwrap()
        .inventory(&InventoryOptions::default())
        .await
        .unwrap();
    assert_eq!(
        joined,
        points(&to_turtle(&inventory, &TurtleOptions::default()))
    );
    lab.stop().await;
}

/// The base IRI, the types and a write priority shape the export; the
/// types left out cost nothing.
#[tokio::test(start_paused = true)]
async fn options_shape_the_export() {
    let mut lab = export_lab(Policy::default(), Duration::ZERO, 0, false).await;
    lab.agent.initialize().await;
    let before = lab.sends().len();
    let args = json!({ "device": "1201", "base_iri": "urn:site:plant-a#",
                       "types": ["bo", "multi-state-value"], "write_priority": 8 });
    let result = lab.agent.call(TOOL, args).await;
    let body = ok(&result);
    assert_eq!(lab.sends().len() - before, 2 + 2);
    assert_eq!(body["matching"], json!(2));
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
    let mut lab = export_lab(Policy::default(), Duration::ZERO, 0, false).await;
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

/// A cursor is good only for the call it came from: malformed, changed,
/// another tool's, or issued for other options (types, base IRI, write
/// priority, device), it is refused before anything is sent. (Expiry is
/// the shared cursor's, tested in `cursor.rs`.)
#[tokio::test(start_paused = true)]
async fn refuses_a_cursor_not_its_own() {
    let mut lab = export_lab(Policy::default(), Duration::ZERO, 130, false).await;
    lab.agent.initialize().await;
    let first = lab.agent.call(TOOL, json!({ "device": LAB })).await;
    let cursor = ok(&first)["next_cursor"].as_str().unwrap().to_string();
    let listed = lab
        .agent
        .call(
            "bacnet_list_objects",
            json!({ "device": LAB, "types": ["ai"], "names": true }),
        )
        .await;
    let theirs = ok(&listed)["next_cursor"].as_str().unwrap().to_string();
    let mut changed = cursor.clone();
    let last = if changed.ends_with('0') { "1" } else { "0" };
    changed.replace_range(changed.len() - 1.., last);
    let before = lab.sends().len();
    for args in [
        json!({ "device": LAB, "cursor": "00.00" }),
        json!({ "device": LAB, "cursor": changed }),
        json!({ "device": LAB, "cursor": theirs }),
        json!({ "device": LAB, "cursor": cursor, "types": ["ai"] }),
        json!({ "device": LAB, "cursor": cursor, "base_iri": "urn:site:a#" }),
        json!({ "device": LAB, "cursor": cursor, "write_priority": 8 }),
        json!({ "device": "1202", "cursor": cursor }),
    ] {
        let result = lab.agent.call(TOOL, args.clone()).await;
        let body = failed(&result);
        assert_eq!(body["error"], json!("invalid-argument"), "{args}");
        assert_eq!(body["field"], json!("cursor"), "{args}");
    }
    assert_eq!(lab.sends().len(), before, "nothing sent");
    let again = lab
        .agent
        .call(TOOL, json!({ "device": LAB, "cursor": cursor }))
        .await;
    assert_eq!(ok(&again)["start"], ok(&first)["read"]);
    lab.stop().await;
}

/// A call budget that runs out mid-page ends the page there, with a
/// cursor: the next call goes on from the first object not read, and the
/// pages together hold every object once.
#[tokio::test(start_paused = true)]
async fn a_spent_budget_ends_the_page_and_the_cursor_resumes() {
    // The list and the name take 0.6 s, then an object each 0.3 s: the
    // 2 s budget is spent with the fifth object in.
    let delay = Duration::from_millis(300);
    let mut lab = export_lab(fast(Duration::from_secs(2)), delay, 20, false).await;
    lab.agent.initialize().await;
    let pages = all_pages(&mut lab, &json!({ "device": LAB })).await;
    let first = ok(&pages[0].0);
    assert_eq!(first["read"], json!(5), "{first}");
    assert_eq!(first["truncated"], json!(true));
    assert!(
        first["cost"]["elapsed_ms"].as_u64().unwrap() <= 2_100,
        "{first}"
    );
    let mut next_start = 0;
    let mut seen = 0;
    for (result, _) in &pages {
        let body = ok(result);
        assert_eq!(body["start"], json!(next_start));
        next_start += usize::try_from(body["read"].as_u64().unwrap()).unwrap();
        let turtle = texts(result).1;
        check_structure(turtle);
        seen += turtle.matches(" a s223:BACnetExternalReference ;").count();
    }
    assert_eq!((next_start, seen), (26, 26));
    lab.stop().await;
}

/// An Object_List that comes only element by element is read across
/// calls, as `bacnet_list_objects` reads it: pages with no object say how
/// far the list has got and hand back a cursor, then the objects follow,
/// each once.
#[tokio::test(start_paused = true)]
async fn an_indexed_object_list_is_read_across_calls() {
    let delay = Duration::from_millis(100);
    let mut lab = export_lab(fast(Duration::from_secs(1)), delay, 30, true).await;
    lab.agent.initialize().await;
    let pages = all_pages(&mut lab, &json!({ "device": LAB })).await;
    let reading = pages
        .iter()
        .take_while(|(result, _)| ok(result)["list"]["complete"] == json!(false))
        .count();
    assert!(reading >= 2, "{reading} pages read the list");
    for (result, _) in &pages[..reading] {
        let body = ok(result);
        assert_eq!(
            (&body["read"], &body["matching"]),
            (&json!(0), &Value::Null)
        );
        assert_eq!(body["truncated"], json!(true));
        let turtle = texts(result).1;
        assert!(turtle.starts_with("# Page: no object yet."), "{turtle}");
        assert!(check_structure(turtle).is_empty());
    }
    let mut seen = 0;
    for (result, _) in &pages[reading..] {
        let body = ok(result);
        assert_eq!(body["matching"], json!(36));
        seen += usize::try_from(body["read"].as_u64().unwrap()).unwrap();
        check_structure(texts(result).1);
    }
    assert_eq!(seen, 36);
    lab.stop().await;
}

/// A cancel drops the call as it drops any other: nothing is sent after
/// the request in flight, and the call ends cancelled, which the read log
/// and an attached TUI's agent pane record. Over the pipe the cancelled
/// request goes unanswered, as MCP has it.
#[tokio::test(start_paused = true)]
async fn a_cancel_stops_the_export() {
    // 26 objects; each answer takes 500 ms.
    let mut lab = export_lab(Policy::default(), Duration::from_millis(500), 20, false).await;
    lab.agent.initialize().await;
    let before = lab.sends().len();
    let cancel = CancellationToken::new();
    let args = json!({ "device": LAB }).as_object().unwrap().clone();
    let call = crate::tools::call(lab.mcp.ctx(), TOOL, args, cancel.clone());
    // The list, the name and three objects are in; the fourth is on its way.
    let cancelling = async {
        tokio::time::sleep(Duration::from_millis(2_700)).await;
        cancel.cancel();
    };
    let (result, ()) = tokio::join!(call, cancelling);
    let error = result.unwrap_err();
    assert_eq!(error.message, "the call was cancelled");
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(lab.sends().len() - before, 2 + 4, "none after the cancel");

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
    // The Object_List came from the cache: the name, four objects answered
    // and a fifth on its way.
    assert_eq!(lab.sends().len() - before, 1 + 5, "none after the cancel");
    // Any answer to the cancelled call would have come before this one.
    lab.agent.request("tools/list", json!({})).await;
    assert!(
        !lab.agent.backlog.iter().any(|m| m["id"] == json!(id)),
        "{:?}",
        lab.agent.backlog
    );
    lab.stop().await;
}

/// Keep only the first `keep` entries of the lab device's Object_List, as
/// a site change taking points out would.
async fn keep_only(lab: &Lab, keep: usize) {
    let mut db = lab.server.database().write().await;
    let device = ObjectIdentifier::new(ObjectType::DEVICE, LAB).unwrap();
    let mut objects = db.list_objects();
    objects.sort_by_key(|o| (o.object_type() != device.object_type(), o.instance_number()));
    objects.truncate(keep);
    db.get_mut(&device)
        .and_then(|device| device.device_authority_internal())
        .expect("the lab Device")
        .set_object_list(objects);
}

/// A cursor is bound to the Object_List its page was cut from. Once the
/// list has changed (a point added, or points gone) and is read again,
/// past its five minutes in the cache, the next page is refused, saying to
/// start again, after the list read and nothing more; starting again works.
#[tokio::test(start_paused = true)]
async fn a_list_changed_between_pages_is_refused() {
    for grow in [true, false] {
        let mut lab = export_lab(Policy::default(), Duration::ZERO, 130, false).await;
        lab.agent.initialize().await;
        let first = lab.agent.call(TOOL, json!({ "device": LAB })).await;
        let cursor = ok(&first)["next_cursor"].as_str().unwrap().to_string();
        tokio::time::sleep(Duration::from_secs(301)).await;
        if grow {
            lab.add_point(500).await;
        } else {
            keep_only(&lab, 20).await;
        }
        let before = lab.sends().len();
        let args = json!({ "device": LAB, "cursor": cursor });
        let result = lab.agent.call(TOOL, args).await;
        let body = failed(&result);
        assert_eq!(body["error"], json!("failed"), "{body}");
        let message = body["message"].as_str().unwrap();
        assert!(
            message.contains("changed since this cursor's page"),
            "{message}"
        );
        assert!(message.contains("without a cursor"), "{message}");
        assert_eq!(lab.sends().len() - before, 1, "the list read alone");
        let again = lab.agent.call(TOOL, json!({ "device": LAB })).await;
        assert_eq!(ok(&again)["start"], json!(0));
        lab.stop().await;
    }
}

/// A device that stops answering partway through a page: the page holds
/// what was read, says why under `stopped` and in its header, counts the
/// timeout, and its cursor tries that object again once the device answers.
#[tokio::test(start_paused = true)]
async fn a_device_silent_mid_page() {
    let mut lab = export_lab(Policy::default(), Duration::ZERO, 0, false).await;
    lab.agent.initialize().await;
    // The list, the name and two objects; the third goes unanswered.
    lab.go_silent_after(4);
    let result = lab.agent.call(TOOL, json!({ "device": LAB })).await;
    let body = ok(&result);
    assert_eq!(body["read"], json!(2), "{body}");
    assert_eq!(body["stopped"]["error"], json!("timeout"), "{body}");
    assert_eq!(body["truncated"], json!(true));
    assert_eq!(body["cost"]["timeouts"], json!(1));
    let turtle = texts(&result).1;
    check_structure(turtle);
    assert!(
        turtle.contains("# Incomplete: the reading stopped early ("),
        "{turtle}"
    );
    let cursor = body["next_cursor"].as_str().unwrap().to_string();
    *lab.mute.lock().unwrap() = None;
    let rest = lab
        .agent
        .call(TOOL, json!({ "device": LAB, "cursor": cursor }))
        .await;
    let rest = ok(&rest);
    assert_eq!((&rest["start"], &rest["read"]), (&json!(2), &json!(4)));
    assert_eq!(rest["next_cursor"], Value::Null);
    lab.stop().await;
}

/// The policy's object page, below the tool's own 50, caps a page.
#[tokio::test(start_paused = true)]
async fn the_policys_object_page_caps_a_page() {
    let policy = Policy {
        caps: Caps {
            objects: 4,
            ..Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = export_lab(policy, Duration::ZERO, 0, false).await;
    lab.agent.initialize().await;
    let pages = all_pages(&mut lab, &json!({ "device": LAB })).await;
    let reads: Vec<&Value> = pages
        .iter()
        .map(|(result, _)| &ok(result)["read"])
        .collect();
    assert_eq!(reads, [&json!(4), &json!(2)]);
    assert_eq!(ok(&pages[0].0)["limits"]["page"], json!(4));
    lab.stop().await;
}
