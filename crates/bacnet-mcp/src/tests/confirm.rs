//! Writes confirmed by a client older than 2026-07-28 (#2158): the server
//! sends it an `elicitation/create` request over the session and waits.
//! A 2025-11-25 client that declares the capability confirms (sent,
//! ledgered, armed), declines or cancels (declined, nothing sent), lets it
//! time out, answers with an error or has its call cancelled (refused,
//! nothing sent, the request cancelled at the client); one without the
//! capability is refused; one confirmation waits at a time, and other
//! calls go on meanwhile. All on paused time.

use std::time::Duration;

use bacnet_ops::safety::ledger::{Ledger, State};
use bacnet_types::primitives::PropertyValue;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::writes::{audit, command, one, slot, writes};
use super::{Agent, Lab};
use crate::policy::ArmMode;

const ASK: &str = "elicitation/create";

impl Agent {
    /// The 2025-11-25 handshake with these capabilities.
    async fn initialize_with(&mut self, capabilities: Value) {
        let reply = self
            .request(
                "initialize",
                json!({ "protocolVersion": "2025-11-25", "capabilities": capabilities,
                        "clientInfo": { "name": "older-agent", "version": "1" } }),
            )
            .await;
        assert!(reply.get("result").is_some(), "{reply}");
        self.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await;
    }

    /// Answer the server's request `ask` with `result`.
    async fn reply_to(&mut self, ask: &Value, result: Value) {
        self.send(json!({ "jsonrpc": "2.0", "id": ask["id"], "result": result }))
            .await;
    }
}

/// A lab offering writes, with an older client that declares form
/// elicitation as 2025-11-25 does (an empty capability).
async fn older(arm: ArmMode) -> (Lab, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut lab = Lab::start(writes(dir.path(), arm)).await;
    lab.agent
        .initialize_with(json!({ "elicitation": {} }))
        .await;
    (lab, dir)
}

/// Start a `bacnet_write` call; its id and the server's request.
async fn write(lab: &mut Lab, args: Value) -> (u64, Value) {
    let params = json!({ "name": "bacnet_write", "arguments": args });
    let id = lab.agent.post("tools/call", params).await;
    let ask = lab.agent.notification(ASK).await;
    (id, ask)
}

/// The call's result.
async fn result(lab: &mut Lab, id: u64) -> Value {
    let reply = lab.agent.answer(id).await;
    reply["result"].clone()
}

/// Acceptance: confirming a T1 write gets it sent, ledgered and audited
/// as confirmed through the server's request, and arms the session (with
/// `mcp_arm = "elicit"`); while the person decides, other calls go on.
#[tokio::test(start_paused = true)]
async fn an_older_client_confirms_and_the_write_is_sent_and_ledgered() {
    let (mut lab, dir) = older(ArmMode::Elicit).await;
    let (id, ask) = write(&mut lab, one(json!(42.5), 12)).await;
    let message = ask["params"]["message"].as_str().unwrap();
    assert!(
        message.contains("device 1201 analog-output 1 present-value @12"),
        "{message}"
    );
    assert!(message.contains("arms a write session"), "{message}");
    assert_eq!(
        ask["params"]["requestedSchema"]["required"],
        json!(["confirm"])
    );
    // Waiting holds no lock: the session reads, and nothing is written.
    let session = lab
        .agent
        .request("resources/read", json!({ "uri": "bacnet://session" }))
        .await;
    let text = session["result"]["contents"][0]["text"].as_str().unwrap();
    let session: Value = serde_json::from_str(text).unwrap();
    assert_eq!(session["level"], json!("read-only"), "{session}");
    assert_eq!(slot(&lab, 12).await, PropertyValue::Null);
    // Nor the workspace's: the TUI could take it meanwhile.
    drop(bacnet_ops::safety::lock::lock(dir.path(), "tui").unwrap());

    let answer = json!({ "action": "accept", "content": { "confirm": true } });
    lab.agent.reply_to(&ask, answer).await;
    let done = result(&mut lab, id).await;
    assert_eq!(done["isError"], json!(false), "{done}");
    let body = &done["structuredContent"];
    assert_eq!(
        body["approval"]["confirmed_via"],
        json!("server-elicitation")
    );
    assert_eq!(body["approval"]["approved_by"], json!("client"));
    assert_eq!(slot(&lab, 12).await, PropertyValue::Real(42.5));
    super::schema::check_output(&mut lab.agent, "bacnet_write", body).await;
    let ledger = Ledger::open(dir.path()).unwrap();
    let [entry] = ledger.entries() else {
        panic!("{:?}", ledger.entries());
    };
    assert_eq!(entry.state, State::Applied);
    let records = audit(dir.path());
    let arm = records.iter().find(|r| r["event"] == "arm").unwrap();
    assert_eq!(arm["confirmed_via"], json!("server-elicitation"), "{arm}");
    let sent = records.iter().find(|r| r["event"] == "write").unwrap();
    assert_eq!(sent["confirmed_via"], json!("server-elicitation"), "{sent}");
    assert_eq!(sent["client"]["claimed_name"], json!("older-agent"));
    lab.stop().await;
}

/// Acceptance: declining is recorded as declined and sends nothing; a
/// pre-image changed while the person decided is refused as stale.
#[tokio::test(start_paused = true)]
async fn declining_sends_nothing_and_a_stale_preview_is_refused() {
    let (mut lab, dir) = older(ArmMode::Elicit).await;
    let (id, ask) = write(&mut lab, one(json!(30), 14)).await;
    let before = lab.sends().len();
    let answer = json!({ "action": "decline" });
    lab.agent.reply_to(&ask, answer).await;
    let done = result(&mut lab, id).await;
    assert_eq!(
        done["structuredContent"]["reason"],
        json!("declined"),
        "{done}"
    );
    assert_eq!(lab.sends().len(), before, "nothing sent");
    assert_eq!(slot(&lab, 14).await, PropertyValue::Null);
    let records = audit(dir.path());
    let declined = records.iter().find(|r| r["event"] == "declined").unwrap();
    assert_eq!(
        declined["asked_via"],
        json!("server-elicitation"),
        "{declined}"
    );
    assert!(Ledger::open(dir.path()).unwrap().entries().is_empty());

    let (id, ask) = write(&mut lab, one(json!(20), 13)).await;
    command(&lab, 77.0, 13).await;
    let answer = json!({ "action": "accept", "content": { "confirm": true } });
    lab.agent.reply_to(&ask, answer).await;
    let done = result(&mut lab, id).await;
    assert_eq!(
        done["structuredContent"]["reason"],
        json!("stale"),
        "{done}"
    );
    assert_eq!(slot(&lab, 13).await, PropertyValue::Real(77.0));
    lab.stop().await;
}

/// The refusals recorded with `reason`, and their `why`.
fn refusals(dir: &std::path::Path, reason: &str) -> Vec<Value> {
    audit(dir)
        .into_iter()
        .filter(|r| r["outcome"]["why"]["reason"] == reason)
        .map(|r| r["why"].clone())
        .collect()
}

/// A cancel is declined, as a form's cancel is: recorded, nothing sent.
#[tokio::test(start_paused = true)]
async fn a_cancel_is_declined_and_sends_nothing() {
    let (mut lab, dir) = older(ArmMode::Elicit).await;
    let (id, ask) = write(&mut lab, one(json!(9), 15)).await;
    let before = lab.sends().len();
    lab.agent
        .reply_to(&ask, json!({ "action": "cancel" }))
        .await;
    let done = result(&mut lab, id).await;
    assert_eq!(
        done["structuredContent"]["reason"],
        json!("declined"),
        "{done}"
    );
    assert_eq!(lab.sends().len(), before, "nothing sent");
    assert_eq!(slot(&lab, 15).await, PropertyValue::Null);
    let records = audit(dir.path());
    let declined = records.iter().find(|r| r["event"] == "declined").unwrap();
    assert_eq!(declined["why"], json!("cancelled"), "{declined}");
    lab.stop().await;
}

/// The wait ends 5 s before the sealed state would (120 s from the
/// preview): an accept just inside it is sent; with no answer the request
/// is cancelled at the client by then, refused as expired with nothing
/// sent; an accept past it is not taken.
#[tokio::test(start_paused = true)]
async fn the_wait_ends_inside_the_sealed_states_lifetime() {
    let (mut lab, dir) = older(ArmMode::Elicit).await;
    let (id, ask) = write(&mut lab, one(json!(1), 14)).await;
    tokio::time::sleep(Duration::from_secs(114)).await;
    let accept = json!({ "action": "accept", "content": { "confirm": true } });
    lab.agent.reply_to(&ask, accept.clone()).await;
    let done = result(&mut lab, id).await;
    assert_eq!(done["isError"], json!(false), "just inside: {done}");
    assert_eq!(slot(&lab, 14).await, PropertyValue::Real(1.0));

    let (id, ask) = write(&mut lab, one(json!(2), 14)).await;
    let asked = Instant::now();
    let done = result(&mut lab, id).await;
    let waited = asked.elapsed();
    assert_eq!(
        done["structuredContent"]["reason"],
        json!("confirmation-expired")
    );
    assert!(
        waited >= Duration::from_secs(114) && waited <= Duration::from_secs(115),
        "{waited:?}"
    );
    let cancelled = lab.agent.notification("notifications/cancelled").await;
    assert_eq!(cancelled["params"]["requestId"], ask["id"], "{cancelled}");

    let (id, ask) = write(&mut lab, one(json!(3), 14)).await;
    tokio::time::sleep(Duration::from_secs(116)).await;
    lab.agent.reply_to(&ask, accept).await;
    let done = result(&mut lab, id).await;
    assert_eq!(
        done["structuredContent"]["reason"],
        json!("confirmation-expired")
    );
    assert_eq!(slot(&lab, 14).await, PropertyValue::Real(1.0), "not taken");
    let whys = refusals(dir.path(), "confirmation-expired");
    assert_eq!(whys.len(), 2, "{whys:?}");
    assert!(
        whys[0].as_str().unwrap().starts_with("no answer within 11"),
        "{whys:?}"
    );
    lab.stop().await;
}

/// Cancelling the tool call while it waits cancels the request at the
/// client too; nothing is sent, and the refusal is recorded.
#[tokio::test(start_paused = true)]
async fn cancelling_the_call_cancels_the_request() {
    let (mut lab, dir) = older(ArmMode::Elicit).await;
    let (id, ask) = write(&mut lab, one(json!(4), 13)).await;
    let before = lab.sends().len();
    lab.agent.cancel(id).await;
    let cancelled = lab.agent.notification("notifications/cancelled").await;
    assert_eq!(cancelled["params"]["requestId"], ask["id"], "{cancelled}");
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(lab.sends().len(), before, "nothing sent");
    assert_eq!(slot(&lab, 13).await, PropertyValue::Null);
    let whys = refusals(dir.path(), "confirmation-expired");
    assert_eq!(whys, [json!("the tool call was cancelled")]);
    lab.stop().await;
}

/// One confirmation waits at a time: a second write needing one is
/// refused (`confirmation-pending`), unasked and with nothing sent, while
/// the first goes on to be confirmed.
#[tokio::test(start_paused = true)]
async fn one_confirmation_waits_at_a_time() {
    let (mut lab, dir) = older(ArmMode::Elicit).await;
    let (first, ask) = write(&mut lab, one(json!(5), 12)).await;
    let params = json!({ "name": "bacnet_write", "arguments": one(json!(6), 16) });
    let second = lab.agent.post("tools/call", params).await;
    let refused = result(&mut lab, second).await;
    assert_eq!(
        refused["structuredContent"]["reason"],
        json!("confirmation-pending"),
        "{refused}"
    );
    assert!(
        lab.agent
            .drain(ASK, Duration::from_secs(1))
            .await
            .is_empty()
    );
    assert_eq!(refusals(dir.path(), "confirmation-pending").len(), 1);
    let accept = json!({ "action": "accept", "content": { "confirm": true } });
    lab.agent.reply_to(&ask, accept).await;
    let done = result(&mut lab, first).await;
    assert_eq!(done["isError"], json!(false), "{done}");
    assert_eq!(slot(&lab, 12).await, PropertyValue::Real(5.0));
    assert_eq!(slot(&lab, 16).await, PropertyValue::Null);
    // Answered, the next write may be asked again.
    let (id, again) = write(&mut lab, one(json!(7), 16)).await;
    lab.agent
        .reply_to(&again, json!({ "action": "decline" }))
        .await;
    let done = result(&mut lab, id).await;
    assert_eq!(
        done["structuredContent"]["reason"],
        json!("declined"),
        "{done}"
    );
    lab.stop().await;
}

/// An error answer is refused, recorded as what it was, nothing sent.
#[tokio::test(start_paused = true)]
async fn an_error_answer_is_recorded_as_one() {
    let (mut lab, dir) = older(ArmMode::Elicit).await;
    let (id, ask) = write(&mut lab, one(json!(8), 11)).await;
    let error = json!({ "jsonrpc": "2.0", "id": ask["id"],
                        "error": { "code": -32603, "message": "form crashed" } });
    lab.agent.send(error).await;
    let done = result(&mut lab, id).await;
    assert_eq!(
        done["structuredContent"]["reason"],
        json!("confirmation-expired")
    );
    assert_eq!(slot(&lab, 11).await, PropertyValue::Null);
    let whys = refusals(dir.path(), "confirmation-expired");
    assert_eq!(
        whys,
        [json!(
            "the client answered with an error (-32603: form crashed)"
        )]
    );
    lab.stop().await;
}

/// An older client without the capability, or with URL elicitation only,
/// is refused with no request sent; T2 is refused before anyone is asked.
#[tokio::test(start_paused = true)]
async fn without_the_capability_a_write_is_refused_unasked() {
    for capabilities in [json!({}), json!({ "elicitation": { "url": {} } })] {
        let dir = tempfile::tempdir().unwrap();
        let mut lab = Lab::start(writes(dir.path(), ArmMode::Elicit)).await;
        lab.agent.initialize_with(capabilities.clone()).await;
        let reply = lab.agent.call("bacnet_write", one(json!(1), 16)).await;
        assert_eq!(
            reply["structuredContent"]["reason"],
            json!("elicitation-unsupported"),
            "{capabilities}: {reply}"
        );
        let asked = lab.agent.drain(ASK, Duration::from_secs(1)).await;
        assert!(asked.is_empty(), "{asked:?}");
        lab.stop().await;
    }
    let (mut lab, _dir) = older(ArmMode::Elicit).await;
    let reply = lab.agent.call("bacnet_write", one(json!(1), 8)).await;
    assert_eq!(
        reply["structuredContent"]["reason"],
        json!("needs-verified-approver")
    );
    assert!(
        lab.agent
            .drain(ASK, Duration::from_secs(1))
            .await
            .is_empty()
    );
    lab.stop().await;
}
