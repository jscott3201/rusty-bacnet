//! An attached agent's writes and reverts (#2185; design §7.3, §9). The
//! TUI holds the workspace's lock for its whole session, so the agent's
//! session can't open the desk itself: the TUI lends its desk instead,
//! running a [`DeskJob`] on its write worker, and the person at the TUI
//! approves each write first ([`TuiApprover`]).
//!
//! A write's first call previews it without the lock, as on stdio, at the
//! tier MCP gives it, then waits outside the call's budget for the TUI's
//! answer. Confirmed, the write is handed to a task of its own, which
//! sends a job to the TUI: the desk reads the targets again (a stale
//! preview isn't sent), records the ledger entries as this agent's, sends
//! and audits them `confirmed_via: "tui"`, `approved_by:` the TUI's user.
//! A decline, a timeout, the TUI closing or the request withdrawn is
//! audited here, with nothing sent. A revert is asked for the same way:
//! the dialog shows the writes that put the pre-images back, each at the
//! tier of the write it undoes, and the job checks the TUI's session may
//! write before it sends anything.

use std::collections::BTreeSet;
use std::sync::Arc;

use bacnet_ops::limits::{Account, charged};
use bacnet_tool::safety::desk::{Caller, Desk, Sent};
use bacnet_tool::safety::ledger::{Entry, Origin};
use bacnet_tool::safety::{Gate, Outcome, Preview, Tier};
use bacnet_transport::port::TransportPort;
use rmcp::ErrorData;
use serde_json::{Map, Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::write::{self, Peer, audit_refusal, items};
use super::write_task::{self, Handoff, hand_off};
use crate::context::{Call, Ctx};
use crate::errors::ToolError;
use crate::writes::tui::{AgentApproval, TuiApprover};
use crate::writes::{Approval, McpCaller, Sending, Via, WaitingApprover};

/// What an attached session's writes go through: the TUI's queue of
/// requests and the desk it lends, for agent `agent`.
pub(crate) struct Attached<T: TransportPort + 'static> {
    pub(crate) approvals: mpsc::Sender<AgentApproval>,
    pub(crate) desk: mpsc::Sender<DeskJob<T>>,
    /// The connection's number on the listener.
    pub(crate) agent: u64,
}

/// A write or revert of an attached agent's, to run on the desk the TUI
/// lends ([`run`](Self::run)). Dropped unrun, nothing is sent and the
/// agent hears so.
pub struct DeskJob<T: TransportPort + 'static> {
    ctx: Arc<Ctx<T>>,
    account: Arc<Account>,
    fields: Map<String, Value>,
    work: Work,
}

enum Work {
    Write {
        preview: Box<Preview>,
        sending: Sending,
        done: oneshot::Sender<Result<Vec<Sent>, ToolError>>,
    },
    Revert {
        wanted: BTreeSet<u64>,
        /// What the person confirmed: a revert is never sent unconfirmed.
        confirmed: Box<Preview>,
        done: oneshot::Sender<Result<Reverted, ToolError>>,
    },
}

/// Why the entries `current` (those the confirmed ids name now) are not
/// what `confirmed` showed: an id taken by another write since, or a write
/// merged into an entry, changes what a revert would send. Their revert,
/// rebuilt, must match item for item (target, priority, the value put back
/// and the value it replaces), on one device this server may reach.
fn changed(confirmed: &Preview, current: &[Entry], policy: &crate::Policy) -> Option<String> {
    let Some(now) = bacnet_tool::safety::plan::undo_preview(current) else {
        return Some("the entries confirmed have changed".into());
    };
    if !policy.allows_device(now.device) {
        return Some(format!(
            "device {} is not in this server's [mcp] devices",
            now.device
        ));
    }
    let same = now.device == confirmed.device
        && now.items.len() == current.len()
        && now.items.len() == confirmed.items.len()
        && now.items.iter().zip(&confirmed.items).all(|(a, b)| {
            a.spec == b.spec
                && a.priority == b.priority
                && a.value.bytes == b.value.bytes
                && a.before == b.before
        });
    (!same).then(|| "the entries confirmed have changed".into())
}

/// A revert's entries and outcomes, and how many of the principal's are
/// left.
pub(crate) type Reverted = (Vec<(Entry, Outcome)>, usize);

impl<T: TransportPort + 'static> std::fmt::Debug for DeskJob<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.work {
            Work::Write { .. } => "write",
            Work::Revert { .. } => "revert",
        };
        f.debug_struct("DeskJob")
            .field("kind", &kind)
            .finish_non_exhaustive()
    }
}

impl<T: TransportPort + 'static> DeskJob<T> {
    /// Run it on `desk`, which the front end `host` holds: as the agent's
    /// principal (its ledger entries are the agent's to revert), at the
    /// host's level and under its refusals, the reads and requests charged
    /// to the agent's call. Each target's label and outcome come back, for
    /// the host to show; the agent gets the full result.
    pub async fn run(self, desk: &mut Desk, host: &dyn Caller) -> Vec<(String, Outcome)> {
        let Self {
            ctx,
            account,
            fields,
            work,
        } = self;
        let engine = match ctx.engine() {
            Ok(engine) => engine,
            Err(error) => {
                // No client to send with: say so, rather than vanish.
                match work {
                    Work::Write { done, .. } => drop(done.send(Err(error))),
                    Work::Revert { done, .. } => drop(done.send(Err(error))),
                }
                return Vec::new();
            }
        };
        let was = desk.set_origin(Origin::mcp(ctx.principal.clone()));
        let before = desk.entries();
        let shown = match work {
            Work::Write {
                preview,
                sending,
                done,
            } => {
                let via = Some(Via::Tui);
                let mut caller = McpCaller::new(&ctx.writes, &ctx.policy.writes, via, fields);
                caller.sending = Some(sending);
                let caller = caller.hosted(host);
                let sent = match desk.write_refusal(&caller, &preview, false) {
                    Some(why) => desk.not_sent(&caller, &preview, &why).await,
                    None => {
                        let (client, limits) = (engine.client(), engine.limits());
                        let execute = desk.execute(client, limits, &caller, &preview, false);
                        charged(account, execute).await
                    }
                };
                let announced = caller.announced();
                ctx.writes.signals.ledger_settled(&before, desk, announced);
                let shown = sent
                    .iter()
                    .map(|s| (s.label.clone(), s.outcome.clone()))
                    .collect();
                let _ = done.send(Ok(sent));
                shown
            }
            Work::Revert {
                wanted,
                confirmed,
                done,
            } => {
                let via = Some(Via::Tui);
                let caller = McpCaller::new(&ctx.writes, &ctx.policy.writes, via, fields);
                let caller = caller.hosted(host);
                // The TUI's session must still be able to write: armed,
                // and not passive or without its ledger.
                if let Some(why) = caller.refusal() {
                    let error = ToolError::refused("tui-refused", why);
                    let outcome =
                        json!({ "outcome": { "result": "refused", "why": error.json() } });
                    desk.record(&caller, "revert", outcome).await;
                    desk.set_origin(was);
                    let _ = done.send(Err(error));
                    return Vec::new();
                }
                let principal = ctx.principal.as_str();
                let chosen = |entry: &Entry| {
                    super::revert::ours(entry, principal) && wanted.contains(&entry.id)
                };
                // Exactly what was confirmed, or nothing (#2185).
                let current: Vec<Entry> = desk.entries().into_iter().filter(chosen).collect();
                let stale = changed(&confirmed, &current, &ctx.policy);
                if let Some(detail) = stale {
                    let fields = json!({ "why": "stale", "detail": detail });
                    desk.record(&caller, "declined", fields).await;
                    desk.set_origin(was);
                    let message =
                        format!("{detail} since the revert was confirmed; nothing was sent");
                    let _ = done.send(Err(ToolError::refused("stale", message)));
                    return Vec::new();
                }
                let revert = desk.revert(engine.client(), engine.limits(), &caller, chosen);
                let undone = charged(account, revert).await;
                ctx.writes.signals.ledger_settled(&before, desk, false);
                let left = desk
                    .entries()
                    .iter()
                    .filter(|e| super::revert::ours(e, principal))
                    .count();
                let shown = undone
                    .iter()
                    .map(|(entry, outcome)| (entry.label(), outcome.clone()))
                    .collect();
                let _ = done.send(Ok((undone, left)));
                shown
            }
        };
        desk.set_origin(was);
        shown
    }
}

/// Why a job never ran: the TUI's session ended first.
const ENDED: &str = "the TUI's session ended before the change went; nothing was sent";

/// A revert of the principal's entries `wanted`, as a job on the TUI's
/// desk, refused as stale unless they still revert as `confirmed`; the
/// work for the call's task.
pub(crate) async fn revert<T: TransportPort + 'static>(
    ctx: Arc<Ctx<T>>,
    account: Arc<Account>,
    (wanted, confirmed): (BTreeSet<u64>, Box<Preview>),
    fields: Map<String, Value>,
) -> Result<Reverted, ToolError> {
    let attached = ctx.attached.as_ref().expect("an attached session");
    let (done, answer) = oneshot::channel();
    let desk = attached.desk.clone();
    let work = Work::Revert {
        wanted,
        confirmed,
        done,
    };
    let job = DeskJob {
        ctx: Arc::clone(&ctx),
        account,
        fields,
        work,
    };
    if desk.send(job).await.is_err() {
        return Err(ToolError::refused("no-approver", ENDED));
    }
    answer
        .await
        .map_err(|_| ToolError::refused("no-approver", ENDED))?
}

/// The preview of an attached write (or of the revert of the entries
/// `reverting`), its tier as MCP gives it, waiting for the TUI: ask the
/// person there, one request at a time, holding no lock and outside the
/// call's budget. Confirmed, the change is handed to a task of its own on
/// a new call with the same account, so the result's cost counts the
/// preview's reads too. Anything else is a refusal, audited, with nothing
/// sent; a cancelled call is an error.
pub(crate) async fn approve<T: TransportPort + 'static>(
    ctx: &Arc<Ctx<T>>,
    (name, raw, peer, cancel): (&str, &Map<String, Value>, &Peer, &CancellationToken),
    call: Call,
    (mut preview, tier, reverting): (Box<Preview>, Tier, Option<BTreeSet<u64>>),
) -> Result<super::Attempt, ErrorData> {
    let attached = ctx.attached.as_ref().expect("an attached session");
    let workspace = ctx.policy.writes.workspace.clone().expect("a workspace");
    preview.gate = Gate::Allowed(tier);
    let approver = TuiApprover {
        approvals: &attached.approvals,
        agent: attached.agent,
        claimed: peer.name.clone(),
        cancel,
        closing: &ctx.closing,
        revert: reverting.is_some(),
    };
    ctx.log_read(name, &call, "awaiting-tui");
    let mut fields = write::fields(&ctx.principal, peer, raw);
    fields.insert("tool".into(), json!(name));
    match approver.approve(&ctx.principal, &preview, tier).await {
        Approval::Approved(via) => {
            let account = Arc::clone(&call.account);
            let call = Call::new(ctx.policy.caps.budget, Arc::clone(&account));
            let sending = write_task::sending();
            let handoff: Handoff = match reverting {
                Some(ids) => {
                    let job = revert(
                        Arc::clone(ctx),
                        Arc::clone(&account),
                        (ids, preview),
                        fields.clone(),
                    );
                    let work = async move {
                        let (done, left) = job.await?;
                        super::revert::body(&done, left)
                    };
                    hand_off(ctx, workspace, account, sending, fields, work)
                }
                None => {
                    let work = send(ctx, &account, preview, fields.clone(), &sending, via);
                    hand_off(ctx, workspace, account, sending, fields, work)
                }
            };
            Ok((call, Ok(Ok(write::Step::Sending(handoff)))))
        }
        Approval::Refused { reason, message } => {
            let error = ToolError::refused(reason, message);
            let mut record = items(&preview);
            record["asked_via"] = json!(Via::Tui.name());
            record["agent"] = json!(attached.agent);
            let event = match (reason, &reverting) {
                ("declined" | "withdrawn", _) => "declined",
                (_, Some(_)) => "revert",
                _ => "write",
            };
            let caller = McpCaller::new(&ctx.writes, &ctx.policy.writes, None, fields);
            audit_refusal(&workspace, &caller, event, &error, record).await;
            if cancel.is_cancelled() && reason == "withdrawn" {
                ctx.log_read(name, &call, "cancelled");
                return Err(ErrorData::internal_error("the call was cancelled", None));
            }
            Ok((call, Ok(Err(error))))
        }
        Approval::Ask { .. } => unreachable!("the TUI answers, never asks"),
    }
}

/// The confirmed write's work: a job on the TUI's desk, and its result.
fn send<T: TransportPort + 'static>(
    ctx: &Arc<Ctx<T>>,
    account: &Arc<Account>,
    preview: Box<Preview>,
    fields: Map<String, Value>,
    sending: &Sending,
    via: Via,
) -> impl Future<Output = Result<Value, ToolError>> + Send + 'static {
    let (ctx, account, sending) = (Arc::clone(ctx), Arc::clone(account), Arc::clone(sending));
    async move {
        let device = preview.device;
        let (done, answer) = oneshot::channel();
        let desk = ctx.attached.as_ref().expect("attached").desk.clone();
        let work = Work::Write {
            preview,
            sending: Arc::clone(&sending),
            done,
        };
        let job = DeskJob {
            ctx: Arc::clone(&ctx),
            account,
            fields,
            work,
        };
        if desk.send(job).await.is_err() {
            return Err(ToolError::refused("no-approver", ENDED));
        }
        match answer.await {
            Ok(Ok(sent)) => write_task::outcome(device, via, &ctx.principal, sent),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(ended(&sending)),
        }
    }
}

/// The answer for a write whose job the TUI dropped mid-way: its
/// `sending` rows say whether it got as far as the ledger.
fn ended(sending: &Sending) -> ToolError {
    let ids = sending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if ids.is_empty() {
        return ToolError::refused("no-approver", ENDED);
    }
    ToolError::Write(json!({
        "error": "pending",
        "applied": "unknown",
        "ledger_ids": ids,
        "ledger": "bacnet://ledger",
        "message": "the TUI's session ended during the write: it may or may not have been sent, \
                    and the ledger keeps its entries (do not retry it)",
    }))
}
