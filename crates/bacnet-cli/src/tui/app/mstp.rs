//! The MS/TP screen's state and the join confirmation (#969). No I/O.
//!
//! Joining a trunk puts a new master in the token ring, which every other
//! station on it notices, so the TUI asks first: a T2 confirmation (a
//! preview, then `y`), as for a change the safety model rates the same
//! (#964). One-shot commands take `--mstp` itself as the request.

use std::time::Duration;

use bacnet_transport::mstp::{HOST_LATE_DEADLINE, LatenessHistogram, MstpDiagnosticsSnapshot};

use super::{App, FLASH_FOR, Link, Overlay};
use crate::mstp::MstpArgs;
use crate::tui::message::Command;
use crate::tui::mstp::{MstpReport, Sample};
use crate::tui::profile::service::Request;

/// A key on the MS/TP screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MstpKey {
    /// Count from now: later readings show what changed since.
    Clear,
    /// Stop or resume following new readings.
    ToggleWatch,
    /// Tab: the monitor's next view (BackTab: the one before).
    Cycle {
        /// Backwards.
        back: bool,
    },
    /// `J`: start the monitor's join wizard.
    Join,
    /// `+` or `-`: the next or previous free MAC the wizard found.
    Pick {
        /// The previous one.
        back: bool,
    },
    /// Enter: take the wizard's proposal to the join confirmation.
    Accept,
    /// Esc: close the wizard, or leave the screen.
    Back,
}

/// The MS/TP screen: the station joined as, the latest reading, and the
/// baseline that clear set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MstpState {
    /// The settings the session joined with.
    pub(crate) station: Option<MstpArgs>,
    /// How the station's loop runs, as the transport read it back.
    pub(crate) thread: Option<String>,
    /// The reading on screen.
    pub(crate) shown: Option<Sample>,
    /// The newest reading, kept while the screen is paused.
    pub(crate) latest: Option<Sample>,
    /// The counters at the last clear.
    pub(crate) baseline: Option<MstpDiagnosticsSnapshot>,
    /// New readings are not shown until watching resumes.
    pub(crate) paused: bool,
}

impl MstpState {
    /// Take a worker report.
    pub(crate) fn report(&mut self, report: MstpReport) {
        match report {
            MstpReport::Joined(station, thread) => {
                *self = Self {
                    station: Some(station),
                    thread: Some(thread),
                    ..Self::default()
                };
            }
            MstpReport::Sample(sample) => {
                self.latest = Some(*sample);
                if !self.paused {
                    self.shown = Some(*sample);
                }
            }
        }
    }

    /// Apply a screen key.
    pub(crate) fn key(&mut self, key: MstpKey) {
        match key {
            MstpKey::Clear => self.baseline = self.latest.map(|sample| sample.counters),
            MstpKey::ToggleWatch => {
                self.paused = !self.paused;
                if !self.paused {
                    self.shown = self.latest;
                }
            }
            // The monitor's keys; `App::mstp_key` handles Back.
            _ => {}
        }
    }

    /// The counters shown: since the last clear, or since the start. A
    /// counter that saturated shows its total, which is all there is.
    pub(crate) fn counters(&self) -> Option<Vec<(String, u64)>> {
        let now = self.shown?.counters;
        let base = self.baseline.unwrap_or_default();
        Some(
            rows(&now)
                .into_iter()
                .zip(rows(&base))
                .map(|((label, value), (_, before))| {
                    (label.to_string(), value.saturating_sub(before))
                })
                .chain([(late_label(), late_timers(&now, &base))])
                .collect(),
        )
    }

    /// How late the station's timers fired, over the same span as
    /// [`Self::counters`].
    pub(crate) fn lateness(&self) -> Option<Lateness> {
        let now = self.shown?.counters.mac.deadline_lateness;
        let mut buckets = now.buckets;
        let all_time: Duration = now.max.into();
        let base = self.baseline.map(|base| base.mac.deadline_lateness);
        if let Some(base) = base {
            for (count, before) in buckets.iter_mut().zip(base.buckets) {
                *count = count.saturating_sub(before);
            }
        }
        // The highest band with a timed wake in the span shown.
        let max = match (buckets.iter().rposition(|&count| count > 0), base) {
            (None, _) => None,
            (Some(_), None) => Some(LatenessMax::Exact(all_time)),
            // The transport keeps only the largest ever: one larger than at
            // the clear came since.
            (Some(_), Some(base)) if now.max > base.max => Some(LatenessMax::Exact(all_time)),
            // Otherwise the largest ever bounds those since, and so does the
            // top of the highest band that moved, which may be tighter.
            (Some(top), Some(_)) => Some(match LatenessHistogram::EDGES.get(top) {
                Some(&edge) if edge <= all_time => LatenessMax::Below(edge),
                _ => LatenessMax::AtMost(all_time),
            }),
        };
        Some(Lateness { buckets, max })
    }
}

/// The timer lateness the screen shows: the transport's histogram since the
/// last clear, or since joining.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Lateness {
    /// Timed wakes in each of the core's lateness buckets: under 1 ms, 1 to
    /// 2, 2 to 5, 5 to 10, 10 to 25, and 25 ms or more.
    pub(crate) buckets: [u64; 6],
    /// The largest lateness, `None` with no timed wake to show.
    pub(crate) max: Option<LatenessMax>,
}

/// The largest timer lateness over the span shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LatenessMax {
    /// The largest, exactly.
    Exact(Duration),
    /// No more than this: the largest since joining, which came before the
    /// clear.
    AtMost(Duration),
    /// Less than this: the upper edge of the highest band that moved since
    /// the clear, when that is below the largest since joining.
    Below(Duration),
}

/// The bucket labels, in [`Lateness::buckets`]' order.
pub(crate) const LATENESS_BANDS: [&str; 6] =
    ["<1 ms", "1-2 ms", "2-5 ms", "5-10 ms", "10-25 ms", "≥25 ms"];

/// The late-timer row's label, from the threshold the transport counts at.
pub(crate) fn late_label() -> String {
    format!("Late timers (≥{} ms)", HOST_LATE_DEADLINE.as_millis())
}

fn late_timers(now: &MstpDiagnosticsSnapshot, base: &MstpDiagnosticsSnapshot) -> u64 {
    now.mac
        .late_deadlines
        .saturating_sub(base.mac.late_deadlines)
}

/// The counters the screen lists, in its order, the late-timer row last.
fn rows(c: &MstpDiagnosticsSnapshot) -> [(&'static str, u64); 15] {
    let m = &c.mac;
    [
        ("Frames received", m.frames_received),
        ("Frames sent", m.frames_transmitted),
        ("NPDUs delivered", m.npdus_delivered),
        ("Tokens passed", m.token_rotations),
        ("Token retries", m.token_retries),
        ("Tokens regenerated", m.token_regenerations),
        ("Invalid frames", m.parser_errors),
        ("Frame aborts", m.frame_aborts),
        ("Reply timeouts", c.wait_for_reply_timeouts),
        ("Replies postponed", c.reply_postponed_tx),
        ("Late replies", m.late_replies),
        ("Own-MAC frames", m.own_address_frames),
        ("TX queue full", c.outbound_queue_full),
        ("Serial read errors", c.serial_read_errors),
        ("Serial write errors", c.serial_write_errors),
    ]
}

/// A key in the join confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JoinKey {
    /// Previous port.
    Up,
    /// Next port.
    Down,
    /// `y`: join.
    Confirm,
}

/// Where a join confirmation came from, which decides what `y` sends and
/// what Esc goes back to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JoinOrigin {
    /// `--mstp` flags: `y` sends [`Command::JoinTrunk`].
    Flags,
    /// A profile: `y` sends its connection request.
    Profile(Box<Command>),
    /// The monitor's join wizard (#970): `y` sends [`Command::JoinTrunk`],
    /// which ends the monitor before joining.
    Monitor {
        /// Why these settings, for the dialog.
        note: String,
    },
}

/// The join confirmation: what joining means, the port to join on, and
/// `y` to go ahead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JoinDialog {
    /// The settings to join with; `port` is the one first offered.
    pub(crate) settings: MstpArgs,
    /// The ports to choose from, the configured one first. A profile's
    /// port, and the monitor's, is the only one.
    pub(crate) ports: Vec<String>,
    /// The port chosen.
    pub(crate) selected: usize,
    /// Where it came from.
    pub(crate) origin: JoinOrigin,
}

impl JoinDialog {
    /// A flags-only session: `settings.port` first, then the other ports
    /// the system lists, in `mstp-ports`' order.
    pub(crate) fn for_flags(settings: MstpArgs, listed: Vec<String>) -> Self {
        let mut ports = vec![settings.port.clone()];
        ports.extend(listed);
        // A listed by-id link to the configured port is the same port
        // (#1928). Resolving names reads the filesystem, once, here.
        let ports = crate::mstp::dedup_resolved(ports, crate::mstp::resolve);
        Self {
            settings,
            ports,
            selected: 0,
            origin: JoinOrigin::Flags,
        }
    }

    /// A profile's connection, sent on confirmation.
    pub(crate) fn for_profile(settings: MstpArgs, then: Command) -> Self {
        Self {
            ports: vec![settings.port.clone()],
            settings,
            selected: 0,
            origin: JoinOrigin::Profile(Box::new(then)),
        }
    }

    /// The monitor wizard's proposal, on the monitor's port.
    pub(crate) fn for_monitor(settings: MstpArgs, note: String) -> Self {
        Self {
            ports: vec![settings.port.clone()],
            settings,
            selected: 0,
            origin: JoinOrigin::Monitor { note },
        }
    }

    /// The port chosen.
    pub(crate) fn port(&self) -> &str {
        &self.ports[self.selected]
    }

    /// Apply a key; on `y`, the command that joins.
    pub(crate) fn key(&mut self, key: JoinKey) -> Option<Command> {
        let last = self.ports.len().saturating_sub(1);
        match key {
            JoinKey::Up => self.selected = self.selected.saturating_sub(1),
            JoinKey::Down => self.selected = (self.selected + 1).min(last),
            JoinKey::Confirm => {
                return Some(match &self.origin {
                    JoinOrigin::Profile(then) => (**then).clone(),
                    JoinOrigin::Flags | JoinOrigin::Monitor { .. } => Command::JoinTrunk {
                        settings: MstpArgs {
                            port: self.port().to_string(),
                            ..self.settings.clone()
                        },
                    },
                });
            }
        }
        None
    }
}

impl App {
    /// Hold a profile's connection request for the join confirmation when
    /// it would join an MS/TP trunk; pass any other command through. A
    /// passive session joins nothing, so it isn't asked.
    pub(super) fn hold_for_join(&mut self, command: Command) -> Option<Command> {
        let Command::Profile(Request::Connect { .. }) = &command else {
            return Some(command);
        };
        let settings = self
            .connection
            .as_ref()
            .and_then(|connection| connection.resolved().ok())
            .and_then(|profile| profile.mstp.map(|mstp| mstp.args()));
        match settings {
            Some(settings) if !self.safety.passive => {
                self.overlay = Overlay::Join(Box::new(JoinDialog::for_profile(settings, command)));
                None
            }
            _ => Some(command),
        }
    }

    /// A key in the join confirmation: `y` joins.
    pub(super) fn join_key(&mut self, key: JoinKey, out: &mut Vec<Command>) {
        let Overlay::Join(dialog) = &mut self.overlay else {
            return;
        };
        let Some(command) = dialog.key(key) else {
            return;
        };
        let text = format!(
            "Joining the MS/TP trunk on {} as station {}.",
            dialog.port(),
            dialog.settings.mac
        );
        if let (JoinOrigin::Monitor { .. }, Command::JoinTrunk { settings }) =
            (&dialog.origin, &command)
        {
            // The monitor keeps listening until the new session starts.
            let settings = settings.clone();
            if !self.confirm_monitor_join(&settings) {
                return;
            }
        }
        tracing::info!("{text}");
        self.overlay = Overlay::None;
        if let Command::Profile(Request::Connect { profile, .. }) = &command {
            let kind = profile.transport.kind();
            (self.transport, self.style) = (kind.label(), kind.style());
        }
        self.link = Link::Connecting;
        self.flash(text, FLASH_FOR);
        out.push(command);
    }

    /// Esc in the join confirmation: back to the profile; for a flags-only
    /// session's first join, which leaves it nothing else to do, quit; for a
    /// later one, stay disconnected.
    pub(super) fn decline_join(&mut self) {
        let Overlay::Join(dialog) = &self.overlay else {
            return;
        };
        if let JoinOrigin::Profile(_) = dialog.origin {
            if let Some(connection) = &mut self.connection {
                connection.pending = false;
            }
            self.overlay = Overlay::Connection;
        } else if let JoinOrigin::Monitor { .. } = dialog.origin {
            // Back to the monitor, its proposal kept.
            self.overlay = Overlay::None;
        } else if self.link == Link::Picking {
            self.quit = true;
        } else {
            // Declining a later join leaves the session disconnected.
            self.overlay = Overlay::None;
        }
    }
}
