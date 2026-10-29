//! `bacnet_write` (#2068; design §7.4): property writes through the TUI's
//! preview tiers, ledger and audit log, approved by a plan grant or, for
//! T1 only, by the client's confirmation.
//!
//! The first call reads the targets and previews the change. A plan grant
//! that covers it sends it at once; otherwise the result is
//! `input_required`, with an elicitation form showing the canonical
//! targets and a sealed `requestState`. The retry brings the form's answer
//! and the state back: the state is opened (once, for this caller, within
//! 120 s), the targets are read again under the workspace's lock, a
//! preview that changed is refused as stale, and the writes go through
//! the shared desk, which records each in the ledger before it is sent.
//! Targets are numbers only: nothing a device reports chooses what is
//! written.

use std::sync::Arc;

use bacnet_ops::parse::{parse_object_specifier, parse_property, parse_value_for};
use bacnet_ops::safety::audit::{Audit, Context};
use bacnet_ops::safety::desk::{self, Caller, Desk, STALE, Sent};
use bacnet_ops::safety::ledger::{Ledger, Origin};
use bacnet_ops::safety::{FrontEnd, Gate, Outcome, Preview, Tier, WriteSpec, item_json};
use bacnet_transport::port::TransportPort;
use bacnet_types::primitives::ObjectIdentifier;
use rmcp::model::{InputRequest, InputRequests, InputRequiredResult, InputResponses};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Spec, schema};
use crate::context::{Call, Ctx, DeviceArg};
use crate::errors::ToolError;
use crate::policy::{MAX_WRITES, Workspace};
use crate::writes::{
    Approval, Approver, ClientElicitation, Deny, McpCaller, PlanApprover, Via, preview_digest,
};

pub(crate) const NAME: &str = "bacnet_write";

/// The key of the confirmation form in `inputRequests`.
pub(crate) const CONFIRM: &str = "confirm";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "Write BACnet properties",
    description: "Write properties of one device's objects, through the same preview tiers, \
                  ledger and audit log as the bacnet TUI. Targets are numeric (device instance, \
                  type:instance objects); values are numbers, booleans, null (relinquish a \
                  commandable slot) or the bacnet value grammar. A T1 write (priority 9 to 16, or \
                  a property that isn't commandable) asks the user to confirm through the client; \
                  T2 and T3 writes are refused (needs-verified-approver) unless a plan grant covers \
                  them. Each applied write is kept in the ledger with its pre-image for \
                  bacnet_revert.",
    input,
    output,
    idempotent: false,
    open_world: true,
    passive: false,
};

fn input() -> Value {
    schema::input(
        json!({
            "device": schema::device(),
            "writes": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_WRITES,
                "items": {
                    "type": "object",
                    "properties": {
                        "object": schema::object(),
                        "property": {
                            "type": "string",
                            "description": "kebab-case name or number, with [index] for one array \
                                            element; present-value when left out",
                        },
                        "value": {
                            "type": ["number", "boolean", "string", "object", "null"],
                            "description": "A number, a boolean, null to relinquish, a typed value \
                                            ({\"value\": 1}) or the value grammar (\"active\")",
                        },
                        "priority": { "type": "integer", "minimum": 1, "maximum": 16 },
                    },
                    "required": ["object", "value"],
                    "additionalProperties": false,
                },
            },
            "batch": {
                "type": "boolean",
                "description": "Send as one WritePropertyMultiple (a T2 change)",
            },
            "reason": { "type": "string", "maxLength": 200 },
        }),
        &["device", "writes"],
    )
}

fn output() -> Value {
    schema::output(json!({
        "device": { "type": "integer" },
        "approval": {
            "type": "object",
            "properties": {
                "confirmed_via": { "enum": ["elicitation", "plan"] },
                "approved_by": { "type": "string" },
            },
        },
        "results": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "target": { "type": "string" },
                    "outcome": { "type": "object" },
                    "applied": { "type": ["boolean", "null"] },
                    "ledger_id": { "type": ["integer", "null"] },
                },
                "required": ["target", "outcome", "applied", "ledger_id"],
            },
        },
        "applied": { "type": "string" },
        "revert": { "type": "string" },
        "reason": { "type": "string" },
        "message": { "type": "string" },
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    device: DeviceArg,
    writes: Vec<Item>,
    #[serde(default)]
    batch: bool,
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    object: String,
    property: Option<String>,
    value: Value,
    priority: Option<u8>,
}

/// What the client says about itself, from the request.
#[derive(Debug, Clone, Default)]
pub(crate) struct Peer {
    /// The name the client claims, cleaned and capped.
    pub(crate) name: Option<String>,
    /// It takes form elicitations over multi round-trip requests.
    pub(crate) form: bool,
    /// The request's id, for the audit log.
    pub(crate) request_id: String,
}

/// The retry's part of a request: the sealed state and the form's answer.
#[derive(Debug, Default)]
pub(crate) struct Retry {
    pub(crate) state: Option<String>,
    pub(crate) responses: Option<InputResponses>,
}

/// A step of the write: done, or waiting for the client's confirmation.
pub(crate) enum Step {
    Done(Value),
    Ask(InputRequiredResult),
}

/// A value as the value grammar's text; `None` relinquishes.
fn value_text(value: &Value) -> Result<Option<String>, ToolError> {
    match value {
        Value::Null => Ok(None),
        Value::Bool(on) => Ok(Some(on.to_string())),
        Value::Number(n) => Ok(Some(n.to_string())),
        Value::String(text) => Ok(Some(text.clone())),
        Value::Object(map) => match map.get("value") {
            Some(inner @ (Value::Number(_) | Value::Bool(_) | Value::String(_))) => {
                value_text(inner)
            }
            _ => Err(ToolError::invalid(
                "value",
                "a typed value needs a \"value\"",
            )),
        },
        Value::Array(_) => Err(ToolError::invalid("value", "one value a write")),
    }
}

/// The writes, checked: numeric targets, values in the grammar.
fn specs(device: u32, args: &Args) -> Result<Vec<WriteSpec>, ToolError> {
    if args.writes.is_empty() || args.writes.len() > MAX_WRITES {
        return Err(ToolError::invalid(
            "writes",
            format!("1 to {MAX_WRITES} writes a call"),
        ));
    }
    if args.writes.len() > 1 && !args.batch {
        return Err(ToolError::invalid(
            "writes",
            "several writes go as one batch: set batch",
        ));
    }
    args.writes
        .iter()
        .map(|item| {
            let (object_type, instance) = parse_object_specifier(&item.object)
                .map_err(|e| ToolError::invalid("object", e))?;
            let object = ObjectIdentifier::new(object_type, instance)
                .map_err(|e| ToolError::invalid("object", e.to_string()))?;
            let (property, index) = match &item.property {
                Some(text) => {
                    parse_property(text).map_err(|e| ToolError::invalid("property", e))?
                }
                None => (bacnet_types::enums::PropertyIdentifier::PRESENT_VALUE, None),
            };
            let value = value_text(&item.value)?;
            if let Some(text) = &value {
                parse_value_for(object_type, property, index, text)
                    .map_err(|e| ToolError::invalid("value", e))?;
            }
            Ok(WriteSpec {
                device,
                object,
                property,
                index,
                value,
                priority: item.priority,
            })
        })
        .collect()
}

/// The audit log's context for this server's workspace.
fn context(workspace: &Workspace) -> Context {
    Context {
        profile: workspace.profile.clone(),
        transport: workspace.transport,
        local: workspace.local.clone(),
    }
}

/// The fields every record of this call carries.
fn fields(principal: &str, peer: &Peer, raw: &Map<String, Value>) -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert("tool".into(), json!(NAME));
    fields.insert("principal".into(), json!(principal));
    fields.insert("client".into(), json!({ "claimed_name": peer.name }));
    fields.insert("args_digest".into(), json!(crate::cursor::digest(raw)));
    fields.insert("request_id".into(), json!(peer.request_id));
    fields
}

/// Record a refusal (nothing sent) without the workspace's lock: the
/// audit log is append-only, and another front end may hold the ledger.
pub(crate) async fn audit_refusal(
    workspace: &Workspace,
    caller: &McpCaller<'_>,
    event: &str,
    why: &ToolError,
    mut fields: Value,
) {
    let audit = match Audit::open(&workspace.dir, context(workspace)) {
        Ok(audit) => audit.written_by(FrontEnd::Mcp, None),
        Err(error) => return tracing::error!("{error}"),
    };
    if let Value::Object(map) = &mut fields {
        map.insert(
            "outcome".into(),
            json!({ "result": "refused", "why": why.json() }),
        );
        for (key, value) in caller.fields() {
            map.entry(key).or_insert(value);
        }
    }
    if let Err(error) = audit.record(caller.level(), event, fields).await {
        tracing::error!("{error}");
    }
}

fn items(preview: &Preview) -> Value {
    json!({
        "items": preview.items.iter().map(item_json).collect::<Vec<_>>(),
        "tier": preview.gate.name(),
    })
}

/// Run one `bacnet_write` call: the first, or the retry with the client's
/// confirmation.
pub(crate) async fn run<T: TransportPort + 'static>(
    ctx: &Arc<Ctx<T>>,
    call: &Call,
    raw: &Map<String, Value>,
    retry: Retry,
    peer: &Peer,
) -> Result<Step, ToolError> {
    let args: Args = serde_json::from_value(Value::Object(raw.clone()))
        .map_err(|e| ToolError::invalid("writes", e.to_string()))?;
    let workspace =
        ctx.policy.writes.workspace.clone().ok_or_else(|| {
            ToolError::refused("write-not-granted", "no workspace for the ledger")
        })?;
    let device = ctx.device(call, &args.device).await?;
    let specs = specs(device.instance, &args)?;
    let fields = fields(&ctx.principal, peer, raw);
    let refusing = McpCaller {
        writes: &ctx.writes,
        policy: &ctx.policy.writes,
        via: None,
        fields: fields.clone(),
    };
    match retry.state {
        None if ctx.writes.plan(&ctx.policy.writes).is_some() => {
            planned(ctx, &workspace, &refusing, &specs, &args).await
        }
        None => preview(ctx, &workspace, &refusing, &specs, &args, raw, peer).await,
        Some(token) => {
            let opened = match ctx.writes.sealer.open(&token, &ctx.principal, NAME) {
                Ok(state) if state.args == crate::cursor::digest(raw) => Ok(state),
                Ok(_) => Err(ToolError::refused(
                    "confirmation-mismatch",
                    "the retry's arguments differ from the ones confirmed; nothing was sent",
                )),
                Err(why) => Err(ToolError::refused(
                    why.reason(),
                    "the confirmation state isn't one this server issued to this caller and \
                     tool, has expired, or was used before; nothing was sent",
                )),
            };
            let state = match opened {
                Ok(state) => state,
                Err(error) => {
                    audit_refusal(&workspace, &refusing, "write", &error, json!({})).await;
                    return Err(error);
                }
            };
            if let Some(why) = declined(retry.responses.as_ref()) {
                let error = ToolError::refused("declined", format!("the user {why}"));
                let fields = json!({ "why": why });
                audit_refusal(&workspace, &refusing, "declined", &error, fields).await;
                return Err(error);
            }
            confirmed(ctx, call, workspace, specs, args, state, fields).await
        }
    }
}

/// Why the form's answer isn't a confirmation, if it isn't.
fn declined(responses: Option<&InputResponses>) -> Option<&'static str> {
    let Some(answer) = responses.and_then(|r| r.get(CONFIRM)) else {
        return Some("gave no answer");
    };
    match answer["action"].as_str() {
        Some("accept") if answer["content"]["confirm"] == json!(true) => None,
        Some("accept") => Some("did not tick confirm"),
        Some("decline") => Some("declined"),
        _ => Some("cancelled"),
    }
}

/// The refusal for a blocked change.
fn blocked(preview: &Preview, why: &str) -> ToolError {
    let six = preview.items.iter().any(|item| item.priority == Some(6));
    let reason = if six { "priority-6" } else { "blocked" };
    ToolError::refused(reason, why.to_string())
}

/// The first call without a plan grant: preview, then ask, or refuse.
async fn preview<T: TransportPort + 'static>(
    ctx: &Arc<Ctx<T>>,
    workspace: &Workspace,
    caller: &McpCaller<'_>,
    specs: &[WriteSpec],
    args: &Args,
    raw: &Map<String, Value>,
    peer: &Peer,
) -> Result<Step, ToolError> {
    let engine = ctx.engine()?;
    // Without the workspace's lock: the ledger file says which slots are
    // ours, and the retry reads everything again under the lock.
    let ledger = Ledger::open(&workspace.dir).ok();
    let preview = desk::prepare(
        engine.client(),
        engine.limits(),
        caller,
        specs,
        args.batch,
        false,
        ledger.as_ref(),
    )
    .await
    .map_err(ToolError::Failed)?;
    let tier = match &preview.gate {
        Gate::Allowed(tier) => *tier,
        Gate::Blocked(why) => {
            let error = blocked(&preview, why);
            audit_refusal(workspace, caller, "write", &error, items(&preview)).await;
            return Err(error);
        }
    };
    let approver: Box<dyn Approver> = if ctx.policy.writes.offered() {
        Box::new(ClientElicitation {
            arm: ctx.policy.writes.arm,
            armed: ctx.writes.is_armed(),
            supported: peer.form,
        })
    } else {
        Box::new(Deny)
    };
    match approver.approve(&ctx.principal, &preview, tier) {
        Approval::Ask { arm } => {
            let state = ctx.writes.sealer.seal(
                &ctx.principal,
                NAME,
                crate::cursor::digest(raw),
                preview_digest(&preview),
                arm,
            );
            Ok(Step::Ask(ask(&preview, tier, arm, state)))
        }
        Approval::Refused { reason, message } => {
            let error = ToolError::refused(reason, message);
            audit_refusal(workspace, caller, "write", &error, items(&preview)).await;
            Err(error)
        }
        Approval::Approved(_) => unreachable!("a client's confirmation is asked for"),
    }
}

/// The `input_required` result: the confirmation form and the state.
fn ask(preview: &Preview, tier: Tier, arm: Option<u32>, state: String) -> InputRequiredResult {
    let mut message = format!(
        "Confirm this {tier:?} BACnet write ({}):\n",
        preview.service()
    );
    for item in &preview.items {
        let before = item.before.as_ref().map_or("NULL", |b| b.text.as_str());
        let (now, _) = crate::untrusted::clean(before, 80);
        let (to, _) = crate::untrusted::clean(&item.value.text, 80);
        message.push_str(&format!("- {}: {now} -> {to}\n", item.label()));
        if let bacnet_ops::safety::Effect::Changes { from, to } = &item.effect {
            let (from, _) = crate::untrusted::clean(from, 80);
            let (to, _) = crate::untrusted::clean(to, 80);
            message.push_str(&format!("  present value: {from} -> {to}\n"));
        }
        for note in &item.notes {
            message.push_str(&format!("  note: {note}\n"));
        }
    }
    message.push_str("Values are as the device reports them.");
    let properties = json!({
        "confirm": { "type": "boolean", "title": "Send this write", "default": false },
    });
    if let Some(minutes) = arm {
        message.push_str(&format!(
            "\nConfirming also arms a write session for {minutes} minutes."
        ));
    }
    let request: InputRequest = serde_json::from_value(json!({
        "method": "elicitation/create",
        "params": {
            "mode": "form",
            "message": message,
            "requestedSchema": {
                "type": "object",
                "properties": properties,
                "required": ["confirm"],
            },
        },
    }))
    .expect("a well-formed elicitation");
    let mut requests = InputRequests::new();
    requests.insert(CONFIRM.into(), request);
    InputRequiredResult::new(Some(requests), Some(state))
}

/// The retry: the client confirmed. Runs on a task of its own, so a write
/// once confirmed is read, sent and recorded even if the call goes away.
async fn confirmed<T: TransportPort + 'static>(
    ctx: &Arc<Ctx<T>>,
    call: &Call,
    workspace: Workspace,
    specs: Vec<WriteSpec>,
    args: Args,
    state: crate::writes::seal::State,
    fields: Map<String, Value>,
) -> Result<Step, ToolError> {
    let ctx = Arc::clone(ctx);
    let account = Arc::clone(&call.account);
    let task = async move {
        let engine = ctx.engine()?;
        let caller = McpCaller {
            writes: &ctx.writes,
            policy: &ctx.policy.writes,
            via: Some(Via::Elicitation),
            fields,
        };
        let mut desk = open(&ctx, &workspace, &caller).await?;
        let fresh = desk
            .prepare(
                engine.client(),
                engine.limits(),
                &caller,
                &specs,
                args.batch,
                false,
            )
            .await;
        let fresh = match fresh {
            Ok(fresh) if preview_digest(&fresh) == state.preview => fresh,
            other => {
                let detail = other
                    .err()
                    .unwrap_or_else(|| "the pre-image changed".into());
                let fields = json!({ "why": "stale", "detail": detail });
                desk.record(&caller, "declined", fields).await;
                return Err(ToolError::refused("stale", format!("{STALE}: {detail}")));
            }
        };
        if let Some(minutes) = state.arm {
            ctx.writes.arm(minutes, args.reason.clone());
            let fields = json!({ "minutes": minutes, "reason": args.reason });
            desk.record(&caller, "arm", fields).await;
        }
        let sent = desk
            .commit(engine.client(), engine.limits(), &caller, &fresh)
            .await;
        outcome(fresh.device, Via::Elicitation, sent)
    };
    tokio::spawn(bacnet_ops::limits::charged(account, task))
        .await
        .map_err(|e| ToolError::Failed(format!("the write task failed: {e}")))?
}

/// The first call under a plan grant: everything under the workspace's
/// lock, with no prompt.
async fn planned<T: TransportPort + 'static>(
    ctx: &Arc<Ctx<T>>,
    workspace: &Workspace,
    refusing: &McpCaller<'_>,
    specs: &[WriteSpec],
    args: &Args,
) -> Result<Step, ToolError> {
    let engine = ctx.engine()?;
    let mut desk = open(ctx, workspace, refusing).await?;
    let preview = desk
        .prepare(
            engine.client(),
            engine.limits(),
            refusing,
            specs,
            args.batch,
            false,
        )
        .await
        .map_err(ToolError::Failed)?;
    let tier = match &preview.gate {
        Gate::Allowed(tier) => *tier,
        Gate::Blocked(why) => {
            let error = blocked(&preview, why);
            desk_refusal(&desk, refusing, &error, &preview).await;
            return Err(error);
        }
    };
    let Some(plan) = ctx.writes.plan(&ctx.policy.writes) else {
        return Err(ToolError::refused("plan-spent", "the plan grant ended"));
    };
    let approver = PlanApprover {
        plan,
        writes: &ctx.writes,
    };
    if let Approval::Refused { reason, message } = approver.approve(&ctx.principal, &preview, tier)
    {
        let error = ToolError::refused(reason, message);
        desk_refusal(&desk, refusing, &error, &preview).await;
        return Err(error);
    }
    let caller = McpCaller {
        via: Some(Via::Plan),
        fields: refusing.fields.clone(),
        ..*refusing
    };
    let sent = desk
        .commit(engine.client(), engine.limits(), &caller, &preview)
        .await;
    outcome(preview.device, Via::Plan, sent)
}

async fn desk_refusal(desk: &Desk, caller: &McpCaller<'_>, error: &ToolError, preview: &Preview) {
    let mut fields = items(preview);
    fields["outcome"] = json!({ "result": "refused", "why": error.json() });
    desk.record(caller, "write", fields).await;
}

/// The workspace's desk, under its lock; refused when another front end
/// holds it (the TUI, say), with nothing sent.
pub(crate) async fn open<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    workspace: &Workspace,
    caller: &McpCaller<'_>,
) -> Result<Desk, ToolError> {
    let origin = Origin::mcp(ctx.principal.clone());
    let mut desk = Desk::open(Some(&workspace.dir), context(workspace), origin);
    if let Some(why) = desk.missing() {
        let error = ToolError::refused("workspace-locked", why.to_string());
        desk.record(
            caller,
            "write",
            json!({ "outcome": { "result": "refused", "why": error.json() } }),
        )
        .await;
        return Err(error);
    }
    desk.settle_abandoned(caller).await;
    Ok(desk)
}

/// The result: each write's outcome and ledger entry, or the error body
/// when not every write was acknowledged.
fn outcome(device: u32, via: Via, sent: Vec<Sent>) -> Result<Step, ToolError> {
    if !sent.is_empty()
        && sent
            .iter()
            .all(|s| s.outcome == Outcome::NotSent(STALE.into()))
    {
        return Err(ToolError::refused("stale", STALE));
    }
    let results: Vec<Value> = sent
        .iter()
        .map(|s| {
            json!({
                "target": s.label,
                "outcome": s.outcome.json(),
                "applied": s.outcome.applied(),
                "ledger_id": s.entry,
            })
        })
        .collect();
    let ids: Vec<u64> = sent.iter().filter_map(|s| s.entry).collect();
    let mut body = json!({
        "device": device,
        "approval": { "confirmed_via": via.name(), "approved_by": via.by() },
        "results": results,
    });
    if !ids.is_empty() {
        body["revert"] = json!(format!(
            "bacnet_revert with entries {ids:?} puts the pre-images back"
        ));
    }
    let error = if sent.iter().any(|s| s.outcome == Outcome::Timeout) {
        Some("timeout")
    } else if sent.iter().any(|s| s.outcome.applied().is_none()) {
        Some("failed")
    } else if sent
        .iter()
        .any(|s| matches!(s.outcome, Outcome::Refused(_) | Outcome::Denied))
    {
        Some("bacnet")
    } else if sent.iter().any(|s| s.outcome != Outcome::Ack) {
        Some("refused")
    } else {
        None
    };
    match error {
        None => Ok(Step::Done(body)),
        Some(error) => {
            body["error"] = json!(error);
            if sent.iter().any(|s| s.outcome.applied().is_none()) {
                body["applied"] = json!("unknown");
            }
            Err(ToolError::Write(body))
        }
    }
}
