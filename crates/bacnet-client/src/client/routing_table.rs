//! The read-only routing-table query (#2612): ask one router on the local
//! link for its table and collect the acknowledgements.
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
//! Only a router on the client's own link can be asked: the message is a
//! local network message with no DNET. A router reached through another
//! router is not covered.

use super::*;
use bacnet_network::layer::{NetworkReport, RoutingTableEntry};
use bacnet_npdu::{RejectMessageReason, RejectMessageToNetwork};
use tokio::sync::broadcast::error::RecvError;

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
    /// [`RoutingTableWait::quiet`].
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
    NoAnswer {
        /// How long the call waited.
        waited: Duration,
    },
    /// The router answered with a Reject-Message-To-Network before any
    /// acknowledgement (Clause 6.4.4), with a reason that can answer a
    /// local message: other error, unknown message type or an addressing
    /// error. Rejects for a DNET (reasons 1, 2 and 4) belong to routed
    /// requests and are ignored.
    Rejected(RejectMessageToNetwork),
    /// The call's arguments were refused and nothing was sent: no entries
    /// kept, a first-answer wait longer than the total, or a router
    /// address that is empty or a broadcast.
    InvalidArgument(String),
    /// The client stopped while the call waited. That is not silence from
    /// the router.
    ClientStopped,
    /// The query was not sent: the link failed or the client's node had
    /// stopped.
    Send(Error),
}

impl core::fmt::Display for RoutingTableError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoAnswer { waited } => write!(
                f,
                "the router sent no routing table within {waited:?}: it did not \
                 answer the query, or is not on this link"
            ),
            Self::Rejected(reject) => write!(
                f,
                "the router rejected the routing-table query: {} (network {})",
                reject.reason, reject.network
            ),
            Self::InvalidArgument(why) => write!(f, "routing-table query refused: {why}"),
            Self::ClientStopped => f.write_str("the client stopped during the routing-table query"),
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
#[derive(Debug, Default)]
pub(super) struct RoutingQueries {
    /// Set while a query waits: the dispatch task then reads a data-less
    /// Initialize-Routing-Table-Ack as an empty table.
    pub(super) outstanding: std::sync::atomic::AtomicBool,
    /// One routing-table query at a time. Network messages carry no invoke
    /// ID, so two queries to one router would each collect both answers.
    /// The mutex guards no data: holding it across the call's awaits is its
    /// whole purpose.
    gate: tokio::sync::Mutex<()>,
}

/// Clears the "query outstanding" flag when the call ends, however it ends.
struct Outstanding<'a>(&'a std::sync::atomic::AtomicBool);

impl Drop for Outstanding<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Read the routing table of the router at `router`, a station on this
    /// client's own link, without changing it.
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
    ///   (reasons 1, 2 and 4) are ignored, as is any reject after the
    ///   first ack.
    /// - [`RoutingTableError::InvalidArgument`] for `wait.max_entries` 0,
    ///   `wait.first_answer` longer than `wait.total`, or a `router` that
    ///   is empty or a broadcast; nothing is sent.
    /// - [`RoutingTableError::ClientStopped`] when the client stops
    ///   mid-call.
    ///
    /// Only a router on the local link can be asked: the query carries no
    /// DNET, so a router behind another router is out of reach, and acks
    /// that arrive routed (with a source network) are not taken. An ack is
    /// taken when it comes from `router`'s MAC, whether it arrives as a
    /// unicast or a broadcast. Network messages carry no invoke ID, so the
    /// client runs one routing-table query at a time: a second call waits
    /// for the first to finish.
    pub async fn routing_table(
        &self,
        router: &[u8],
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
        if !self.network.is_routing_table_target(router) {
            return Err(RoutingTableError::InvalidArgument(
                "the router must be one station's own address, not empty or a broadcast".into(),
            ));
        }
        // Held for the whole call, across its awaits: it guards no data.
        let _one_at_a_time = self.routing_query.gate.lock().await;
        // Subscribe first, so no answer can arrive before the receiver.
        let mut rx = self.network_reports();
        self.seat_open()?;
        self.routing_query
            .outstanding
            .store(true, std::sync::atomic::Ordering::Release);
        let _outstanding = Outstanding(&self.routing_query.outstanding);
        self.network.send_routing_table_query(router).await?;
        let start = tokio::time::Instant::now();
        let end = start + wait.total;
        let mut last_ack = start;
        let mut table = RoutingTable {
            entries: Vec::new(),
            acks: 0,
            truncated: false,
        };
        loop {
            let deadline = if table.acks == 0 {
                start + wait.first_answer
            } else {
                (last_ack + wait.quiet).min(end)
            };
            let received = match tokio::time::timeout_at(deadline, rx.recv()).await {
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
                            waited: deadline - start,
                        });
                    }
                    // The quiet interval is the normal end; the total is a
                    // bound that cut the answer short.
                    table.truncated |= deadline >= end;
                    break;
                }
            };
            if received.source_mac.as_slice() != router || received.source_network.is_some() {
                continue;
            }
            match received.report {
                NetworkReport::RoutingTable(entries) => {
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
                        && ![
                            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
                            RejectMessageReason::ROUTER_BUSY,
                            RejectMessageReason::MESSAGE_TOO_LONG,
                        ]
                        .contains(&reject.reason) =>
                {
                    return Err(RoutingTableError::Rejected(reject));
                }
                _ => {}
            }
        }
        Ok(table)
    }
}

#[cfg(test)]
#[path = "routing_table_tests.rs"]
mod tests;
