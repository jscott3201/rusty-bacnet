//! Part B's tools at the edges: devices that go silent, spent budgets,
//! cancelled queries and arguments refused as protocol errors.

use std::time::Duration;

use serde_json::{Value, json};

use super::Lab;
use super::site::{failed, ok};
use crate::Policy;
use crate::policy::{Caps, Rates};

/// Rates that never bind, and a call budget of `budget`.
fn budget(budget: Duration) -> Policy {
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

/// Call `tool` with `args` and then each `next_cursor` in turn; every
/// page's structured content.
async fn all_pages(lab: &mut Lab, tool: &str, args: &Value) -> Vec<Value> {
    let mut pages = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..50 {
        let mut call = args.clone();
        if let Some(cursor) = &cursor {
            call["cursor"] = json!(cursor);
        }
        let page = lab.agent.call(tool, call).await;
        let body = ok(&page).clone();
        cursor = body["next_cursor"].as_str().map(str::to_string);
        pages.push(body);
        if cursor.is_none() {
            return pages;
        }
    }
    panic!("{tool} never stopped paging");
}

/// A device that goes silent part way through an element-by-element
/// Object_List is searched as far as it was read and listed as
/// incomplete, with why; the search doesn't come back to it.
#[tokio::test(start_paused = true)]
async fn find_lists_a_device_silent_mid_list_as_incomplete() {
    let mut lab = Lab::start_lab(Policy::default(), Duration::ZERO, 20, true).await;
    lab.agent.initialize().await;
    // The whole read (refused), the count, then three elements.
    lab.go_silent_after(5);
    let result = lab
        .agent
        .call(
            "bacnet_find_object",
            json!({ "types": ["ai"], "range": "1201" }),
        )
        .await;
    let body = ok(&result);
    let incomplete = &body["incomplete"][0];
    assert_eq!(incomplete["device"], json!(1201), "{body}");
    assert_eq!(incomplete["read"], json!(3), "{body}");
    assert_eq!(incomplete["elements"], json!(24), "{body}");
    assert!(incomplete["why"].as_str().unwrap().contains("element 4"));
    assert_eq!(body["next_cursor"], Value::Null);
    let found = body["matches"].as_array().unwrap();
    assert!(!found.is_empty() && found.len() < 3, "{body}");
    lab.stop().await;
}

/// A device that stops answering while its names are read is reported
/// incomplete and left: no cursor comes back to it, so a silent device
/// doesn't cost every later page a timeout (pinned on purpose).
#[tokio::test(start_paused = true)]
async fn find_leaves_a_device_silent_while_naming() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.agent.initialize().await;
    // The Object_List, then the first object's name.
    lab.go_silent_after(2);
    let result = lab
        .agent
        .call(
            "bacnet_find_object",
            json!({ "name": "zone", "range": "1201" }),
        )
        .await;
    let body = ok(&result);
    let incomplete = &body["incomplete"][0];
    assert_eq!(incomplete["device"], json!(1201), "{body}");
    assert_eq!(incomplete["read"], json!(1));
    assert!(
        incomplete["why"]
            .as_str()
            .unwrap()
            .starts_with("stopped reading names")
    );
    assert_eq!(body["matches"], json!([]));
    assert_eq!(body["next_cursor"], Value::Null);
    assert_eq!(result["structuredContent"]["cost"]["timeouts"], json!(1));
    lab.stop().await;
}

/// A device that stops answering after the first object of a read: the
/// read returns what it read, stopped at the timeout, and the cost counts
/// it (#2131).
#[tokio::test(start_paused = true)]
async fn read_counts_a_device_silent_after_its_first_object() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.agent.initialize().await;
    lab.go_silent_after(1);
    let result = lab
        .agent
        .call(
            "bacnet_read",
            json!({ "device": "1201", "reads": [{ "object": "ai:1" }, { "object": "ai:2" }] }),
        )
        .await;
    let body = ok(&result);
    assert_eq!(body["stopped"]["error"], json!("timeout"), "{body}");
    assert_eq!(result["structuredContent"]["cost"]["timeouts"], json!(1));
    lab.stop().await;
}

/// An Object_List longer than one call's budget: the first page stops with
/// the list half read and a cursor to the same device, and the pages after
/// go on reading it and then find every match once.
#[tokio::test(start_paused = true)]
async fn find_resumes_a_list_cut_by_the_budget() {
    let policy = budget(Duration::from_secs(2));
    let mut lab = Lab::start_lab(policy, Duration::from_millis(50), 150, true).await;
    lab.agent.initialize().await;
    let pages = all_pages(
        &mut lab,
        "bacnet_find_object",
        &json!({ "types": ["ai"], "range": "1201" }),
    )
    .await;
    assert!(pages.len() >= 3, "{}", pages.len());
    assert_eq!(pages[0]["matches"], json!([]), "{}", pages[0]);
    assert_eq!(pages[0]["searched"], json!([1201]));
    assert_eq!(pages[0]["incomplete"], json!([]));
    let mut found: Vec<String> = pages
        .iter()
        .flat_map(|page| page["matches"].as_array().unwrap().clone())
        .map(|m| m["object"].as_str().unwrap().to_string())
        .collect();
    let count = found.len();
    found.sort();
    found.dedup();
    assert_eq!((count, found.len()), (152, 152), "every match once");
    lab.stop().await;
}

/// Alarms from a slow device under a short budget: no answer is asked for
/// once the budget is spent, and what was read comes back with a cursor
/// that goes on after it, so every event comes once.
#[tokio::test(start_paused = true)]
async fn alarms_stop_at_the_budget_and_resume() {
    let policy = budget(Duration::from_secs(1));
    let mut lab = Lab::start_site(policy, Duration::from_millis(600)).await;
    lab.agent.initialize().await;
    let pages = all_pages(&mut lab, "bacnet_alarms", &json!({ "device": 1201 })).await;
    assert!(pages.len() >= 2, "{pages:?}");
    let first = &pages[0];
    assert_eq!(first["more_events"], json!(true), "{first}");
    assert_eq!(first["truncated"], json!(true));
    assert!(first["cost"]["requests"].as_u64().unwrap() <= 2, "{first}");
    let instances: Vec<u64> = pages
        .iter()
        .flat_map(|page| page["events"].as_array().unwrap().clone())
        .map(|event| event["object"]["instance"].as_u64().unwrap())
        .collect();
    assert_eq!(instances, [1, 2, 3]);
    lab.stop().await;
}

/// When the events spend the budget, the alarm and enrollment summaries
/// aren't sent: each is reported skipped for the budget.
#[tokio::test(start_paused = true)]
async fn summaries_wait_for_a_call_with_time() {
    let policy = budget(Duration::from_secs(1));
    let mut lab = Lab::start_site(policy, Duration::from_millis(600)).await;
    lab.agent.initialize().await;
    let result = lab
        .agent
        .call(
            "bacnet_alarms",
            json!({ "device": 1201, "summaries": true }),
        )
        .await;
    let body = ok(&result);
    assert_eq!(body["more_events"], json!(true), "{body}");
    assert_eq!(body["alarm_summary"], Value::Null);
    assert_eq!(body["enrollment_summary"], Value::Null);
    for which in ["alarm", "enrollment"] {
        assert_eq!(
            body["summary_errors"][which]["skipped"],
            json!("budget"),
            "{body}"
        );
    }
    let requests = body["cost"]["requests"].as_u64().unwrap();
    assert_eq!(
        requests,
        u64::try_from(body["events"].as_array().unwrap().len()).unwrap()
    );
    lab.stop().await;
}

/// The wait for routers ends with the call's budget: a 2 s budget with
/// wait_s 30 returns what was heard, not a timeout.
#[tokio::test(start_paused = true)]
async fn the_router_wait_ends_with_the_budget() {
    let policy = Policy {
        broad_discovery: true,
        ..budget(Duration::from_secs(2))
    };
    let mut lab = Lab::start_site(policy, Duration::ZERO).await;
    lab.agent.initialize().await;
    let ask = lab
        .agent
        .call("bacnet_network_map", json!({ "send": true, "wait_s": 30 }))
        .await;
    let body = ok(&ask);
    assert_eq!(body["sent"], json!(true));
    assert!(
        body["cost"]["elapsed_ms"].as_u64().unwrap() <= 2_500,
        "{body}"
    );
    assert_eq!(body["routers_heard_s"], json!(2));
    lab.stop().await;
}

/// A router query cancelled while it waits keeps its claim (the broadcast
/// went, so the next waits its minute) but records no routers heard.
#[tokio::test(start_paused = true)]
async fn a_cancelled_router_query_records_nothing_heard() {
    let policy = Policy {
        broad_discovery: true,
        ..Policy::default()
    };
    let mut lab = Lab::start_site(policy, Duration::ZERO).await;
    lab.agent.initialize().await;
    let id = lab
        .agent
        .post(
            "tools/call",
            json!({ "name": "bacnet_network_map",
                    "arguments": { "send": true, "wait_s": 20 } }),
        )
        .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    lab.agent
        .send(
            json!({ "jsonrpc": "2.0", "method": "notifications/cancelled",
                      "params": { "requestId": id, "reason": "test" } }),
        )
        .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let map = lab.agent.call("bacnet_network_map", json!({})).await;
    assert_eq!(ok(&map)["routers_heard_s"], Value::Null, "{map}");
    let again = lab
        .agent
        .call("bacnet_network_map", json!({ "send": true }))
        .await;
    assert_eq!(failed(&again)["reason"], json!("rate-limited"));
    assert_eq!(lab.sends().len(), 1, "one broadcast in all");
    lab.stop().await;
}

/// A description matches past the characters a result shows: the search
/// keeps more of the text than the envelope carries.
#[tokio::test(start_paused = true)]
async fn find_matches_past_the_shown_text() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.agent.initialize().await;
    let result = lab
        .agent
        .call(
            "bacnet_find_object",
            json!({ "description": "TAIL-MARKER", "types": ["ai"], "range": "1201" }),
        )
        .await;
    let body = ok(&result);
    let found = body["matches"].as_array().unwrap();
    assert_eq!(found.len(), 1, "{body}");
    assert_eq!(found[0]["object"], json!("analog-input:3"));
    let shown = &found[0]["description"];
    assert_eq!(shown["truncated"], json!(true));
    let text = shown["untrusted"].as_str().unwrap();
    assert_eq!(text.chars().count(), 256);
    assert!(
        !text.contains("tail-marker"),
        "the marker is past the shown text"
    );
    lab.stop().await;
}

/// A trend window that ends before it starts is refused as arguments
/// outside the schema are.
#[tokio::test(start_paused = true)]
async fn a_backwards_window_is_a_protocol_error() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.agent.initialize().await;
    let reply = lab
        .agent
        .request(
            "tools/call",
            json!({ "name": "bacnet_read_trend",
                    "arguments": { "device": 1201, "object": "trend-log:1",
                                   "from": "2026-10-25T02:00", "to": "2026-10-25T01:00" } }),
        )
        .await;
    assert_eq!(reply["error"]["code"], json!(-32602), "{reply}");
    assert!(lab.sends().is_empty());
    lab.stop().await;
}
