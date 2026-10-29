//! Writes handed to tasks of their own (#2068): past the call's budget
//! the reply is pending with the entry ids, never "nothing was sent"; a
//! second write meanwhile finds the workspace locked; a task that panics
//! leaves no `sending` entry behind; and a shutdown waits for a write
//! still going. All on paused time.

use std::sync::Arc;
use std::time::Duration;

use bacnet_ops::limits::Account;
use bacnet_ops::safety::ledger::{Ledger, Origin, State};
use bacnet_ops::safety::{Effect, Gate, Occupancy, Planned, Stored, Tier, WriteSpec, run_id};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde_json::{Map, json};

use super::schema::check_output;
use super::writes::{audit, call, meta, one, slot, writes};
use super::{LAB, Lab};
use crate::policy::{ArmMode, PlanGrant};
use crate::tools::write_task::{hand_off, sending};

fn plan() -> PlanGrant {
    PlanGrant {
        devices: vec![(LAB, LAB)],
        properties: vec![PropertyIdentifier::PRESENT_VALUE],
        priorities: (8, 16),
        max_writes: 10,
        expires_unix: crate::writes::unix_now() + 3_600,
        reason: None,
    }
}

/// A lab whose device answers each request after 500 ms, with a plan.
async fn planned_lab(dir: &std::path::Path, budget: Duration) -> Lab {
    let mut policy = writes(dir, ArmMode::Never);
    policy.writes.plan = Some(plan());
    policy.caps.budget = budget;
    let mut lab = Lab::start_with(policy, Duration::from_millis(500)).await;
    lab.agent.initialize().await;
    lab
}

/// Hold the device's gate from just after the write's pre-read, which
/// holds it first, for `hold`: the write is recorded in the ledger, then
/// waits for its turn.
fn hold_the_gate(lab: &Lab, hold: Duration) -> tokio::task::JoinHandle<()> {
    let limits = Arc::clone(&lab.limits);
    tokio::spawn(async move {
        // The pre-read went at once and holds the gate for 500 ms; this
        // queues behind it, and the write's own turn behind this.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let turn = limits.turn(LAB).await;
        tokio::time::sleep(hold).await;
        drop(turn);
    })
}

/// Past the call's budget the write is pending, with its ledger entry; a
/// second write meanwhile is refused, the workspace being held; once the
/// gate frees the first is sent and settles.
#[tokio::test(start_paused = true)]
async fn a_write_past_its_budget_is_pending_not_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut lab = planned_lab(dir.path(), Duration::from_secs(1)).await;
    let params = json!({ "name": "bacnet_write", "arguments": one(json!(4), 13),
                         "_meta": meta() });
    let id = lab.agent.post("tools/call", params).await;
    let holder = hold_the_gate(&lab, Duration::from_secs(25));
    let reply = lab.agent.answer(id).await;
    let reply = &reply["result"];
    let body = &reply["structuredContent"];
    assert_eq!(reply["isError"], json!(true), "{reply}");
    assert_eq!(body["error"], json!("pending"), "{body}");
    assert_eq!(body["applied"], json!("unknown"), "{body}");
    assert_eq!(body["ledger_ids"], json!([1]), "{body}");
    assert!(body.get("retry_after_s").is_none(), "{body}");
    assert_eq!(body["ledger"], json!("bacnet://ledger"), "{body}");
    check_output(&mut lab.agent, "bacnet_write", body).await;

    let second = call(&mut lab, "bacnet_write", one(json!(5), 14), json!({})).await;
    assert_eq!(
        second["structuredContent"]["reason"],
        json!("workspace-locked"),
        "{second}"
    );
    holder.await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(slot(&lab, 13).await, PropertyValue::Real(4.0));
    assert_eq!(slot(&lab, 14).await, PropertyValue::Null);
    let ledger = Ledger::open(dir.path()).unwrap();
    assert_eq!(ledger.entries()[0].state, State::Applied);
    lab.stop().await;
}

/// Closing the server's input while a write waits at the device's gate:
/// the shutdown waits for it, so its outcome is in the ledger and the
/// audit log by the time the server has stopped.
#[tokio::test(start_paused = true)]
async fn a_shutdown_waits_for_a_write_still_going() {
    let dir = tempfile::tempdir().unwrap();
    let mut lab = planned_lab(dir.path(), Duration::from_secs(30)).await;
    let params = json!({ "name": "bacnet_write", "arguments": one(json!(6), 15),
                         "_meta": meta() });
    lab.agent.post("tools/call", params).await;
    let holder = hold_the_gate(&lab, Duration::from_secs(5));
    tokio::time::sleep(Duration::from_secs(1)).await;
    lab.agent.close().await;
    (&mut lab.serving).await.unwrap();
    let records = audit(dir.path());
    assert!(
        records
            .iter()
            .any(|r| r["event"] == "write" && r["outcome"]["result"] == "ack"),
        "{records:?}"
    );
    assert_eq!(
        Ledger::open(dir.path()).unwrap().entries()[0].state,
        State::Applied
    );
    holder.await.unwrap();
    lab.shutdown = None;
    lab.stop().await;
}

/// A write task that panics after recording its entry as `sending` leaves
/// it settled as unknown, audited with the call's fields.
#[tokio::test(start_paused = true)]
async fn a_panicking_write_task_settles_its_entry() {
    let dir = tempfile::tempdir().unwrap();
    let mut lab = planned_lab(dir.path(), Duration::from_secs(30)).await;
    let ctx = Arc::clone(lab.mcp.shared_ctx());
    let workspace = ctx.policy.writes.workspace.clone().unwrap();
    let path = workspace.dir.clone();
    let item = Planned {
        spec: WriteSpec {
            device: LAB,
            object: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property: PropertyIdentifier::PRESENT_VALUE,
            index: None,
            value: Some("1".into()),
            priority: Some(12),
        },
        priority: Some(12),
        value: Stored {
            bytes: vec![0x44, 0x3f, 0x80, 0, 0],
            text: "1".into(),
        },
        commandable: true,
        before: None,
        occupancy: Occupancy::Empty,
        effect: Effect::NotPresentValue,
        out_of_service: None,
        notes: Vec::new(),
        gate: Gate::Allowed(Tier::T1),
    };
    let work = async move {
        let mut ledger = Ledger::open(&path)
            .unwrap()
            .written_by(Origin::mcp("tester"));
        ledger.begin(&item, run_id(), "t");
        ledger.save().unwrap();
        panic!("the write task panics after recording its entry");
    };
    let mut fields = Map::new();
    fields.insert("tool".into(), json!("bacnet_write"));
    let account = Arc::new(Account::new(None));
    let handoff = hand_off(&ctx, workspace, account, sending(), fields, work);
    assert!(handoff.task.await.unwrap().is_err());
    let ledger = Ledger::open(dir.path()).unwrap();
    assert_eq!(ledger.entries()[0].state, State::Unknown);
    let records = audit(dir.path());
    let settled = records.iter().find(|r| r["event"] == "unknown").unwrap();
    assert_eq!(settled["tool"], json!("bacnet_write"), "{settled}");
    assert_eq!(settled["front_end"], json!("mcp"), "{settled}");
    drop(ctx);
    lab.agent.close().await;
    lab.stop().await;
}
