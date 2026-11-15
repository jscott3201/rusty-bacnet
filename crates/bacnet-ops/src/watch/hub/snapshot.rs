//! A point's [`Snapshot`] as the hub keeps it: its latest rows and how its
//! watch goes ([`Phase`], [`Source`], [`Why`]).

use tokio::time::Instant;

use super::super::Why;
use super::super::sample::Via;
use super::super::support::CovSupport;
use crate::format::RawRow;

/// Where a point's values come from now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// COV notifications under a managed subscription.
    Cov,
    /// Polls, because the device refused COV or stopped renewing it.
    Poll,
}

/// How a point's watch is going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Its first read hasn't finished.
    Starting,
    /// Subscribed, or the last poll answered.
    Live,
    /// The last read failed as a whole without the device going quiet:
    /// it answered with an error for the whole object, or the request was
    /// never sent (the link refused it, or no route was left), whether the
    /// watch reads one property or several (#2610). See
    /// [`Snapshot::note`], which holds the error's text.
    Failed,
    /// The device didn't answer the last request.
    Unreachable,
    /// The watch ended (the hub shut down).
    Stopped,
    /// Nothing watches it any more: no route to its address is known and
    /// [`WatchOptions::missing`](super::WatchOptions::missing) says not to
    /// wait ([`Why::Missing`]), or its device answers at another address
    /// now ([`Why::Moved`]). See [`Snapshot::note`].
    Gone,
}

/// A point's latest values and how its watch is going.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Each property's latest raw value, or why the device gave none.
    pub rows: Vec<RawRow>,
    /// Where the values come from; `None` before the first read.
    pub source: Option<Source>,
    /// How the latest values came; `None` before any.
    pub via: Option<Via>,
    /// How the watch is going.
    pub state: Phase,
    /// Why the point is polled or not live, as text.
    pub note: Option<String>,
    /// The hub's sequence number at the last change; 0 before any.
    pub seq: u64,
    /// When the values last changed.
    pub updated: Option<Instant>,
    /// COV notifications applied.
    pub notifications: u64,
    /// Changes folded into another's signal.
    pub coalesced: u64,
    /// Polls answered.
    pub polls: u64,
    /// Renewals the device took.
    pub renewals: u64,
    /// Why the point is polled, or was given up, as its start, a failed
    /// renewal or a move found; a poll's error, which goes to
    /// [`note`](Self::note), leaves it. `None` while subscribed, and
    /// before the first start ends.
    pub why: Option<Why>,
    /// What is known of COV for this point: the device's own answer to
    /// its subscription once asked, before that what the device declared
    /// or did for its other points.
    pub cov: CovSupport,
}

impl Default for Snapshot {
    /// A point not read yet: starting, no rows, nothing counted.
    fn default() -> Self {
        Self::new()
    }
}

impl Snapshot {
    pub(in crate::watch) fn new() -> Self {
        Self {
            rows: Vec::new(),
            source: None,
            via: None,
            state: Phase::Starting,
            note: None,
            seq: 0,
            updated: None,
            notifications: 0,
            coalesced: 0,
            polls: 0,
            renewals: 0,
            why: None,
            cov: CovSupport::Unknown,
        }
    }
}
