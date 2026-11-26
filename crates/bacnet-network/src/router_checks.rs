//! The rule a node changes a network's router by, without I/O (#2823,
//! #2831): what one router query (Who-Is-Router-To-Network, Clause 6.4.1)
//! found, the two strikes a known router is replaced on, and the pacing of
//! the checks that ask.
//!
//! The standard leaves the way a node learns a network's router to the
//! node (Clause 6.5.3), and none of the frames it could learn from is
//! authenticated. So a router this stack uses is learned, and replaced,
//! only from answers to queries of its own, by one rule:
//!
//! - **A router query** is broadcast on the local link for one network.
//!   Each I-Am-Router-To-Network (Clause 6.4.2) from a station on this link
//!   that lists the network is an answer ([`Routers::record`]). The query
//!   decides only when exactly one router answered it and no answer may
//!   have been lost ([`Answer::One`]); several, none, or a possible loss
//!   decide nothing.
//! - **Replacing a known router** takes two checks a minute apart
//!   ([`STRIKE_GAP`]), each finding the known router silent (to a request
//!   through it and to the query) and the same one other router alone
//!   answering, with nothing carried back through the known router since
//!   the first began ([`Strike::step`]). Anything else keeps the router and
//!   clears the strike.
//! - **Pacing** ([`Pacing`]): a key is checked at once when first cued,
//!   then at most once per hold, which starts at [`FIRST_HOLD`] and doubles
//!   to [`LAST_HOLD`] while cues keep coming; the store is bounded and
//!   ranked, so a cue for a new key never pushes out one that ranks higher.
//!
//! The client drives it for its discovery table's routed paths; the server
//! and the endpoint, through [`crate::router_learning`], for the routers
//! their own requests go through.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use bacnet_types::MacAddr;

use crate::layer::{NetworkReport, ReceivedNetworkReport};

/// How long after a key's first check the next may run. Each check that
/// leaves it as it was doubles the hold, up to [`LAST_HOLD`].
pub const FIRST_HOLD: Duration = Duration::from_secs(60);

/// The longest hold: as long as a silent row takes to age out of the
/// client's discovery table.
pub const LAST_HOLD: Duration = Duration::from_secs(600);

/// How long after a check that found the known router silent and one other
/// router alone answering the second check runs, whatever the hold: the
/// router changes only when that one finds the same.
pub const STRIKE_GAP: Duration = Duration::from_secs(60);

/// The routers that answered one router query for a network.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Routers {
    /// The router in use answered.
    pub old: bool,
    /// The others that answered, at most two: one decides, two refuse.
    pub others: Vec<MacAddr>,
}

impl Routers {
    /// Take `received` as an answer to a query for `network`, whose router
    /// in use is `old` (`None` while none is known): a router announcement
    /// (I-Am-Router-To-Network) from a station on this link (no SNET) that
    /// lists `network`. Anything else is not an answer.
    pub fn record(&mut self, received: &ReceivedNetworkReport, network: u16, old: Option<&[u8]>) {
        let NetworkReport::IAmRouterToNetwork(listed) = &received.report else {
            return;
        };
        if received.source_network.is_some() || !listed.contains(&network) {
            return;
        }
        if old.is_some_and(|old| received.source_mac.as_slice() == old) {
            self.old = true;
        } else if self.others.len() < 2 && !self.others.contains(&received.source_mac) {
            self.others.push(received.source_mac.clone());
        }
    }

    /// What the query found.
    pub fn answer(&self) -> Answer {
        if self.old {
            return Answer::Old;
        }
        match self.others.as_slice() {
            [] => Answer::Nobody,
            [one] => Answer::One(one.clone()),
            _ => Answer::Several,
        }
    }
}

/// What one router query found, read from its [`Routers`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The router in use answered: it still routes to the network.
    Old,
    /// No router answered.
    Nobody,
    /// Several other routers answered: none is taken.
    Several,
    /// Exactly one other router answered: the one a first learn takes, and
    /// a strike against the router in use.
    One(MacAddr),
}

impl Answer {
    /// Why an answer other than [`Answer::One`] keeps the router in use,
    /// for a log line; `None` for [`Answer::One`].
    pub fn kept_why(&self) -> Option<&'static str> {
        match self {
            Self::Old => Some("the router still answers for the network"),
            Self::Nobody => Some("no router answered for the network"),
            Self::Several => Some("several other routers answered for the network"),
            Self::One(_) => None,
        }
    }
}

/// A check that found the router in use silent and one other router alone
/// answering its router query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strike {
    /// The router that answered alone.
    pub to: MacAddr,
    /// When the check began: anything the old router carried from the
    /// network since then clears the strike.
    pub since: Instant,
}

/// What a check decided once one other router alone answered its query
/// ([`Strike::step`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The router in use carried something from the network since the
    /// first strike's check began (or this check's): it still works.
    Answered,
    /// The first strike: the second check runs [`STRIKE_GAP`] later.
    Struck(Strike),
    /// A second strike for another router than the first named: the
    /// router in use stays, and the strike clears.
    Differs,
    /// The second strike for the same router: the network's router is
    /// this one now.
    Moved(MacAddr),
}

impl Strike {
    /// Since when anything the router in use carried counts for it in a
    /// check that began at `began`: the first strike's start, when the
    /// last check made one, so a second check counts what came between
    /// the two.
    pub fn since(strike: Option<&Self>, began: Instant) -> Instant {
        strike.map_or(began, |strike| strike.since)
    }

    /// The step of a check that began at `began`, with the first strike
    /// `strike` held, once `to` alone answered its query. `heard` is
    /// whether the router in use carried anything from the network since
    /// [`Self::since`]; a caller with a lock reads it under the lock it
    /// moves the router under.
    pub fn step(strike: Option<&Self>, to: MacAddr, began: Instant, heard: bool) -> Step {
        if heard {
            return Step::Answered;
        }
        match strike {
            None => Step::Struck(Self { to, since: began }),
            Some(first) if first.to != to => Step::Differs,
            Some(_) => Step::Moved(to),
        }
    }
}

/// What a check decided, for its pacing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checked {
    /// The key stays, and a strike is cleared: the next check waits for a
    /// cue and the hold.
    Kept,
    /// The first strike: the second check runs [`STRIKE_GAP`] later.
    Struck(Strike),
    /// The check settled the key (the router moved, was learned, or nothing
    /// is left to check): it is forgotten.
    Ended,
}

/// One key's pacing.
#[derive(Debug)]
struct Paced<R> {
    /// A cue came since the last check.
    wanted: bool,
    /// The next check runs no sooner.
    due: Instant,
    /// The hold after the next check.
    hold: Duration,
    /// The rank when last cued or checked, for eviction.
    rank: R,
    /// The first strike, when the last check made one.
    strike: Option<Strike>,
}

/// The keys waiting for a check, each with its pacing, in a store of at
/// most `cap`, ranked by `R` (higher ranks first, and kept longer).
#[derive(Debug)]
pub struct Pacing<K, R> {
    keys: BTreeMap<K, Paced<R>>,
    cap: usize,
}

impl<K: Ord + Clone, R: Ord + Copy> Pacing<K, R> {
    /// An empty store of at most `cap` keys.
    pub const fn new(cap: usize) -> Self {
        Self {
            keys: BTreeMap::new(),
            cap,
        }
    }

    /// Mark `key`, ranked `rank`, for a check: at once when it is new. A
    /// new key when the store is full takes the place of the lowest-ranked
    /// one, if it outranks it, and is dropped otherwise.
    pub fn cue(&mut self, key: K, rank: R, now: Instant) {
        if let Some(paced) = self.keys.get_mut(&key) {
            paced.wanted = true;
            paced.rank = rank;
            return;
        }
        if self.keys.len() >= self.cap {
            let lowest = self
                .keys
                .iter()
                .min_by_key(|(_, paced)| paced.rank)
                .map(|(key, paced)| (key.clone(), paced.rank));
            match lowest {
                Some((lowest, below)) if below < rank => {
                    self.keys.remove(&lowest);
                }
                _ => return,
            }
        }
        self.keys.insert(
            key,
            Paced {
                wanted: true,
                due: now,
                hold: FIRST_HOLD,
                rank,
                strike: None,
            },
        );
    }

    /// Up to `limit` keys whose check is due at `now`, the highest ranked
    /// first. Each key is ranked again by `rank_of` first: a key it gives
    /// no rank, and a key left alone for a whole [`LAST_HOLD`] past its due,
    /// is forgotten.
    pub fn due(
        &mut self,
        now: Instant,
        limit: usize,
        mut rank_of: impl FnMut(&K) -> Option<R>,
    ) -> Vec<K> {
        self.keys.retain(|key, paced| {
            let Some(current) = rank_of(key) else {
                return false;
            };
            paced.rank = current;
            paced.wanted || paced.due.checked_add(LAST_HOLD).is_none_or(|end| now < end)
        });
        let mut due: Vec<(&K, &Paced<R>)> = self
            .keys
            .iter()
            .filter(|(_, paced)| paced.wanted && paced.due <= now)
            .collect();
        due.sort_by_key(|(_, paced)| std::cmp::Reverse(paced.rank));
        due.into_iter()
            .take(limit)
            .map(|(key, _)| key.clone())
            .collect()
    }

    /// When the next check is due: the earliest key with a cue waiting.
    pub fn next_due(&self) -> Option<Instant> {
        self.keys
            .values()
            .filter(|paced| paced.wanted)
            .map(|paced| paced.due)
            .min()
    }

    /// The first strike `key` holds, if its last check made one.
    pub fn strike(&self, key: &K) -> Option<Strike> {
        self.keys.get(key).and_then(|paced| paced.strike.clone())
    }

    /// Record a check of `key` that ended at `now`.
    pub fn checked(&mut self, key: &K, checked: Checked, now: Instant) {
        match checked {
            Checked::Ended => {
                self.keys.remove(key);
            }
            Checked::Kept => {
                if let Some(paced) = self.keys.get_mut(key) {
                    paced.strike = None;
                    paced.wanted = false;
                    paced.due = now + paced.hold;
                    paced.hold = (paced.hold * 2).min(LAST_HOLD);
                }
            }
            Checked::Struck(strike) => {
                if let Some(paced) = self.keys.get_mut(key) {
                    paced.strike = Some(strike);
                    paced.wanted = true;
                    paced.due = now + STRIKE_GAP;
                }
            }
        }
    }

    /// Clear the strike `key` holds: what it counted against has changed.
    pub fn clear_strike(&mut self, key: &K) {
        if let Some(paced) = self.keys.get_mut(key) {
            paced.strike = None;
        }
    }

    /// Whether `key` is waiting for a check, or paced after one.
    pub fn contains(&self, key: &K) -> bool {
        self.keys.contains_key(key)
    }

    /// Forget every key.
    pub fn clear(&mut self) {
        self.keys.clear();
    }
}

#[cfg(test)]
#[path = "router_checks_tests.rs"]
mod tests;
