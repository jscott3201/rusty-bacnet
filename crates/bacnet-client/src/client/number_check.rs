//! The router check before a number the client reaches through a router is
//! taken (#2826, maintainer decision 2026-11-26).
//!
//! A window's answers agreed on a number the client holds routed rows on,
//! or a routed path to: devices it reaches through a router there. Taking
//! it would make them local. A router lists the networks it can reach,
//! never the one its port is on (Clause 6.6.3.2), so a link can't be a
//! network a router on it routes to. Before taking the number the client
//! broadcasts its own router query (Who-Is-Router-To-Network) for it and
//! listens for one APDU timeout:
//!
//! - no router on this link answers: the number is taken, and the routed
//!   rows on it fold (#2572), one check later than they used to;
//! - a router answers that it routes there: the number is refused;
//! - an answer may have been lost, to a lagging report receiver or to the
//!   layer's or the Number owner's queue: the number is refused too.
//!
//! A refusal keeps what the client held and counts in
//! [`BACnetClient::network_number_disagreements`](super::BACnetClient::network_number_disagreements):
//! the answers and a router disagree, or may have. A number with no routed
//! row or path on it is taken without a check: nothing the client reaches
//! moves. Each check is one query for one window that agreed, and windows
//! come only from the client's own questions, which are paced.
use super::i_have::collect_until;
use super::network_number::Asking;
use super::*;
use bacnet_network::layer::{NetworkReport, ReceivedNetworkReport};
use bacnet_npdu::NetworkMessageType;

/// What the router check sends with and where it listens.
pub(super) struct RouterCheck<'a, T: TransportPort> {
    pub(super) network: &'a NetworkLayer<T>,
    pub(super) reports: &'a broadcast::Sender<ReceivedNetworkReport>,
    pub(super) asking: &'a Asking,
    pub(super) paths: &'a RoutedPathLimits,
    /// How long the check listens: the client's APDU timeout.
    pub(super) wait: Duration,
}

/// What the router check found for a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Routed {
    /// No router on this link routes to it.
    Nowhere,
    /// A router on this link routes to it.
    Here,
    /// An answer may have been lost, or the query didn't go.
    Unknown,
}

impl<T: TransportPort + 'static> RouterCheck<'_, T> {
    /// Whether taking `number` needs the check: the client holds a routed
    /// row on it in `table`, or a routed path to it.
    pub(super) fn needed(&self, table: &SyncMutex<DeviceTable>, number: u16) -> bool {
        table.lock().routes_on(number) || self.paths.has_path_to(number)
    }

    /// Ask the routers on this link which of them routes to `number`, and
    /// listen for the wait.
    pub(super) async fn routed(&self, number: u16) -> Routed {
        let mut rx = self.reports.subscribe();
        let before = self.asking.losses();
        let asked = number.to_be_bytes();
        let query = self.network.send_network_message(
            NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK,
            &asked,
            None,
        );
        if let Err(error) = query.await {
            debug!(%error, number, "the router check's query failed");
            return Routed::Unknown;
        }
        let deadline = tokio::time::Instant::now() + self.wait;
        let mut here = false;
        let lost = collect_until(&mut rx, deadline, "I-Am-Router-To-Network", |received| {
            if let NetworkReport::IAmRouterToNetwork(listed) = received.report
                && received.source_network.is_none()
                && listed.contains(&number)
            {
                here = true;
            }
        })
        .await;
        if here {
            Routed::Here
        } else if lost > 0 || self.asking.losses() != before {
            Routed::Unknown
        } else {
            Routed::Nowhere
        }
    }
}
