//! The Network screen (#966): B/IP infrastructure as this client sees it.
//!
//! - The foreign-device panel: the BBMD, the time-to-live, the last
//!   BVLC-Result, a countdown to the next registration, and a banner when
//!   the BBMD refuses the registration (Annex J.5.2.2) or leaves it
//!   unanswered.
//! - The BDT and FDT of the target BBMD, with what each mask means and the
//!   seconds each FDT entry has left; our own FDT entry is marked, since the
//!   address the BBMD recorded is the one it forwards broadcasts to.
//! - The BDT consistency matrix of every BBMD the target's table leads to.
//!
//! The target is the BBMD the client registers with, or the one named with
//! `:bbmd ADDRESS`. Every read is a request the user starts (`r`, `m`), apart
//! from one FDT read when the registration is first accepted.

use std::net::SocketAddrV4;
use std::time::{Duration, Instant};

use bacnet_ops::label::kebab;
use bacnet_transport::bip::{ForeignRegistrationOutcome, ForeignRegistrationSnapshot};

use super::watch::Screen;
use super::{App, FLASH_FOR, Link};
use crate::tui::message::Command;
use crate::tui::network::routers::RouterMap;
use crate::tui::network::{BdtRow, FdtRow, Foreign, NetworkCommand, NetworkEvent, own_entry};
use bacnet_ops::bbmd::{Matrix, Unfollowed};

/// The table below the foreign-device panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum View {
    /// The target's BDT.
    #[default]
    Bdt,
    /// The target's FDT.
    Fdt,
    /// The consistency matrix.
    Matrix,
    /// The routers heard and the networks they reach (#960), on any
    /// transport.
    Networks,
}

/// Where the foreign-device registration stands: the status bar's `FD`
/// field and the Network screen's banner both read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Registration {
    /// The BBMD accepted it and the lease holds.
    Held,
    /// No answer yet: the first attempt is out, or a renewal is.
    Pending,
    /// The BBMD's last answer refused it.
    Refused,
    /// None accepted, and this many attempts went unanswered.
    NoAnswer {
        /// Attempts with no answer.
        unanswered: u64,
    },
    /// A lease lapsed and no renewal since was accepted.
    Lapsed,
}

impl Registration {
    /// The status bar's word for it.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Held => "held",
            Self::Pending => "pending",
            Self::Refused => "refused",
            Self::NoAnswer { .. } => "no answer",
            Self::Lapsed => "lapsed",
        }
    }
}

/// A key on the Network screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NetworkKey {
    /// `r`: read the target's tables again, or survey again on the matrix.
    Refresh,
    /// `m`: survey every BBMD for the matrix.
    Survey,
    /// Tab: the next table, or the previous one.
    Cycle {
        /// Go backwards.
        back: bool,
    },
}

/// A table read, with when it arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fetched<T> {
    /// The BBMD read.
    pub(crate) bbmd: SocketAddrV4,
    /// When the answer arrived.
    pub(crate) at: Instant,
    /// The table, or why it could not be read.
    pub(crate) result: Result<T, String>,
}

/// The Network screen's state.
#[derive(Debug, Clone, Default)]
pub(crate) struct NetworkState {
    /// The foreign-device registration the connection asked for.
    pub(crate) foreign: Option<Foreign>,
    /// The BBMD whose tables are shown.
    pub(crate) target: Option<SocketAddrV4>,
    /// The latest registration snapshot and when it arrived.
    pub(crate) registration: Option<(ForeignRegistrationSnapshot, Instant)>,
    /// The target's BDT.
    pub(crate) bdt: Option<Fetched<Vec<BdtRow>>>,
    /// The target's FDT.
    pub(crate) fdt: Option<Fetched<Vec<FdtRow>>>,
    /// Our own entry in `fdt`, if one has our address.
    pub(crate) own: Option<usize>,
    /// Our own address, as the FDT read reported it.
    pub(crate) local: Option<SocketAddrV4>,
    /// The last survey and when it finished.
    pub(crate) matrix: Option<(Matrix, Instant)>,
    /// The BDT entries the last survey did not follow, and why (#2130).
    pub(crate) not_followed: Vec<Unfollowed>,
    /// A table read is out.
    pub(crate) reading: bool,
    /// A survey is out.
    pub(crate) surveying: bool,
    /// The table shown.
    pub(crate) view: View,
    /// The routers heard and the networks they reach.
    pub(crate) routers: RouterMap,
    /// The local network's number and whether it was configured, from the
    /// last Network-Number-Is heard.
    pub(crate) local_number: Option<(u16, bool)>,
    /// When the routers were last asked.
    pub(crate) routers_asked: Option<Instant>,
}

impl NetworkState {
    /// The connection registers as a foreign device: start the panel on its
    /// BBMD, whether it accepted the first attempt or is still retrying.
    pub(crate) fn foreign_device(&mut self, foreign: Foreign) {
        self.foreign = Some(foreign);
        self.target.get_or_insert(foreign.bbmd);
    }

    /// Time until the next registration attempt at `now`.
    pub(crate) fn next_attempt(&self, now: Instant) -> Option<Duration> {
        let (snapshot, at) = self.registration.as_ref()?;
        let left = snapshot.next_attempt_in?;
        Some(left.saturating_sub(now.saturating_duration_since(*at)))
    }

    /// Time left on the lease the BBMD's last success proves, at `now`.
    pub(crate) fn lease_left(&self, now: Instant) -> Option<Duration> {
        let (snapshot, at) = self.registration.as_ref()?;
        let left = snapshot.lease_remaining.filter(|_| snapshot.registered)?;
        Some(left.saturating_sub(now.saturating_duration_since(*at)))
    }

    /// Where the foreign-device registration stands (#1894), for the
    /// status bar and the banner alike: `None` without a registration and
    /// no refusal on record.
    ///
    /// It keys on the counters rather than the last outcome, since a fresh
    /// attempt reads `Pending` while the earlier ones went unanswered.
    pub(crate) fn registration_state(&self) -> Option<Registration> {
        let Some((snapshot, _)) = self.registration.as_ref() else {
            return self.foreign.map(|_| Registration::Pending);
        };
        if snapshot.last_outcome == Some(ForeignRegistrationOutcome::Refused) {
            return Some(Registration::Refused);
        }
        self.foreign?;
        if snapshot.registered {
            return Some(Registration::Held);
        }
        if snapshot.lapses > 0 {
            return Some(Registration::Lapsed);
        }
        // Attempts that ended with no answer. The last one doesn't count
        // while in flight, or when it never reached the BBMD (not sent, or
        // cancelled). The snapshot keeps no per-outcome totals, so earlier
        // attempts that failed locally can't be told from timeouts.
        let not_the_bbmds = matches!(
            snapshot.last_outcome,
            Some(
                ForeignRegistrationOutcome::Pending
                    | ForeignRegistrationOutcome::LocalError
                    | ForeignRegistrationOutcome::Cancelled
            )
        );
        let unanswered = snapshot
            .attempts
            .saturating_sub(snapshot.accepted + snapshot.refused + u64::from(not_the_bbmds));
        if snapshot.accepted == 0 && unanswered > 0 {
            return Some(Registration::NoAnswer { unanswered });
        }
        Some(Registration::Pending)
    }

    /// The registration banner: while the BBMD refuses the registration,
    /// or once registrations go unanswered (none accepted after the first
    /// attempt, or a lease lapsed with no newer success).
    pub(crate) fn registration_banner(&self) -> Option<String> {
        let (snapshot, _) = self.registration.as_ref()?;
        let bbmd = snapshot
            .last_bbmd
            .map_or_else(|| "The BBMD".to_string(), |bbmd| format!("BBMD {bbmd}"));
        match self.registration_state()? {
            Registration::Refused => {
                let result = snapshot
                    .last_result
                    .map_or_else(String::new, |code| format!(" ({})", kebab(code)));
                Some(format!(
                    " REFUSED {bbmd} refused the foreign-device registration{result}; \
                     broadcasts through it fail until it accepts. Check its foreign-device policy."
                ))
            }
            Registration::Lapsed => Some(format!(
                " LAPSED The registration with {bbmd} lapsed, and no renewal since was \
                 accepted; broadcasts through it are not delivered."
            )),
            Registration::NoAnswer { unanswered } => {
                let plural = if unanswered == 1 { "" } else { "s" };
                Some(format!(
                    " NO ANSWER {bbmd} has not answered {unanswered} foreign-device \
                     registration{plural}; broadcasts through it are not delivered."
                ))
            }
            Registration::Held | Registration::Pending => None,
        }
    }

    /// Whether the tables shown are from the BBMD we register with, so our
    /// own FDT entry can be judged; false without a registration.
    pub(crate) fn tables_are_ours(&self) -> bool {
        self.foreign.is_some_and(|f| self.target == Some(f.bbmd))
    }

    /// Seconds an FDT entry had left at the read, less the time since.
    pub(crate) fn remaining(&self, entry: &FdtRow, now: Instant) -> u64 {
        let elapsed = self
            .fdt
            .as_ref()
            .map_or(0, |fdt| now.saturating_duration_since(fdt.at).as_secs());
        u64::from(entry.remaining).saturating_sub(elapsed)
    }

    /// Whether an FDT entry is near expiry at `now` (#2130): the shared rule
    /// ([`FdtRow::expiring`]) on the seconds the screen shows, so the marker
    /// and the countdown beside it agree.
    pub(crate) fn expiring(&self, entry: &FdtRow, now: Instant) -> bool {
        let left = u16::try_from(self.remaining(entry, now)).unwrap_or(u16::MAX);
        FdtRow {
            remaining: left,
            ..*entry
        }
        .expiring()
    }
}

impl App {
    /// A Network screen report.
    pub(super) fn network_event(&mut self, event: NetworkEvent) {
        let now = self.now;
        let state = &mut self.network;
        match event {
            NetworkEvent::Registration(snapshot) => state.registration = Some((snapshot, now)),
            NetworkEvent::Tables {
                bbmd,
                local,
                bdt,
                fdt,
            } => {
                // An answer for another BBMD, such as the automatic FDT read
                // of the registration's BBMD after `:bbmd`, is not this one's.
                if state.target != Some(bbmd) {
                    return;
                }
                state.reading = false;
                state.own = fdt.as_ref().ok().and_then(|fdt| own_entry(fdt, local));
                state.local = Some(local);
                state.bdt = Some(Fetched {
                    bbmd,
                    at: now,
                    result: bdt,
                });
                state.fdt = Some(Fetched {
                    bbmd,
                    at: now,
                    result: fdt,
                });
            }
            NetworkEvent::Survey {
                seed,
                probes,
                not_followed,
            } => {
                // A survey from a BBMD that is no longer the target would
                // show its matrix under the new one.
                if state.target != Some(seed) {
                    return;
                }
                state.surveying = false;
                state.matrix = Some((Matrix::build(&probes), now));
                state.not_followed = not_followed;
            }
            NetworkEvent::Refused { command, reason } => {
                match command {
                    NetworkCommand::Tables { bbmd } if state.target == Some(bbmd) => {
                        state.reading = false;
                    }
                    NetworkCommand::Survey { seed } if state.target == Some(seed) => {
                        state.surveying = false;
                    }
                    NetworkCommand::Tables { .. } | NetworkCommand::Survey { .. } => {}
                }
                self.flash(reason, FLASH_FOR);
            }
        }
    }

    /// A key on the Network screen.
    pub(super) fn network_key(&mut self, key: NetworkKey, out: &mut Vec<Command>) {
        match key {
            NetworkKey::Cycle { back } => {
                let views = [View::Bdt, View::Fdt, View::Matrix, View::Networks];
                let at = views.iter().position(|v| *v == self.network.view);
                let step = if back { views.len() - 1 } else { 1 };
                self.network.view = views[(at.unwrap_or(0) + step) % views.len()];
            }
            NetworkKey::Refresh if self.network.view == View::Matrix => self.survey(out),
            NetworkKey::Refresh if self.network.view == View::Networks => self.ask_routers(out),
            NetworkKey::Refresh => self.read_tables(out),
            NetworkKey::Survey => {
                self.network.view = View::Matrix;
                self.survey(out);
            }
        }
    }

    /// `:bbmd [ADDRESS]`: that BBMD's tables, or with no address those of
    /// the BBMD the session registers with.
    pub(super) fn show_bbmd(&mut self, bbmd: Option<SocketAddrV4>, out: &mut Vec<Command>) {
        match bbmd.or(self.network.foreign.map(|f| f.bbmd)) {
            Some(bbmd) => self.set_bbmd(bbmd, out),
            None => self.flash(
                "No foreign-device registration: type :bbmd ADDRESS.",
                FLASH_FOR,
            ),
        }
    }

    /// The registration's BBMD while the screen shows another one's
    /// tables (after `:bbmd OTHER`), so the footer can say how to go back.
    pub(crate) fn away_from_own_bbmd(&self) -> Option<SocketAddrV4> {
        let own = self.network.foreign?.bbmd;
        (self.network.target.is_some_and(|target| target != own)).then_some(own)
    }

    /// `:bbmd ADDRESS`: show that BBMD's tables, and read them.
    fn set_bbmd(&mut self, bbmd: SocketAddrV4, out: &mut Vec<Command>) {
        let state = &mut self.network;
        if state.target != Some(bbmd) {
            state.target = Some(bbmd);
            state.bdt = None;
            state.fdt = None;
            state.own = None;
            state.matrix = None;
            state.not_followed.clear();
            // Answers to the old target's requests are dropped now.
            state.reading = false;
            state.surveying = false;
        }
        self.screen = Screen::Network;
        self.read_tables(out);
    }

    /// The BBMD to read, once the session may read it.
    fn network_target(&mut self) -> Option<SocketAddrV4> {
        if self.transport != "BIP" {
            self.flash("The Network screen needs BACnet/IP.", FLASH_FOR);
            return None;
        }
        if !self.may_transmit() {
            return None;
        }
        if !matches!(self.link, Link::Up { .. }) {
            self.flash("Not connected yet.", FLASH_FOR);
            return None;
        }
        if self.network.target.is_none() {
            self.flash(
                "No BBMD: connect with --foreign-bbmd, or type :bbmd ADDRESS.",
                FLASH_FOR,
            );
        }
        self.network.target
    }

    fn read_tables(&mut self, out: &mut Vec<Command>) {
        if let Some(bbmd) = self.network_target() {
            self.network.reading = true;
            out.push(Command::Network(NetworkCommand::Tables { bbmd }));
        }
    }

    fn survey(&mut self, out: &mut Vec<Command>) {
        if let Some(seed) = self.network_target() {
            self.network.surveying = true;
            out.push(Command::Network(NetworkCommand::Survey { seed }));
        }
    }
}
