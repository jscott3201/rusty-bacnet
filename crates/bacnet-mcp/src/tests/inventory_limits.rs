//! `bacnet_inventory` (#2779) at its limits: the policy's object page, a
//! device without ReadPropertyMultiple (one ReadProperty a property), a
//! page the call's budget ends at the default rates, a later batch still
//! being read well past the budget, and a first one stuck past the grace.

use std::time::Duration;

use serde_json::{Value, json};

use super::inventory::{TOOL, all_pages, inventory_lab, listed, object, point, points};
use super::site::{failed, ok};
use super::{LAB, Lab};
use crate::Policy;
use crate::policy::{Caps, Rates};

/// The policy's object page, below the tool's 50, ends a page as
/// `page-full`; the last page is `complete`.
#[tokio::test(start_paused = true)]
async fn the_policys_object_page_ends_a_page() {
    let policy = Policy {
        caps: Caps {
            objects: 4,
            ..Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = inventory_lab(policy, Duration::ZERO, 0, false).await;
    lab.agent.initialize().await;
    let pages = all_pages(&mut lab, &json!({ "device": LAB })).await;
    let ended: Vec<(&Value, &Value)> = pages
        .iter()
        .map(|(result, _)| (&ok(result)["read"], &ok(result)["ended"]))
        .collect();
    assert_eq!(
        ended,
        [
            (&json!(4), &json!("page-full")),
            (&json!(2), &json!("complete"))
        ]
    );
    assert_eq!(ok(&pages[0].0)["limits"]["page"], json!(4));
    lab.stop().await;
}

/// The requests a call sent on the wire, and its result.
async fn sent(lab: &mut Lab, args: Value) -> (usize, Value) {
    let before = lab.sends().len();
    let result = lab.agent.call(TOOL, args).await;
    (lab.sends().len() - before, result)
}

/// A device that refuses ReadPropertyMultiple: one refused request, which
/// the client remembers, then one ReadProperty a property, so the options
/// cost requests here. An analog input reads 3 with no option, 7 with
/// values and 12 with every option; an analog output 3, 8 (its
/// Current_Command_Priority too) and 14. The records read the same.
#[tokio::test(start_paused = true)]
async fn a_device_without_rpm_costs_a_request_a_property() {
    let mut lab = inventory_lab(Policy::default(), Duration::ZERO, 0, false).await;
    lab.refuse_rpm();
    lab.agent.initialize().await;
    let types = json!(["ai", "ao"]);
    let (plain, result) = sent(&mut lab, json!({ "device": LAB, "types": types })).await;
    // The list, the refused ReadPropertyMultiple, then AI:1, AI:3, AO:1.
    assert_eq!(plain, 1 + 1 + 3 + 3 + 3);
    let ai1 = point(&result, "analog-input:1");
    assert_eq!(ai1["units"]["name"], json!("degrees-fahrenheit"));
    assert_eq!(ok(&result)["problems"]["count"], json!(0));
    let args = json!({ "device": LAB, "types": types, "values": true });
    let (values, result) = sent(&mut lab, args).await;
    assert_eq!(values, 7 + 7 + 8);
    let ai1 = point(&result, "analog-input:1");
    assert_eq!(ai1["present_value"]["decoded"], json!(72.5));
    let ao1 = point(&result, "analog-output:1");
    assert_eq!(
        ao1["command_priority"]["value"],
        json!("relinquish default")
    );
    let args = json!({ "device": LAB, "types": types, "values": true, "settings": true,
                       "details": true });
    let (all, result) = sent(&mut lab, args).await;
    assert_eq!(all, 12 + 12 + 14);
    assert_eq!(ok(&result)["problems"]["count"], json!(0));
    assert_eq!(ok(&result)["ended"], json!("complete"));
    lab.stop().await;
}

/// At the default policy (5 requests a second for the principal after a
/// burst of 20, a 30 s budget) a device without ReadPropertyMultiple, read
/// with values and settings, takes 10 requests an analog input, 2 s: the
/// budget ends the page as `budget` with the records read so far, within
/// one object's time of it, and the cursor resumes at the first object not
/// read.
#[tokio::test(start_paused = true)]
async fn the_budget_ends_a_page_at_the_default_rates() {
    let mut lab = inventory_lab(Policy::default(), Duration::ZERO, 30, false).await;
    lab.refuse_rpm();
    lab.agent.initialize().await;
    let analog: Vec<String> = listed(&lab)
        .await
        .into_iter()
        .filter(|name| name.starts_with("analog-input:"))
        .collect();
    let args = json!({ "device": LAB, "types": ["ai"], "values": true, "settings": true });
    let before = lab.sends().len();
    let first = lab.agent.call(TOOL, args.clone()).await;
    let body = ok(&first);
    let read = points(&first).len();
    assert_eq!(body["ended"], json!("budget"), "{body}");
    assert_eq!(body["truncated"], json!(true));
    assert!((10..analog.len()).contains(&read), "{read}");
    // The list, the refused request, 10 an object.
    assert_eq!(lab.sends().len() - before, 2 + 10 * read);
    let elapsed = body["cost"]["elapsed_ms"].as_u64().unwrap();
    assert!((30_000..=32_000).contains(&elapsed), "{elapsed}");
    let seen: Vec<String> = points(&first).iter().map(object).collect();
    assert_eq!(seen, analog[..read]);
    assert_eq!(body["next_object"], json!(analog[read]));
    let mut again = args;
    again["cursor"] = body["next_cursor"].clone();
    let rest = lab.agent.call(TOOL, again).await;
    assert_eq!(ok(&rest)["start"], json!(read));
    assert_eq!(object(&points(&rest)[0]), analog[read]);
    lab.stop().await;
}

/// A later batch still being read 7 s past the budget (half the call's
/// grace) is let go, so the batches before it come back as `budget`
/// rather than the call timing out: here a token a second and analog
/// inputs of 12 ReadProperty requests each. The first batch, two inputs
/// (24 properties fit a request's 32 before the refusal is known), is
/// read whole; the device's refusal then makes a batch one object, and the
/// third input straddles the 28 s budget.
#[tokio::test(start_paused = true)]
async fn a_batch_well_past_the_budget_is_let_go() {
    let policy = Policy {
        rates: Rates {
            principal: 1,
            burst: 1,
            ..Rates::default()
        },
        caps: Caps {
            budget: Duration::from_secs(28),
            ..Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = inventory_lab(policy, Duration::ZERO, 2, false).await;
    lab.refuse_rpm();
    lab.agent.initialize().await;
    let args = json!({ "device": LAB, "types": ["ai"], "values": true, "settings": true,
                       "details": true });
    let result = lab.agent.call(TOOL, args.clone()).await;
    let body = ok(&result);
    // The list at 0 s; the refused RPM at 1 s, AI:1 from 2 s to 13 s and
    // AI:3 from 14 s to 25 s; AI:10 from 26 s, let go at 35 s.
    assert_eq!(body["read"], json!(2), "{body}");
    assert_eq!(body["ended"], json!("budget"));
    assert_eq!(body["next_object"], json!("analog-input:10"));
    assert_eq!(body["cost"]["timeouts"], json!(1), "the batch let go");
    let elapsed = body["cost"]["elapsed_ms"].as_u64().unwrap();
    assert!((35_000..36_000).contains(&elapsed), "{elapsed}");
    // The next page starts at the input let go.
    let mut again = args;
    again["cursor"] = body["next_cursor"].clone();
    let rest = lab.agent.call(TOOL, again).await;
    assert_eq!(object(&points(&rest)[0]), "analog-input:10");
    // A batch of one let go teaches nothing: the device's batches stay as
    // they were.
    assert!(lab.mcp.ctx().slow.lock().unwrap().is_empty());
    lab.stop().await;
}

/// A page's first batch still being read the call's grace less a second
/// past the budget fails the call as a timeout, before the call's own
/// timeout would, naming the batch's first object, how many the batch
/// held and the ways past it: here another caller takes the device's gate
/// between the ReadProperty requests (a token a second) of a batch of two
/// inputs and keeps it.
#[tokio::test(start_paused = true)]
async fn a_stuck_first_object_is_named() {
    let policy = Policy {
        rates: Rates {
            principal: 1,
            burst: 1,
            ..Rates::default()
        },
        ..Policy::default()
    };
    let mut lab = inventory_lab(policy, Duration::ZERO, 0, false).await;
    lab.refuse_rpm();
    lab.agent.initialize().await;
    // The Object_List, read and kept.
    lab.agent
        .call("bacnet_list_objects", json!({ "device": LAB }))
        .await;
    let limits = std::sync::Arc::clone(&lab.limits);
    let holder = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(2_500)).await;
        let at = bacnet_ops::DeviceAddress::local(&super::LAB_MAC).unwrap();
        let turn = limits.turn(&at).await;
        tokio::time::sleep(Duration::from_secs(300)).await;
        drop(turn);
    });
    let args = json!({ "device": LAB, "types": ["ai"], "values": true });
    let result = lab.agent.call(TOOL, args.clone()).await;
    let body = failed(&result);
    assert_eq!(body["error"], json!("timeout"), "{body}");
    assert_eq!(
        (&body["device"], &body["object"]),
        (&json!(LAB), &json!("analog-input:1"))
    );
    let hint = body["hint"].as_str().unwrap();
    assert!(
        hint.starts_with("the 2 objects from it to analog-input:3, read together, outlasted")
            && hint.contains("Call again the same way: the next call reads fewer")
            && hint.contains("analog-input, out"),
        "{hint}"
    );
    assert_eq!(body["cost"]["timeouts"], json!(1));
    // The 30 s budget, then 14 s: before the call's own timeout at 45 s.
    let elapsed = body["cost"]["elapsed_ms"].as_u64().unwrap();
    assert!((44_000..45_000).contains(&elapsed), "{elapsed}");
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("reading analog-input:1"), "{text}");
    // Called again, the batch is the one object, still stuck behind the
    // gate: #2786's error, naming it alone.
    let again = lab.agent.call(TOOL, args).await;
    let body = failed(&again);
    assert_eq!(body["object"], json!("analog-input:1"), "{body}");
    let hint = body["hint"].as_str().unwrap();
    assert!(
        hint.starts_with("it alone outlasted the call's budget. Read it with bacnet_read"),
        "{hint}"
    );
    // The two inputs' 14 properties halved; the one stuck alone after
    // them changes nothing.
    let at = bacnet_ops::DeviceAddress::local(&super::LAB_MAC).unwrap();
    let slow = lab
        .mcp
        .ctx()
        .slow
        .lock()
        .unwrap()
        .get(&at)
        .map(|limit| limit.properties);
    assert_eq!(slow, Some(7));
    holder.abort();
    lab.stop().await;
}

/// The lab with 20 trend logs besides its other objects, each reading two
/// properties, so 16 go in a batch.
async fn logs_lab(policy: Policy) -> Lab {
    let lab = inventory_lab(policy, Duration::ZERO, 0, false).await;
    for n in 1..=20 {
        let log = bacnet_objects::trend::TrendLogObject::new(n, format!("Log {n}"), 10).unwrap();
        lab.add_object(Box::new(log)).await;
    }
    lab
}

/// A device slower than a page's first-batch deadline at a full batch
/// makes progress all the same: the first call's batch of 16 trend logs
/// (a refused ReadPropertyMultiple, then 32 ReadProperty requests at a
/// token a second) outlasts the 5 s budget and 14 s past it and fails,
/// naming its range; the next call asks for half, 8, which fits, and the
/// pages after it go on one log at a time (the refusal is known by then).
/// Every log comes back exactly once, in order.
#[tokio::test(start_paused = true)]
async fn a_batch_too_slow_for_any_page_shrinks_until_it_fits() {
    let policy = Policy {
        rates: Rates {
            principal: 1,
            burst: 1,
            ..Rates::default()
        },
        caps: Caps {
            budget: Duration::from_secs(5),
            ..Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = logs_lab(policy).await;
    lab.refuse_rpm();
    lab.agent.initialize().await;
    lab.agent
        .call("bacnet_list_objects", json!({ "device": LAB }))
        .await;
    let args = json!({ "device": LAB, "types": ["trend-log"] });
    let (mut cursor, mut seen, mut stuck) = (None::<String>, Vec::new(), 0);
    for _ in 0..30 {
        let mut call = args.clone();
        if let Some(cursor) = &cursor {
            call["cursor"] = json!(cursor);
        }
        let result = lab.agent.call(TOOL, call).await;
        if result["isError"] == json!(true) {
            let body = failed(&result);
            assert_eq!(body["object"], json!("trend-log:1"), "{body}");
            let hint = body["hint"].as_str().unwrap();
            assert!(
                hint.starts_with("the 16 objects from it to trend-log:16"),
                "{hint}"
            );
            stuck += 1;
            continue;
        }
        seen.extend(points(&result).iter().map(object));
        cursor = ok(&result)["next_cursor"].as_str().map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(stuck, 1, "one failed call, then progress");
    let logs: Vec<String> = (1..=20).map(|n| format!("trend-log:{n}")).collect();
    assert_eq!(seen, logs, "every log once, in order");
    lab.stop().await;
}

/// A later batch of several objects still being read 7 s past the budget
/// is let go, the batch before it kept, and the next call asks for half as
/// many there: another caller takes the device's gate while the first
/// batch is answered and keeps it.
#[tokio::test(start_paused = true)]
async fn a_later_batch_of_several_is_let_go_and_the_next_asks_fewer() {
    let policy = Policy {
        rates: Rates {
            global: 1000,
            network: 1000,
            principal: 1000,
            burst: 1000,
            ..Rates::default()
        },
        caps: Caps {
            budget: Duration::from_secs(2),
            ..Caps::default()
        },
        ..Policy::default()
    };
    let delay = Duration::from_millis(300);
    let mut lab = inventory_lab(policy, delay, 30, false).await;
    lab.agent.initialize().await;
    lab.agent
        .call("bacnet_list_objects", json!({ "device": LAB }))
        .await;
    let limits = std::sync::Arc::clone(&lab.limits);
    // The first batch goes at 0 s and is answered at 0.3 s; the holder
    // queues for the gate at 0.15 s, takes it then, and keeps it.
    let holder = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let at = bacnet_ops::DeviceAddress::local(&super::LAB_MAC).unwrap();
        let turn = limits.turn(&at).await;
        tokio::time::sleep(Duration::from_secs(300)).await;
        drop(turn);
    });
    let from = super::inventory::sent_frames(&lab);
    let first = lab.agent.call(TOOL, json!({ "device": LAB })).await;
    let body = ok(&first);
    // Eleven objects (32 properties) read; the next ten let go at 9 s.
    assert_eq!(body["read"], json!(11), "{body}");
    assert_eq!(body["ended"], json!("budget"));
    assert_eq!(body["cost"]["timeouts"], json!(1));
    let elapsed = body["cost"]["elapsed_ms"].as_u64().unwrap();
    assert!((9_000..9_500).contains(&elapsed), "{elapsed}");
    holder.abort();
    let cursor = body["next_cursor"].clone();
    let rest = lab
        .agent
        .call(TOOL, json!({ "device": LAB, "cursor": cursor }))
        .await;
    assert_eq!(ok(&rest)["start"], json!(11));
    // The batch let go, ten analog inputs (30 properties), never went: it
    // waited for the gate. The next call asks for 15 properties, five.
    let batches = super::inventory::asked(&lab, from);
    assert_eq!(batches[0].len(), 11);
    assert_eq!(batches[1].len(), 5, "half the batch let go");
    lab.stop().await;
}
