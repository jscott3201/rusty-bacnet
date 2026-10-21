//! The SC screen (#967): the hub connector as this client sees it.
//!
//! - The status panel: state, the hub in use (primary or failover, Annex
//!   AB.5.2), its VMAC, the latest reconnect attempt and backoff, and the
//!   heartbeat's age and round trip (Annex AB.6.3).
//! - The identity panel: the local VMAC, the profile's device UUID, the hub
//!   VMAC, and every duplicate-VMAC NAK seen (Annex AB.6.2.2).
//! - The hub's certificate chain, the connection doctor's phase timeline for
//!   the latest failure, and this node's own credentials as the pre-dial
//!   lint read them (#968). Tab switches between the three.
//! - A timeline of the connector's events: connects, losses, reconnect
//!   attempts, failover and the primary's return.
//!
//! State comes from the SC worker's reports; nothing here does I/O.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use bacnet_transport::sc::{ScActiveHub, ScConnectionState};

use super::App;
use crate::tui::sc::credentials::{Credentials, Level};
use crate::tui::sc::doctor::{self, Diagnosis};
use crate::tui::sc::{
    hub_name, vmac_text, CertSummary, Fault, LinkEvent, LinkSnapshot, ScFailure, ScIdentity,
    ScReport,
};

/// Timeline entries kept.
pub(crate) const TIMELINE: usize = 100;
/// Link states kept for the state history.
pub(crate) const TRANSITIONS: usize = 64;
/// Duplicate-VMAC NAKs kept.
pub(crate) const DUPLICATES: usize = 16;

/// What the lower half of the screen shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum View {
    /// The hub's certificate chain.
    #[default]
    Status,
    /// The connection doctor.
    Doctor,
    /// This node's own credentials.
    Credentials,
}

/// A key on the SC screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScKey {
    /// Tab: the next view; Shift-Tab (`back`): the previous one.
    Cycle {
        /// Backwards.
        back: bool,
    },
}

/// How serious a timeline entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Severity {
    /// Progress.
    Info,
    /// A loss or a failed dial.
    Warn,
    /// Reconnection stopped.
    Error,
}

/// One timeline line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    /// When it arrived; for a merged line, when the latest one did.
    pub(crate) at: Instant,
    /// How serious.
    pub(crate) severity: Severity,
    /// What happened.
    pub(crate) text: String,
    /// How many times in a row: failed restore probes with the same cause
    /// share one line, so one per restore interval can't push the loss and
    /// failover lines out of the timeline.
    pub(crate) repeats: u32,
}

/// A link state and the hub it was for, as the app saw it change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Transition {
    /// When.
    pub(crate) at: Instant,
    /// The state.
    pub(crate) state: ScConnectionState,
    /// The hub.
    pub(crate) hub: Option<ScActiveHub>,
}

/// The latest failure, for the doctor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LastFailure {
    /// When it arrived.
    pub(crate) at: Instant,
    /// The hub dialled, when known.
    pub(crate) hub: Option<ScActiveHub>,
    /// The failure.
    pub(crate) failure: ScFailure,
    /// Its reading.
    pub(crate) diagnosis: Diagnosis,
}

/// One duplicate-VMAC NAK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Duplicate {
    /// When.
    pub(crate) at: Instant,
    /// The hub that sent it, when known.
    pub(crate) hub: Option<ScActiveHub>,
    /// The connector drew a new random VMAC.
    pub(crate) reseeded: bool,
}

/// The SC screen's state.
#[derive(Debug, Clone, Default)]
pub(crate) struct ScState {
    /// What the session dials.
    pub(crate) identity: Option<ScIdentity>,
    /// The latest link snapshot and when it arrived.
    pub(crate) link: Option<(LinkSnapshot, Instant)>,
    /// Link states in the order seen, oldest first.
    pub(crate) transitions: VecDeque<Transition>,
    /// The latest reconnect attempt: hub, number and backoff.
    pub(crate) attempt: Option<(ScActiveHub, u32, Duration)>,
    /// The connected hub's certificates, leaf first.
    pub(crate) chain: Vec<CertSummary>,
    /// The latest failure.
    pub(crate) failure: Option<LastFailure>,
    /// Duplicate-VMAC NAKs, oldest first.
    pub(crate) duplicates: VecDeque<Duplicate>,
    /// The event timeline, oldest first.
    pub(crate) timeline: VecDeque<Entry>,
    /// Events the worker lost behind the transport's event stream.
    pub(crate) missed: u64,
    /// The lower view.
    pub(crate) view: View,
    /// This node's credentials from the latest dial's lint, or why the lint
    /// could not read them.
    pub(crate) credentials: Option<Result<Credentials, String>>,
}

/// A timeline line for an event.
pub(crate) fn describe(event: &LinkEvent) -> (Severity, String) {
    match event {
        LinkEvent::Connected { hub, hub_vmac } => (
            Severity::Info,
            format!(
                "connected to the {} hub (hub VMAC {})",
                hub_name(*hub),
                vmac_text(hub_vmac)
            ),
        ),
        LinkEvent::Disconnected { hub, fault } => (
            Severity::Warn,
            format!(
                "lost the {} hub: {}",
                hub_name(*hub),
                doctor::diagnose(fault).summary
            ),
        ),
        LinkEvent::ConnectFailed { hub, fault } => (
            Severity::Warn,
            format!(
                "dial to the {} hub failed: {}",
                hub_name(*hub),
                doctor::diagnose(fault).summary
            ),
        ),
        LinkEvent::Reconnecting {
            hub,
            attempt,
            backoff,
        } => (
            Severity::Info,
            format!(
                "reconnect attempt {attempt} to the {} hub after {} ms",
                hub_name(*hub),
                backoff.as_millis()
            ),
        ),
        LinkEvent::FailoverEngaged => (
            Severity::Warn,
            "primary hub unreachable: switching to the failover hub".into(),
        ),
        LinkEvent::PrimaryRestored => (
            Severity::Info,
            "primary hub is back: leaving the failover hub".into(),
        ),
        LinkEvent::PrimaryRestoreFailed { fault } => (
            Severity::Warn,
            format!(
                "primary hub still unreachable: {}",
                doctor::diagnose(fault).summary
            ),
        ),
        LinkEvent::GaveUp { attempts } => (
            Severity::Error,
            format!("gave up reconnecting after {attempts} attempts"),
        ),
    }
}

impl ScState {
    fn note(&mut self, at: Instant, severity: Severity, text: String) {
        if self.timeline.len() == TIMELINE {
            self.timeline.pop_front();
        }
        self.timeline.push_back(Entry {
            at,
            severity,
            text,
            repeats: 1,
        });
    }

    /// Fold a failed restore probe into the line before it when that line
    /// says the same; true when it did.
    fn repeat(&mut self, at: Instant, text: &str) -> bool {
        match self.timeline.back_mut() {
            Some(last) if last.text == text => {
                last.repeats = last.repeats.saturating_add(1);
                last.at = at;
                true
            }
            _ => false,
        }
    }

    /// Record a state when it differs from the last one recorded.
    fn enter(&mut self, at: Instant, state: ScConnectionState, hub: Option<ScActiveHub>) {
        if self
            .transitions
            .back()
            .is_some_and(|last| (last.state, last.hub) == (state, hub))
        {
            return;
        }
        if self.transitions.len() == TRANSITIONS {
            self.transitions.pop_front();
        }
        self.transitions.push_back(Transition { at, state, hub });
    }

    fn failed(&mut self, at: Instant, hub: Option<ScActiveHub>, failure: ScFailure) {
        if failure.fault.is_duplicate_vmac() {
            if self.duplicates.len() == DUPLICATES {
                self.duplicates.pop_front();
            }
            let reseeded = matches!(failure.fault, Fault::Nak { reseeded: true, .. });
            self.duplicates.push_back(Duplicate { at, hub, reseeded });
        }
        let diagnosis = doctor::diagnose(&failure.fault);
        self.failure = Some(LastFailure {
            at,
            hub,
            failure,
            diagnosis,
        });
    }

    /// Update the snapshot's connection fields from an event, so the panel
    /// is current before the next poll.
    fn patch(&mut self, at: Instant, state: ScConnectionState, hub: Option<ScActiveHub>) {
        let (link, when) = self.link.get_or_insert((
            LinkSnapshot {
                state,
                hub,
                hub_vmac: None,
                heartbeat_age: None,
                heartbeat_rtt: None,
            },
            at,
        ));
        link.state = state;
        link.hub = hub.or(link.hub);
        if state != ScConnectionState::Connected {
            link.hub_vmac = None;
            link.heartbeat_age = None;
        }
        *when = at;
    }

    fn event(&mut self, at: Instant, event: LinkEvent) {
        let (severity, text) = describe(&event);
        let repeated =
            matches!(event, LinkEvent::PrimaryRestoreFailed { .. }) && self.repeat(at, &text);
        if !repeated {
            self.note(at, severity, text);
        }
        match event {
            LinkEvent::Connected { hub, hub_vmac } => {
                self.attempt = None;
                self.enter(at, ScConnectionState::Connected, Some(hub));
                self.patch(at, ScConnectionState::Connected, Some(hub));
                if let Some((link, _)) = &mut self.link {
                    link.hub_vmac = Some(hub_vmac);
                }
            }
            LinkEvent::Disconnected { hub, fault } => {
                self.chain.clear();
                self.enter(at, ScConnectionState::Disconnected, Some(hub));
                self.patch(at, ScConnectionState::Disconnected, Some(hub));
                if fault != Fault::LocalClose {
                    let detail = describe(&LinkEvent::Disconnected {
                        hub,
                        fault: fault.clone(),
                    })
                    .1;
                    let failure = ScFailure {
                        fault,
                        detail,
                        presented: Vec::new(),
                    };
                    self.failed(at, Some(hub), failure);
                }
            }
            LinkEvent::ConnectFailed { hub, fault } => {
                let detail = describe(&LinkEvent::ConnectFailed {
                    hub,
                    fault: fault.clone(),
                })
                .1;
                let failure = ScFailure {
                    fault,
                    detail,
                    presented: Vec::new(),
                };
                self.failed(at, Some(hub), failure);
            }
            LinkEvent::Reconnecting {
                hub,
                attempt,
                backoff,
            } => {
                self.attempt = Some((hub, attempt, backoff));
                self.enter(at, ScConnectionState::Connecting, Some(hub));
                self.patch(at, ScConnectionState::Connecting, Some(hub));
            }
            LinkEvent::FailoverEngaged => {
                self.enter(
                    at,
                    ScConnectionState::Connecting,
                    Some(ScActiveHub::Failover),
                );
                self.patch(
                    at,
                    ScConnectionState::Connecting,
                    Some(ScActiveHub::Failover),
                );
            }
            // The failover link stays up: the timeline line is enough.
            LinkEvent::PrimaryRestored | LinkEvent::PrimaryRestoreFailed { .. } => {}
            LinkEvent::GaveUp { .. } => {
                let hub = self.link.as_ref().and_then(|(link, _)| link.hub);
                self.enter(at, ScConnectionState::Disconnected, hub);
                self.patch(at, ScConnectionState::Disconnected, hub);
            }
        }
    }

    /// Apply a report that arrived at `at`.
    pub(crate) fn apply(&mut self, at: Instant, report: ScReport) {
        match report {
            ScReport::Dialing(identity) => {
                self.identity = Some(identity);
                self.enter(
                    at,
                    ScConnectionState::Connecting,
                    Some(ScActiveHub::Primary),
                );
            }
            ScReport::Failed(failure) => {
                self.note(
                    at,
                    Severity::Error,
                    format!(
                        "could not connect: {}",
                        doctor::diagnose(&failure.fault).summary
                    ),
                );
                self.enter(at, ScConnectionState::Disconnected, None);
                self.failed(at, None, *failure);
                // A failed build is what the doctor is for, unless the
                // credential lint refused it: that view says why.
                self.view = if self.lint_refused() {
                    View::Credentials
                } else {
                    View::Doctor
                };
            }
            ScReport::Credentials(credentials) => self.credentials = Some(*credentials),
            ScReport::Status(snapshot) => {
                self.enter(at, snapshot.state, snapshot.hub);
                self.link = Some((snapshot, at));
            }
            ScReport::Event(event) => self.event(at, event),
            ScReport::Gap { missed } => {
                self.missed += missed;
                self.note(
                    at,
                    Severity::Warn,
                    format!("... {missed} events missed; status resynced"),
                );
            }
            ScReport::Chain(chain) => self.chain = chain,
        }
    }

    /// The latest lint found a hard failure or could not read the files.
    pub(crate) fn lint_refused(&self) -> bool {
        match &self.credentials {
            Some(Ok(c)) => c.findings.iter().any(|(level, _)| *level == Level::Failed),
            Some(Err(_)) => true,
            None => false,
        }
    }

    /// The heartbeat age at `now`: the polled age plus the time since.
    pub(crate) fn heartbeat_age(&self, now: Instant) -> Option<Duration> {
        let (link, at) = self.link.as_ref()?;
        Some(link.heartbeat_age? + now.saturating_duration_since(*at))
    }
}

impl App {
    /// An SC worker report.
    pub(super) fn sc_report(&mut self, report: ScReport) {
        let now = self.now;
        self.sc.apply(now, report);
    }

    /// A key on the SC screen.
    pub(super) fn sc_key(&mut self, key: ScKey) {
        match key {
            ScKey::Cycle { back } => {
                const ORDER: [View; 3] = [View::Status, View::Doctor, View::Credentials];
                let at = ORDER.iter().position(|v| *v == self.sc.view).unwrap_or(0);
                let step = if back { ORDER.len() - 1 } else { 1 };
                self.sc.view = ORDER[(at + step) % ORDER.len()];
            }
        }
    }
}
