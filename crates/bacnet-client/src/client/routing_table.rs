//! The read-only routing-table query (#2612): ask one router on the local
//! link or behind a selected next hop for its table and collect the acknowledgements.
//!
//! The query is an Initialize-Routing-Table with a port count of zero, which
//! asks a router to return its table and change nothing (Clauses 6.4.7 and
//! 6.6.3.8). The network layer builds it with that one payload and no
//! caller can name another, so a table update cannot be sent from here.
//!
//! A table too long for one Initialize-Routing-Table-Ack comes as several,
//! and nothing marks the last (Clause 6.6.3.9). The call therefore collects
//! acks until a quiet interval, inside a total bound and an entry bound,
//! and says whether it stopped at a bound.
//!
use super::*;
use bacnet_network::layer::{NetworkReport, RoutingTableEntry};
use bacnet_npdu::{RejectMessageReason, RejectMessageToNetwork};
use bacnet_types::sync::MutexExt;
use tokio::sync::broadcast::error::RecvError;

use crate::poll_gate::PollGate;

/// Most entries a [`BACnetClient::routing_table`] call keeps by default.
/// A network number is 16 bits, so no table lists more than 65,534
/// networks; the default is far above any real table and bounds memory
/// against a router that never stops answering.
pub const DEFAULT_ROUTING_TABLE_MAX_ENTRIES: usize = 4096;

/// The bounds of a [`BACnetClient::routing_table`] call. `first_answer` may
/// not exceed `total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingTableWait {
    /// How long to wait for the first acknowledgement (or a reject) before
    /// giving up with [`RoutingTableError::NoAnswer`]. Default 3 seconds.
    pub first_answer: Duration,
    /// How long after the last accepted acknowledgement to wait for the
    /// next, with none ending the call (other reports do not restart it).
    /// Default 500 milliseconds.
    pub quiet: Duration,
    /// The longest the whole call listens, from the send. When it ends
    /// the call returns what arrived, marked
    /// [`RoutingTable::truncated`]. Default 10 seconds.
    pub total: Duration,
    /// The most entries kept; more end the call as truncated. Default
    /// [`DEFAULT_ROUTING_TABLE_MAX_ENTRIES`]. At least 1.
    pub max_entries: usize,
}

impl Default for RoutingTableWait {
    fn default() -> Self {
        Self {
            first_answer: Duration::from_secs(3),
            quiet: Duration::from_millis(500),
            total: Duration::from_secs(10),
            max_entries: DEFAULT_ROUTING_TABLE_MAX_ENTRIES,
        }
    }
}

/// A router's table, as [`BACnetClient::routing_table`] collected it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingTable {
    /// The entries, in the order the router listed them, across every
    /// acknowledgement. An empty list is a router that answered with an
    /// empty table: a router that did not answer at all is
    /// [`RoutingTableError::NoAnswer`].
    pub entries: Vec<RoutingTableEntry>,
    /// How many acknowledgements carried them.
    pub acks: usize,
    /// Whether the call stopped at the total bound or the entry bound, or
    /// lost acknowledgements because it fell behind, rather than after a
    /// quiet interval. It covers those cases only: a quiet end does not
    /// prove the table whole (on MS/TP, acks after the first wait for the
    /// router's token, Clause 9), so a slow trunk wants a larger
    /// [`RoutingTableWait::quiet`]. A late ack of an earlier query to the
    /// same router can also be counted by the next one (nothing pairs
    /// answers with questions), so leave a gap between repeated queries;
    /// entries are reported as received, so a network number can appear
    /// twice.
    pub truncated: bool,
}

/// Why [`BACnetClient::routing_table`] returned no table.
#[derive(Debug)]
#[non_exhaustive]
pub enum RoutingTableError {
    /// The router sent no acknowledgement and no reject within the first
    /// answer bound. A router that ignores the query (an older one, say)
    /// looks the same as one that is not there, so this does not say
    /// which. On B/IP, a router that answers from another address or port
    /// than the one queried shows up here too. It is not an empty table.
    ///
    /// For a routed query, an older router may answer a table query only
    /// on its own link: it then answers the next hop, never this client,
    /// so the call ends here as well. Nothing else tells the two apart.
    /// So does a router whose control gate refuses table queries from
    /// other networks: a routed query reaches it with an SNET, and it drops
    /// the query silently. A hosted router on the hardened control policy
    /// with no authorizer does this, as does a `bacnet-router-core` host
    /// that refuses `ControlClass::InitQuery` (#2715).
    /// A far router that rejects the query as an unknown message (reason
    /// 3) can end here too: its reject arrives with its near-side network
    /// as SNET and names a network of its own choosing (Clause 6.4.4).
    NoAnswer {
        /// How long the call waited.
        waited: Duration,
        /// The queried router's network, for a routed query.
        routed: Option<u16>,
    },
    /// The queried station rejected the network message before the first
    /// acknowledgement, or its selected next hop rejected forwarding to
    /// the requested DNET.
    Rejected(RejectMessageToNetwork),
    /// The call's arguments were refused and nothing was sent: no entries
    /// kept, a first-answer wait longer than the total, or a router
    /// address that names no one station.
    InvalidArgument(String),
    /// The client's dispatch task ended while the call waited (the client
    /// stopped, its link closed or a seat's node stopped). That is not
    /// silence from the router.
    ClientStopped,
    /// No unique local next hop advertised the requested router's network.
    RouteUnresolved {
        /// The destination network.
        network: u16,
        /// The number of distinct local peers that advertised it.
        candidates: usize,
    },
    /// Reports were lost during router discovery, so the client could not
    /// establish that a single next hop advertised the destination.
    DiscoveryIncomplete {
        /// The destination network.
        network: u16,
        /// Network reports lost before the collector read them.
        lost: u64,
    },
    /// The query was not sent: the link failed, refused it for its own
    /// state, configuration, platform or limits (a BACnet/SC link not
    /// connected to its hub, say), or the client's node had stopped.
    Send(Error),
}

impl core::fmt::Display for RoutingTableError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoAnswer {
                waited,
                routed: None,
            } => write!(
                f,
                "the router sent no routing table within {waited:?}: it did not \
                 answer the query from the expected address"
            ),
            Self::NoAnswer {
                waited,
                routed: Some(network),
            } => write!(
                f,
                "the router on network {network} sent no routing table back through \
                 the next hop within {waited:?}: it may be absent, or an older router \
                 that answers a table query only on its own link; query the next hop, \
                 or ask from a node on network {network}"
            ),
            Self::Rejected(reject) => write!(
                f,
                "the router rejected the routing-table query: {} (network {})",
                reject.reason, reject.network
            ),
            Self::InvalidArgument(why) => write!(f, "routing-table query refused: {why}"),
            Self::ClientStopped => f.write_str("the client stopped during the routing-table query"),
            Self::DiscoveryIncomplete { network, lost } => write!(
                f,
                "lost {lost} reports discovering routers for network {network}. Specify an explicit next hop"
            ),
            Self::RouteUnresolved {
                network,
                candidates,
            } => write!(
                f,
                "found {candidates} local routers to network {network}. Specify an explicit next hop"
            ),
            Self::Send(error) => write!(f, "routing-table query not sent: {error}"),
        }
    }
}

impl std::error::Error for RoutingTableError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Send(error) => Some(error),
            _ => None,
        }
    }
}

impl From<Error> for RoutingTableError {
    fn from(error: Error) -> Self {
        Self::Send(error)
    }
}

/// What [`BACnetClient::routing_table`] shares with the dispatch task.
#[derive(Debug)]
pub(super) struct RoutingQueries {
    /// The expected peer and station. The dispatch task can treat a
    /// data-less Initialize-Routing-Table-Ack from this source as an empty
    /// table. Locked only for an ack with no data, never per APDU.
    pub(super) outstanding: std::sync::Mutex<Option<QueryTarget>>,
    /// Ends (its sender is dropped) when the dispatch task does, however it
    /// ends: stop, abort, a closed link or a seat's node stopping. A call
    /// waits on it, so it returns `ClientStopped` at once, and so does a
    /// confirmed request waiting for its pacing turn (#2720). It is never
    /// sent on, so it costs nothing per APDU.
    pub(super) ended: tokio::sync::watch::Receiver<()>,
    /// One routing-table query at a time. Network messages carry no invoke
    /// ID, so two queries to one router would each collect both answers.
    /// The gate guards no data: holding it across the call's awaits is its
    /// whole purpose. A call takes it only as it is polled (#2759).
    gate: PollGate,
}

impl RoutingQueries {
    /// State for a client whose dispatch task holds the other end of
    /// `ended`.
    pub(super) fn new(ended: tokio::sync::watch::Receiver<()>) -> Self {
        Self {
            outstanding: std::sync::Mutex::new(None),
            ended,
            gate: PollGate::default(),
        }
    }
}

/// Clears the "query outstanding" router when the call ends, however it
/// ends.
struct Outstanding<'a>(&'a std::sync::Mutex<Option<QueryTarget>>);

impl Drop for Outstanding<'_> {
    fn drop(&mut self) {
        *self.0.locked() = None;
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Read the routing table of a local or routed router station.
    ///
    /// Sends an Initialize-Routing-Table with a port count of zero
    /// (Clauses 6.4.7 and 6.6.3.8) and collects the
    /// Initialize-Routing-Table-Acks the router returns, as
    /// [`NetworkReport::RoutingTable`] messages from `router`. The data is
    /// fixed by the network layer: this call cannot update a table.
    ///
    /// A table too long for one ack comes as several with no marker for the
    /// last (Clause 6.6.3.9), so the call returns when `wait.quiet` has
    /// passed since the last accepted ack (other reports do not restart
    /// it), or at `wait.total`, or at `wait.max_entries`; the last two are
    /// reported as [`RoutingTable::truncated`]. A quiet end does not prove
    /// the table whole: on MS/TP the acks after the first wait for the
    /// router's token (Clause 9), so use a larger `quiet` on a slow trunk.
    ///
    /// Outcomes:
    /// - acks arrived: `Ok`, the entries in the router's order. A router
    ///   whose table is empty answers once with no entries: `Ok` with an
    ///   empty list. An older router's ack with no data at all is also an
    ///   empty table: this client never sends the update form, so while
    ///   its query is outstanding such an ack can only answer the query.
    /// - nothing came from `router` within `wait.first_answer`:
    ///   [`RoutingTableError::NoAnswer`]. An older router that ignores the
    ///   message and an absent one both end here; neither is an empty
    ///   table.
    /// - a Reject-Message-To-Network from `router` with a reason that can
    ///   answer a local message (0, 3, 6 or an unknown value) came before
    ///   any ack: [`RoutingTableError::Rejected`]. Rejects for a DNET
    ///   (reasons 1, 2 and 4) are ignored for a local query, as is any
    ///   reject after the first ack.
    /// - [`RoutingTableError::InvalidArgument`] for `wait.max_entries` 0,
    ///   `wait.first_answer` longer than `wait.total`, or a `router` that
    ///   names no one station (empty, a broadcast or group address, or
    ///   malformed for the link); nothing is sent.
    /// - [`RoutingTableError::ClientStopped`] when the client stops
    ///   mid-call.
    ///
    /// A routed station can name its next hop, or use the station's cached
    /// route. Without either, a Who-Is-Router-To-Network asks for local
    /// routers advertising the DNET. This takes one additional first-answer
    /// interval and requires exactly one peer, or returns
    /// [`RoutingTableError::RouteUnresolved`]. No Device I-Am is required.
    /// A routed ack must arrive from the selected peer with the original
    /// router's SNET/SADR. A device which only answers locally may time out.
    /// A reject that comes through that peer naming the requested DNET
    /// ends a routed query before its first ack with
    /// [`RoutingTableError::Rejected`], whether the peer sent it (no SNET)
    /// or relayed it from a router further on (that router's SNET), since
    /// a router relays a reject back toward the requester (Clause 6.6.3.5).
    /// Reason 4 (message too long) is ignored: the query is too short to
    /// draw it, so it answers a larger request to that network.
    ///
    /// Nothing pairs an answer with its question, so a late ack of an
    /// earlier query can be counted by the next query to the same router,
    /// and a reject from the selected hop naming the same DNET may also
    /// concern a concurrent routed request. For repeated table queries,
    /// leave a gap between repeated queries. Entries are reported as
    /// received, not deduplicated, so a network number can appear twice.
    ///
    /// Network messages carry no invoke ID, so the client runs one
    /// routing-table query at a time: a second call waits for the first,
    /// and its clock starts when it reaches the gate. A call left unpolled
    /// while it waits its turn holds nothing and is passed over until it is
    /// polled again (#2759). Calls polled when woken go in the order they
    /// began to wait: strictly on a current-thread runtime, while on a
    /// multi-thread runtime one slow to be polled can be passed. A call that
    /// has the gate holds it until it returns or is dropped, polled or not:
    /// drive it, or wrap it in a timeout. With `wait.first_answer` equal to
    /// `wait.total`, a complete answer that arrives at the end can be marked
    /// [`RoutingTable::truncated`]. Every bound must fit the clock
    /// (`InvalidArgument` otherwise).
    pub async fn routing_table(
        &self,
        router: &(impl ToStation + ?Sized),
        wait: RoutingTableWait,
    ) -> Result<RoutingTable, RoutingTableError> {
        if wait.max_entries == 0 {
            return Err(RoutingTableError::InvalidArgument(
                "at least one entry must be kept".into(),
            ));
        }
        if wait.first_answer > wait.total {
            return Err(RoutingTableError::InvalidArgument(format!(
                "the first-answer wait ({:?}) is longer than the total ({:?})",
                wait.first_answer, wait.total
            )));
        }
        // Held for the whole call, across its awaits: it guards no data.
        let _one_at_a_time = self.routing_query.gate.lock().await;
        // The call's clock starts here, once it has the gate. Every instant
        // is checked: a bound too large for the clock is an argument error,
        // with nothing sent.
        let start = tokio::time::Instant::now();
        let overflow =
            || RoutingTableError::InvalidArgument("a wait is too long for the clock".into());
        start.checked_add(wait.total).ok_or_else(overflow)?;
        start.checked_add(wait.quiet).ok_or_else(overflow)?;
        let mut ended = self.routing_query.ended.clone();
        if ended.has_changed().is_err() {
            return Err(RoutingTableError::ClientStopped);
        }
        self.link_open()?;
        let target = tokio::select! {
            result = self.table_query_target(router.station(), wait.first_answer) => result?,
            _ = ended.changed() => return Err(RoutingTableError::ClientStopped),
        };
        // Route resolution has its own first-answer window. The collection
        // clocks start here, after a next hop has been selected.
        let start = tokio::time::Instant::now();
        let end = start.checked_add(wait.total).ok_or_else(overflow)?;
        let first_deadline = start.checked_add(wait.first_answer).ok_or_else(overflow)?;
        let mut rx = self.network_reports();
        *self.routing_query.outstanding.locked() = Some(target.clone());
        let _outstanding = Outstanding(&self.routing_query.outstanding);
        self.network
            .send_routing_table_query(&target.hop, target.station.as_ref())
            .await
            .map_err(|error| match error {
                // The router's address in a form the link refuses.
                Error::InvalidArgument(refusal) => {
                    RoutingTableError::InvalidArgument(refusal.to_string())
                }
                Error::Transport(e) if e.kind() == std::io::ErrorKind::InvalidInput => {
                    RoutingTableError::InvalidArgument(e.to_string())
                }
                // The link's state, configuration, platform or limits
                // refused the send, which no argument of the call caused,
                // such as a BACnet/SC link not connected to its hub (#1833).
                other => RoutingTableError::Send(other),
            })?;
        let mut last_ack = start;
        let mut table = RoutingTable {
            entries: Vec::new(),
            acks: 0,
            truncated: false,
        };
        loop {
            let deadline = if table.acks == 0 {
                first_deadline
            } else {
                last_ack
                    .checked_add(wait.quiet)
                    .map_or(end, |at| at.min(end))
            };
            let step = tokio::select! {
                step = tokio::time::timeout_at(deadline, rx.recv()) => step,
                // The sender is never used: this resolves only when the
                // dispatch task has ended.
                _ = ended.changed() => return Err(RoutingTableError::ClientStopped),
            };
            let received = match step {
                Ok(Ok(received)) => received,
                Ok(Err(RecvError::Lagged(missed))) => {
                    warn!(
                        missed,
                        "routing-table collector fell behind; acks were lost"
                    );
                    table.truncated = true;
                    continue;
                }
                Ok(Err(RecvError::Closed)) => return Err(RoutingTableError::ClientStopped),
                Err(_) => {
                    if table.acks == 0 {
                        return Err(RoutingTableError::NoAnswer {
                            waited: deadline.saturating_duration_since(start),
                            routed: target.station.as_ref().map(|station| station.network),
                        });
                    }
                    // The quiet interval is the normal end; the total is a
                    // bound that cut the answer short.
                    table.truncated |= deadline >= end;
                    break;
                }
            };
            match received.report {
                NetworkReport::RoutingTable(entries)
                    if target.answers(&received.source_mac, received.source_network.as_ref()) =>
                {
                    table.acks += 1;
                    last_ack = tokio::time::Instant::now();
                    let room = wait.max_entries - table.entries.len();
                    if entries.len() > room {
                        table.entries.extend(entries.into_iter().take(room));
                        table.truncated = true;
                        break;
                    }
                    table.entries.extend(entries);
                }
                // Reasons 1, 2 and 4 concern a DNET, which a local message
                // has none of: they answer our routed requests, not this
                // query.
                NetworkReport::RejectMessageToNetwork(reject)
                    if table.acks == 0
                        && target.rejects(
                            &received.source_mac,
                            received.source_network.as_ref(),
                            &reject,
                        ) =>
                {
                    return Err(RoutingTableError::Rejected(reject));
                }
                _ => {}
            }
        }
        Ok(table)
    }
}

#[path = "routing_table_target.rs"]
mod target;
pub(super) use target::QueryTarget;

#[cfg(test)]
#[path = "routing_table_tests.rs"]
mod tests;
