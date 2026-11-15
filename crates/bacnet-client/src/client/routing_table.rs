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
use bacnet_npdu::RejectMessageToNetwork;
use tokio::sync::broadcast::error::RecvError;

/// Most entries a [`BACnetClient::routing_table`] call keeps by default.
/// A network number is 16 bits, so no table lists more than 65,534
/// networks; the default is far above any real table and bounds memory
/// against a router that never stops answering.
pub const DEFAULT_ROUTING_TABLE_MAX_ENTRIES: usize = 4096;

/// The bounds of a [`BACnetClient::routing_table`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingTableWait {
    /// How long to wait for the first acknowledgement (or a reject) before
    /// giving up with [`RoutingTableError::NoAnswer`]. Default 3 seconds.
    pub first_answer: Duration,
    /// How long after one acknowledgement to wait for the next, with none
    /// meaning the table is complete. Default 500 milliseconds.
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
    /// quiet interval. A table cut short has no marker of its own, so this
    /// is the only sign the list may be incomplete.
    pub truncated: bool,
}

/// Why [`BACnetClient::routing_table`] returned no table.
#[derive(Debug)]
#[non_exhaustive]
pub enum RoutingTableError {
    /// The router sent no acknowledgement and no reject within the first
    /// answer bound. A router that ignores the query (an older one, or
    /// one configured to drop it) looks the same as one that is not
    /// there, so this does not say which. It is not an empty table.
    NoAnswer {
        /// How long the call waited.
        waited: Duration,
    },
    /// The router answered with a Reject-Message-To-Network before any
    /// acknowledgement (Clause 6.4.4): a router that does not know the
    /// message type refuses it this way.
    Rejected(RejectMessageToNetwork),
    /// The query was not sent: the call's arguments were refused, or the
    /// link failed.
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
    /// last (Clause 6.6.3.9), so the call returns after `wait.quiet` with no
    /// further ack, or at `wait.total`, or at `wait.max_entries`; the last
    /// two are reported as [`RoutingTable::truncated`].
    ///
    /// Outcomes:
    /// - acks arrived: `Ok`, the entries in the router's order. A router
    ///   whose table is empty answers once with no entries: `Ok` with an
    ///   empty list.
    /// - nothing came from `router` within `wait.first_answer`:
    ///   [`RoutingTableError::NoAnswer`]. An older router that ignores the
    ///   message, one that drops it by policy, and an absent one all end
    ///   here; none is an empty table.
    /// - a Reject-Message-To-Network from `router` came before any ack:
    ///   [`RoutingTableError::Rejected`], with the router's reason. A
    ///   reject after the first ack is ignored.
    ///
    /// Only a router on the local link can be asked: the query carries no
    /// DNET, so a router behind another router is out of reach, and acks
    /// that arrive routed (with a source network) are not taken. `router`
    /// must be the router's own address; a broadcast address is refused.
    pub async fn routing_table(
        &self,
        router: &[u8],
        wait: RoutingTableWait,
    ) -> Result<RoutingTable, RoutingTableError> {
        if wait.max_entries == 0 {
            return Err(RoutingTableError::Send(Error::Encoding(
                "a routing-table query must keep at least one entry".into(),
            )));
        }
        // Subscribe first, so no answer can arrive before the receiver.
        let mut rx = self.network_reports();
        self.seat_open()?;
        self.network.send_routing_table_query(router).await?;
        let start = tokio::time::Instant::now();
        let end = start + wait.total;
        let mut table = RoutingTable {
            entries: Vec::new(),
            acks: 0,
            truncated: false,
        };
        loop {
            let deadline = if table.acks == 0 {
                (start + wait.first_answer).min(end)
            } else {
                (tokio::time::Instant::now() + wait.quiet).min(end)
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
                Ok(Err(RecvError::Closed)) => {
                    // The client stopped while it waited.
                    if table.acks == 0 {
                        return Err(RoutingTableError::NoAnswer {
                            waited: start.elapsed(),
                        });
                    }
                    table.truncated = true;
                    break;
                }
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
                    let room = wait.max_entries - table.entries.len();
                    if entries.len() > room {
                        table.entries.extend(entries.into_iter().take(room));
                        table.truncated = true;
                        break;
                    }
                    table.entries.extend(entries);
                }
                NetworkReport::RejectMessageToNetwork(reject) if table.acks == 0 => {
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
