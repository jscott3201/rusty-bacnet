//! `bacnet_revert` (#2068): put back the pre-images of this principal's
//! ledger entries, latest write first, through the shared desk.
//!
//! As in the TUI, reverting needs no armed session and no prompt: it only
//! undoes changes this principal made, each kept in the ledger with what
//! was there before. Entries another front end or principal wrote are
//! refused. It runs on a task of its own under the workspace's lock, so a
//! revert once started is recorded whatever happens to the call.

use std::collections::BTreeSet;
use std::sync::Arc;

use bacnet_tool::safety::ledger::{Entry, Ledger};
use bacnet_tool::safety::{FrontEnd, Outcome};
use bacnet_transport::port::TransportPort;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::write::{Peer, Step};
use super::write_task::{self, hand_off, open};
use super::{Spec, schema};
use crate::context::{Call, Ctx};
use crate::errors::ToolError;
use crate::writes::McpCaller;

pub(crate) const NAME: &str = "bacnet_revert";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "Revert BACnet writes",
    description: "Put back what was there before this agent's writes: name ledger entries (the \
                  ledger_id of bacnet_write's results, or bacnet://ledger), or all of them. A \
                  commandable slot is relinquished (or given back another commander's value); \
                  another property gets its old value. Only this principal's entries.",
    input,
    output,
    idempotent: false,
    open_world: true,
    passive: false,
};

fn input() -> Value {
    schema::input(
        json!({
            "entries": {
                "type": "array",
                "items": { "type": "integer", "minimum": 1 },
                "maxItems": 100,
                "description": "Ledger ids to revert",
            },
            "all": {
                "type": "boolean",
                "description": "Revert every entry of this principal's",
            },
        }),
        &[],
    )
}

fn output() -> Value {
    schema::output(json!({
        "results": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "ledger_id": { "type": "integer" },
                    "target": { "type": "string" },
                    "outcome": { "type": "object" },
                    "applied": { "type": ["boolean", "null"] },
                },
                "required": ["ledger_id", "target", "outcome", "applied"],
            },
        },
        "left": { "type": "integer" },
        "reason": { "type": "string" },
        "message": { "type": "string" },
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    entries: Option<Vec<u64>>,
    #[serde(default)]
    all: bool,
}

/// `bacnet_revert`'s description on a session attached to the TUI.
pub(super) const ATTACHED: &str = "Put back what was there before this principal's MCP writes \
    (every agent of this OS user's, through the TUI or stdio): name ledger entries (the ledger_id \
    of bacnet_write's results, or bacnet://ledger) on one device, or all of them. The person at \
    the TUI confirms the revert in its preview dialog, at the tier of the writes it undoes; the \
    call waits for that answer.";

/// An entry this principal wrote through MCP.
pub(super) fn ours(entry: &Entry, principal: &str) -> bool {
    entry.origin.front_end == FrontEnd::Mcp && entry.origin.principal.as_deref() == Some(principal)
}

/// Run one `bacnet_revert` call.
pub(crate) async fn run<T: TransportPort + 'static>(
    ctx: &Arc<Ctx<T>>,
    call: &Call,
    raw: &Map<String, Value>,
    peer: &Peer,
) -> Result<Step, ToolError> {
    let args: Args = serde_json::from_value(Value::Object(raw.clone()))
        .map_err(|e| ToolError::invalid("entries", e.to_string()))?;
    let wanted: Option<BTreeSet<u64>> = match (&args.entries, args.all) {
        (Some(ids), false) if !ids.is_empty() => Some(ids.iter().copied().collect()),
        (None, true) => None,
        _ => {
            return Err(ToolError::invalid(
                "entries",
                "name the entries to revert, or set all (not both)",
            ));
        }
    };
    ctx.engine()?;
    let workspace =
        ctx.policy.writes.workspace.clone().ok_or_else(|| {
            ToolError::refused("write-not-granted", "no workspace for the ledger")
        })?;
    // Checked before the lock: an entry that isn't this principal's, or a
    // device out of reach, refuses the call with nothing sent.
    let entries = Ledger::open(&workspace.dir)
        .map(|l| l.entries().to_vec())
        .unwrap_or_default();
    if let Some(ids) = &wanted {
        for id in ids {
            match entries.iter().find(|e| e.id == *id) {
                None => {
                    return Err(ToolError::invalid(
                        "entries",
                        format!("no ledger entry {id}"),
                    ));
                }
                Some(entry) if !ours(entry, &ctx.principal) => {
                    return Err(ToolError::refused(
                        "not-yours",
                        format!("ledger entry {id} was written by another front end or principal"),
                    ));
                }
                Some(_) => {}
            }
        }
    }
    if let Some(entry) = entries.iter().find(|e| {
        ours(e, &ctx.principal)
            && wanted.as_ref().is_none_or(|ids| ids.contains(&e.id))
            && !ctx.policy.allows_device(e.device)
    }) {
        return Err(ToolError::refused(
            "device-not-allowed",
            format!(
                "device {} is not in this server's [mcp] devices",
                entry.device
            ),
        ));
    }
    let mut fields = Map::new();
    fields.insert("tool".into(), json!(NAME));
    fields.insert("principal".into(), json!(ctx.principal));
    fields.insert("client".into(), json!({ "claimed_name": peer.name }));
    fields.insert("args_digest".into(), json!(crate::cursor::digest(raw)));
    fields.insert("request_id".into(), json!(peer.request_id));
    let account = Arc::clone(&call.account);
    let sending = write_task::sending();
    if ctx.attached.is_some() {
        // The TUI holds the workspace: the person there confirms the
        // revert, and its desk does it (#2185).
        return attached(ctx, &entries, wanted, (workspace, account, sending, fields));
    }
    let work = {
        let ctx = Arc::clone(ctx);
        let workspace = workspace.clone();
        let fields = fields.clone();
        async move {
            let engine = ctx.engine()?;
            let caller = McpCaller::new(&ctx.writes, &ctx.policy.writes, None, fields);
            let mut desk = open(&ctx, &workspace, &caller).await?;
            let principal = ctx.principal.clone();
            let before = desk.entries();
            let done = desk
                .revert(engine.client(), engine.limits(), &caller, |entry| {
                    ours(entry, &principal)
                        && wanted.as_ref().is_none_or(|ids| ids.contains(&entry.id))
                })
                .await;
            ctx.writes.signals.ledger_settled(&before, &desk, false);
            let left = desk
                .entries()
                .iter()
                .filter(|e| ours(e, &principal))
                .count();
            body(&done, left)
        }
    };
    Ok(Step::Sending(hand_off(
        ctx, workspace, account, sending, fields, work,
    )))
}

/// An attached session's revert: the writes that put the chosen entries'
/// pre-images back, for the TUI to confirm at the tier of the writes they
/// undo. Refused across devices, since the dialog confirms one device; with
/// nothing to put back, nothing is asked and nothing sent.
fn attached<T: TransportPort + 'static>(
    ctx: &Arc<Ctx<T>>,
    entries: &[Entry],
    wanted: Option<BTreeSet<u64>>,
    (workspace, account, sending, fields): (
        crate::policy::Workspace,
        Arc<bacnet_ops::limits::Account>,
        crate::writes::Sending,
        Map<String, Value>,
    ),
) -> Result<Step, ToolError> {
    use bacnet_tool::safety::{Gate, Tier};
    // Only entries whose old value is known: the dialog shows each one's
    // revert, and the desk sends exactly those (`write_attached`).
    let chosen: Vec<Entry> = entries
        .iter()
        .filter(|e| {
            ours(e, &ctx.principal)
                && wanted.as_ref().is_none_or(|ids| ids.contains(&e.id))
                && e.undo().is_some()
        })
        .cloned()
        .collect();
    let ids: BTreeSet<u64> = chosen.iter().map(|e| e.id).collect();
    let preview = bacnet_tool::safety::plan::undo_preview(&chosen);
    match preview {
        Some(mut preview) if !preview.items.is_empty() => {
            // The planner's tier, never blocked here: `undo_preview` plans
            // as `dangerous`, since a priority 6 entry was written so, and
            // its revert is T3 at that priority. Over MCP a write to
            // Relinquish_Default is T3, and so is undoing one.
            let planned = match preview.gate {
                Gate::Allowed(tier) => tier,
                Gate::Blocked(_) => Tier::T3,
            };
            let tier = super::write::mcp_tier(&preview, planned);
            preview.gate = Gate::Allowed(tier);
            Ok(Step::Wait(Box::new(preview), tier, Some(ids)))
        }
        None if !chosen.is_empty() => Err(ToolError::refused(
            "several-devices",
            "attached to the TUI, a revert goes to one device at a time: name the entries of \
             one device",
        )),
        // Nothing to put back: no prompt, and no job, so nothing unconfirmed
        // can be sent, whatever entry holds these ids by then. Each is
        // reported from the ledger as read.
        _ => {
            let left = entries.iter().filter(|e| ours(e, &ctx.principal)).count();
            let unknown = || Outcome::NotSent(bacnet_tool::safety::desk::UNKNOWN_PRE_IMAGE.into());
            let done: Vec<(Entry, Outcome)> = entries
                .iter()
                .filter(|e| {
                    ours(e, &ctx.principal) && wanted.as_ref().is_none_or(|ids| ids.contains(&e.id))
                })
                .map(|e| (e.clone(), unknown()))
                .collect();
            let work = async move { body(&done, left) };
            Ok(Step::Sending(hand_off(
                ctx, workspace, account, sending, fields, work,
            )))
        }
    }
}

/// The result: each entry's outcome, and how many of this principal's are
/// left; an error result when not every entry was put back.
pub(super) fn body(done: &[(Entry, Outcome)], left: usize) -> Result<Value, ToolError> {
    let results: Vec<Value> = done
        .iter()
        .map(|(entry, outcome)| {
            json!({
                "ledger_id": entry.id,
                "target": entry.label(),
                "outcome": outcome.json(),
                "applied": outcome.applied(),
            })
        })
        .collect();
    let mut body = json!({ "results": results, "left": left });
    if done.iter().any(|(_, o)| o.applied() != Some(true)) {
        body["error"] = json!(if done.iter().any(|(_, o)| o.applied().is_none()) {
            "timeout"
        } else {
            "failed"
        });
        return Err(ToolError::Write(body));
    }
    Ok(body)
}
