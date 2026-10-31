//! A plan grant's write budget kept in the workspace (#2159): it holds
//! across a restart, authorizing again starts it fresh, and a count the
//! server can't read for this authorization, or can't save, refuses the
//! plan's writes with nothing sent. All on paused time.

use std::path::Path;
use std::sync::atomic::Ordering;

use bacnet_ops::safety::ledger::Ledger;
use bacnet_types::primitives::PropertyValue;
use serde_json::{Value, json};

use super::Lab;
use super::plan::{authorized, plan};
use super::writes::{call, one, slot, writes};
use crate::Policy;
use crate::policy::{ArmMode, PlanGrant};
use crate::writes::budget;

/// Requests a planned write of AO:1's Present_Value reads first.
const PRE_READS: usize = 1;

/// A lab whose policy has `grant`, recording to `dir`.
async fn lab(dir: &Path, grant: PlanGrant) -> Lab {
    let mut policy: Policy = writes(dir, ArmMode::Never);
    policy.writes.plan = Some(grant);
    Lab::start(policy).await
}

/// `n` planned writes, each approved by the plan.
async fn spend(lab: &mut Lab, n: u8) {
    for k in 0..n {
        let reply = call(lab, "bacnet_write", one(json!(k), 12), json!({})).await;
        assert_eq!(
            reply["structuredContent"]["approval"]["confirmed_via"],
            json!("plan"),
            "write {k}: {reply}"
        );
    }
}

/// A write's refusal reason.
async fn refusal(lab: &mut Lab, value: u8) -> Value {
    let reply = call(lab, "bacnet_write", one(json!(value), 16), json!({})).await;
    reply["structuredContent"]["reason"].clone()
}

/// Acceptance: a 50-write plan, 30 writes, a restart (a new server on the
/// same workspace), and only 20 more are approved; authorizing again
/// starts a fresh count.
#[tokio::test(start_paused = true)]
async fn a_plans_budget_holds_across_a_restart_until_authorized_again() {
    let dir = tempfile::tempdir().unwrap();
    let grant = authorized(dir.path(), plan(50)).unwrap();
    let mut first = lab(dir.path(), grant.clone()).await;
    spend(&mut first, 30).await;
    first.stop().await;
    assert_eq!(budget::read(dir.path(), &grant), Ok(30));

    let mut second = lab(dir.path(), grant.clone()).await;
    let session = second.agent.read("bacnet://session").await.unwrap();
    assert_eq!(session["grants"][0]["used"], json!(30), "{session}");
    spend(&mut second, 20).await;
    // Spent: with mcp_arm "never" nothing else approves.
    assert_eq!(refusal(&mut second, 1).await, json!("not-armed"));
    assert_eq!(slot(&second, 16).await, PropertyValue::Null);
    second.stop().await;
    assert_eq!(budget::read(dir.path(), &grant), Ok(50));

    let again = authorized(
        dir.path(),
        PlanGrant {
            authorized_unix: grant.authorized_unix + 60,
            ..grant
        },
    )
    .unwrap();
    let mut third = lab(dir.path(), again.clone()).await;
    spend(&mut third, 1).await;
    third.stop().await;
    assert_eq!(budget::read(dir.path(), &again), Ok(1));
}

/// No count for this authorization (missing, corrupt, or another
/// authorization's) refuses the plan's writes, with nothing sent; a count
/// past the budget, as a corrupt edit might leave, approves nothing.
#[tokio::test(start_paused = true)]
async fn a_count_that_cant_be_trusted_refuses_the_plans_writes() {
    let grant = plan(10);
    let bad: [(&str, Option<String>); 4] = [
        ("missing", None),
        ("corrupt", Some("{\"version\":1,".into())),
        (
            "another authorization",
            Some(format!(
                r#"{{"version":1,"plan":"{}","used":0}}"#,
                budget::key(&PlanGrant {
                    authorized_unix: grant.authorized_unix - 1,
                    ..grant.clone()
                })
            )),
        ),
        (
            "past the budget",
            Some(format!(
                r#"{{"version":1,"plan":"{}","used":4000000000}}"#,
                budget::key(&grant)
            )),
        ),
    ];
    for (case, text) in bad {
        let dir = tempfile::tempdir().unwrap();
        if let Some(text) = text {
            std::fs::write(dir.path().join(budget::FILE), text).unwrap();
        }
        let mut lab = lab(dir.path(), grant.clone()).await;
        let reason = refusal(&mut lab, 2).await;
        let expected = if case == "past the budget" {
            "not-armed"
        } else {
            "plan-budget-unavailable"
        };
        assert_eq!(reason, json!(expected), "{case}");
        assert_eq!(slot(&lab, 16).await, PropertyValue::Null, "{case}");
        assert!(Ledger::open(dir.path()).unwrap().entries().is_empty());
        lab.stop().await;
    }
}

/// The count is saved before the write goes: when it can't be saved, the
/// write is refused and nothing is sent or ledgered; once it can, writes
/// go again and each is counted.
#[tokio::test(start_paused = true)]
async fn a_count_that_cant_be_saved_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let grant = authorized(dir.path(), plan(10)).unwrap();
    let mut lab = lab(dir.path(), grant.clone()).await;
    lab.mcp
        .ctx()
        .writes
        .budget_fault
        .store(true, Ordering::Relaxed);
    let before = lab.sends().len();
    assert_eq!(refusal(&mut lab, 3).await, json!("plan-budget-unavailable"));
    assert_eq!(slot(&lab, 16).await, PropertyValue::Null);
    // The pre-read went (the preview is read under the lock); no write.
    let sent = lab.sends().len() - before;
    assert_eq!(sent, PRE_READS, "only the pre-read");
    assert!(Ledger::open(dir.path()).unwrap().entries().is_empty());
    assert_eq!(budget::read(dir.path(), &grant), Ok(0));
    lab.mcp
        .ctx()
        .writes
        .budget_fault
        .store(false, Ordering::Relaxed);
    let reply = call(&mut lab, "bacnet_write", one(json!(3), 16), json!({})).await;
    assert_eq!(reply["isError"], json!(false), "{reply}");
    assert_eq!(slot(&lab, 16).await, PropertyValue::Real(3.0));
    assert_eq!(budget::read(dir.path(), &grant), Ok(1));
    lab.stop().await;
}

/// While another holder has the workspace's lock, a planned write is
/// refused (`workspace-locked`) and the count stays as it was.
#[tokio::test(start_paused = true)]
async fn a_held_workspace_leaves_the_count_alone() {
    let dir = tempfile::tempdir().unwrap();
    let grant = authorized(dir.path(), plan(10)).unwrap();
    let mut lab = lab(dir.path(), grant.clone()).await;
    let held = bacnet_ops::safety::lock::lock(dir.path(), "tui").unwrap();
    let before = lab.sends().len();
    assert_eq!(refusal(&mut lab, 1).await, json!("workspace-locked"));
    assert_eq!(lab.sends().len(), before, "nothing sent, not even a read");
    assert_eq!(budget::read(dir.path(), &grant), Ok(0));
    drop(held);
    lab.stop().await;
}

/// The count another server saved is what a write is checked against:
/// past the budget there, `spend` refuses (`plan-spent`) though this
/// server's own count had room, and nothing is sent.
#[tokio::test(start_paused = true)]
async fn the_saved_count_refuses_a_write_past_the_budget() {
    let dir = tempfile::tempdir().unwrap();
    let grant = authorized(dir.path(), plan(2)).unwrap();
    let mut lab = lab(dir.path(), grant.clone()).await;
    budget::write(dir.path(), &grant, 2).unwrap();
    assert_eq!(refusal(&mut lab, 1).await, json!("plan-spent"));
    assert_eq!(slot(&lab, 16).await, PropertyValue::Null);
    assert_eq!(budget::read(dir.path(), &grant), Ok(2));
    lab.stop().await;
}

/// A count put back to an older value (a backup restored) can't lower
/// what the running server has counted.
#[tokio::test(start_paused = true)]
async fn a_count_put_back_does_not_give_writes_back() {
    let dir = tempfile::tempdir().unwrap();
    let grant = authorized(dir.path(), plan(5)).unwrap();
    let mut lab = lab(dir.path(), grant.clone()).await;
    spend(&mut lab, 3).await;
    budget::write(dir.path(), &grant, 0).unwrap();
    spend(&mut lab, 1).await;
    assert_eq!(budget::read(dir.path(), &grant), Ok(4));
    spend(&mut lab, 1).await;
    assert_eq!(refusal(&mut lab, 9).await, json!("not-armed"), "spent");
    lab.stop().await;
}

/// A count the server can't trust at its start shows in the session as
/// unavailable, with why, not as a count of zero.
#[tokio::test(start_paused = true)]
async fn the_session_says_when_the_count_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = lab(dir.path(), plan(10)).await;
    let session = first.agent.read("bacnet://session").await.unwrap();
    let grant = &session["grants"][0];
    assert_eq!(grant["used"], Value::Null, "{session}");
    let why = grant["count_unavailable"].as_str().unwrap();
    assert!(why.contains("missing"), "{why}");
    assert_eq!(
        refusal(&mut first, 1).await,
        json!("plan-budget-unavailable")
    );
    first.stop().await;

    let grant = authorized(dir.path(), plan(10)).unwrap();
    let mut second = lab(dir.path(), grant).await;
    let session = second.agent.read("bacnet://session").await.unwrap();
    assert_eq!(session["grants"][0]["used"], json!(0), "{session}");
    assert_eq!(session["grants"][0]["count_unavailable"], Value::Null);
    second.stop().await;
}
