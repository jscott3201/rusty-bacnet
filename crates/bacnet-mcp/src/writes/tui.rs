//! The TUI as an attached agent's approver (#2185; design §7.3, §9): the
//! person at the TUI confirms each of the agent's writes, at every tier,
//! in the TUI's own preview dialog.
//!
//! An agent's session runs on the listener's runtime; the dialog runs on
//! the TUI's loop. [`TuiApprover`] hands the preview across in an
//! [`AgentApproval`] over a Tokio channel and waits on a oneshot for the
//! answer. Nothing is locked while it waits, and nothing it waits on holds
//! a lock: the TUI answers through the oneshot and drops it.
//!
//! **Cancellation.** Every way the wait can end resolves the call, and
//! none sends anything except a confirmation:
//!
//! - The TUI confirms ([`AgentApproval::confirm`]), declines, or times the
//!   dialog out after [`LIFETIME`] ([`AgentApproval::decline`]), or
//!   refuses the request outright, for a full queue say
//!   ([`AgentApproval::refuse`]).
//! - The TUI quits, its session ends or it panics: every clone of the
//!   request is dropped with it, the oneshot's sender goes, and the wait
//!   wakes `declined`.
//! - The agent cancels the call (`withdrawn`), or its session ends
//!   (`declined`: it disconnected, or the TUI's listener shuts down): the
//!   wait stops and drops its end of the oneshot. The TUI sees the request withdrawn
//!   ([`AgentApproval::withdrawn`]) and takes it off its queue unshown,
//!   or closes its dialog.
//! - Past [`WAIT_AT_MOST`], which covers a full queue answered one by one
//!   at the dialog's lifetime, the wait gives up, so no request hangs even
//!   when the TUI never shows it (another dialog kept open, say).
//!
//! A confirmation that races the end of the wait isn't lost: the wait
//! closes its end, then takes an answer already sent. The TUI's
//! [`confirm`](AgentApproval::confirm) only reports success when the
//! answer was delivered, and the TUI sends nothing otherwise; so the agent
//! never hears `declined` for a write that goes, nor the reverse.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bacnet_tool::safety::{Preview, Tier};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{Approval, Via, WaitingApprover, refused};

/// How many agent requests the TUI holds at once, the one on screen
/// included; one more is refused (`approval-queue-full`).
pub const MAX_QUEUED: usize = 4;

/// How long the TUI's dialog waits for an answer before it declines the
/// request: the TUI's own preview lifetime.
pub const LIFETIME: Duration = bacnet_tool::safety::PREVIEW_LIFETIME;

/// The longest an agent's call waits for the TUI: a full queue, each
/// request answered at the last moment, and a little more.
pub const WAIT_AT_MOST: Duration = Duration::from_secs(LIFETIME.as_secs() * MAX_QUEUED as u64 + 10);

/// Why the TUI declined a request it showed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Declined {
    /// The person declined it.
    ByUser,
    /// Nobody answered within [`LIFETIME`].
    TimedOut,
    /// The dialog was closed some other way.
    Closed,
}

/// Why the TUI refused a request without showing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// [`MAX_QUEUED`] requests are waiting already.
    QueueFull,
    /// The TUI's write session isn't armed.
    NotArmed,
    /// The TUI's profile sets `writes = "never"`.
    WritesOff,
    /// The TUI isn't connected, so it has no write session.
    NotConnected,
}

impl Refusal {
    fn reason(self) -> &'static str {
        match self {
            Self::QueueFull => "approval-queue-full",
            Self::NotArmed => "not-armed",
            Self::WritesOff => "write-not-granted",
            Self::NotConnected => "no-approver",
        }
    }

    fn message(self) -> String {
        match self {
            Self::QueueFull => format!(
                "the TUI already holds {MAX_QUEUED} agent requests; wait for them to be answered \
                 and try again; nothing was sent"
            ),
            Self::NotArmed => "the TUI's write session isn't armed: the person at the TUI \
                               arms one (W) before agents' writes can be confirmed; nothing \
                               was sent"
                .into(),
            Self::WritesOff => "the TUI's profile sets writes = \"never\"; nothing was sent".into(),
            Self::NotConnected => {
                "the TUI has no connected session to write through; nothing was sent".into()
            }
        }
    }
}

/// The TUI's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Confirmed,
    Declined(Declined),
    Refused(Refusal),
}

/// An attached agent's write, waiting for the person at the TUI. Clones
/// share one answer: the first given counts. Dropped with no answer (the
/// TUI quit or panicked), it is declined.
#[derive(Clone)]
pub struct AgentApproval {
    inner: Arc<Inner>,
}

struct Inner {
    agent: u64,
    claimed: Option<String>,
    preview: Preview,
    revert: bool,
    answer: Mutex<Option<oneshot::Sender<Decision>>>,
    /// Why the agent's side stopped waiting, once it did ([`Gone`]).
    gone: AtomicU8,
}

/// Why an agent's call stopped waiting for the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gone {
    /// The agent cancelled the call.
    Cancelled,
    /// The agent's session ended: it disconnected, or the listener is
    /// shutting down.
    Left,
    /// The call waited [`WAIT_AT_MOST`] and gave up.
    GaveUp,
}

impl std::fmt::Debug for AgentApproval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentApproval")
            .field("agent", &self.inner.agent)
            .field("device", &self.inner.preview.device)
            .finish_non_exhaustive()
    }
}

impl PartialEq for AgentApproval {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl AgentApproval {
    /// The agent's connection number on the listener, as its audit records
    /// name it.
    pub fn agent(&self) -> u64 {
        self.inner.agent
    }

    /// The name the agent's client claims, cleaned and capped: only ever a
    /// claim.
    pub fn claimed_name(&self) -> Option<&str> {
        self.inner.claimed.as_deref()
    }

    /// The change, as previewed for the agent; its gate is the tier over
    /// MCP, which may be stricter than the TUI's own. For a revert, the
    /// writes that put the pre-images back.
    pub fn preview(&self) -> &Preview {
        &self.inner.preview
    }

    /// Whether it reverts the agent's earlier writes rather than writing.
    pub fn reverts(&self) -> bool {
        self.inner.revert
    }

    /// Why the agent stopped waiting, when it said so; `None` while it
    /// waits, or when its call simply went away.
    pub fn gone(&self) -> Option<Gone> {
        match self.inner.gone.load(Ordering::SeqCst) {
            1 => Some(Gone::Cancelled),
            2 => Some(Gone::Left),
            3 => Some(Gone::GaveUp),
            _ => None,
        }
    }

    /// True once nobody waits for an answer: the agent cancelled the
    /// call, disconnected or gave up waiting, or an answer was given.
    pub fn withdrawn(&self) -> bool {
        self.inner
            .answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_none_or(oneshot::Sender::is_closed)
    }

    /// Confirm: true when the agent heard it and will send the write; false
    /// when it had withdrawn the request first, and nothing will be sent.
    pub fn confirm(&self) -> bool {
        self.answer(Decision::Confirmed)
    }

    /// Decline, `why`; nothing is sent.
    pub fn decline(&self, why: Declined) {
        self.answer(Decision::Declined(why));
    }

    /// Refuse without showing it, `why`; nothing is sent.
    pub fn refuse(&self, why: Refusal) {
        self.answer(Decision::Refused(why));
    }

    fn answer(&self, decision: Decision) -> bool {
        let sender = self
            .inner
            .answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        sender.is_some_and(|sender| sender.send(decision).is_ok())
    }
}

/// The TUI as approver for one call of an attached agent's: requests go
/// to the TUI on `approvals`, named for `agent` (and the name its client
/// `claimed`); the call's `cancel` and the session's `closing` withdraw
/// it.
pub(crate) struct TuiApprover<'a> {
    pub(crate) approvals: &'a mpsc::Sender<AgentApproval>,
    pub(crate) agent: u64,
    pub(crate) claimed: Option<String>,
    pub(crate) cancel: &'a CancellationToken,
    pub(crate) closing: &'a CancellationToken,
    /// It asks to revert, not to write.
    pub(crate) revert: bool,
}

/// What ended the wait other than an answer.
enum Stopped {
    Gone,
    Cancelled,
    Closing,
    TooLong,
}

impl TuiApprover<'_> {
    /// Wait for `until`, the call's cancel, the session's end or
    /// `deadline`, whichever comes first.
    async fn bounded<F: Future>(&self, until: F, deadline: Instant) -> Result<F::Output, Stopped> {
        tokio::select! {
            biased;
            done = until => Ok(done),
            // Before the call's cancel: a shutdown cancels both.
            () = self.closing.cancelled() => Err(Stopped::Closing),
            () = self.cancel.cancelled() => Err(Stopped::Cancelled),
            () = tokio::time::sleep_until(deadline) => Err(Stopped::TooLong),
        }
    }

    async fn wait(&self, preview: &Preview) -> Result<Decision, Stopped> {
        let deadline = Instant::now() + WAIT_AT_MOST;
        let (answer, mut decided) = oneshot::channel();
        let inner = Arc::new(Inner {
            agent: self.agent,
            claimed: self.claimed.clone(),
            preview: preview.clone(),
            revert: self.revert,
            answer: Mutex::new(Some(answer)),
            gone: AtomicU8::new(0),
        });
        let request = AgentApproval {
            inner: Arc::clone(&inner),
        };
        // A request still in the channel when the wait stops is dropped
        // with it, or found withdrawn by the TUI.
        if self
            .bounded(self.approvals.send(request), deadline)
            .await?
            .is_err()
        {
            return Err(Stopped::Gone);
        }
        let stopped = match self.bounded(&mut decided, deadline).await {
            Ok(Ok(decision)) => return Ok(decision),
            Ok(Err(_)) => Stopped::Gone,
            Err(stopped) => stopped,
        };
        let why = match &stopped {
            Stopped::Cancelled => 1,
            Stopped::Closing => 2,
            Stopped::TooLong => 3,
            Stopped::Gone => 0,
        };
        inner.gone.store(why, Ordering::SeqCst);
        // Closed first, so no answer can come after this look.
        decided.close();
        decided.try_recv().map_err(|_| stopped)
    }
}

impl WaitingApprover for TuiApprover<'_> {
    fn approve<'a>(
        &'a self,
        _principal: &'a str,
        preview: &'a Preview,
        _tier: Tier,
    ) -> std::pin::Pin<Box<dyn Future<Output = Approval> + Send + 'a>> {
        Box::pin(async move {
            match self.wait(preview).await {
                Ok(Decision::Confirmed) => Approval::Approved(Via::Tui),
                Ok(Decision::Declined(why)) => refused(
                    "declined",
                    match why {
                        Declined::ByUser => "the person at the TUI declined it".to_string(),
                        Declined::TimedOut => {
                            format!("nobody answered in the TUI within {} s", LIFETIME.as_secs())
                        }
                        Declined::Closed => "the TUI closed the request unanswered".to_string(),
                    },
                ),
                Ok(Decision::Refused(why)) => refused(why.reason(), why.message()),
                Err(Stopped::Gone) => refused("declined", "the TUI closed before an answer"),
                Err(Stopped::Cancelled) => refused("withdrawn", "the tool call was cancelled"),
                Err(Stopped::Closing) => refused(
                    "declined",
                    "the session ended before an answer: the TUI is closing, or the agent left",
                ),
                Err(Stopped::TooLong) => refused(
                    "declined",
                    format!("the TUI didn't answer within {} s", WAIT_AT_MOST.as_secs()),
                ),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use bacnet_tool::safety::Gate;

    use super::*;

    fn preview() -> Preview {
        Preview {
            device: 100,
            items: Vec::new(),
            batch: false,
            gate: Gate::Allowed(Tier::T2),
        }
    }

    /// Ask through a fresh approver while `tui` plays the TUI's part with
    /// the request it gets.
    async fn ask(tui: impl AsyncFnOnce(AgentApproval)) -> Approval {
        let (approvals, mut queue) = mpsc::channel(1);
        let (cancel, closing) = (CancellationToken::new(), CancellationToken::new());
        let approver = TuiApprover {
            approvals: &approvals,
            agent: 3,
            claimed: Some("agent".into()),
            cancel: &cancel,
            closing: &closing,
            revert: false,
        };
        let preview = preview();
        let asked = approver.approve("tester", &preview, Tier::T2);
        let answered = async move { tui(queue.recv().await.unwrap()).await };
        tokio::join!(asked, answered).0
    }

    /// Confirmed, the call goes on as the TUI's approval.
    #[tokio::test(start_paused = true)]
    async fn a_confirmation_approves() {
        let answer = ask(async |request| assert!(request.confirm())).await;
        assert_eq!(answer, Approval::Approved(Via::Tui));
    }

    /// The TUI quitting or panicking drops the request unanswered: declined.
    #[tokio::test(start_paused = true)]
    async fn a_request_dropped_unanswered_is_declined() {
        let answer = ask(async |request| drop(request)).await;
        assert!(
            matches!(
                answer,
                Approval::Refused {
                    reason: "declined",
                    ..
                }
            ),
            "{answer:?}"
        );
    }

    /// No answer at all ends the wait past its bound, declined, so no call
    /// hangs on a TUI that never shows the request; a confirmation after
    /// that is refused, so nothing is sent for it.
    #[tokio::test(start_paused = true)]
    async fn an_unanswered_request_gives_up_and_a_late_confirm_fails() {
        let (keep, kept) = oneshot::channel();
        let answer = ask(async |request| {
            let _ = keep.send(request);
        })
        .await;
        assert!(
            matches!(&answer, Approval::Refused { reason: "declined", message } if message.contains("didn't answer")),
            "{answer:?}"
        );
        let request = kept.await.unwrap();
        assert!(request.withdrawn());
        assert!(!request.confirm(), "nobody waits for it any more");
    }
}
