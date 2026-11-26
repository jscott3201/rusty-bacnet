//! The router a node's own confirmed requests to a network go through,
//! learned and replaced only through the node's own router query (#2831).
//!
//! The server and the endpoint send requests of their own to stations on
//! other networks (event notifications to an address on another network, a
//! node's source Audit records). The router such a request goes through is
//! a [`RouterLearning`]'s, by the rule in [`crate::router_checks`]:
//!
//! - **No router known:** the request goes with the link's broadcast
//!   address (Clause 6.5.3) and the owner says so ([`RouterLearning::unrouted`]).
//!   The learner then broadcasts its router query (Who-Is-Router-To-Network,
//!   Clause 6.4.1) for the network, at once the first time, then paced, and
//!   takes the router only when exactly one answers it and no answer may
//!   have been lost.
//! - **A router known:** requests go through it. Silence does not forget
//!   it. When a request through it draws no answer, the owner says so
//!   ([`RouterLearning::silent`]) and the learner checks it: a Who-Is to
//!   that station through the router, then, only if nothing came back
//!   through it, the router query. It moves to another router only after
//!   two such checks a minute apart find the known router silent and the
//!   same one other router alone answering, with nothing carried back
//!   through the known router in between ([`crate::router_checks::Strike`]).
//! - **Nothing else teaches a router:** not the answers to the owner's
//!   requests, nor the sources of requests that arrive, nor a router
//!   announcement (I-Am-Router-To-Network) the learner did not ask for.
//!   Frames the known router carries back from its network only keep it
//!   ([`RouterLearning::heard`]).
//!
//! A router query is a network-layer message, not a service the node
//! initiates, so it goes out while DeviceCommunicationControl restricts
//! initiation (Clause 16.1); the Who-Is of a check does not, so no known
//! router is checked, or replaced, meanwhile.
//!
//! A station on the link that is the only one to answer the router query
//! twice, a minute apart, while the known router carries nothing back is
//! taken: the check can't tell it from a real replacement.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bacnet_npdu::{NetworkMessageType, NetworkPriority};
use bacnet_transport::port::TransportPort;
use bacnet_types::MacAddr;
use bacnet_types::error::Error;
use bacnet_types::sync::MutexExt;
use tokio::sync::{Notify, broadcast};
use tracing::{debug, warn};

use crate::layer::{
    NetworkLayer, QueueAdmissionCounters, ReceivedNetworkControl, ReceivedNetworkReport,
};
use crate::router_checks::{Answer, Checked, Pacing, Routers, Step, Strike};

/// The most networks a learner tracks. A network past it is never learned:
/// requests to it keep going with the link's broadcast address.
pub const MAX_NETWORKS: usize = 64;

/// The most checks one wake of [`RouterLearning::run`] runs.
pub const CHECKS_PER_WAKE: usize = 4;

/// How many router reports wait for a slow learner before it misses some
/// (and takes that query's answers as possibly lost).
const REPORT_CAPACITY: usize = 64;

/// The learners' clock: Tokio's, which reads the system's monotonic clock
/// unless a test has paused it.
fn now() -> Instant {
    tokio::time::Instant::now().into_std()
}

/// The router announcements (I-Am-Router-To-Network) a link received,
/// shared out to the learners on it, with a count of the controls its queue
/// turned away: a learner waiting for answers can tell that one may have
/// been lost.
#[derive(Clone, Debug)]
pub struct RouterReports {
    tx: broadcast::Sender<ReceivedNetworkReport>,
    drops: Option<QueueAdmissionCounters>,
}

impl RouterReports {
    /// Reports from a control queue whose turned-away controls `drops`
    /// counts: `None` when the owner can't know, which makes every query's
    /// answers possibly lost, so nothing is ever learned from them.
    pub fn new(drops: Option<QueueAdmissionCounters>) -> Self {
        Self {
            tx: broadcast::channel(REPORT_CAPACITY).0,
            drops,
        }
    }

    /// Hand `control` to the learners, when it is a router announcement
    /// (I-Am-Router-To-Network) and one is waiting for answers.
    pub fn offer(&self, control: &ReceivedNetworkControl) {
        if control.npdu.message_type != Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK)
            || self.tx.receiver_count() == 0
        {
            return;
        }
        if let Some(report) = ReceivedNetworkReport::parse(control) {
            let _ = self.tx.send(report);
        }
    }

    /// The controls turned away so far: `None` when the owner can't know.
    fn dropped(&self) -> Option<u64> {
        self.drops.as_ref().map(|drops| {
            let counts = drops.snapshot();
            counts.full_drops.saturating_add(counts.closed_drops)
        })
    }
}

/// One network a learner tracks.
#[derive(Debug)]
struct Network {
    /// The router requests to it go through.
    router: Option<MacAddr>,
    /// When the router last carried a frame from the network.
    heard: Option<Instant>,
    /// The station a check asks through the router: the one a request
    /// through it last went to without an answer.
    probe: Option<MacAddr>,
    /// When the network was first tracked.
    since: Instant,
}

/// How much a network's check matters: one whose router is known first,
/// then the one tracked longest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Rank {
    routed: bool,
    oldest: Reverse<Instant>,
}

impl Network {
    fn rank(&self) -> Rank {
        Rank {
            routed: self.router.is_some(),
            oldest: Reverse(self.since),
        }
    }
}

#[derive(Debug)]
struct Book {
    networks: BTreeMap<u16, Network>,
    pacing: Pacing<u16, Rank>,
}

impl Book {
    /// `network`'s entry, made while there is room.
    fn track(&mut self, network: u16, at: Instant) -> Option<&mut Network> {
        if false {
            return None;
        }
        Some(self.networks.entry(network).or_insert(Network {
            router: None,
            heard: None,
            probe: None,
            since: at,
        }))
    }

    fn cue(&mut self, network: u16, at: Instant) {
        if let Some(rank) = self.networks.get(&network).map(Network::rank) {
            self.pacing.cue(network, rank, at);
        }
    }

    /// Whether `router`, still `network`'s, carried a frame since `since`.
    fn heard_since(&self, network: u16, router: &MacAddr, since: Instant) -> bool {
        self.networks.get(&network).is_some_and(|entry| {
            entry.router.as_ref() == Some(router) && entry.heard.is_some_and(|heard| heard >= since)
        })
    }
}

/// The routers a node's own requests to other networks go through, with
/// the checks that learn and replace them (see the [module docs](self)).
/// The owner sends its requests by [`Self::router`], tells the learner
/// what it saw ([`Self::unrouted`], [`Self::silent`], [`Self::heard`]) and
/// runs [`Self::run`] beside them.
#[derive(Debug)]
pub struct RouterLearning {
    book: Mutex<Book>,
    cue: Notify,
}

impl Default for RouterLearning {
    fn default() -> Self {
        Self::new()
    }
}

impl RouterLearning {
    /// A learner that knows no router yet.
    pub fn new() -> Self {
        Self {
            book: Mutex::new(Book {
                networks: BTreeMap::new(),
                pacing: Pacing::new(MAX_NETWORKS),
            }),
            cue: Notify::new(),
        }
    }

    /// The router requests to `network` go through, if one is known.
    pub fn router(&self, network: u16) -> Option<MacAddr> {
        self.book
            .locked()
            .networks
            .get(&network)
            .and_then(|entry| entry.router.clone())
    }

    /// A request to `network` went with the link's broadcast address for
    /// want of a router: ask which router reaches it, at once the first
    /// time, then at most once per hold.
    pub fn unrouted(&self, network: u16) {
        let at = now();
        let mut book = self.book.locked();
        if book
            .track(network, at)
            .is_some_and(|entry| entry.router.is_none())
        {
            book.cue(network, at);
            drop(book);
            self.cue.notify_one();
        }
    }

    /// A request to `station` on `network` sent through `router` drew no
    /// answer: check that router, while it is still the network's.
    pub fn silent(&self, network: u16, router: &[u8], station: &[u8]) {
        let at = now();
        let mut book = self.book.locked();
        let Some(entry) = book.networks.get_mut(&network) else {
            return;
        };
        if entry.router.as_deref() != Some(router) {
            return;
        }
        entry.probe = Some(MacAddr::from_slice(station));
        book.cue(network, at);
        drop(book);
        self.cue.notify_one();
    }

    /// A frame from `network` arrived from link station `from`: when that
    /// is the network's router, it still carries the network's traffic. It
    /// teaches nothing else.
    pub fn heard(&self, network: u16, from: &[u8]) {
        let mut book = self.book.locked();
        if let Some(entry) = book.networks.get_mut(&network)
            && entry.router.as_deref() == Some(from)
        {
            entry.heard = Some(now());
        }
    }

    /// Take `router` as `network`'s router at once, as a router query
    /// would: for tests of an owner's sends through a known router, this
    /// crate's and other crates'. Not for production builds.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn learn_for_test(&self, network: u16, router: &[u8]) {
        let mut book = self.book.locked();
        if let Some(entry) = book.track(network, now()) {
            entry.router = Some(MacAddr::from_slice(router));
            entry.heard = None;
        }
    }

    /// Run the checks as they fall due, for as long as the owner polls
    /// this: on `network`'s link, with the router answers `reports` shares
    /// out, each step waiting `wait` (the owner's APDU timeout) for its
    /// answers. `may_initiate` is whether the node may initiate requests
    /// now (DeviceCommunicationControl). No lock is held across an await,
    /// so the owner stops it by aborting its task at any point.
    pub async fn run<T: TransportPort + 'static>(
        &self,
        network: &NetworkLayer<T>,
        reports: &RouterReports,
        may_initiate: impl Fn() -> bool + Sync,
        wait: Duration,
    ) {
        loop {
            let due = self.book.locked().pacing.next_due();
            tokio::select! {
                () = self.cue.notified() => {}
                () = sleep_until_due(due) => {}
            }
            let due = {
                let mut book = self.book.locked();
                let Book { networks, pacing } = &mut *book;
                let due = pacing.due(now(), CHECKS_PER_WAKE, |network| {
                    networks.get(network).map(Network::rank)
                });
                // A network with no router whose check the pacing has let
                // go of (no request wanted it for a whole hold) goes too,
                // so the room is there for others.
                networks.retain(|dnet, entry| entry.router.is_some() || pacing.contains(dnet));
                due
            };
            for dnet in due {
                let checked = self
                    .check(network, reports, &may_initiate, wait, dnet)
                    .await;
                self.book.locked().pacing.checked(&dnet, checked, now());
            }
        }
    }

    /// Check `dnet`: learn its router, or check the one known.
    async fn check<T: TransportPort + 'static>(
        &self,
        network: &NetworkLayer<T>,
        reports: &RouterReports,
        may_initiate: &(impl Fn() -> bool + Sync),
        wait: Duration,
        dnet: u16,
    ) -> Checked {
        let (old, probe, strike) = {
            let book = self.book.locked();
            let Some(entry) = book.networks.get(&dnet) else {
                return Checked::Ended;
            };
            (
                entry.router.clone(),
                entry.probe.clone(),
                book.pacing.strike(&dnet),
            )
        };
        let Some(old) = old else {
            return self.learn(network, reports, wait, dnet).await;
        };
        // The Who-Is is a service the node initiates: none while it may not
        // (#2617), and with nothing asked, nothing moves.
        let Some(probe) = probe.filter(|_| may_initiate()) else {
            return Checked::Kept;
        };
        let began = now();
        let since = Strike::since(strike.as_ref(), began);
        if let Err(error) = network
            .send_apdu_routed(&WHO_IS, dnet, &probe, &old, false, NetworkPriority::Normal)
            .await
        {
            debug!(%error, network = dnet, "a router check sent no Who-Is; the router stays");
            return Checked::Kept;
        }
        tokio::time::sleep(wait).await;
        if self.book.locked().heard_since(dnet, &old, since) {
            return Checked::Kept;
        }
        let routers = match routers_to(network, reports, dnet, Some(&old), wait).await {
            Ok(Some(routers)) => routers,
            Ok(None) => {
                debug!(
                    network = dnet,
                    "router answers may have been lost; the router stays"
                );
                return Checked::Kept;
            }
            Err(error) => {
                debug!(%error, network = dnet, "a router check sent no router query; the router stays");
                return Checked::Kept;
            }
        };
        let to = match routers.answer() {
            Answer::One(to) => to,
            answer => {
                debug!(network = dnet, why = answer.kept_why(), "the router stays");
                return Checked::Kept;
            }
        };
        let mut book = self.book.locked();
        let heard = book.heard_since(dnet, &old, since);
        match Strike::step(strike.as_ref(), to, began, heard) {
            Step::Answered | Step::Differs => Checked::Kept,
            Step::Struck(strike) => Checked::Struck(strike),
            Step::Moved(to) => {
                if let Some(entry) = book.networks.get_mut(&dnet) {
                    entry.router = Some(to.clone());
                    entry.heard = None;
                    entry.probe = None;
                }
                drop(book);
                warn!(
                    network = dnet,
                    from = ?old,
                    to = ?to,
                    "requests to network {dnet} now go through another router: in two checks a \
                     minute apart the old one carried no answer to this node's Who-Is or router \
                     query, and the same other router alone answered"
                );
                Checked::Ended
            }
        }
    }

    /// Ask which router reaches `dnet`, none being known, and take it when
    /// exactly one answers and nothing may have been lost.
    async fn learn<T: TransportPort + 'static>(
        &self,
        network: &NetworkLayer<T>,
        reports: &RouterReports,
        wait: Duration,
        dnet: u16,
    ) -> Checked {
        let routers = match routers_to(network, reports, dnet, None, wait).await {
            Ok(Some(routers)) => routers,
            Ok(None) => {
                debug!(
                    network = dnet,
                    "router answers may have been lost; none learned"
                );
                return Checked::Kept;
            }
            Err(error) => {
                debug!(%error, network = dnet, "no router query sent; none learned");
                return Checked::Kept;
            }
        };
        let Answer::One(router) = routers.answer() else {
            debug!(
                network = dnet,
                answers = routers.others.len(),
                "no one router alone answered the router query; requests keep going by broadcast"
            );
            return Checked::Kept;
        };
        let mut book = self.book.locked();
        let Some(entry) = book.networks.get_mut(&dnet) else {
            return Checked::Ended;
        };
        entry.router = Some(router.clone());
        entry.heard = None;
        drop(book);
        debug!(network = dnet, router = ?router, "learned the router to a network from the router query");
        Checked::Ended
    }
}

/// A Who-Is with no range.
const WHO_IS: [u8; 2] = [0x10, 0x08];

/// Broadcast the router query for `dnet` and collect, for `wait`, the
/// routers on this link that answer for it: `None` when an answer may have
/// been lost, to a lagging receiver or to the link's control queue.
async fn routers_to<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    reports: &RouterReports,
    dnet: u16,
    old: Option<&MacAddr>,
    wait: Duration,
) -> Result<Option<Routers>, Error> {
    let mut rx = reports.tx.subscribe();
    let dropped = reports.dropped();
    network
        .send_network_message(
            NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK,
            &dnet.to_be_bytes(),
            None,
        )
        .await?;
    let deadline = tokio::time::Instant::now() + wait;
    let mut routers = Routers::default();
    let mut lost = false;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(received)) => routers.record(&received, dnet, old.map(MacAddr::as_slice)),
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => lost = true,
            Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => break,
        }
    }
    let whole = !lost
        && matches!((dropped, reports.dropped()), (Some(before), Some(after)) if before == after);
    Ok(whole.then_some(routers))
}

/// Sleep until `due`, or forever with nothing due.
async fn sleep_until_due(due: Option<Instant>) {
    match due {
        Some(due) => tokio::time::sleep_until(tokio::time::Instant::from_std(due)).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
#[path = "router_learning_tests.rs"]
mod tests;
