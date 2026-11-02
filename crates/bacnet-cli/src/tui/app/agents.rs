//! Attached agents' writes in the app (#2185; design §9): each one opens
//! the write preview dialog, under an "agent request" banner naming the
//! agent, with the dialog's 300 ms key guard and 120 s lifetime. Requests
//! queue one at a time, oldest first, with no "approve all"; the bar and
//! the banner show how many wait, and one past the cap (`MAX_QUEUED` of
//! `bacnet_mcp::attach`, 4) is refused. So is one while no write session
//! is armed here, or with writes off.
//!
//! Confirming hands the write back to the agent's session, which runs it
//! on the write worker's desk; declining, the lifetime running out, or the
//! dialog closing any other way sends nothing, and the agent's session
//! audits it. A request the agent withdrew (it cancelled the call or
//! disconnected) leaves the queue unshown, or closes its dialog. Without
//! the `mcp` feature there are none, and this is all empty.

#[cfg(feature = "mcp")]
use bacnet_mcp::attach::{AgentApproval, Declined, MAX_QUEUED, Refusal};

use super::App;
#[cfg(feature = "mcp")]
use super::safety::{Modal, modal};
#[cfg(feature = "mcp")]
use super::{FLASH_FOR, Overlay};
#[cfg(feature = "mcp")]
use crate::tui::profile::Writes;
#[cfg(feature = "mcp")]
use crate::tui::safety::Change;

/// How the person answered an agent's dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentAnswer {
    /// Confirmed at its tier.
    Confirmed,
    /// Esc.
    Declined,
    /// The dialog's lifetime ran out.
    TimedOut,
    /// Confirmed, but the write session here had expired.
    NotArmed,
}

/// The agents' write requests: the one on screen, with its dialog's
/// request id, and those waiting behind it, oldest first.
#[derive(Debug, Default)]
pub(crate) struct Agents {
    #[cfg(feature = "mcp")]
    shown: Option<(u64, AgentApproval)>,
    #[cfg(feature = "mcp")]
    waiting: std::collections::VecDeque<AgentApproval>,
}

impl Agents {
    /// Whether dialog `req` shows an agent's request.
    pub(crate) fn shows(&self, req: u64) -> bool {
        #[cfg(feature = "mcp")]
        return self.shown.as_ref().is_some_and(|(id, _)| *id == req);
        #[cfg(not(feature = "mcp"))]
        {
            let _ = req;
            false
        }
    }

    /// The requests held: the one on screen and those waiting.
    pub(crate) fn held(&self) -> usize {
        #[cfg(feature = "mcp")]
        return self.waiting.len() + usize::from(self.shown.is_some());
        #[cfg(not(feature = "mcp"))]
        0
    }

    /// The banner of dialog `req`, when it shows an agent's request: who
    /// asks (its client's name is only a claim), and the queue behind it.
    pub(crate) fn banner(&self, req: u64) -> Option<(String, String)> {
        #[cfg(feature = "mcp")]
        return self
            .shown
            .as_ref()
            .filter(|(id, _)| *id == req)
            .map(|(_, request)| {
                let who = match request.claimed_name() {
                    Some(name) => format!("agent {} (its client says \"{name}\")", request.agent()),
                    None => format!("agent {}", request.agent()),
                };
                let queue = match self.waiting.len() {
                    0 => "none waiting behind it".to_string(),
                    n => format!("{n} more waiting"),
                };
                (
                    format!("AGENT REQUEST from {who}"),
                    format!(
                        "Queue: {queue} (at most {MAX_QUEUED}). Esc declines; unanswered, it is \
                     declined after 120 s."
                    ),
                )
            });
        #[cfg(not(feature = "mcp"))]
        {
            let _ = req;
            None
        }
    }
}

impl App {
    /// Answer dialog `req` `how`, when it shows an agent's request, and
    /// open the next one; false when it is the person's own.
    pub(super) fn agent_answer(&mut self, req: u64, how: AgentAnswer) -> bool {
        #[cfg(feature = "mcp")]
        {
            if !self.safety.agents.shows(req) {
                return false;
            }
            let Some((_, request)) = self.safety.agents.shown.take() else {
                return false;
            };
            let agent = request.agent();
            let text = match how {
                AgentAnswer::Confirmed if request.confirm() => {
                    format!("Agent {agent}'s write confirmed: sending it.")
                }
                AgentAnswer::Confirmed => {
                    format!("Agent {agent} withdrew its request first: nothing was sent.")
                }
                AgentAnswer::Declined => {
                    request.decline(Declined::ByUser);
                    format!("Declined agent {agent}'s write: nothing was sent.")
                }
                AgentAnswer::TimedOut => {
                    request.decline(Declined::TimedOut);
                    format!("Agent {agent}'s request expired unconfirmed: nothing was sent.")
                }
                AgentAnswer::NotArmed => {
                    request.refuse(Refusal::NotArmed);
                    "The write session has expired; nothing was sent.".to_string()
                }
            };
            self.flash(text, FLASH_FOR);
            self.dirty = true;
            self.show_next_agent();
            true
        }
        #[cfg(not(feature = "mcp"))]
        {
            let _ = (req, how);
            false
        }
    }

    /// Once a second: let go of requests their agents withdrew, close an
    /// agent's dialog that is gone or withdrawn, and open the next.
    pub(super) fn agents_tick(&mut self) {
        #[cfg(feature = "mcp")]
        {
            let before = self.safety.agents.held();
            self.safety.agents.waiting.retain(|r| !r.withdrawn());
            let on_screen = |app: &App, id: u64| {
                matches!(&app.overlay, Overlay::Safety(m)
                    if matches!(**m, Modal::Confirm { req, .. } if req == id))
            };
            if let Some((id, request)) = &self.safety.agents.shown {
                let (id, agent) = (*id, request.agent());
                if request.withdrawn() {
                    self.safety.agents.shown = None;
                    if on_screen(self, id) {
                        self.overlay = Overlay::None;
                    }
                    self.flash(
                        format!("Agent {agent} withdrew its request: nothing was sent."),
                        FLASH_FOR,
                    );
                } else if !on_screen(self, id) {
                    // Closed some other way: declined.
                    request.decline(Declined::Closed);
                    self.safety.agents.shown = None;
                }
            }
            self.dirty |= before != self.safety.agents.held();
            self.show_next_agent();
        }
    }

    /// An agent's write `request`: queued, unless no armed session can take
    /// it here or the queue is full.
    #[cfg(feature = "mcp")]
    pub(crate) fn agent_request(&mut self, request: AgentApproval) {
        let refusal = match &self.safety.policy {
            _ if self.safety.passive => Some(Refusal::NotConnected),
            None => Some(Refusal::NotConnected),
            Some(policy) if policy.writes == Writes::Never => Some(Refusal::WritesOff),
            Some(_) if self.safety.armed.is_none() => Some(Refusal::NotArmed),
            Some(_) if self.safety.agents.held() >= MAX_QUEUED => Some(Refusal::QueueFull),
            Some(_) => None,
        };
        let agent = request.agent();
        if let Some(why) = refusal {
            request.refuse(why);
            let text = match why {
                Refusal::QueueFull => format!(
                    "Refused agent {agent}'s write: {MAX_QUEUED} requests are waiting already."
                ),
                Refusal::NotArmed => format!(
                    "Refused agent {agent}'s write: arm a write session (W) to take agents' writes."
                ),
                _ => format!("Refused agent {agent}'s write: writes are off here."),
            };
            return self.flash(text, FLASH_FOR);
        }
        self.safety.agents.waiting.push_back(request);
        self.dirty = true;
        self.show_next_agent();
    }

    /// Open the oldest waiting request, once nothing else is on screen.
    #[cfg(feature = "mcp")]
    fn show_next_agent(&mut self) {
        if self.overlay != Overlay::None || self.safety.agents.shown.is_some() {
            return;
        }
        while let Some(request) = self.safety.agents.waiting.pop_front() {
            if request.withdrawn() {
                continue;
            }
            let req = self.safety_req();
            self.overlay = modal(Modal::Confirm {
                req,
                change: Change::Write(Box::new(request.preview().clone())),
                typed: String::new(),
                error: None,
                opened: self.now,
            });
            self.safety.agents.shown = Some((req, request));
            self.dirty = true;
            return;
        }
    }
}
