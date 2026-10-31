//! The part of a write that runs on a task of its own (#2068, design
//! §7.4): a confirmed retry, a planned write or a revert, from the
//! workspace's lock to the settled ledger. A call cancelled or out of time
//! only stops waiting: the task goes on to send, settle and record. The
//! tasks are tracked on the server, and a shutdown waits for them, at most
//! one call's budget plus its grace (45 s by default), before it stops; a
//! task still running then is dropped with the runtime, and the next
//! holder of the workspace's lock settles its `sending` entries as
//! unknown. A task that panics has them settled at once.

use std::sync::{Arc, Mutex, PoisonError};

use bacnet_ops::limits::{Account, charged};
use bacnet_ops::safety::desk::{Desk, STALE, Sent};
use bacnet_ops::safety::ledger::Origin;
use bacnet_ops::safety::{Gate, Outcome, Preview, WriteSpec};
use bacnet_transport::port::TransportPort;
use serde_json::{Map, Value, json};
use tokio::task::JoinHandle;

use super::write::{Args, blocked, context, items};
use crate::context::Ctx;
use crate::errors::ToolError;
use crate::policy::Workspace;
use crate::writes::seal::State;
use crate::writes::{Approval, Approver, McpCaller, PlanApprover, Sending, Via, preview_digest};

/// A write handed to its task: the task, and the ledger ids it has
/// recorded as `sending` so far.
pub(crate) struct Handoff {
    pub(crate) task: JoinHandle<Result<Value, ToolError>>,
    pub(crate) sending: Sending,
}

impl Handoff {
    /// The answer for a call that stopped waiting: the write may or may
    /// not have gone, so no retry is suggested; the ledger will say.
    pub(crate) fn pending(&self) -> ToolError {
        let ids = self
            .sending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if ids.is_empty() {
            // Not in the ledger yet, so not sent yet; it may still go.
            return ToolError::Write(json!({
                "error": "pending",
                "sent": false,
                "ledger_ids": ids,
                "ledger": "bacnet://ledger",
                "message": "the write was handed to its own task and nothing has been sent yet; \
                            it may still be sent, and bacnet://ledger will show it (do not retry \
                            it)",
            }));
        }
        ToolError::Write(json!({
            "error": "pending",
            "applied": "unknown",
            "ledger_ids": ids,
            "ledger": "bacnet://ledger",
            "message": "the write was handed to its own task and is still running: it may or \
                        may not have been sent; bacnet://ledger shows its entries as they settle \
                        (do not retry it)",
        }))
    }
}

/// Run `work` on a task of its own, tracked by the server, charged to
/// `account`. If it panics, the workspace's `sending` entries are settled
/// as unknown, audited with the call's `fields`.
pub(crate) fn hand_off<T, F>(
    ctx: &Arc<Ctx<T>>,
    workspace: Workspace,
    account: Arc<Account>,
    sending: Sending,
    fields: Map<String, Value>,
    work: F,
) -> Handoff
where
    T: TransportPort + 'static,
    F: Future<Output = Result<Value, ToolError>> + Send + 'static,
{
    let ctx = Arc::clone(ctx);
    let tracker = ctx.handoffs.clone();
    let task = tracker.spawn(async move {
        match tokio::spawn(charged(account, work)).await {
            Ok(result) => result,
            Err(failed) => {
                tracing::error!("a write task failed: {failed}");
                // Its desk, and the lock, went with it. Whoever takes the
                // lock next settles what it left `sending`: this, unless
                // another front end got there first and did it already.
                let caller = McpCaller::new(&ctx.writes, &ctx.policy.writes, None, fields);
                let origin = Origin::mcp(ctx.principal.clone());
                let mut desk = Desk::open(Some(&workspace.dir), context(&workspace), origin);
                let before = desk.entries();
                desk.settle_abandoned(&caller).await;
                ctx.writes.signals.ledger_settled(&before, &desk);
                Err(ToolError::Failed(format!(
                    "the write task failed: {failed}"
                )))
            }
        }
    });
    Handoff { task, sending }
}

/// A new place for a write's `sending` ids.
pub(crate) fn sending() -> Sending {
    Arc::new(Mutex::new(Vec::new()))
}

/// The retry: the client confirmed. Read again under the lock, refuse a
/// stale preview, arm if asked, send.
pub(crate) async fn confirmed<T: TransportPort + 'static>(
    ctx: Arc<Ctx<T>>,
    workspace: Workspace,
    specs: Vec<WriteSpec>,
    args: Args,
    state: State,
    fields: Map<String, Value>,
    sending: Sending,
) -> Result<Value, ToolError> {
    let engine = ctx.engine()?;
    let mut caller = McpCaller::new(
        &ctx.writes,
        &ctx.policy.writes,
        Some(Via::Elicitation),
        fields,
    );
    caller.sending = Some(sending);
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
    let before = desk.entries();
    let sent = desk
        .commit(engine.client(), engine.limits(), &caller, &fresh)
        .await;
    ctx.writes.signals.ledger_settled(&before, &desk);
    outcome(fresh.device, Via::Elicitation, sent)
}

/// A write under a plan grant: everything under the workspace's lock,
/// with no prompt.
pub(crate) async fn planned<T: TransportPort + 'static>(
    ctx: Arc<Ctx<T>>,
    workspace: Workspace,
    specs: Vec<WriteSpec>,
    args: Args,
    fields: Map<String, Value>,
    sending: Sending,
) -> Result<Value, ToolError> {
    let engine = ctx.engine()?;
    let refusing = McpCaller::new(&ctx.writes, &ctx.policy.writes, None, fields.clone());
    let mut desk = open(&ctx, &workspace, &refusing).await?;
    let preview = desk
        .prepare(
            engine.client(),
            engine.limits(),
            &refusing,
            &specs,
            args.batch,
            false,
        )
        .await
        .map_err(ToolError::Failed)?;
    let tier = match &preview.gate {
        Gate::Allowed(tier) => *tier,
        Gate::Blocked(why) => {
            let error = blocked(&preview, why);
            desk_refusal(&desk, &refusing, &error, &preview).await;
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
        desk_refusal(&desk, &refusing, &error, &preview).await;
        return Err(error);
    }
    let mut caller = McpCaller::new(&ctx.writes, &ctx.policy.writes, Some(Via::Plan), fields);
    caller.sending = Some(sending);
    let before = desk.entries();
    let sent = desk
        .commit(engine.client(), engine.limits(), &caller, &preview)
        .await;
    ctx.writes.signals.ledger_settled(&before, &desk);
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
    let before = desk.entries();
    desk.settle_abandoned(caller).await;
    ctx.writes.signals.ledger_settled(&before, &desk);
    Ok(desk)
}

/// The result: each write's outcome and ledger entry, or the error body
/// when not every write was acknowledged.
fn outcome(device: u32, via: Via, sent: Vec<Sent>) -> Result<Value, ToolError> {
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
        None => Ok(body),
        Some(error) => {
            body["error"] = json!(error);
            if sent.iter().any(|s| s.outcome.applied().is_none()) {
                body["applied"] = json!("unknown");
            }
            Err(ToolError::Write(body))
        }
    }
}
