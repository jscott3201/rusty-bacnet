//! Writes over MCP (#2068) against the lab device: the tool list, a T1
//! write confirmed through the client and ledgered, the tiers the client
//! can't approve, bad confirmations, a stale preview, a timeout, revert,
//! a held workspace and a legacy client. All on paused time.

use std::path::Path;

use bacnet_ops::safety::ledger::{Ledger, State};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::schema::check_output;
use super::{LAB, Lab};
use crate::Policy;
use crate::policy::{ArmMode, Workspace, WriteMode, WritePolicy};

/// A policy that offers writes, confirmed by the client, on `dir`.
pub(super) fn writes(dir: &Path, arm: ArmMode) -> Policy {
    Policy {
        writes: WritePolicy {
            mode: WriteMode::Elicit,
            arm,
            plan: None,
            workspace: Some(Workspace {
                dir: dir.to_path_buf(),
                profile: Some("lab".into()),
                transport: "BIP",
                local: "10.0.0.1:47808".into(),
            }),
        },
        ..Policy::default()
    }
}

/// The lab's AO:1.
fn ao1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap()
}

/// AO:1's priority slot `n`, on the device itself.
pub(super) async fn slot(lab: &Lab, n: u32) -> PropertyValue {
    let db = lab.server.database().read().await;
    let object = db.get(&ao1()).unwrap();
    object
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(n))
        .unwrap()
}

/// Command AO:1 at `priority` on the device, as another commander would.
pub(super) async fn command(lab: &Lab, value: f32, priority: u8) {
    let operator = bacnet_objects::command_source::CommandOrigin::Local {
        owner_device: ObjectIdentifier::new(ObjectType::DEVICE, LAB).unwrap(),
        initiating_object: None,
    };
    let mut db = lab.server.database().write().await;
    let object = db.get_mut(&ao1()).unwrap();
    object
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(value),
            Some(priority),
            &operator,
        )
        .unwrap();
}

/// The `_meta` of a 2026-07-28 client that takes form elicitations.
pub(super) fn meta() -> Value {
    let mut meta = super::modern_meta();
    meta["io.modelcontextprotocol/clientCapabilities"] = json!({ "elicitation": { "form": {} } });
    meta
}

/// One write of AO:1's Present_Value at `priority`.
pub(super) fn one(value: Value, priority: u8) -> Value {
    json!({
        "device": LAB,
        "writes": [{ "object": "ao:1", "value": value, "priority": priority }],
    })
}

/// A 2026-07-28 `tools/call` of `tool`, with `extra` params (the retry's).
pub(super) async fn call(lab: &mut Lab, tool: &str, arguments: Value, extra: Value) -> Value {
    let mut params = json!({ "name": tool, "arguments": arguments, "_meta": meta() });
    if let Value::Object(extra) = extra {
        for (key, value) in extra {
            params[key] = value;
        }
    }
    let reply = lab.agent.request("tools/call", params).await;
    reply
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("{tool} failed: {reply}"))
}

/// The retry's params: the state, and the form answered.
pub(super) fn answered(state: &Value, action: &str, confirm: bool) -> Value {
    json!({
        "requestState": state,
        "inputResponses": { "confirm": { "action": action, "content": { "confirm": confirm } } },
    })
}

/// The audit log's records.
pub(super) fn audit(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join("audit.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// WriteProperty requests the lab device has seen: its slot is the proof,
/// so count sends instead where nothing may go.
fn sends(lab: &Lab) -> usize {
    lab.sends().len()
}

async fn start(arm: ArmMode) -> (Lab, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut lab = Lab::start(writes(dir.path(), arm)).await;
    lab.agent.initialize().await;
    (lab, dir)
}

/// Acceptance 1: with `mcp_writes = "never"` (the default) the write tools
/// are absent; with `elicit` they are listed, marked as destructive.
#[tokio::test(start_paused = true)]
async fn the_write_tools_are_listed_only_when_the_profile_offers_writes() {
    let mut plain = Lab::start(Policy::default()).await;
    plain.agent.initialize().await;
    let tools = plain.agent.request("tools/list", json!({})).await;
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"bacnet_write") && !names.contains(&"bacnet_revert"));
    plain.stop().await;

    let dir = tempfile::tempdir().unwrap();
    let mut lab = Lab::start(writes(dir.path(), ArmMode::Elicit)).await;
    let hello = lab.agent.initialize().await;
    let instructions = hello["result"]["instructions"].as_str().unwrap();
    assert!(
        instructions.contains("only with approval") && instructions.contains("bacnet_revert"),
        "{instructions}"
    );
    assert!(
        !instructions.contains("Read-only") && !instructions.contains("cannot write"),
        "{instructions}"
    );
    let tools = lab.agent.request("tools/list", json!({})).await;
    let tools = tools["result"]["tools"].as_array().unwrap().clone();
    for name in ["bacnet_write", "bacnet_revert"] {
        let tool = tools.iter().find(|t| t["name"] == json!(name)).unwrap();
        assert_eq!(tool["annotations"]["readOnlyHint"], json!(false), "{tool}");
        assert_eq!(
            tool["annotations"]["destructiveHint"],
            json!(true),
            "{tool}"
        );
    }
    lab.stop().await;
}

/// Acceptance 2 and 6: a T1 write is previewed (nothing written), the
/// client confirms, it is sent, ledgered and audited, and bacnet_revert
/// puts the pre-image back.
#[tokio::test(start_paused = true)]
async fn a_t1_write_is_confirmed_sent_ledgered_and_reverted() {
    let (mut lab, dir) = start(ArmMode::Elicit).await;
    let first = call(&mut lab, "bacnet_write", one(json!(42.5), 12), json!({})).await;
    assert_eq!(first["resultType"], json!("input_required"), "{first}");
    let form = &first["inputRequests"]["confirm"];
    assert_eq!(form["method"], json!("elicitation/create"), "{form}");
    let message = form["params"]["message"].as_str().unwrap();
    assert!(
        message.contains("device 1201 analog-output 1 present-value @12"),
        "{message}"
    );
    assert!(message.contains("arms a write session"), "{message}");
    assert_eq!(
        slot(&lab, 12).await,
        PropertyValue::Null,
        "nothing written yet"
    );
    let state = first["requestState"].clone();

    let done = call(
        &mut lab,
        "bacnet_write",
        one(json!(42.5), 12),
        answered(&state, "accept", true),
    )
    .await;
    assert_eq!(done["isError"], json!(false), "{done}");
    let body = &done["structuredContent"];
    assert_eq!(
        body["results"][0]["outcome"]["result"],
        json!("ack"),
        "{body}"
    );
    assert_eq!(body["approval"]["confirmed_via"], json!("elicitation"));
    assert_eq!(slot(&lab, 12).await, PropertyValue::Real(42.5));
    check_output(&mut lab.agent, "bacnet_write", body).await;

    let ledger = Ledger::open(dir.path()).unwrap();
    let [entry] = ledger.entries() else {
        panic!("{:?}", ledger.entries());
    };
    assert_eq!(entry.state, State::Applied);
    assert_eq!(entry.origin.principal.as_deref(), Some("tester"));
    assert_eq!(body["results"][0]["ledger_id"], json!(entry.id));
    let records = audit(dir.path());
    let arm = records.iter().find(|r| r["event"] == "arm").unwrap();
    assert_eq!(arm["confirmed_via"], json!("elicitation"));
    let write = records.iter().find(|r| r["event"] == "write").unwrap();
    for (key, value) in [
        ("front_end", json!("mcp")),
        ("principal", json!("tester")),
        ("tool", json!("bacnet_write")),
        ("confirmed_via", json!("elicitation")),
        ("approved_by", json!("client")),
        ("level", json!("write-session")),
    ] {
        assert_eq!(write[key], value, "{key}: {write}");
    }
    assert_eq!(write["client"]["claimed_name"], json!("modern-agent"));
    let read = lab.agent.read("bacnet://ledger").await.unwrap();
    assert_eq!(read["entries"][0]["ledger_id"], json!(entry.id), "{read}");

    let reverted = call(
        &mut lab,
        "bacnet_revert",
        json!({ "entries": [entry.id] }),
        json!({}),
    )
    .await;
    assert_eq!(reverted["isError"], json!(false), "{reverted}");
    check_output(
        &mut lab.agent,
        "bacnet_revert",
        &reverted["structuredContent"],
    )
    .await;
    assert_eq!(slot(&lab, 12).await, PropertyValue::Null, "the pre-image");
    assert!(Ledger::open(dir.path()).unwrap().entries().is_empty());
    lab.stop().await;
}

/// Acceptance 2: T2 (priority 8), T3 (priority 1) and priority 6 are
/// refused before any confirmation is asked for, and recorded.
#[tokio::test(start_paused = true)]
async fn t2_t3_and_priority_6_are_refused() {
    let (mut lab, dir) = start(ArmMode::Elicit).await;
    for (priority, reason) in [
        (8, "needs-verified-approver"),
        (1, "needs-verified-approver"),
        (6, "priority-6"),
    ] {
        let reply = call(
            &mut lab,
            "bacnet_write",
            one(json!(10), priority),
            json!({}),
        )
        .await;
        assert_eq!(reply["isError"], json!(true), "{reply}");
        let body = &reply["structuredContent"];
        assert_eq!(body["error"], json!("refused"), "{body}");
        assert_eq!(body["reason"], json!(reason), "{body}");
        check_output(&mut lab.agent, "bacnet_write", body).await;
        assert_eq!(slot(&lab, u32::from(priority)).await, PropertyValue::Null);
    }
    let refused = audit(dir.path())
        .into_iter()
        .filter(|r| r["outcome"]["result"] == "refused")
        .count();
    assert_eq!(refused, 3);
    lab.stop().await;
}

/// Acceptance 3: a tampered, expired or replayed state is refused, and
/// nothing is sent for it; a declined form is recorded as declined. (The
/// state's binding to its principal is `writes::seal`'s unit test.)
#[tokio::test(start_paused = true)]
async fn bad_confirmations_send_nothing() {
    let (mut lab, dir) = start(ArmMode::Elicit).await;
    let args = one(json!(30), 14);
    let asked = |reply: &Value| reply["requestState"].clone();

    let state = asked(&call(&mut lab, "bacnet_write", args.clone(), json!({})).await);
    let mut tampered = state.as_str().unwrap().to_string();
    let last = tampered.pop().unwrap();
    tampered.push(if last == '0' { '1' } else { '0' });
    let before = sends(&lab);
    let reply = call(
        &mut lab,
        "bacnet_write",
        args.clone(),
        answered(&json!(tampered), "accept", true),
    )
    .await;
    assert_eq!(
        reply["structuredContent"]["reason"],
        json!("confirmation-invalid")
    );
    assert_eq!(sends(&lab), before, "nothing sent");

    let state = asked(&call(&mut lab, "bacnet_write", args.clone(), json!({})).await);
    tokio::time::advance(std::time::Duration::from_secs(121)).await;
    let before = sends(&lab);
    let reply = call(
        &mut lab,
        "bacnet_write",
        args.clone(),
        answered(&state, "accept", true),
    )
    .await;
    assert_eq!(
        reply["structuredContent"]["reason"],
        json!("confirmation-expired")
    );
    assert_eq!(sends(&lab), before, "nothing sent");

    // Another value, priority or array index than the one confirmed.
    let indexed = json!({ "device": LAB, "writes": [{ "object": "ao:1",
        "property": "present-value[1]", "value": 30, "priority": 14 }] });
    let mut state = Value::Null;
    for other in [one(json!(31), 14), one(json!(30), 15), indexed] {
        state = asked(&call(&mut lab, "bacnet_write", args.clone(), json!({})).await);
        let reply = call(
            &mut lab,
            "bacnet_write",
            other.clone(),
            answered(&state, "accept", true),
        )
        .await;
        assert_eq!(
            reply["structuredContent"]["reason"],
            json!("confirmation-mismatch"),
            "{other}: {reply}"
        );
    }
    let reply = call(
        &mut lab,
        "bacnet_write",
        args.clone(),
        answered(&state, "accept", true),
    )
    .await;
    assert_eq!(
        reply["structuredContent"]["reason"],
        json!("confirmation-replayed"),
        "a state opens once, even when it was refused"
    );

    let state = asked(&call(&mut lab, "bacnet_write", args.clone(), json!({})).await);
    let before = sends(&lab);
    let reply = call(
        &mut lab,
        "bacnet_write",
        args.clone(),
        answered(&state, "decline", false),
    )
    .await;
    assert_eq!(reply["structuredContent"]["reason"], json!("declined"));
    assert_eq!(sends(&lab), before, "nothing sent");
    assert!(audit(dir.path()).iter().any(|r| r["event"] == "declined"));
    assert_eq!(slot(&lab, 14).await, PropertyValue::Null);
    lab.stop().await;
}

/// Acceptance 4: the slot changes between the preview and the retry, so
/// the write is refused as stale, recorded, and not sent.
#[tokio::test(start_paused = true)]
async fn a_change_since_the_preview_is_refused_as_stale() {
    let (mut lab, dir) = start(ArmMode::Elicit).await;
    let args = one(json!(20), 13);
    let first = call(&mut lab, "bacnet_write", args.clone(), json!({})).await;
    command(&lab, 77.0, 13).await;
    let reply = call(
        &mut lab,
        "bacnet_write",
        args,
        answered(&first["requestState"], "accept", true),
    )
    .await;
    let body = &reply["structuredContent"];
    assert_eq!(body["reason"], json!("stale"), "{body}");
    assert_eq!(
        slot(&lab, 13).await,
        PropertyValue::Real(77.0),
        "not overwritten"
    );
    let records = audit(dir.path());
    let stale = records.iter().find(|r| r["event"] == "declined").unwrap();
    assert_eq!(stale["why"], json!("stale"), "{stale}");
    assert!(Ledger::open(dir.path()).unwrap().entries().is_empty());
    lab.stop().await;
}

/// Acceptance 5: the device goes silent once the targets are read again,
/// so the write's outcome is unknown: the ledger keeps it as `unknown`.
#[tokio::test(start_paused = true)]
async fn a_write_without_an_answer_stays_in_the_ledger_as_unknown() {
    let (mut lab, dir) = start(ArmMode::Elicit).await;
    let args = one(json!(55), 15);
    let first = call(&mut lab, "bacnet_write", args.clone(), json!({})).await;
    // The re-read under the lock is one request; the write gets no answer.
    lab.go_silent_after(1);
    let reply = call(
        &mut lab,
        "bacnet_write",
        args,
        answered(&first["requestState"], "accept", true),
    )
    .await;
    let body = &reply["structuredContent"];
    assert_eq!(reply["isError"], json!(true), "{reply}");
    assert_eq!(body["error"], json!("timeout"), "{body}");
    assert_eq!(body["applied"], json!("unknown"), "{body}");
    check_output(&mut lab.agent, "bacnet_write", body).await;
    let ledger = Ledger::open(dir.path()).unwrap();
    let [entry] = ledger.entries() else {
        panic!("{:?}", ledger.entries());
    };
    assert_eq!(entry.state, State::Unknown);
    lab.stop().await;
}

/// Without `mcp_arm = "elicit"` the client can't arm a session, and a
/// client that can't take a form, or the TUI holding the workspace,
/// refuses too, with nothing written.
#[tokio::test(start_paused = true)]
async fn unarmed_unsupported_and_locked_writes_are_refused() {
    let (mut lab, dir) = start(ArmMode::Never).await;
    let reply = call(&mut lab, "bacnet_write", one(json!(1), 16), json!({})).await;
    assert_eq!(reply["structuredContent"]["reason"], json!("not-armed"));
    lab.stop().await;
    drop(dir);

    let (mut lab, dir) = start(ArmMode::Elicit).await;
    let reply = lab.agent.call("bacnet_write", one(json!(1), 16)).await;
    assert_eq!(
        reply["structuredContent"]["reason"],
        json!("elicitation-unsupported"),
        "{reply}"
    );
    // A confirmed write, with the TUI holding the workspace's lock.
    let first = call(&mut lab, "bacnet_write", one(json!(2), 16), json!({})).await;
    let held = bacnet_ops::safety::lock::lock(dir.path(), "tui").unwrap();
    let reply = call(
        &mut lab,
        "bacnet_write",
        one(json!(2), 16),
        answered(&first["requestState"], "accept", true),
    )
    .await;
    let body = &reply["structuredContent"];
    assert_eq!(body["reason"], json!("workspace-locked"), "{body}");
    // Windows does not let another handle read a locked file's holder.
    #[cfg(unix)]
    assert!(
        body["message"].as_str().unwrap().contains("bacnet tui"),
        "{body}"
    );
    drop(held);
    assert_eq!(slot(&lab, 16).await, PropertyValue::Null);
    lab.stop().await;
}

/// Over MCP a Priority_Array element is never written (the slot is
/// commanded through present-value instead), and a write to
/// Relinquish_Default is T3, so the client can't confirm it.
#[tokio::test(start_paused = true)]
async fn priority_array_and_relinquish_default_are_out_of_the_clients_reach() {
    let (mut lab, dir) = start(ArmMode::Elicit).await;
    let before = sends(&lab);
    let slot_write = json!({ "device": LAB, "writes": [{ "object": "ao:1",
        "property": "priority-array[12]", "value": 5 }] });
    let reply = call(&mut lab, "bacnet_write", slot_write, json!({})).await;
    assert_eq!(
        reply["structuredContent"]["reason"],
        json!("read-only-property"),
        "{reply}"
    );
    assert_eq!(sends(&lab), before, "nothing sent, not even a read");
    // By number too, and in a batch with a write that would be allowed:
    // the whole batch is refused, and each refusal recorded.
    for property in ["87[12]", "priority-array[3]"] {
        let mixed = json!({ "device": LAB, "batch": true, "writes": [
            { "object": "ao:1", "value": 5, "priority": 12 },
            { "object": "ao:1", "property": property, "value": 5 }] });
        let reply = call(&mut lab, "bacnet_write", mixed, json!({})).await;
        assert_eq!(
            reply["structuredContent"]["reason"],
            json!("read-only-property"),
            "{property}: {reply}"
        );
    }
    assert_eq!(sends(&lab), before);
    let refused = audit(dir.path())
        .into_iter()
        .filter(|r| r["outcome"]["why"]["reason"] == "read-only-property")
        .count();
    assert_eq!(refused, 3, "each refusal recorded");
    let fallback = json!({ "device": LAB, "writes": [{ "object": "ao:1",
        "property": "relinquish-default", "value": 5 }] });
    let reply = call(&mut lab, "bacnet_write", fallback, json!({})).await;
    assert_eq!(
        reply["structuredContent"]["reason"],
        json!("needs-verified-approver"),
        "{reply}"
    );
    assert!(reply.get("inputRequests").is_none(), "{reply}");
    lab.stop().await;
}
