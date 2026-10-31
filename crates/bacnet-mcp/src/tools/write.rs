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
//! A client older than 2026-07-28 that takes the older elicitation
//! requests is asked by the server instead (`write_confirm`, #2158), and
//! its answer goes through the same retry. Targets are numbers only:
//! nothing a device reports chooses what is written.

use std::sync::Arc;

use bacnet_ops::parse::{parse_object_specifier, parse_property, parse_value_for};
use bacnet_ops::safety::audit::{Audit, Context};
use bacnet_ops::safety::desk::{self, Caller};
use bacnet_ops::safety::ledger::Ledger;
use bacnet_ops::safety::{FrontEnd, Gate, Preview, Tier, WriteSpec, item_json};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;
use rmcp::RoleServer;
use rmcp::model::{InputRequest, InputRequests, InputRequiredResult, InputResponses};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::write_task::{self, Handoff, hand_off};
use super::{Spec, schema};
use crate::context::{Call, Ctx, DeviceArg};
use crate::errors::ToolError;
use crate::policy::{MAX_WRITES, Workspace};
use crate::writes::{Approval, Approver, ClientElicitation, Deny, McpCaller, Via, preview_digest};

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
                "confirmed_via": { "enum": ["elicitation", "server-elicitation", "plan"] },
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
        "sent": { "type": "boolean" },
        "ledger_ids": { "type": "array", "items": { "type": "integer" } },
        "ledger": { "type": "string" },
        "revert": { "type": "string" },
        "reason": { "type": "string" },
        "message": { "type": "string" },
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Args {
    device: DeviceArg,
    writes: Vec<Item>,
    #[serde(default)]
    pub(super) batch: bool,
    pub(super) reason: Option<String>,
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
    /// The session, when the client is older than 2026-07-28 and takes
    /// the older elicitation requests: the server asks it to confirm.
    pub(crate) session: Option<rmcp::service::Peer<RoleServer>>,
    /// The request's id, for the audit log.
    pub(crate) request_id: String,
}

/// The retry's part of a request: the sealed state and the form's answer.
#[derive(Debug, Default)]
pub(crate) struct Retry {
    pub(crate) state: Option<String>,
    pub(crate) responses: Option<InputResponses>,
    /// The answer came from an elicitation request the server sent, not
    /// from a retry the client made.
    pub(crate) server: bool,
}

/// A step of a write or revert: waiting for the client's confirmation, or
/// handed to a task of its own, which the call then waits for.
pub(crate) enum Step {
    Ask(InputRequiredResult),
    Sending(Handoff),
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
                None => (PropertyIdentifier::PRESENT_VALUE, None),
            };
            if property == PropertyIdentifier::PRIORITY_ARRAY {
                return Err(ToolError::refused(
                    "read-only-property",
                    "Priority_Array is read-only: command or relinquish a slot by writing \
                     present-value with that priority (null relinquishes)",
                ));
            }
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
pub(super) fn context(workspace: &Workspace) -> Context {
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

pub(super) fn items(preview: &Preview) -> Value {
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
    let fields = fields(&ctx.principal, peer, raw);
    let refusing = McpCaller::new(&ctx.writes, &ctx.policy.writes, None, fields.clone());
    let specs = match specs(device.instance, &args) {
        Ok(specs) => specs,
        Err(error @ ToolError::Refused { .. }) => {
            // A policy refusal (Priority_Array), recorded like the others.
            audit_refusal(&workspace, &refusing, "write", &error, json!({})).await;
            return Err(error);
        }
        Err(error) => return Err(error),
    };
    let account = Arc::clone(&call.account);
    match retry.state {
        None if ctx.writes.plan(&ctx.policy.writes).is_some() => {
            let sending = write_task::sending();
            let work = write_task::planned(
                Arc::clone(ctx),
                workspace.clone(),
                specs,
                args,
                fields.clone(),
                Arc::clone(&sending),
            );
            Ok(Step::Sending(hand_off(
                ctx, workspace, account, sending, fields, work,
            )))
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
            let via = if retry.server {
                Via::ServerElicitation
            } else {
                Via::Elicitation
            };
            if let Some(why) = declined(retry.responses.as_ref()) {
                let error = ToolError::refused("declined", format!("the user {why}"));
                let fields = json!({ "why": why, "asked_via": via.name() });
                audit_refusal(&workspace, &refusing, "declined", &error, fields).await;
                return Err(error);
            }
            let sending = write_task::sending();
            let work = write_task::confirmed(
                Arc::clone(ctx),
                workspace.clone(),
                (specs, args),
                (state, via),
                fields.clone(),
                Arc::clone(&sending),
            );
            Ok(Step::Sending(hand_off(
                ctx, workspace, account, sending, fields, work,
            )))
        }
    }
}

/// The refusal of a write the server asked an older client to confirm
/// (#2158) when no answer came: in time, or at all (`why`). Audited;
/// nothing was sent. The sealed state was never handed out, so nothing
/// can redeem it.
pub(crate) async fn unconfirmed<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    raw: &Map<String, Value>,
    peer: &Peer,
    why: &str,
) -> ToolError {
    let error = ToolError::refused(
        "confirmation-expired",
        format!("the write wasn't confirmed: {why}; nothing was sent"),
    );
    if let Some(workspace) = &ctx.policy.writes.workspace {
        let fields = fields(&ctx.principal, peer, raw);
        let caller = McpCaller::new(&ctx.writes, &ctx.policy.writes, None, fields);
        let fields = json!({ "why": why, "asked_via": Via::ServerElicitation.name() });
        audit_refusal(workspace, &caller, "write", &error, fields).await;
    }
    error
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
pub(super) fn blocked(preview: &Preview, why: &str) -> ToolError {
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
        Gate::Allowed(tier) => mcp_tier(&preview, *tier),
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
            supported: peer.form || peer.session.is_some(),
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

/// The tier a change takes over MCP: the planner's, except that a write
/// to Relinquish_Default, the value a point falls back to when every slot
/// is empty, is T3, so a client's confirmation can't approve it.
fn mcp_tier(preview: &Preview, tier: Tier) -> Tier {
    let fallback = preview
        .items
        .iter()
        .any(|item| item.spec.property == PropertyIdentifier::RELINQUISH_DEFAULT);
    if fallback { Tier::T3 } else { tier }
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
