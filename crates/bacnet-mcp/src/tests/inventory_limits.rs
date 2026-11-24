//! `bacnet_inventory` (#2779) at its limits: the policy's object page, a
//! device without ReadPropertyMultiple (one ReadProperty a property), a
//! page the call's budget ends at the default rates, and an object still
//! being read well past the budget.

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

/// An object still being read 7 s past the budget (half the call's
/// grace) is let go, so the records before it come back as `budget`
/// rather than the call timing out: here a token a second, an analog
/// input of 12 ReadProperty requests, and a 14 s budget that the second
/// object straddles.
#[tokio::test(start_paused = true)]
async fn an_object_well_past_the_budget_is_let_go() {
    let policy = Policy {
        rates: Rates {
            principal: 1,
            burst: 1,
            ..Rates::default()
        },
        caps: Caps {
            budget: Duration::from_secs(14),
            ..Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = inventory_lab(policy, Duration::ZERO, 0, false).await;
    lab.refuse_rpm();
    lab.agent.initialize().await;
    let args = json!({ "device": LAB, "types": ["ai"], "values": true, "settings": true,
                       "details": true });
    let result = lab.agent.call(TOOL, args).await;
    let body = ok(&result);
    // The list at 0 s, AI:1 from 1 s to 13 s, AI:3 from 14 s, let go at 21 s.
    assert_eq!(body["read"], json!(1), "{body}");
    assert_eq!(body["ended"], json!("budget"));
    assert_eq!(body["next_object"], json!("analog-input:3"));
    assert_eq!(body["cost"]["timeouts"], json!(1), "the object let go");
    let elapsed = body["cost"]["elapsed_ms"].as_u64().unwrap();
    assert!((21_000..22_000).contains(&elapsed), "{elapsed}");
    lab.stop().await;
}

/// A page's first object still being read the call's grace less a second
/// past the budget fails the call as a timeout, before the call's own
/// timeout would, naming the object and the ways past it: here another
/// caller takes the device's gate between the object's ReadProperty
/// requests (a token a second) and keeps it.
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
    let result = lab.agent.call(TOOL, args).await;
    let body = failed(&result);
    assert_eq!(body["error"], json!("timeout"), "{body}");
    assert_eq!(
        (&body["device"], &body["object"]),
        (&json!(LAB), &json!("analog-input:1"))
    );
    let hint = body["hint"].as_str().unwrap();
    assert!(
        hint.contains("bacnet_read") && hint.contains("analog-input, out"),
        "{hint}"
    );
    assert_eq!(body["cost"]["timeouts"], json!(1));
    // The 30 s budget, then 14 s: before the call's own timeout at 45 s.
    let elapsed = body["cost"]["elapsed_ms"].as_u64().unwrap();
    assert!((44_000..45_000).contains(&elapsed), "{elapsed}");
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("reading analog-input:1"), "{text}");
    holder.abort();
    lab.stop().await;
}
