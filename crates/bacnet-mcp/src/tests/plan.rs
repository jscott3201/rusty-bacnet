//! Plan grants (#2068, acceptance 8): writes a person approved in advance
//! run without a prompt, each audited as approved by the plan, up to the
//! plan's budget; a write outside the plan is refused.

use std::time::Duration;

use bacnet_ops::safety::ledger::{Ledger, State};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::PropertyValue;
use serde_json::{Value, json};

use super::schema::check_output;
use super::writes::{audit, call, meta, one, slot, writes};
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

/// A plan reaching priority 1 lets a T3 write through; priority 6 stays
/// blocked whatever the plan; a device outside the grant is refused; a
/// plan past its expiry approves nothing.
#[tokio::test(start_paused = true)]
async fn a_plans_reach_holds_on_every_side() {
    let dir = tempfile::tempdir().unwrap();
    let mut policy = writes(dir.path(), ArmMode::Never);
    policy.writes.plan = Some(PlanGrant {
        priorities: (1, 16),
        ..plan(10)
    });
    let mut lab = Lab::start(policy).await;
    let reply = call(&mut lab, "bacnet_write", one(json!(3), 1), json!({})).await;
    assert_eq!(reply["isError"], json!(false), "{reply}");
    assert_eq!(slot(&lab, 1).await, PropertyValue::Real(3.0));
    let reply = call(&mut lab, "bacnet_write", one(json!(3), 6), json!({})).await;
    assert_eq!(reply["structuredContent"]["reason"], json!("priority-6"));
    assert_eq!(slot(&lab, 6).await, PropertyValue::Null);
    // Another device the lab answers for, outside the grant.
    let other = json!({ "device": super::ALIASES.start, "writes": [
        { "object": "ao:1", "value": 1, "priority": 12 }] });
    let reply = call(&mut lab, "bacnet_write", other, json!({})).await;
    assert_eq!(reply["structuredContent"]["reason"], json!("outside-plan"));
    lab.stop().await;

    let dir = tempfile::tempdir().unwrap();
    let mut policy = writes(dir.path(), ArmMode::Never);
    policy.writes.plan = Some(PlanGrant {
        expires_unix: crate::writes::unix_now() - 1,
        ..plan(10)
    });
    let mut lab = Lab::start(policy).await;
    let reply = call(&mut lab, "bacnet_write", one(json!(3), 12), json!({})).await;
    assert_eq!(
        reply["structuredContent"]["reason"],
        json!("not-armed"),
        "{reply}"
    );
    assert_eq!(slot(&lab, 12).await, PropertyValue::Null);
    lab.stop().await;
}

/// A planned write cancelled while its request is out still settles: the
/// ledger keeps it as applied, and the audit log has its outcome.
#[tokio::test(start_paused = true)]
async fn a_cancelled_planned_write_still_settles() {
    let dir = tempfile::tempdir().unwrap();
    let mut policy = writes(dir.path(), ArmMode::Never);
    policy.writes.plan = Some(plan(10));
    // The device answers each request after 400 ms: the pre-read by 400,
    // the write by about 800.
    let mut lab = Lab::start_with(policy, Duration::from_millis(400)).await;
    let params = json!({ "name": "bacnet_write", "arguments": one(json!(9), 12),
                         "_meta": meta() });
    let id = lab.agent.post("tools/call", params).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    lab.agent.cancel(id).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(slot(&lab, 12).await, PropertyValue::Real(9.0));
    let ledger = Ledger::open(dir.path()).unwrap();
    let [entry] = ledger.entries() else {
        panic!("{:?}", ledger.entries());
    };
    assert_eq!(entry.state, State::Applied, "settled, not left sending");
    let records = audit(dir.path());
    assert!(
        records
            .iter()
            .any(|r| r["event"] == "write" && r["outcome"]["result"] == "ack"),
        "{records:?}"
    );
    lab.stop().await;
}

/// A planned write that waits past the call's budget (here at the
/// device's gate, which the test holds) is answered as pending, never as
/// refused with nothing sent; once the gate frees it is sent and settles.
#[tokio::test(start_paused = true)]
async fn a_write_past_its_budget_is_pending_not_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut policy = writes(dir.path(), ArmMode::Never);
    policy.writes.plan = Some(plan(10));
    policy.caps.budget = Duration::from_secs(1);
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize().await;
    let gate = lab.limits.turn(LAB).await;
    let reply = call(&mut lab, "bacnet_write", one(json!(4), 13), json!({})).await;
    let body = &reply["structuredContent"];
    assert_eq!(reply["isError"], json!(true), "{reply}");
    assert_eq!(body["error"], json!("pending"), "{body}");
    assert_eq!(body["applied"], json!("unknown"), "{body}");
    assert!(body.get("retry_after_s").is_none(), "{body}");
    assert_eq!(body["ledger"], json!("bacnet://ledger"), "{body}");
    check_output(&mut lab.agent, "bacnet_write", body).await;
    drop(gate);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(slot(&lab, 13).await, PropertyValue::Real(4.0));
    let ledger = Ledger::open(dir.path()).unwrap();
    assert_eq!(ledger.entries()[0].state, State::Applied);
    lab.stop().await;
}
