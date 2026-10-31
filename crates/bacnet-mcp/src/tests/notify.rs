//! `bacnet://session` and `bacnet://ledger` updates (#2160): a stream on
//! both hears one update per arm, settle and revert; an armed session's
//! end is announced with no request arriving; a plan grant's spend and a
//! legacy session's subscriptions are covered too. All on paused time.

use std::time::Duration;

use serde_json::{Value, json};

use super::Lab;
use super::plan::plan;
use super::writes::{answered, call, one, writes};
use crate::policy::ArmMode;

const SESSION: &str = "bacnet://session";
const LEDGER: &str = "bacnet://ledger";
const UPDATED: &str = "notifications/resources/updated";

/// The URIs of the updates that arrived or arrive within a second.
async fn updates(lab: &mut Lab) -> Vec<String> {
    lab.agent
        .drain(UPDATED, Duration::from_secs(1))
        .await
        .iter()
        .map(|n| n["params"]["uri"].as_str().unwrap().to_string())
        .collect()
}

/// Acceptance: a confirmed write that arms a session is one session
/// update (the arm) and one ledger update (the settle); its revert is one
/// ledger update; the preview and a refused write send none.
#[tokio::test(start_paused = true)]
async fn a_stream_on_both_hears_each_arm_settle_and_revert_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut lab = Lab::start(writes(dir.path(), ArmMode::Elicit)).await;
    let (_stream, accepted) = lab.agent.listen(&[SESSION, LEDGER]).await;
    assert_eq!(accepted, [SESSION, LEDGER]);

    let first = call(&mut lab, "bacnet_write", one(json!(21), 12), json!({})).await;
    assert_eq!(updates(&mut lab).await, Vec::<String>::new(), "a preview");
    let done = call(
        &mut lab,
        "bacnet_write",
        one(json!(21), 12),
        answered(&first["requestState"], "accept", true),
    )
    .await;
    assert_eq!(done["isError"], json!(false), "{done}");
    assert_eq!(updates(&mut lab).await, [SESSION, LEDGER]);

    let id = done["structuredContent"]["results"][0]["ledger_id"].clone();
    let reverted = call(
        &mut lab,
        "bacnet_revert",
        json!({ "entries": [id] }),
        json!({}),
    )
    .await;
    assert_eq!(reverted["isError"], json!(false), "{reverted}");
    assert_eq!(updates(&mut lab).await, [LEDGER]);

    // T2 is refused before anything is recorded: nothing changed.
    let refused = call(&mut lab, "bacnet_write", one(json!(1), 8), json!({})).await;
    assert_eq!(refused["isError"], json!(true), "{refused}");
    assert_eq!(updates(&mut lab).await, Vec::<String>::new());
    lab.stop().await;
}

/// The armed session's end is announced when it comes, with no request
/// to mark it, and the session then reads as expired.
#[tokio::test(start_paused = true)]
async fn a_sessions_end_is_announced_without_a_request() {
    let dir = tempfile::tempdir().unwrap();
    let mut lab = Lab::start(writes(dir.path(), ArmMode::Elicit)).await;
    let (_stream, _) = lab.agent.listen(&[SESSION]).await;
    let first = call(&mut lab, "bacnet_write", one(json!(5), 16), json!({})).await;
    call(
        &mut lab,
        "bacnet_write",
        one(json!(5), 16),
        answered(&first["requestState"], "accept", true),
    )
    .await;
    assert_eq!(updates(&mut lab).await, [SESSION], "the arm");
    // 15 minutes from the arm, a second of which `updates` took.
    tokio::time::sleep(Duration::from_secs(15 * 60 - 3)).await;
    let early: Vec<Value> = lab.agent.drain(UPDATED, Duration::ZERO).await;
    assert!(early.is_empty(), "not yet: {early:?}");
    assert_eq!(updates(&mut lab).await, Vec::<String>::new());
    assert_eq!(updates(&mut lab).await, [SESSION], "the end");
    let session = lab.agent.read(SESSION).await.unwrap();
    assert_eq!(
        session["level"],
        json!("write-session (expired)"),
        "{session}"
    );
    lab.stop().await;
}

/// A legacy session subscribed to both hears a planned write's spend
/// (the session shows the budget) and its settle; once the plan is spent,
/// a refused write announces nothing.
#[tokio::test(start_paused = true)]
async fn a_legacy_session_hears_a_planned_write_and_the_plans_end() {
    let dir = tempfile::tempdir().unwrap();
    let mut policy = writes(dir.path(), ArmMode::Never);
    policy.writes.plan = Some(plan(1));
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize_legacy().await;
    for uri in [SESSION, LEDGER] {
        lab.agent.subscribe(uri).await.unwrap();
    }
    let reply = lab.agent.call("bacnet_write", one(json!(3), 12)).await;
    assert_eq!(reply["isError"], json!(false), "{reply}");
    let mut heard = updates(&mut lab).await;
    heard.sort();
    assert_eq!(heard, [LEDGER, SESSION]);
    let session = lab.agent.read(SESSION).await.unwrap();
    assert_eq!(session["grants"][0]["in_force"], json!(false), "{session}");
    let reply = lab.agent.call("bacnet_write", one(json!(4), 12)).await;
    assert_eq!(reply["structuredContent"]["reason"], json!("not-armed"));
    assert_eq!(updates(&mut lab).await, Vec::<String>::new());
    lab.stop().await;
}
