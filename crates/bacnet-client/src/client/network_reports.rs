//! Router discovery and network reports (Clause 6.4, #960): the client
//! sends Who-Is-Router-To-Network and What-Is-Network-Number, and hands the
//! I-Am-Router-To-Network, Network-Number-Is and Reject-Message-To-Network
//! messages it receives to the subscribers of
//! [`BACnetClient::network_reports`]. The client keeps the network-control
//! channel: the dispatch task reads each control once, publishes its report,
//! and then passes it on to the local Number controls or the routed-path
//! limits as before.

use super::i_have::collect_until;
use super::*;
use bacnet_network::layer::{NetworkReport, ReceivedNetworkControl, ReceivedNetworkReport};
use bacnet_network::network_number::NumberControl;
use bacnet_npdu::NetworkMessageType;
use bacnet_router_core::message::Networks;

/// Network-report broadcast channel capacity. A subscriber this far behind
/// loses the oldest and is told how many with `RecvError::Lagged`; the
/// dispatch task never waits for a subscriber.
pub const NETWORK_REPORT_CHANNEL_CAPACITY: usize = 64;

/// A router heard from during [`BACnetClient::discover_routers`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredRouter {
    /// The router's MAC on the local network: the next hop to the networks
    /// below.
    pub mac: MacAddr,
    /// Every network its I-Am-Router-To-Network messages listed, ascending,
    /// each once.
    pub networks: Vec<u16>,
}

/// Read `control` as a report and hand it to the subscribers; `number` is
/// what [`NumberControl::parse`] made of it. With no subscriber nothing is
/// parsed. Never waits.
///
/// While a routing-table query waits on a router (`queried`), an
/// Initialize-Routing-Table-Ack with no data from its expected link peer
/// and original source station is published as an empty table: this client never sends the
/// update form, so such an ack can only be that router's older-style answer
/// to the query. From any other source, or outside a query, it is no
/// report.
pub(super) fn publish(
    tx: &broadcast::Sender<ReceivedNetworkReport>,
    control: &ReceivedNetworkControl,
    number: Option<NumberControl>,
    queried: &std::sync::Mutex<Option<super::routing_table::QueryTarget>>,
) {
    if tx.receiver_count() == 0 {
        return;
    }
    let report = ReceivedNetworkReport::parse_with(control, number).or_else(|| {
        let empty_ack = control.npdu.message_type
            == Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK)
            && control.npdu.payload.is_empty()
            && queried
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .is_some_and(|target| {
                    target.answers(&control.source_mac, control.npdu.source.as_ref())
                });
        empty_ack.then(|| ReceivedNetworkReport {
            source_mac: control.source_mac.clone(),
            source_network: control.npdu.source.clone(),
            report: NetworkReport::RoutingTable(Vec::new()),
        })
    });
    if let Some(report) = report {
        let _ = tx.send(report);
    }
}

/// Add one I-Am-Router-To-Network's `listed` networks to the entry for
/// `mac`, made at the end when it is new.
fn record(routers: &mut Vec<DiscoveredRouter>, mac: MacAddr, listed: Vec<u16>) {
    let index = match routers.iter().position(|router| router.mac == mac) {
        Some(index) => index,
        None => {
            routers.push(DiscoveredRouter {
                mac,
                networks: Vec::new(),
            });
            routers.len() - 1
        }
    };
    let networks = &mut routers[index].networks;
    networks.extend(listed);
    networks.sort_unstable();
    networks.dedup();
}

/// Refuse a network number no router can be asked about: 0, or 0xFFFF,
/// the global broadcast.
fn check_network(network: u16) -> Result<(), Error> {
    if network == 0 || network == u16::MAX {
        return Err(Error::Encoding(format!(
            "network {network} is not a network number a router can reach; use 1..=65534"
        )));
    }
    Ok(())
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Get a receiver for the network reports this client receives: the
    /// routers that announce themselves, Network-Number-Is broadcasts and
    /// the rejects of messages it sent. Each call returns a new independent
    /// receiver. A receiver that falls [`NETWORK_REPORT_CHANNEL_CAPACITY`]
    /// behind loses the oldest.
    pub fn network_reports(&self) -> broadcast::Receiver<ReceivedNetworkReport> {
        self.network_report_tx.subscribe()
    }

    /// Send a Who-Is-Router-To-Network (Clause 6.4.1), asking which router
    /// reaches `network`, or, for `None`, every router for all the networks
    /// it reaches. It is broadcast on the local network when
    /// `destination_mac` is `None`, else sent to that router alone. Answers
    /// come as [`NetworkReport::IAmRouterToNetwork`] through
    /// [`Self::network_reports`]. `network` 0 and 0xFFFF are refused.
    pub async fn who_is_router_to_network(
        &self,
        network: Option<u16>,
        destination_mac: Option<&[u8]>,
    ) -> Result<(), Error> {
        let mut payload = [0u8; 2];
        let len = match network {
            Some(network) => {
                check_network(network)?;
                Networks::write([network], &mut payload)
                    .map_err(|e| Error::Encoding(e.to_string()))?
            }
            None => 0,
        };
        self.link_open()?;
        self.network
            .send_network_message(
                NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK,
                &payload[..len],
                destination_mac,
            )
            .await
    }

    /// The number of the network this client is on, once a
    /// Network-Number-Is has taught it ([`Self::what_is_network_number`]
    /// asks, and the client asks by itself before its first Who-Is or when
    /// it first stores a routed device, #2572); `None` until then, and
    /// always on a link that takes no network number controls. A routed
    /// address naming this network is a station on this one.
    pub fn network_number(&self) -> Option<u16> {
        self.network.local_network_number().get()
    }

    /// Send a What-Is-Network-Number (Clause 6.4.19), broadcast when
    /// `destination_mac` is `None`. Any node holding this network's number,
    /// a router in particular, broadcasts a Network-Number-Is in reply,
    /// which comes as [`NetworkReport::NetworkNumberIs`] through
    /// [`Self::network_reports`]. Ask once and keep the answer: the
    /// standard doesn't want the question repeated. A question sent counts
    /// as the one the client otherwise asks on first need (#2572), so a
    /// later Who-Is doesn't ask again.
    pub async fn what_is_network_number(
        &self,
        destination_mac: Option<&[u8]>,
    ) -> Result<(), Error> {
        self.link_open()?;
        self.network
            .send_network_message(
                NetworkMessageType::WHAT_IS_NETWORK_NUMBER,
                &[],
                destination_mac,
            )
            .await?;
        // It is the client's question too: a later Who-Is or routed row
        // doesn't ask again (#2572).
        self.device_table.asking.asked_by_caller();
        Ok(())
    }

    /// Send a Who-Is-Router-To-Network for `network` (or every network).
    /// With a destination, send to that local station and keep only reports
    /// from its own MAC without a routed source. With `None`, broadcast and
    /// collect the I-Am-Router-To-Network messages heard for
    /// `wait`: one entry per router MAC, in the order first heard. For a
    /// `network`, only the routers that list it are kept. No answer is an
    /// empty list, not an error.
    pub async fn discover_routers(
        &self,
        network: Option<u16>,
        destination_mac: Option<&[u8]>,
        wait: Duration,
    ) -> Result<Vec<DiscoveredRouter>, Error> {
        self.collect_routers(network, destination_mac, wait, destination_mac.is_some())
            .await
    }

    pub(super) async fn collect_routers(
        &self,
        network: Option<u16>,
        destination_mac: Option<&[u8]>,
        wait: Duration,
        local_only: bool,
    ) -> Result<Vec<DiscoveredRouter>, Error> {
        // Subscribe first, so no answer can arrive before the receiver.
        let mut rx = self.network_reports();
        self.who_is_router_to_network(network, destination_mac)
            .await?;
        let deadline = tokio::time::Instant::now() + wait;
        let mut routers: Vec<DiscoveredRouter> = Vec::new();
        collect_until(&mut rx, deadline, "I-Am-Router-To-Network", |received| {
            let NetworkReport::IAmRouterToNetwork(listed) = received.report else {
                return;
            };
            if (local_only && received.source_network.is_some())
                || destination_mac.is_some_and(|peer| received.source_mac.as_slice() != peer)
                || network.is_some_and(|wanted| !listed.contains(&wanted))
            {
                return;
            }
            record(&mut routers, received.source_mac, listed);
        })
        .await;
        Ok(routers)
    }
}

#[cfg(test)]
#[path = "network_reports_tests.rs"]
mod tests;
