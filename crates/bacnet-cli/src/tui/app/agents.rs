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
//! disconnected) leaves the queue unshown, or closes its dialog. Disarming
//! here refuses every request held at once, `not-armed`, rather than at
//! confirm time. Without the `mcp` feature there are none, and this is all
//! empty. The agent pane (screen 8) lists them with their ages, beside the
//! agents' calls (`agent_pane`).

#[cfg(feature = "mcp")]
use std::time::{Duration, Instant};

#[cfg(feature = "mcp")]
use bacnet_mcp::attach::{AgentApproval, Declined, Gone, MAX_QUEUED, Refusal};

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
/// request id, and those waiting behind it, oldest first, each with when
/// it came; and the agent pane.
#[derive(Debug, Default)]
pub(crate) struct Agents {
    #[cfg(feature = "mcp")]
    shown: Option<(u64, AgentApproval, Instant)>,
    #[cfg(feature = "mcp")]
    waiting: std::collections::VecDeque<(Instant, AgentApproval)>,
    /// The agent pane's state (#2185).
    pub(crate) pane: super::agent_pane::AgentPane,
    /// Requests a frame test lists after the real ones.
    #[cfg(all(test, feature = "mcp"))]
    pub(crate) staged: Vec<Held>,
}

/// A request held here, as the agent pane lists it.
#[cfg(feature = "mcp")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Held {
    /// The agent's connection number.
    pub(crate) agent: u64,
    /// The name its client claims, without quotes of its own.
    pub(crate) client: Option<String>,
    /// `write` or `revert`.
    pub(crate) what: &'static str,
    /// The device it would write.
    pub(crate) device: u32,
    /// How long it has waited here.
    pub(crate) age: Duration,
    /// On screen: how long until its dialog declines it.
    pub(crate) left: Option<Duration>,
}

impl Agents {
    /// Whether dialog `req` shows an agent's request.
    pub(crate) fn shows(&self, req: u64) -> bool {
        #[cfg(feature = "mcp")]
        return self.shown.as_ref().is_some_and(|(id, ..)| *id == req);
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
            .filter(|(id, ..)| *id == req)
            .map(|(_, request, _)| {
                let claimed = claimed(request);
                let who = match claimed {
                    Some(name) => format!("agent {} (its client says \"{name}\")", request.agent()),
                    None => format!("agent {}", request.agent()),
                };
                let queue = match self.waiting.len() {
                    0 => "none waiting behind it".to_string(),
                    n => format!("{n} more waiting"),
                };
                (
                    match request.reverts() {
                        true => format!("AGENT REQUEST to revert, from {who}"),
                        false => format!("AGENT REQUEST from {who}"),
                    },
                    format!(
                        "Queue: {queue} (at most {MAX_QUEUED}). Esc declines; unanswered, it is \
                         declined after {} s.",
                        super::safety::PREVIEW_LIFETIME.as_secs()
                    ),
                )
            });
        #[cfg(not(feature = "mcp"))]
        {
            let _ = req;
            None
        }
    }

    /// The requests held at `now`, the one on screen first (`opened` then),
    /// then those waiting, oldest first.
    #[cfg(feature = "mcp")]
    pub(crate) fn held_at(&self, now: Instant, opened: Option<Instant>) -> Vec<Held> {
        {
            let held = |request: &AgentApproval, came: Instant, left: Option<Duration>| Held {
                agent: request.agent(),
                client: claimed(request),
                what: if request.reverts() { "revert" } else { "write" },
                device: request.preview().device,
                age: now.saturating_duration_since(came),
                left,
            };
            let lifetime = super::safety::PREVIEW_LIFETIME;
            let shown = self.shown.iter().map(|(_, request, came)| {
                let left =
                    opened.map(|at| lifetime.saturating_sub(now.saturating_duration_since(at)));
                held(request, *came, left)
            });
            let waiting = self.waiting.iter().map(|(came, r)| held(r, *came, None));
            let held = shown.chain(waiting);
            #[cfg(test)]
            let held = held.chain(self.staged.iter().cloned());
            held.collect()
        }
    }
}

/// The name `request`'s client claims, without quotes of its own.
#[cfg(feature = "mcp")]
fn claimed(request: &AgentApproval) -> Option<String> {
    request.claimed_name().map(|n| n.replace(['"', '\\'], ""))
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
            let Some((_, request, _)) = self.safety.agents.shown.take() else {
                return false;
            };
            let agent = request.agent();
            let what = if request.reverts() { "revert" } else { "write" };
            let text = match how {
                AgentAnswer::Confirmed if request.confirm() => {
                    format!("Agent {agent}'s {what} confirmed: sending it.")
                }
                AgentAnswer::Confirmed => {
                    format!("Agent {agent} withdrew its request first: nothing was sent.")
                }
                AgentAnswer::Declined => {
                    request.decline(Declined::ByUser);
                    format!("Declined agent {agent}'s {what}: nothing was sent.")
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
        self.agent_pane_tick();
        #[cfg(feature = "mcp")]
        {
            let before = self.safety.agents.held();
            self.safety.agents.waiting.retain(|(_, r)| !r.withdrawn());
            let on_screen = |app: &App, id: u64| {
                matches!(&app.overlay, Overlay::Safety(m)
                    if matches!(**m, Modal::Confirm { req, .. } if req == id))
            };
            if let Some((id, request, _)) = &self.safety.agents.shown {
                let (id, agent) = (*id, request.agent());
                if request.withdrawn() {
                    let why = match request.gone() {
                        Some(Gone::GaveUp) => "stopped waiting (its call ran out of time)",
                        Some(Gone::Left) => "disconnected",
                        _ => "withdrew its request",
                    };
                    self.safety.agents.shown = None;
                    if on_screen(self, id) {
                        self.overlay = Overlay::None;
                    }
                    self.flash(format!("Agent {agent} {why}: nothing was sent."), FLASH_FOR);
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

    /// Follow the write session's arming (after every action): disarmed,
    /// every request held is refused at once, `not-armed`, and its dialog
    /// closes, rather than each being refused when confirmed (#2185).
    pub(super) fn agents_follow_arm(&mut self) {
        let armed = self.safety.armed.is_some();
        let disarmed = self.safety.agents.pane.armed_now(armed) && !armed;
        #[cfg(not(feature = "mcp"))]
        let _ = disarmed;
        #[cfg(feature = "mcp")]
        if disarmed {
            let mut refused = 0;
            if let Some((req, request, _)) = self.safety.agents.shown.take() {
                if matches!(&self.overlay, Overlay::Safety(m)
                    if matches!(**m, Modal::Confirm { req: shown, .. } if shown == req))
                {
                    self.overlay = Overlay::None;
                }
                refused += usize::from(!request.withdrawn());
                request.refuse(Refusal::NotArmed);
            }
            for (_, request) in self.safety.agents.waiting.drain(..) {
                refused += usize::from(!request.withdrawn());
                request.refuse(Refusal::NotArmed);
            }
            if refused > 0 {
                let text = format!(
                    "Disarmed: read-only. Refused {refused} agent request(s) waiting here \
                     (not-armed); nothing was sent."
                );
                self.flash(text, FLASH_FOR);
            }
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
        let what = if request.reverts() { "revert" } else { "write" };
        if let Some(why) = refusal {
            request.refuse(why);
            let text = match why {
                Refusal::QueueFull => format!(
                    "Refused agent {agent}'s {what}: {MAX_QUEUED} requests are waiting already."
                ),
                Refusal::NotArmed => format!(
                    "Refused agent {agent}'s {what}: arm a write session (W) to take agents' writes."
                ),
                Refusal::WritesOff => {
                    format!("Refused agent {agent}'s {what}: writes are off here.")
                }
                Refusal::NotConnected => format!(
                    "Refused agent {agent}'s {what}: there is no connected session to write through."
                ),
            };
            return self.flash(text, FLASH_FOR);
        }
        self.safety.agents.waiting.push_back((self.now, request));
        self.dirty = true;
        self.show_next_agent();
    }

    /// Open the oldest waiting request, once nothing else is on screen.
    #[cfg(feature = "mcp")]
    fn show_next_agent(&mut self) {
        // Not while text is typed into a filter: the dialog would take its
        // keys.
        if self.overlay != Overlay::None
            || self.filter_editing
            || self.safety.agents.shown.is_some()
        {
            return;
        }
        while let Some((came, request)) = self.safety.agents.waiting.pop_front() {
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
            self.safety.agents.shown = Some((req, request, came));
            self.dirty = true;
            return;
        }
    }
}
