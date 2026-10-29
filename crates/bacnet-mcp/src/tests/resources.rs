//! Resources over the pipe (#2067): the lists, each resource's content,
//! the envelope on device text, cancellation, and the `bacnet_watch`
//! fallback for hosts without subscriptions.

use std::time::Duration;

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde_json::{Value, json};

use super::listen::present_value;
use super::site::{failed, ok};
use super::{Lab, hostile};
use crate::Policy;

/// The listed resources and templates are named from identifiers only.
#[tokio::test(start_paused = true)]
async fn lists_name_resources_from_identifiers_only() {
    let mut lab = Lab::start(Policy::default()).await;
    let listed = lab.agent.modern("resources/list", json!({})).await;
    let resources = listed["result"]["resources"].as_array().unwrap().clone();
    let uris: Vec<&str> = resources
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    assert_eq!(
        uris,
        [
            "bacnet://devices",
            "bacnet://alarms",
            "bacnet://session",
            "bacnet://ledger"
        ]
    );
    let templates = lab
        .agent
        .modern("resources/templates/list", json!({}))
        .await;
    let templates = templates["result"]["resourceTemplates"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(templates.len(), 5);
    for item in resources.iter().chain(&templates) {
        let text = item.to_string();
        assert!(
            !text.contains("Lab device") && !text.contains("Zone Temp"),
            "{text}"
        );
        assert_eq!(item["mimeType"], json!("application/json"));
    }
    // A 2026-07-28 client learns that it can subscribe, and so does one on
    // an older revision, which subscribes with resources/subscribe (#2114).
    let discover = lab.agent.modern("server/discover", json!({})).await;
    let resources = &discover["result"]["capabilities"]["resources"];
    assert_eq!(resources["subscribe"], json!(true), "{discover}");
    let hello = lab
        .agent
        .request(
            "initialize",
            json!({ "protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": { "name": "old-agent", "version": "1" } }),
        )
        .await;
    assert_eq!(hello["result"]["protocolVersion"], json!("2025-06-18"));
    let resources = &hello["result"]["capabilities"]["resources"];
    assert!(resources.is_object(), "{hello}");
    assert_eq!(resources["subscribe"], json!(true), "{hello}");
    lab.stop().await;
}

/// An unwatched point is read once, with values in the tools' shapes;
/// device text in a property is enveloped, cleaned and capped.
#[tokio::test(start_paused = true)]
async fn a_point_is_read_once_and_device_text_is_enveloped() {
    let mut lab = Lab::start(Policy::default()).await;
    let content = lab
        .agent
        .read("bacnet://device/1201/object/ai:1")
        .await
        .unwrap();
    assert_eq!(content["watched"], json!(false));
    assert_eq!(content["source"], json!("read"));
    assert_eq!(present_value(&content), json!(72.5));
    let rows = content["values"].as_array().unwrap();
    let name = rows
        .iter()
        .find(|row| row["property"] == json!("object-name"))
        .unwrap();
    assert_eq!(name["value"]["typed"]["untrusted"], json!("Zone Temp"));
    assert!(content["cost"]["requests"].as_u64().unwrap() >= 1);

    let uri = "bacnet://device/1201/object/analog-input:3/property/description";
    let content = lab.agent.read(uri).await.unwrap();
    assert_eq!(content["uri"], json!(uri));
    assert_eq!(content["property"], json!("description"));
    let typed = &content["values"][0]["value"]["typed"];
    let text = typed["untrusted"].as_str().unwrap();
    assert!(text.contains("hxxps://evil.example/x"), "{text}");
    assert!(
        !text.contains('\u{202E}') && !text.contains('\u{E0049}'),
        "{text}"
    );
    assert_eq!(typed["truncated"], json!(true));
    assert_eq!(text.chars().count(), 256);
    assert!(!text.contains("tail-marker") && hostile().contains("tail-marker"));

    // Not discovered, or not a resource: refused, nothing sent.
    let before = lab.sends().len();
    let error = lab
        .agent
        .read("bacnet://device/4242/object/ai:1")
        .await
        .unwrap_err();
    assert_eq!(error["code"], json!(-32602));
    assert_eq!(error["data"]["error"], json!("unknown-device"));
    let error = lab
        .agent
        .read("bacnet://device/1201/thing")
        .await
        .unwrap_err();
    assert_eq!(error["data"]["field"], json!("uri"));
    assert_eq!(lab.sends().len(), before);
    lab.stop().await;
}

/// The device summary is cached; the session and the ledger send nothing.
#[tokio::test(start_paused = true)]
async fn device_session_and_ledger_resources() {
    let mut lab = Lab::start(Policy::default()).await;
    let device = lab.agent.read("bacnet://device/1201").await.unwrap();
    assert_eq!(device["device"], json!(1201));
    assert_eq!(device["name"]["untrusted"], json!("Lab device"));
    assert_eq!(device["cached"], json!(false));
    let before = lab.sends().len();
    let again = lab.agent.read("bacnet://device/1201").await.unwrap();
    assert_eq!(again["cached"], json!(true));
    let session = lab.agent.read("bacnet://session").await.unwrap();
    assert_eq!(session["level"], json!("read-only"));
    assert_eq!(session["cov"]["max"], json!(256));
    assert_eq!(session["cov"]["lifetime_s"], json!(300));
    let ledger = lab.agent.read("bacnet://ledger").await.unwrap();
    assert_eq!(ledger["entries"], json!([]));
    assert_eq!(lab.sends().len(), before);
    lab.stop().await;
}

/// The alarm resources read GetEventInformation at most once a minute per
/// device; a subscription to them polls on that schedule.
#[tokio::test(start_paused = true)]
async fn alarms_are_read_at_most_once_a_minute() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    let alarms = lab.agent.read("bacnet://device/1201/alarms").await.unwrap();
    let events = alarms["events"].as_array().unwrap();
    assert_eq!(events.len(), 3, "{alarms}");
    let before = lab.sends().len();
    let again = lab.agent.read("bacnet://device/1201/alarms").await.unwrap();
    assert_eq!(again["events"], alarms["events"]);
    assert_eq!(
        lab.sends().len(),
        before,
        "within the minute, from the cache"
    );
    let all = lab.agent.read("bacnet://alarms").await.unwrap();
    let first = all["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["device"] == json!(1201))
        .unwrap();
    assert_eq!(first["events"], alarms["events"]);
    // The routed devices behind the lab don't answer: their errors show.
    assert_eq!(all["errors"]["300"]["error"], json!("timeout"), "{all}");
    tokio::time::sleep(Duration::from_secs(61)).await;
    let before = lab.sends().len();
    lab.agent.read("bacnet://device/1201/alarms").await.unwrap();
    assert!(lab.sends().len() > before, "read again after a minute");
    let (_, accepted) = lab.agent.listen(&["bacnet://device/1201/alarms"]).await;
    assert_eq!(accepted, ["bacnet://device/1201/alarms"]);
    lab.stop().await;
}

/// A resource read cancelled while it waits sends nothing more.
#[tokio::test(start_paused = true)]
async fn a_cancelled_read_stops() {
    let mut lab = Lab::start_with(Policy::default(), Duration::from_secs(3)).await;
    let id = lab
        .agent
        .post(
            "resources/read",
            json!({ "_meta": super::modern_meta(), "uri": "bacnet://device/1201" }),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    lab.agent.cancel(id).await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(lab.sends().len() <= 1, "{:?}", lab.sends());
    lab.stop().await;
}

/// The tool-only fallback: a first call returns every value, a call with
/// its cursor returns only what changed, and reading sends nothing.
#[tokio::test(start_paused = true)]
async fn bacnet_watch_returns_what_changed_since_the_cursor() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize().await;
    let points = json!([{ "device": 1201, "object": "ai:1" },
                        { "device": 1201, "object": "ao:1", "property": "present-value" }]);
    let first = lab
        .agent
        .call("bacnet_watch", json!({ "points": points }))
        .await;
    let body = ok(&first).clone();
    let rows = body["points"].as_array().unwrap();
    assert!(
        rows.iter().all(|row| row["changed"] == json!(true)),
        "{body}"
    );
    assert_eq!(present_value(&rows[0]), json!(72.5));
    assert_eq!(rows[0]["source"], json!("cov"));
    assert_eq!(
        rows[1]["uri"],
        json!("bacnet://device/1201/object/analog-output:1/property/present-value")
    );
    assert_eq!(lab.cov_table().await.len(), 2);
    let cursor = body["next_cursor"].clone();
    // The device's first notification on each subscription may come just
    // after the call; it would show as a change.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let before = lab.sends().len();
    let second = lab
        .agent
        .call(
            "bacnet_watch",
            json!({ "points": points, "cursor": cursor }),
        )
        .await;
    let body = ok(&second).clone();
    for row in body["points"].as_array().unwrap() {
        assert_eq!(row["changed"], json!(false), "{row}");
        assert!(row.get("values").is_none());
    }
    assert_eq!(
        lab.sends().len(),
        before,
        "nothing sent for an unchanged point"
    );

    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    lab.server
        .set_present_value_local(&ai1, PropertyValue::Real(80.0))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let third = lab
        .agent
        .call(
            "bacnet_watch",
            json!({ "points": points, "cursor": body["next_cursor"] }),
        )
        .await;
    let rows = ok(&third)["points"].as_array().unwrap().clone();
    assert_eq!(rows[0]["changed"], json!(true));
    assert_eq!(present_value(&rows[0]), json!(80.0));
    assert_eq!(rows[1]["changed"], json!(false));

    // Another call's cursor doesn't fit these points.
    let other = lab
        .agent
        .call(
            "bacnet_watch",
            json!({ "points": [{ "device": 1201, "object": "ai:3" }],
                    "cursor": ok(&third)["next_cursor"] }),
        )
        .await;
    assert_eq!(failed(&other)["field"], json!("cursor"));

    // Unasked for, the points go after ten minutes and their subscriptions
    // are cancelled.
    tokio::time::sleep(Duration::from_secs(11 * 60)).await;
    assert!(lab.cov_table().await.is_empty());
    lab.stop().await;
}

/// The fallback holds to the principal's cap, and refuses a device out of
/// reach point by point.
#[tokio::test(start_paused = true)]
async fn bacnet_watch_keeps_to_the_cap_and_the_reach() {
    let policy = Policy {
        devices: Some(vec![(1201, 1201)]),
        caps: crate::policy::Caps {
            watches: 1,
            ..crate::policy::Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize().await;
    let result = lab
        .agent
        .call(
            "bacnet_watch",
            json!({ "points": [{ "device": 1201, "object": "ai:1" }] }),
        )
        .await;
    ok(&result);
    let result = lab
        .agent
        .call(
            "bacnet_watch",
            json!({ "points": [{ "device": 1201, "object": "ai:3" },
                               { "device": 1202, "object": "ai:1" }] }),
        )
        .await;
    let rows: Vec<Value> = ok(&result)["points"].as_array().unwrap().clone();
    assert_eq!(rows[0]["error"]["reason"], json!("watch-cap"), "{rows:?}");
    assert_eq!(rows[1]["error"]["reason"], json!("device-not-allowed"));
    lab.stop().await;
}

/// A policy that doesn't validate is refused, not a panic.
#[test]
fn a_bad_policy_is_refused_when_the_server_is_made() {
    use bacnet_transport::bip::BipTransport;
    let policy = Policy {
        caps: crate::policy::Caps {
            process_watches: 0,
            ..crate::policy::Caps::default()
        },
        ..Policy::default()
    };
    assert!(crate::McpServer::<BipTransport>::new(None, policy, "tester").is_err());
}

/// A subscribed alarm resource whose events don't change sends no update:
/// a device read for the first time on a tick is unknown, not changed.
#[tokio::test(start_paused = true)]
async fn unchanged_alarms_send_no_update() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.known().await;
    let uris = ["bacnet://alarms", "bacnet://device/1201/alarms"];
    let (_, accepted) = lab.agent.listen(&uris).await;
    assert_eq!(accepted, uris);
    let updates = lab
        .agent
        .drain("notifications/resources/updated", Duration::from_secs(200))
        .await;
    assert!(updates.is_empty(), "{updates:?}");
    lab.stop().await;
}
