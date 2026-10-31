//! Writes confirmed by a client older than 2026-07-28 (#2158): the server
//! sends it an `elicitation/create` request over the session and waits.
//! A 2025-11-25 client that declares the capability confirms (sent,
//! ledgered, armed), declines (nothing sent), cancels or lets it time out
//! (refused, nothing sent); one without the capability is refused, and
//! other calls go on while a confirmation waits. All on paused time.

use std::time::Duration;

use bacnet_ops::safety::ledger::{Ledger, State};
use bacnet_types::primitives::PropertyValue;
use serde_json::{Value, json};

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

/// No answer within the confirmation's 120 s, or a cancel, is refused as
/// expired with nothing sent; on the timeout the client is told the
/// request was cancelled.
#[tokio::test(start_paused = true)]
async fn no_answer_or_a_cancel_expires_the_confirmation() {
    let (mut lab, dir) = older(ArmMode::Elicit).await;
    let (id, ask) = write(&mut lab, one(json!(9), 15)).await;
    let before = lab.sends().len();
    // Nothing answers: the paused clock runs on to the timeout.
    let done = result(&mut lab, id).await;
    let body = &done["structuredContent"];
    assert_eq!(body["reason"], json!("confirmation-expired"), "{done}");
    let cancelled = lab.agent.notification("notifications/cancelled").await;
    assert_eq!(cancelled["params"]["requestId"], ask["id"], "{cancelled}");
    assert_eq!(lab.sends().len(), before, "nothing sent");

    let (id, ask) = write(&mut lab, one(json!(9), 15)).await;
    lab.agent
        .reply_to(&ask, json!({ "action": "cancel" }))
        .await;
    let done = result(&mut lab, id).await;
    let body = &done["structuredContent"];
    assert_eq!(body["reason"], json!("confirmation-expired"), "{done}");
    assert_eq!(slot(&lab, 15).await, PropertyValue::Null);
    let expired = audit(dir.path())
        .into_iter()
        .filter(|r| r["outcome"]["why"]["reason"] == "confirmation-expired")
        .count();
    assert_eq!(expired, 2, "each recorded");
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
