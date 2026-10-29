//! What the monitor's statistics say about the trunk: each station's role,
//! the token ring, Max_Master findings, and a free MAC to join with. Pure
//! functions of a sample; no I/O.
//!
//! Max_Master is read from polls (Clause 9.5.6): a master polls the
//! addresses between itself and its successor, one at a time, wrapping
//! from its Max_Master to 0. The statistics engine reports
//! `MaxMaster::Exact` only once it has seen that wrap, and otherwise
//! `MaxMaster::AtLeast` the highest address polled. A "too low" finding
//! needs an exact value; a lower bound can only ever be too high.

use std::time::Duration;

use bacnet_transport::mstp::monitor::{MaxMaster, MonitorStats, StationStats};

use super::MonitorSample;
use crate::mstp::MAX_MASTER_MAC;

/// A station heard from before but not for this long is shown as silent.
pub(crate) const SILENT_AFTER: Duration = Duration::from_secs(10);

/// A Max_Master more than this above the highest station is reported as
/// wasting polls. A rule of thumb, not a value from the standard.
pub(crate) const HIGH_HEADROOM: u8 = 8;

/// A run of more than this many empty addresses between two masters is
/// reported: it is polled address by address for as long as it stays
/// empty. A rule of thumb, not a value from the standard.
pub(crate) const GAP_LIMIT: u8 = 8;

/// The default thresholds: a token rotation over 2 s, a hold over 500 ms.
pub(crate) const DEFAULT_ROTATION: Duration = Duration::from_secs(2);
/// See [`DEFAULT_ROTATION`].
pub(crate) const DEFAULT_HOLD: Duration = Duration::from_millis(500);

/// What a station looks like from the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// Passes the token or polls for masters.
    Master,
    /// Only answers.
    Slave,
    /// Sends routed NPDUs or network-layer messages.
    Router,
    /// Heard from before, or sent requests, but quiet now.
    Silent,
}

impl Role {
    /// The label the screen shows.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Master => "master",
            Self::Slave => "slave",
            Self::Router => "router",
            Self::Silent => "silent",
        }
    }

    /// One letter for the address map.
    pub(crate) fn letter(self) -> char {
        match self {
            Self::Master => 'M',
            Self::Slave => 'S',
            Self::Router => 'R',
            Self::Silent => 'x',
        }
    }
}

/// One station in the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StationRow {
    /// Its MAC.
    pub(crate) mac: u8,
    /// What it does.
    pub(crate) role: Role,
    /// Its device instance, once an I-Am from it was seen.
    pub(crate) device: Option<u32>,
    /// Valid frames it sent.
    pub(crate) frames: u64,
    /// When it last sent one, from the monitor's start.
    pub(crate) last_seen: Option<Duration>,
}

/// Requests or Tokens went to this station: something expects it there.
fn addressed(station: &StationStats) -> bool {
    let received = &station.frames_received;
    received.total() > received.poll_for_master
}

/// The stations in the grid, by MAC: those heard from, and those sent
/// requests or Tokens that never answered.
pub(crate) fn stations(sample: &MonitorSample) -> Vec<StationRow> {
    let stats = &sample.stats;
    let now = stats.last_activity.unwrap_or_default();
    stats
        .stations
        .iter()
        .filter(|(_, station)| station.is_present() || addressed(station))
        .map(|(&mac, station)| {
            let quiet = station
                .last_seen
                .is_none_or(|seen| now.saturating_sub(seen) > SILENT_AFTER);
            let role = if quiet {
                Role::Silent
            } else if sample.routers.contains(&mac) {
                Role::Router
            } else if station.is_master() {
                Role::Master
            } else {
                Role::Slave
            };
            StationRow {
                mac,
                role,
                device: sample.devices.get(&mac).copied(),
                frames: station.frames_sent.total(),
                last_seen: station.last_seen,
            }
        })
        .collect()
}

/// Masters on the line: stations that passed the token or polled.
pub(crate) fn masters(stats: &MonitorStats) -> impl Iterator<Item = (u8, &StationStats)> {
    stats
        .stations
        .iter()
        .filter(|&(&mac, station)| mac <= MAX_MASTER_MAC && station.is_master())
        .map(|(&mac, station)| (mac, station))
}

/// The successor chain from the lowest master, as far as it goes: back to
/// where it started, or to a station whose successor isn't known.
pub(crate) fn chain(stats: &MonitorStats) -> Vec<u8> {
    let Some((first, _)) = masters(stats).find(|(_, s)| s.next_station.is_some()) else {
        return Vec::new();
    };
    let mut chain = vec![first];
    let mut at = first;
    while let Some(next) = stats.station(at).and_then(|s| s.next_station) {
        chain.push(next);
        if next == first || chain.len() > usize::from(MAX_MASTER_MAC) + 2 {
            break;
        }
        at = next;
    }
    chain
}

/// Rotation times over every master.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Rotation {
    /// Samples.
    pub(crate) count: u64,
    /// The shortest.
    pub(crate) min: Option<Duration>,
    /// The mean.
    pub(crate) mean: Option<Duration>,
    /// The longest.
    pub(crate) max: Option<Duration>,
}

/// The ring's rotation time, every master's samples together.
pub(crate) fn rotation(stats: &MonitorStats) -> Rotation {
    let mut out = Rotation::default();
    let mut total = Duration::ZERO;
    for (_, station) in masters(stats) {
        let r = &station.token_rotation;
        out.count += r.count;
        total = total.saturating_add(r.total);
        out.min = r.min.into_iter().chain(out.min).min();
        out.max = r.max.into_iter().chain(out.max).max();
    }
    out.mean = u32::try_from(out.count)
        .ok()
        .filter(|count| *count > 0)
        .map(|count| total / count);
    out
}

/// How much a finding matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Severity {
    /// Something is or may be broken.
    Warning,
    /// Wasted time, worth tidying.
    Notice,
}

/// One thing the analysis found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Finding {
    /// How much it matters.
    pub(crate) severity: Severity,
    /// What it says, with the stations it names and a suggestion.
    pub(crate) text: String,
}

/// The thresholds `:mstp rotation MS` and `:mstp hold MS` set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Thresholds {
    /// A rotation longer than this is reported.
    pub(crate) rotation: Duration,
    /// A token hold longer than this is reported.
    pub(crate) hold: Duration,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            rotation: DEFAULT_ROTATION,
            hold: DEFAULT_HOLD,
        }
    }
}

/// The highest MAC up to 127 that anything was heard from or expected at:
/// the Max_Master every master needs to reach it.
pub(crate) fn highest_station(sample: &MonitorSample) -> Option<u8> {
    let stats = &sample.stats;
    let heard = stats
        .stations
        .iter()
        .filter(|(_, s)| s.is_present() || addressed(s))
        .map(|(mac, _)| *mac);
    heard
        .chain(sample.devices.keys().copied())
        .filter(|mac| *mac <= MAX_MASTER_MAC)
        .max()
}

/// "Max_Master is 3" or "Max_Master is at least 3".
fn described(value: MaxMaster) -> String {
    match value {
        MaxMaster::Exact(v) => format!("Max_Master is {v}"),
        MaxMaster::AtLeast(v) => format!("Max_Master is at least {v}"),
    }
}

/// Max_Master, gap and threshold findings, warnings first.
pub(crate) fn findings(sample: &MonitorSample, thresholds: &Thresholds) -> Vec<Finding> {
    let stats = &sample.stats;
    let mut out = Vec::new();
    let mut add = |severity, text| out.push(Finding { severity, text });
    let highest = highest_station(sample).unwrap_or(0);
    for (mac, station) in masters(stats) {
        let Some(value) = station.max_master else {
            continue;
        };
        if let MaxMaster::Exact(v) = value {
            // Only an exact value can be too low: a station above it is
            // never polled by this master, so never brought into the ring.
            let above: Vec<String> = sample_above(sample, v);
            // A master the ring never polls only answers, so it looks like
            // a slave: say "if".
            let which = match above.as_slice() {
                [] => None,
                [one] => Some(format!("{one}: if {one} is a master")),
                many => Some(format!("{}: if any of them is a master", many.join(", "))),
            };
            if let Some(which) = which {
                add(
                    Severity::Warning,
                    format!(
                        "mstp:{mac}: Max_Master is {v} (its polls wrap there), below \
                         {which}, it never gets the token from mstp:{mac}. Suggest \
                         Max_Master {highest} on every master."
                    ),
                );
            }
        }
        if value.value() > highest.saturating_add(HIGH_HEADROOM) {
            add(
                Severity::Notice,
                format!(
                    "mstp:{mac}: {} but no station is above {highest}: it spends poll time \
                     on empty addresses. Suggest Max_Master {highest}, plus room for \
                     stations to come.",
                    described(value)
                ),
            );
        }
    }
    for (mac, station) in masters(stats) {
        let Some(next) = station.next_station.filter(|next| *next > mac) else {
            continue;
        };
        let empty = next - mac - 1;
        if empty > GAP_LIMIT {
            add(
                Severity::Notice,
                format!(
                    "mstp:{mac} polls the {empty} empty addresses before mstp:{next} one by \
                     one, for as long as they stay empty (Clause 9.5.6). Numbering the \
                     masters closer together saves those polls."
                ),
            );
        }
    }
    let worst = rotation(stats).max.filter(|max| *max > thresholds.rotation);
    if let Some(max) = worst {
        add(
            Severity::Warning,
            format!(
                "Token rotation reached {}, over the {} threshold.",
                millis(max),
                millis(thresholds.rotation)
            ),
        );
    }
    for (mac, station) in masters(stats) {
        if let Some(max) = station.token_hold.max.filter(|max| *max > thresholds.hold) {
            add(
                Severity::Warning,
                format!(
                    "mstp:{mac} held the token for {}, over the {} threshold.",
                    millis(max),
                    millis(thresholds.hold)
                ),
            );
        }
    }
    out.sort_by_key(|finding| finding.severity);
    out
}

/// Stations up to 127 above `value`, as `mstp:N`.
fn sample_above(sample: &MonitorSample, value: u8) -> Vec<String> {
    let stats = &sample.stats;
    stats
        .stations
        .iter()
        .filter(|&(&mac, s)| {
            mac > value && mac <= MAX_MASTER_MAC && (s.is_present() || addressed(s))
        })
        .map(|(mac, _)| format!("mstp:{mac}"))
        .collect()
}

/// A duration as milliseconds, or seconds from 10 s.
pub(crate) fn millis(duration: Duration) -> String {
    if duration >= Duration::from_secs(10) {
        format!("{} s", duration.as_secs())
    } else if duration >= Duration::from_secs(1) {
        format!("{:.1} s", duration.as_secs_f64())
    } else {
        format!("{} ms", duration.as_millis())
    }
}

/// What the join wizard proposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Proposal {
    /// Free MACs that a master polls and that are within every master's
    /// Max_Master as far as the line shows, lowest first.
    pub(crate) candidates: Vec<u8>,
    /// The one offered first: the lowest above the highest master, else
    /// the lowest.
    pub(crate) preferred: usize,
    /// The Max_Master to join with, before taking the MAC into account.
    pub(crate) max_master: u8,
    /// What the line could not show.
    pub(crate) notes: Vec<String>,
}

impl Proposal {
    /// The Max_Master to join with as station `mac`: the ring's, and never
    /// below the station's own MAC.
    pub(crate) fn max_master_for(&self, mac: u8) -> u8 {
        self.max_master.max(mac)
    }
}

/// What the line proves about `master`'s Max_Master: an exact value as it
/// is; a lower bound raised to the master's own MAC and to its successor,
/// which it found by polling within its range (Clause 9.5.6.5, 9.5.6.6);
/// nothing for a master that never polled.
fn lower_bound(master: u8, station: &StationStats) -> Option<u8> {
    match station.max_master? {
        MaxMaster::Exact(v) => Some(v),
        MaxMaster::AtLeast(v) => Some(v.max(master).max(station.next_station.unwrap_or(0))),
    }
}

/// Whether `master` provably polls `mac`: `mac` lies between it and its
/// successor, within the Max_Master its polls showed. A master that never
/// passed the token (a sole master) polls all the way round to itself.
fn polled_by(master: u8, station: &StationStats, mac: u8) -> bool {
    let next = station.next_station.unwrap_or(master);
    if next > master {
        return mac > master && mac < next;
    }
    match station.max_master {
        Some(bound) if mac > master => mac <= bound.value(),
        // Past the wrap only once the wrap was seen.
        Some(MaxMaster::Exact(_)) => mac < next,
        _ => false,
    }
}

/// A free MAC to join as, and the Max_Master to join with.
///
/// # Errors
///
/// Why there is nothing to propose: no master heard yet, or no free MAC
/// that a master polls within every Max_Master the line shows.
pub(crate) fn propose(sample: &MonitorSample) -> Result<Proposal, String> {
    let stats = &sample.stats;
    let masters: Vec<(u8, &StationStats)> = masters(stats).collect();
    if masters.is_empty() {
        return Err(
            "No master has passed the token or polled yet: check the baud rate and \
                    the wiring, or listen longer."
                .into(),
        );
    }
    let taken = |mac: u8| {
        sample.devices.contains_key(&mac)
            || stats
                .station(mac)
                .is_some_and(|s| s.is_present() || addressed(s))
    };
    let bounds: Vec<u8> = masters
        .iter()
        .filter_map(|(mac, s)| lower_bound(*mac, s))
        .collect();
    let candidates: Vec<u8> = (0..=MAX_MASTER_MAC)
        .filter(|mac| !taken(*mac))
        .filter(|mac| bounds.iter().all(|bound| mac <= bound))
        .filter(|mac| masters.iter().any(|(m, s)| polled_by(*m, s, *mac)))
        .collect();
    if candidates.is_empty() {
        return Err(
            "No free MAC is both polled by a master and within every Max_Master \
                    seen: listen longer, or raise Max_Master on the masters."
                .into(),
        );
    }
    // Above the highest master, else in a gap above the lowest, else 0.
    let top = masters.iter().map(|(mac, _)| *mac).max().unwrap_or(0);
    let bottom = masters.iter().map(|(mac, _)| *mac).min().unwrap_or(0);
    let preferred = candidates
        .iter()
        .position(|mac| *mac > top)
        .or_else(|| candidates.iter().position(|mac| *mac > bottom))
        .unwrap_or(0);
    let exact = masters
        .iter()
        .filter_map(|(_, s)| match s.max_master {
            Some(MaxMaster::Exact(v)) => Some(v),
            _ => None,
        })
        .max();
    // The ring's own value, raised to reach every station heard: joining
    // shouldn't copy a Max_Master that leaves one out.
    let max_master = exact
        .into_iter()
        .chain(
            exact
                .is_none()
                .then(|| bounds.iter().copied().max())
                .flatten(),
        )
        .chain(highest_station(sample))
        .max()
        .unwrap_or(top);
    let mut notes = Vec::new();
    for (mac, station) in &masters {
        if station.max_master.is_none() {
            notes.push(format!(
                "mstp:{mac} polled nothing, so its Max_Master can't be seen: check it \
                 reaches the MAC you join as."
            ));
        }
    }
    if exact.is_none() {
        notes.push(
            "No master's polls have wrapped yet, so Max_Master is a lower bound: listen \
             longer for an exact value."
                .into(),
        );
    }
    Ok(Proposal {
        candidates,
        preferred,
        max_master,
        notes,
    })
}
