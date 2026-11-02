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
//! audited here, with nothing sent. A revert needs no confirmation, as on
//! stdio; it is a job too.

use std::collections::BTreeSet;
use std::sync::Arc;

use bacnet_ops::limits::{Account, charged};
use bacnet_ops::safety::desk::{Caller, Desk, Sent};
use bacnet_ops::safety::ledger::{Entry, Origin};
use bacnet_ops::safety::{Gate, Outcome, Preview, Tier};
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
        done: oneshot::Sender<Vec<Sent>>,
    },
    Revert {
        wanted: Option<BTreeSet<u64>>,
        done: oneshot::Sender<(Vec<(Entry, Outcome)>, usize)>,
    },
}

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
        let Ok(engine) = ctx.engine() else {
            return Vec::new();
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
                let _ = done.send(sent);
                shown
            }
            Work::Revert { wanted, done } => {
                let caller = McpCaller::new(&ctx.writes, &ctx.policy.writes, None, fields);
                let caller = caller.hosted(host);
                let principal = ctx.principal.as_str();
                let chosen = |entry: &Entry| {
                    super::revert::ours(entry, principal)
                        && wanted.as_ref().is_none_or(|ids| ids.contains(&entry.id))
                };
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
                let _ = done.send((undone, left));
                shown
            }
        };
        desk.set_origin(was);
        shown
    }
}

/// Why a job never ran: the TUI's session ended first.
const ENDED: &str = "the TUI's session ended before the change went; nothing was sent";

/// A revert of `wanted` (all of this principal's entries when `None`), as
/// a job on the TUI's desk; the work for the call's task.
pub(crate) async fn revert<T: TransportPort + 'static>(
    ctx: Arc<Ctx<T>>,
    account: Arc<Account>,
    wanted: Option<BTreeSet<u64>>,
    fields: Map<String, Value>,
) -> Result<(Vec<(Entry, Outcome)>, usize), ToolError> {
    let attached = ctx.attached.as_ref().expect("an attached session");
    let (done, answer) = oneshot::channel();
    let desk = attached.desk.clone();
    let work = Work::Revert { wanted, done };
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
        .map_err(|_| ToolError::refused("no-approver", ENDED))
}

/// The preview of an attached write, its tier as MCP gives it, waiting
/// for the TUI: ask the person there, one request at a time, holding no
/// lock and outside the call's budget. Confirmed, the write is handed to a
/// task of its own on a new call with the same account, so the result's
/// cost counts the preview's reads too. Anything else is a refusal,
/// audited, with nothing sent; a cancelled call is an error.
pub(crate) async fn approve<T: TransportPort + 'static>(
    ctx: &Arc<Ctx<T>>,
    (name, raw, peer, cancel): (&str, &Map<String, Value>, &Peer, &CancellationToken),
    call: Call,
    mut preview: Box<Preview>,
    tier: Tier,
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
    };
    ctx.log_read(name, &call, "awaiting-tui");
    let fields = write::fields(&ctx.principal, peer, raw);
    match approver.approve(&ctx.principal, &preview, tier).await {
        Approval::Approved(via) => {
            let account = Arc::clone(&call.account);
            let call = Call::new(ctx.policy.caps.budget, Arc::clone(&account));
            let sending = write_task::sending();
            let work = send(ctx, &account, preview, fields.clone(), &sending, via);
            let handoff: Handoff = hand_off(ctx, workspace, account, sending, fields, work);
            Ok((call, Ok(Ok(write::Step::Sending(handoff)))))
        }
        Approval::Refused { reason, message } => {
            let error = ToolError::refused(reason, message);
            let mut record = items(&preview);
            record["asked_via"] = json!(Via::Tui.name());
            record["agent"] = json!(attached.agent);
            let event = if reason == "declined" || reason == "withdrawn" {
                "declined"
            } else {
                "write"
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
            Ok(sent) => write_task::outcome(device, via, &ctx.principal, sent),
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
