//! `bacnet_read_trend` against the lab's logs: paging across ReadRange
//! answers and calls, the summary against a brute-force computation
//! (#2066 acceptance 7), time windows, budgets and cancellation.

use std::time::Duration;

use serde_json::{Value, json};

use super::Lab;
use super::site::{
    LONG_RECORDS, STATE_RECORDS, TREND_RECORDS, failed, multi_state, ok, state_value,
    trend_flagged, trend_value,
};
use crate::Policy;
use crate::policy::{Caps, Rates};

/// The summary numbers of every record of trend-log:1, by brute force.
fn expected() -> (f64, f64, f64, u64) {
    let values: Vec<f64> = (0..TREND_RECORDS)
        .map(|i| f64::from(trend_value(i)))
        .collect();
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mean = values.iter().sum::<f64>() / f64::from(TREND_RECORDS);
    let flagged = (0..TREND_RECORDS).filter(|&i| trend_flagged(i)).count();
    (min, max, mean, u64::try_from(flagged).unwrap())
}

fn close(a: &Value, b: f64) -> bool {
    (a.as_f64().unwrap() - b).abs() < 1e-9
}

/// Acceptance 7: the whole log read in one call pages across at least
/// three ReadRange answers, some cut short by the device (MORE_ITEMS),
/// and the summary matches a brute-force computation over every record.
#[tokio::test(start_paused = true)]
async fn the_summary_pages_the_whole_log_and_matches_brute_force() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.agent.initialize().await;
    let result = lab
        .agent
        .call(
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "trend-log:1", "summary": true }),
        )
        .await;
    let body = ok(&result);
    assert!(body["pages"].as_u64().unwrap() >= 3, "{body}");
    assert!(body["more_items"].as_u64().unwrap() >= 1, "{body}");
    assert_eq!(body["records"], json!([]), "summary only");
    assert_eq!(body["done"], json!(true));
    assert_eq!(body["next_cursor"], Value::Null);
    let summary = &body["summary"];
    let (min, max, mean, flagged) = expected();
    assert_eq!(summary["records"], json!(TREND_RECORDS));
    assert_eq!(summary["flagged"], json!(flagged));
    let numeric = &summary["columns"][0]["numeric"];
    assert_eq!(numeric["count"], json!(TREND_RECORDS));
    assert!(close(&numeric["min"], min), "{numeric}");
    assert!(close(&numeric["max"], max), "{numeric}");
    assert!(close(&numeric["mean"], mean), "{numeric}");
    assert_eq!(summary["first_time"], json!("2026-10-25 00:00:00.00"));
    assert_eq!(summary["last_time"], json!("2026-10-25 04:19:00.00"));
    // Each ReadRange is charged: the counts, then a request a page.
    assert!(
        result["structuredContent"]["cost"]["requests"]
            .as_u64()
            .unwrap()
            >= body["pages"].as_u64().unwrap() + 2
    );
    lab.stop().await;
}

/// Records come back across calls with signed cursors: three calls of 100
/// cover the log once, in order, and their records give the same numbers
/// as the brute force.
#[tokio::test(start_paused = true)]
async fn records_page_across_calls() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.agent.initialize().await;
    let args = json!({ "device": 1201, "object": "trend-log:1", "max_records": 100 });
    let mut records = Vec::new();
    let mut cursor: Option<String> = None;
    let mut first_cursor = None;
    let mut calls = 0;
    loop {
        let mut call = args.clone();
        if let Some(cursor) = &cursor {
            call["cursor"] = json!(cursor);
        }
        let page = lab.agent.call("bacnet_read_trend", call).await;
        calls += 1;
        let body = ok(&page).clone();
        records.extend(body["records"].as_array().unwrap().iter().cloned());
        match body["next_cursor"].as_str() {
            Some(next) => {
                first_cursor.get_or_insert_with(|| next.to_string());
                cursor = Some(next.to_string());
            }
            None => {
                assert_eq!(body["done"], json!(true));
                break;
            }
        }
        assert!(calls < 5, "the paging ends");
    }
    assert_eq!(calls, 3);
    assert_eq!(records.len(), usize::try_from(TREND_RECORDS).unwrap());
    let seqs: Vec<u64> = records.iter().map(|r| r["seq"].as_u64().unwrap()).collect();
    let wanted: Vec<u64> = (1..=u64::from(TREND_RECORDS)).collect();
    assert_eq!(seqs, wanted);
    let values: Vec<f64> = records
        .iter()
        .map(|r| r["values"][0].as_f64().unwrap())
        .collect();
    let (min, max, mean, flagged) = expected();
    let got_min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let got_max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    #[allow(clippy::cast_precision_loss)]
    let got_mean = values.iter().sum::<f64>() / values.len() as f64;
    assert!((got_min - min).abs() < 1e-9 && (got_max - max).abs() < 1e-9);
    assert!((got_mean - mean).abs() < 1e-9);
    let marked = records
        .iter()
        .filter(|r| r["flagged"] == json!(true))
        .count();
    assert_eq!(u64::try_from(marked).unwrap(), flagged);
    let first_flagged = records
        .iter()
        .find(|r| r["flagged"] == json!(true))
        .unwrap();
    assert_eq!(first_flagged["status_flags"]["names"], json!(["in-alarm"]));
    // A cursor is for its own arguments only.
    let mut other = args.clone();
    other["max_records"] = json!(50);
    other["cursor"] = json!(first_cursor.unwrap());
    let refused = lab.agent.call("bacnet_read_trend", other).await;
    assert_eq!(failed(&refused)["field"], json!("cursor"));
    lab.stop().await;
}

/// A time window keeps the records after `from` and up to `to`; an
/// enumerated log counts its states; a bad object is an argument error.
#[tokio::test(start_paused = true)]
async fn windows_states_and_bad_arguments() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.agent.initialize().await;
    let window = lab
        .agent
        .call(
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "trend-log:1",
                    "from": "2026-10-25T01:00", "to": "2026-10-25 01:09" }),
        )
        .await;
    let body = ok(&window);
    let times: Vec<&str> = body["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["time"].as_str().unwrap())
        .collect();
    assert_eq!(times.len(), 9, "after 01:00, up to 01:09: {times:?}");
    assert_eq!(times[0], "2026-10-25 01:01:00.00");
    assert_eq!(times[8], "2026-10-25 01:09:00.00");
    assert_eq!(body["done"], json!(true));

    let states = lab
        .agent
        .call(
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "trend-log:2" }),
        )
        .await;
    let column = &ok(&states)["summary"]["columns"][0];
    assert_eq!(column["states"], json!({ "0": 7, "1": 3 }), "{column}");
    assert_eq!(column["numeric"], Value::Null);

    for (args, field) in [
        (json!({ "device": 1201, "object": "ai:1" }), "object"),
        (
            json!({ "device": 1201, "object": "trend-log:1", "from": "2026-02-30T00:00" }),
            "from",
        ),
        (
            json!({ "device": 1201, "object": "trend-log:1", "max_records": 5000 }),
            "max_records",
        ),
    ] {
        let result = lab.agent.call("bacnet_read_trend", args.clone()).await;
        assert_eq!(failed(&result)["field"], json!(field), "{args}");
    }
    let missing = lab
        .agent
        .call(
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "trend-log:9" }),
        )
        .await;
    assert_eq!(failed(&missing)["error"], json!("bacnet"));
    lab.stop().await;
}

/// A slow device and a short budget: the call stops at its budget with a
/// cursor, and the next call goes on with no record lost or repeated.
#[tokio::test(start_paused = true)]
async fn a_spent_budget_returns_a_cursor() {
    let policy = Policy {
        rates: Rates {
            global: 1000,
            network: 1000,
            principal: 1000,
            burst: 1000,
            ..Rates::default()
        },
        caps: Caps {
            budget: Duration::from_secs(1),
            ..Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = Lab::start_site(policy, Duration::from_millis(400)).await;
    lab.agent.initialize().await;
    let args = json!({ "device": 1201, "object": "trend-log:1", "max_records": 2000 });
    let first = lab.agent.call("bacnet_read_trend", args.clone()).await;
    let body = ok(&first).clone();
    assert_eq!(body["truncated"], json!(true), "{body}");
    let got = body["records"].as_array().unwrap().len();
    assert!(
        got > 0 && got < usize::try_from(TREND_RECORDS).unwrap(),
        "{got}"
    );
    let mut seqs: Vec<u64> = body["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["seq"].as_u64().unwrap())
        .collect();
    let mut cursor = body["next_cursor"].as_str().map(str::to_string);
    while let Some(token) = cursor {
        let mut call = args.clone();
        call["cursor"] = json!(token);
        let page = lab.agent.call("bacnet_read_trend", call).await;
        let body = ok(&page).clone();
        seqs.extend(
            body["records"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["seq"].as_u64().unwrap()),
        );
        cursor = body["next_cursor"].as_str().map(str::to_string);
    }
    let wanted: Vec<u64> = (1..=u64::from(TREND_RECORDS)).collect();
    assert_eq!(seqs, wanted);
    lab.stop().await;
}

/// A call stops when its structured content is full, part way through a
/// device's answer, and the cursor resumes at the first record left out.
#[tokio::test(start_paused = true)]
async fn a_full_page_resumes_mid_answer() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.agent.initialize().await;
    let args = json!({ "device": 1201, "object": "trend-log:3", "max_records": 2000 });
    let mut seqs = Vec::new();
    let mut cursor: Option<String> = None;
    let mut calls = 0;
    loop {
        let mut call = args.clone();
        if let Some(cursor) = &cursor {
            call["cursor"] = json!(cursor);
        }
        let page = lab.agent.call("bacnet_read_trend", call).await;
        calls += 1;
        let body = ok(&page).clone();
        let records = body["records"].as_array().unwrap();
        assert!(records.len() < usize::try_from(LONG_RECORDS).unwrap());
        seqs.extend(records.iter().map(|r| r["seq"].as_u64().unwrap()));
        match body["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
        assert!(calls < 10);
    }
    assert!(calls >= 2, "{calls}");
    let wanted: Vec<u64> = (1..=u64::from(LONG_RECORDS)).collect();
    assert_eq!(seqs, wanted);
    lab.stop().await;
}

/// A cancelled trend read stops sending.
#[tokio::test(start_paused = true)]
async fn a_cancelled_trend_read_stops() {
    let mut lab = Lab::start_site(Policy::default(), Duration::from_millis(500)).await;
    lab.agent.initialize().await;
    let id = lab
        .agent
        .post(
            "tools/call",
            json!({ "name": "bacnet_read_trend",
                    "arguments": { "device": 1201, "object": "trend-log:1", "summary": true } }),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    lab.agent
        .send(
            json!({ "jsonrpc": "2.0", "method": "notifications/cancelled",
                      "params": { "requestId": id, "reason": "test" } }),
        )
        .await;
    let at_cancel = lab.sends().len();
    tokio::time::sleep(Duration::from_secs(20)).await;
    let sent = lab.sends().len();
    assert!(
        sent <= at_cancel + 1,
        "{} went after the cancel",
        sent - at_cancel
    );
    assert!(sent < 6, "{sent}");
    lab.stop().await;
}

fn requests(result: &Value) -> u64 {
    result["structuredContent"]["cost"]["requests"]
        .as_u64()
        .unwrap()
}

/// A log whose Log_DeviceObjectProperty names Event_State gets its values
/// and state counts named, for one request more than its pages, however
/// many pages the call reads (#2091). The same records with an empty
/// reference cost the same, since the reference is read either way, and
/// keep their numbers.
#[tokio::test(start_paused = true)]
async fn a_referenced_log_names_its_values_for_one_request_more() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.agent.initialize().await;
    let named = lab
        .agent
        .call(
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "trend-log:4" }),
        )
        .await;
    let unnamed = lab
        .agent
        .call(
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "trend-log:5" }),
        )
        .await;
    let (body, plain) = (ok(&named), ok(&unnamed));
    let pages = body["pages"].as_u64().unwrap();
    assert!(pages >= 3, "{body}");
    assert_eq!(body["gaps"], json!([]));
    // From the oldest record: Total_Record_Count, Record_Count and
    // Total_Record_Count, a ReadRange a page, and the reference once.
    assert_eq!(requests(&named), 3 + pages + 1, "{body}");
    assert_eq!(plain["pages"], json!(pages));
    assert_eq!(requests(&unnamed), requests(&named));

    let records = body["records"].as_array().unwrap();
    assert_eq!(records.len(), usize::try_from(STATE_RECORDS).unwrap());
    assert_eq!(state_value(1), 2);
    assert_eq!(
        records[0]["values"][0],
        json!({ "value": 0, "name": "normal" })
    );
    assert_eq!(
        records[1]["values"][0],
        json!({ "value": 2, "name": "offnormal" })
    );
    assert_eq!(
        records[2]["values"][0],
        json!({ "value": 3, "name": "high-limit" })
    );
    let third = STATE_RECORDS / 3;
    assert_eq!(
        body["summary"]["columns"][0]["states"],
        json!({ "normal": third, "offnormal": third, "high-limit": third })
    );
    assert_eq!(plain["records"][1]["values"][0], json!({ "value": 2 }));
    assert_eq!(
        plain["summary"]["columns"][0]["states"],
        json!({ "0": third, "2": third, "3": third })
    );
    lab.stop().await;
}

/// A silent device fails the call after one timeout: the reference is
/// read only once the device has answered a page.
#[tokio::test(start_paused = true)]
async fn a_silent_device_costs_one_timeout() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.agent.initialize().await;
    lab.go_silent_after(0);
    let before = lab.sends().len();
    let result = lab
        .agent
        .call(
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "trend-log:4" }),
        )
        .await;
    let body = failed(&result);
    assert_eq!(body["error"], json!("timeout"), "{result}");
    assert_eq!(body["cost"]["timeouts"], json!(1), "{result}");
    assert_eq!(body["cost"]["requests"], json!(1), "{result}");
    assert_eq!(lab.sends().len() - before, 1, "one request, one timeout");
    lab.stop().await;
}

/// An envelope of `text` from multi-state-value:1 or binary-value:1.
fn device_text(text: &str, object: &str, property: &str) -> Value {
    json!({
        "untrusted": text,
        "source": format!("device 1201 {object} {property}"),
        "truncated": false,
    })
}

/// A Trend Log Multiple sampling multi-state-value:1, binary-value:1 and
/// the missing multi-state-value:9: the states are counted by state, and the
/// first two columns' keys labelled with their objects' state texts in
/// envelopes, cleaned; the values keep their typed form, and the missing
/// object's refused read and a state past State_Text leave numbers. The
/// texts cost one request an object, once a call however many pages it
/// reads (#2122).
#[tokio::test(start_paused = true)]
async fn state_columns_are_labelled_with_enveloped_state_texts() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.agent.initialize().await;
    let result = lab
        .agent
        .call(
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "trend-log-multiple:1" }),
        )
        .await;
    let body = ok(&result);
    let pages = body["pages"].as_u64().unwrap();
    assert!(pages >= 2, "{body}");
    // The counts, a ReadRange a page, the reference, and the texts of
    // three objects.
    assert_eq!(requests(&result), 3 + pages + 1 + 3, "{body}");

    let records = body["records"].as_array().unwrap();
    assert_eq!(records.len(), usize::try_from(STATE_RECORDS).unwrap());
    assert_eq!(multi_state(1), 2);
    assert_eq!(
        records[1]["values"],
        json!([2, { "value": 1, "name": "active" }, 1])
    );
    let quarter = STATE_RECORDS / 4;
    let half = STATE_RECORDS / 2;
    let columns = &body["summary"]["columns"];
    assert_eq!(
        columns[0]["states"],
        json!({ "1": quarter, "2": quarter, "3": quarter, "4": quarter })
    );
    assert_eq!(columns[0]["numeric"], Value::Null);
    let speed = |text: &str| device_text(text, "multi-state-value:1", "state-text");
    // The text as the device sent it, under the envelope's own cap.
    let low = "Low".repeat(50);
    assert_eq!(
        columns[0]["state_texts"],
        json!({ "1": speed("Off"), "2": speed(&low), "3": speed("High") })
    );
    assert_eq!(
        columns[1]["states"],
        json!({ "active": half, "inactive": half })
    );
    assert_eq!(
        columns[1]["state_texts"],
        json!({
            "active": device_text("Running", "binary-value:1", "active-text"),
            "inactive": device_text("Stopped", "binary-value:1", "inactive-text"),
        })
    );
    assert_eq!(columns[2]["states"], json!({ "1": STATE_RECORDS }));
    assert_eq!(columns[2]["state_texts"], Value::Null, "refused");
    lab.stop().await;
}

/// A device that goes silent after the reference costs one timeout: the
/// first state text read times out and no other is sent, and the records
/// read come back, their states counted without texts. The cost counts
/// the timeout (#2131).
#[tokio::test(start_paused = true)]
async fn a_device_silent_at_the_state_texts_costs_one_timeout() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.agent.initialize().await;
    // The counts, one page and the reference get through.
    lab.go_silent_after(3 + 1 + 1);
    let before = lab.sends().len();
    let result = lab
        .agent
        .call(
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "trend-log-multiple:1", "max_records": 5 }),
        )
        .await;
    let body = ok(&result);
    assert_eq!(body["pages"], json!(1), "{body}");
    assert_eq!(lab.sends().len() - before, 3 + 1 + 1 + 1, "{body}");
    // One timeout of the client's 1 s, not one an object.
    let elapsed = result["structuredContent"]["cost"]["elapsed_ms"]
        .as_u64()
        .unwrap();
    assert!((1_000..2_000).contains(&elapsed), "{result}");
    // The call succeeds, but the timeout counts (#2131).
    assert_eq!(result["structuredContent"]["cost"]["timeouts"], json!(1));
    let columns = &body["summary"]["columns"];
    assert_eq!(columns[0]["state_texts"], Value::Null, "{body}");
    assert_eq!(columns[1]["states"]["active"], json!(2), "{body}");
    lab.stop().await;
}

/// A device that goes silent after its first page: the reference read
/// times out, the page read comes back unnamed, and the cost counts the
/// timeout (#2131). Read on, the next page times out too and counts again.
#[tokio::test(start_paused = true)]
async fn a_device_silent_at_the_reference_counts_its_timeout() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.agent.initialize().await;
    // The counts and one page get through.
    lab.go_silent_after(3 + 1);
    let before = lab.sends().len();
    let result = lab
        .agent
        .call(
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "trend-log:4", "max_records": 5 }),
        )
        .await;
    let body = ok(&result);
    assert_eq!(body["pages"], json!(1), "{body}");
    assert_eq!(lab.sends().len() - before, 3 + 1 + 1, "{body}");
    assert_eq!(body["records"][1]["values"][0], json!({ "value": 2 }));
    let cost = &result["structuredContent"]["cost"];
    assert_eq!(cost["timeouts"], json!(1), "{result}");

    lab.go_silent_after(3 + 1);
    let result = lab
        .agent
        .call(
            "bacnet_read_trend",
            json!({ "device": 1201, "object": "trend-log:4" }),
        )
        .await;
    let body = ok(&result);
    assert_eq!(body["pages"], json!(1), "{body}");
    assert_eq!(body["stopped"]["error"], json!("timeout"), "{body}");
    let cost = &result["structuredContent"]["cost"];
    assert_eq!(cost["timeouts"], json!(2), "{result}");
    lab.stop().await;
}
