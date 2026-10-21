//! The MS/TP monitor screen's state and the safe-join wizard (#970). No I/O.
//!
//! `bacnet tui --mstp-monitor PORT` opens the MS/TP screen on a monitor
//! session: the Passive level, nothing transmitted. `J` starts the wizard:
//! it listens for a while, then proposes a free MAC and a Max_Master;
//! Enter takes the proposal to the join confirmation of #969 (T2), and `y`
//! there ends the monitor and joins.

use std::time::Duration;

use super::mstp::{JoinDialog, MstpKey};
use super::{App, Overlay, FLASH_FOR};
use crate::mstp::MstpArgs;
use crate::tui::monitor::analysis::{self, Proposal, Thresholds};
use crate::tui::monitor::{MonitorPlan, MonitorReport, MonitorSample};

/// How long the wizard listens before it proposes, by default.
pub(crate) const DEFAULT_LISTEN: Duration = Duration::from_secs(10);

/// The monitor screen's views, in Tab order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum MonitorTab {
    /// The address map and the station table.
    #[default]
    Stations,
    /// The successor chain, rotation and per-master timing.
    Ring,
    /// Max_Master and threshold findings, and the join wizard.
    Findings,
    /// Error counters and lost-token events.
    Errors,
}

impl MonitorTab {
    /// The tab's title.
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Stations => "Stations",
            Self::Ring => "Token ring",
            Self::Findings => "Findings",
            Self::Errors => "Errors",
        }
    }

    /// Every tab, in order.
    pub(crate) const ALL: [Self; 4] = [Self::Stations, Self::Ring, Self::Findings, Self::Errors];

    fn cycle(self, back: bool) -> Self {
        let at = Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0);
        let step = if back { Self::ALL.len() - 1 } else { 1 };
        Self::ALL[(at + step) % Self::ALL.len()]
    }
}

/// The join wizard's progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Wizard {
    /// Listening since this time on the monitor's clock.
    Listening {
        /// When `J` was pressed.
        since: Duration,
    },
    /// A proposal, with the MAC chosen from its candidates.
    Proposed {
        /// What the analysis proposes.
        proposal: Proposal,
        /// The index of the MAC chosen.
        selected: usize,
    },
    /// Nothing to propose, and why.
    Failed(String),
}

/// The error counters the screen lists, in its order.
pub(crate) type ErrorRows = [(&'static str, u64); 7];

/// The monitor screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MonitorState {
    /// The port listened to.
    pub(crate) port: String,
    /// Its baud rate.
    pub(crate) baud: u32,
    /// Max_Info_Frames to join with.
    pub(crate) max_info_frames: u8,
    /// False with `--passive`: the wizard proposes, but never joins.
    pub(crate) join_allowed: bool,
    /// The reading on screen.
    pub(crate) shown: Option<Box<MonitorSample>>,
    /// The newest reading, kept while the screen is paused.
    pub(crate) latest: Option<Box<MonitorSample>>,
    /// The error counters at the last clear.
    pub(crate) baseline: Option<ErrorRows>,
    /// New readings are not shown until watching resumes.
    pub(crate) paused: bool,
    /// The view shown.
    pub(crate) tab: MonitorTab,
    /// When rotations and holds are reported.
    pub(crate) thresholds: Thresholds,
    /// How long the wizard listens.
    pub(crate) listen: Duration,
    /// The join wizard, once started.
    pub(crate) wizard: Option<Wizard>,
    /// A join sent and not yet taken up: the settings, until the new
    /// session starts or the join is refused.
    pub(crate) joining: Option<MstpArgs>,
}

impl MonitorState {
    /// A monitor on `port` at `baud`, before its first reading.
    pub(crate) fn new(port: String, baud: u32, max_info_frames: u8, join_allowed: bool) -> Self {
        Self {
            port,
            baud,
            max_info_frames,
            join_allowed,
            shown: None,
            latest: None,
            baseline: None,
            paused: false,
            tab: MonitorTab::default(),
            thresholds: Thresholds::default(),
            listen: DEFAULT_LISTEN,
            wizard: None,
            joining: None,
        }
    }

    /// Take a worker report; the error, for one that ended the monitor.
    pub(crate) fn report(&mut self, report: MonitorReport) -> Option<String> {
        match report {
            MonitorReport::Started { port, baud } => {
                (self.port, self.baud) = (port, baud);
                (self.shown, self.latest, self.baseline) = (None, None, None);
                self.wizard = None;
            }
            MonitorReport::Sample(sample) => {
                if !self.paused {
                    self.shown = Some(sample.clone());
                }
                self.latest = Some(sample);
                self.advance_wizard();
            }
            MonitorReport::Ended(error) => return Some(error),
        }
        None
    }

    /// Propose once the wizard has listened long enough.
    fn advance_wizard(&mut self) {
        let (Some(Wizard::Listening { since }), Some(latest)) = (&self.wizard, &self.latest) else {
            return;
        };
        if latest.elapsed.saturating_sub(*since) < self.listen {
            return;
        }
        self.wizard = Some(match analysis::propose(latest) {
            Ok(proposal) => Wizard::Proposed {
                selected: proposal.preferred,
                proposal,
            },
            Err(why) => Wizard::Failed(why),
        });
    }

    /// The error counters shown: since the last clear, or since the start.
    pub(crate) fn errors(&self) -> Option<ErrorRows> {
        let now = rows(self.shown.as_deref()?);
        let base = self.baseline.unwrap_or(now.map(|(label, _)| (label, 0)));
        Some(std::array::from_fn(|i| {
            (now[i].0, now[i].1.saturating_sub(base[i].1))
        }))
    }

    /// Check the chosen MAC against the newest reading before joining with
    /// it: still free and still polled, the proposal is refreshed and its
    /// settings returned. Otherwise the wizard proposes again from that
    /// reading, and the error says why.
    pub(crate) fn recheck(&mut self) -> Result<MstpArgs, String> {
        let (Some(Wizard::Proposed { proposal, selected }), Some(latest)) =
            (&self.wizard, self.latest.as_deref())
        else {
            return Err("No proposal to join with.".into());
        };
        let mac = proposal.candidates[*selected];
        let fresh = match analysis::propose(latest) {
            Ok(fresh) => fresh,
            Err(why) => {
                self.wizard = Some(Wizard::Failed(why.clone()));
                return Err(why);
            }
        };
        let kept = fresh.candidates.iter().position(|c| *c == mac);
        self.wizard = Some(Wizard::Proposed {
            selected: kept.unwrap_or(fresh.preferred),
            proposal: fresh,
        });
        match kept {
            Some(_) => self.join_settings().ok_or_else(|| "No proposal.".into()),
            None => Err(format!(
                "mstp:{mac} is no longer free or polled; see the new proposal."
            )),
        }
    }

    /// The settings the proposal joins with, on the monitor's port.
    pub(crate) fn join_settings(&self) -> Option<MstpArgs> {
        let Some(Wizard::Proposed { proposal, selected }) = &self.wizard else {
            return None;
        };
        let mac = *proposal.candidates.get(*selected)?;
        Some(MstpArgs {
            port: self.port.clone(),
            baud: self.baud,
            mac,
            max_master: proposal.max_master_for(mac),
            max_info_frames: self.max_info_frames,
        })
    }
}

/// The counters the Errors view lists.
fn rows(sample: &MonitorSample) -> ErrorRows {
    let e = &sample.stats.errors;
    [
        ("Header CRC errors", e.header_crc),
        ("Data CRC errors", e.data_crc),
        ("Malformed or oversize", e.malformed),
        ("Partial frames", e.frame_aborts),
        ("Noise octets", e.noise_octets),
        ("Lost tokens", sample.stats.lost_tokens),
        ("Live records dropped", sample.dropped),
    ]
}

/// A `:mstp` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Setting {
    /// Report rotations longer than this.
    Rotation(Duration),
    /// Report token holds longer than this.
    Hold(Duration),
    /// How long the join wizard listens.
    Listen(Duration),
}

impl App {
    /// Open on the monitor of `plan`: the Passive level, the MS/TP screen.
    /// `join_allowed` is false with `--passive`.
    pub(crate) fn start_monitor(&mut self, plan: &MonitorPlan, join_allowed: bool) {
        self.monitor = Some(Box::new(MonitorState::new(
            plan.port.clone(),
            plan.baud,
            plan.max_info_frames,
            join_allowed,
        )));
        self.safety.passive = true;
        self.screen = super::watch::Screen::Mstp;
    }

    /// A key on the MS/TP screen: the monitor's, while one runs and the
    /// session hasn't joined; the participant panel's otherwise.
    pub(super) fn mstp_key(&mut self, key: MstpKey) {
        let monitoring = self.mstp.station.is_none() && self.monitor.is_some();
        let Some(monitor) = self.monitor.as_deref_mut().filter(|_| monitoring) else {
            match key {
                MstpKey::Back => self.screen = super::watch::Screen::Main,
                key => self.mstp.key(key),
            }
            return;
        };
        match key {
            MstpKey::Clear => monitor.baseline = monitor.shown.as_deref().map(rows),
            MstpKey::ToggleWatch => {
                monitor.paused = !monitor.paused;
                if !monitor.paused {
                    monitor.shown.clone_from(&monitor.latest);
                }
            }
            MstpKey::Cycle { back } => monitor.tab = monitor.tab.cycle(back),
            MstpKey::Join => {
                let since = monitor
                    .latest
                    .as_ref()
                    .map(|s| s.elapsed)
                    .unwrap_or_default();
                monitor.wizard = Some(Wizard::Listening { since });
                monitor.tab = MonitorTab::Findings;
                monitor.advance_wizard();
            }
            MstpKey::Pick { back } => {
                if let Some(Wizard::Proposed { proposal, selected }) = &mut monitor.wizard {
                    let last = proposal.candidates.len().saturating_sub(1);
                    *selected = if back {
                        selected.saturating_sub(1)
                    } else {
                        (*selected + 1).min(last)
                    };
                }
            }
            MstpKey::Accept => self.open_monitor_join(),
            MstpKey::Back if monitor.wizard.is_some() => monitor.wizard = None,
            MstpKey::Back => self.screen = super::watch::Screen::Main,
        }
    }

    /// Enter on a proposal: the join confirmation (T2) with its settings.
    fn open_monitor_join(&mut self) {
        let Some(monitor) = self.monitor.as_deref_mut() else {
            return;
        };
        if monitor.joining.is_some() || monitor.join_settings().is_none() {
            return;
        }
        if !monitor.join_allowed {
            self.flash("Passive: this session never joins the trunk.", FLASH_FOR);
            return;
        }
        // The line may have changed since the proposal.
        let settings = match monitor.recheck() {
            Ok(settings) => settings,
            Err(why) => return self.flash(why, FLASH_FOR),
        };
        let note = format!(
            "Free after {} s of listening. The monitor closes the port first.",
            monitor.listen.as_secs()
        );
        self.overlay = Overlay::Join(Box::new(JoinDialog::for_monitor(settings, note)));
    }

    /// `:mstp rotation|hold MS` and `:mstp listen S`.
    pub(super) fn mstp_setting(&mut self, setting: Setting) {
        let Some(monitor) = self.monitor.as_deref_mut() else {
            self.flash("No MS/TP monitor in this session.", FLASH_FOR);
            return;
        };
        let text = match setting {
            Setting::Rotation(at) => {
                monitor.thresholds.rotation = at;
                format!("Rotations over {} are reported.", analysis::millis(at))
            }
            Setting::Hold(at) => {
                monitor.thresholds.hold = at;
                format!("Token holds over {} are reported.", analysis::millis(at))
            }
            Setting::Listen(span) => {
                monitor.listen = span;
                format!("The join wizard listens for {} s.", span.as_secs())
            }
        };
        self.flash(text, FLASH_FOR);
    }

    /// `y` on the monitor's proposal: check it once more, then send the join
    /// and keep monitoring until the new session starts. `false`, with the
    /// dialog closed and the reason shown, when the proposal no longer holds.
    pub(super) fn confirm_monitor_join(&mut self, settings: &MstpArgs) -> bool {
        let Some(monitor) = self.monitor.as_deref_mut() else {
            return false;
        };
        match monitor.recheck() {
            Ok(fresh) if fresh == *settings => {
                monitor.joining = Some(fresh);
                true
            }
            outcome => {
                self.overlay = Overlay::None;
                let why = outcome
                    .err()
                    .unwrap_or_else(|| "The proposal changed; review it again (Enter).".into());
                self.flash(why, FLASH_FOR);
                false
            }
        }
    }

    /// The join from the monitor was refused (by the coordinator, or the
    /// command queue): keep listening, and say why. `false` when no join
    /// from the monitor was waiting.
    pub(super) fn monitor_join_refused(&mut self, error: &str) -> bool {
        let Some(monitor) = self.monitor.as_deref_mut() else {
            return false;
        };
        if monitor.joining.take().is_none() {
            return false;
        }
        self.link = super::Link::Passive;
        self.flash(format!("Not joined: {error}. Still listening."), FLASH_FOR);
        true
    }

    /// A new session started: a join from the monitor has taken over. The
    /// session transmits from now on, and a later join is confirmed again
    /// like `--mstp`'s.
    pub(super) fn monitor_joined(&mut self) {
        let Some(settings) = self.monitor.as_deref_mut().and_then(|m| m.joining.take()) else {
            return;
        };
        self.join_again = Some(Box::new(JoinDialog::for_flags(settings, vec![])));
        self.monitor = None;
        self.safety.passive = false;
    }

    /// A monitor report; one that ended the monitor fails the link.
    pub(super) fn monitor_report(&mut self, report: MonitorReport) {
        let Some(monitor) = self.monitor.as_deref_mut() else {
            return;
        };
        if let Some(error) = monitor.report(report) {
            self.link = super::Link::Failed(crate::tui::message::Failure::text(error));
        }
    }
}
