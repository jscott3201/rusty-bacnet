//! Plan grants (#2068, acceptance 8): writes a person approved in advance
//! run without a prompt, each audited as approved by the plan, up to the
//! plan's budget; a write outside the plan is refused.

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::PropertyValue;
use serde_json::{Value, json};

use super::writes::{audit, call, one, slot, writes};
use super::{LAB, Lab};
use crate::policy::{ArmMode, PlanGrant};

fn plan(max_writes: u32) -> PlanGrant {
    PlanGrant {
        devices: vec![(LAB, LAB)],
        properties: vec![PropertyIdentifier::PRESENT_VALUE],
        priorities: (8, 16),
        max_writes,
        expires_unix: crate::writes::unix_now() + 3_600,
        reason: Some("commissioning".into()),
    }
}

#[tokio::test(start_paused = true)]
async fn fifty_planned_writes_run_without_prompts_and_others_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut policy = writes(dir.path(), ArmMode::Never);
    policy.writes.plan = Some(plan(51));
    let mut lab = Lab::start(policy).await;
    for n in 0..50u8 {
        // Priority 8 is T2: the plan covers it, where a client can't.
        let priority = if n % 2 == 0 { 8 } else { 12 };
        let reply = call(&mut lab, "bacnet_write", one(json!(n), priority), json!({})).await;
        assert_eq!(reply["isError"], json!(false), "write {n}: {reply}");
        assert!(reply.get("inputRequests").is_none(), "no prompt");
        let body = &reply["structuredContent"];
        assert_eq!(body["approval"]["confirmed_via"], json!("plan"), "{body}");
    }
    assert_eq!(slot(&lab, 8).await, PropertyValue::Real(48.0));
    assert_eq!(slot(&lab, 12).await, PropertyValue::Real(49.0));
    let records = audit(dir.path());
    let planned: Vec<&Value> = records
        .iter()
        .filter(|r| r["event"] == "write" && r["outcome"]["result"] == "ack")
        .collect();
    assert_eq!(planned.len(), 50);
    for record in planned {
        assert_eq!(record["approved_by"], json!("plan"), "{record}");
        assert_eq!(record["plan"]["reason"], json!("commissioning"), "{record}");
    }
    // Priority 3 and Out_Of_Service are outside it; nothing is sent.
    let outside = [
        one(json!(1), 3),
        json!({ "device": LAB, "writes": [{ "object": "ao:1", "property": "out-of-service",
                                            "value": true }] }),
    ];
    for args in outside {
        let reply = call(&mut lab, "bacnet_write", args, json!({})).await;
        assert_eq!(
            reply["structuredContent"]["reason"],
            json!("outside-plan"),
            "{reply}"
        );
    }
    assert_eq!(slot(&lab, 3).await, PropertyValue::Null);
    // The last write of the budget, then the plan is spent: with mcp_arm
    // "never" nothing else can approve.
    let reply = call(&mut lab, "bacnet_write", one(json!(7), 16), json!({})).await;
    assert_eq!(reply["isError"], json!(false), "{reply}");
    let reply = call(&mut lab, "bacnet_write", one(json!(8), 16), json!({})).await;
    assert_eq!(
        reply["structuredContent"]["reason"],
        json!("not-armed"),
        "{reply}"
    );
    lab.stop().await;
}
