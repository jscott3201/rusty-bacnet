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
use bacnet_types::sync::MutexExt;

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
            && queried.locked().as_ref().is_some_and(|target| {
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

/// The router core's refusal to write a network list, as the static
/// [`Error::Encode`] (#1833).
#[cold]
fn network_list_refused(error: bacnet_router_core::message::DataError) -> Error {
    use bacnet_router_core::message::DataError;
    use bacnet_types::error::EncodeError;
    let octets = |count: usize| u32::try_from(count).unwrap_or(u32::MAX);
    match error {
        DataError::NoRoom { needed, available } => {
            EncodeError::new("message data of {} octets doesn't fit in {}")
                .with_values(octets(needed), octets(available))
        }
        _ => EncodeError::new("network list refused"),
    }
    .into()
}

/// Refuse a network number no router can be asked about: 0, or 0xFFFF,
/// the global broadcast.
fn check_network(network: u16) -> Result<(), Error> {
    if network == 0 || network == u16::MAX {
        return Err(Error::InvalidArgument(
            Refusal::new("network {} is not a network number a router can reach; use 1..=65534")
                .with_value(u64::from(network)),
        ));
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

    /// Broadcast a Who-Is-Router-To-Network (Clause 6.4.1) on the local
    /// network, asking which router reaches `network`, or, for `None`,
    /// every router for all the networks it reaches. Answers come as
    /// [`NetworkReport::IAmRouterToNetwork`] through
    /// [`Self::network_reports`]. `network` 0 and 0xFFFF are refused.
    /// [`Self::who_is_router_to_network_directed`] asks one router.
    pub async fn who_is_router_to_network(&self, network: Option<u16>) -> Result<(), Error> {
        self.send_who_is_router(None, network).await
    }

    /// Send a Who-Is-Router-To-Network to the one router at `router`, its
    /// MAC on the local network, as [`Self::who_is_router_to_network`]
    /// broadcasts it. A `router` that names no one station (empty, a
    /// broadcast or group address, or malformed for the link) is refused
    /// with [`Error::Transport`] of kind
    /// [`InvalidInput`](std::io::ErrorKind::InvalidInput), and nothing is
    /// sent.
    ///
    /// The router answers with a local broadcast, not to the asker alone
    /// (Clause 6.6.3.3). A B/IP router on another IP subnet takes the
    /// question, but its answer reaches this client only when a BBMD
    /// forwards it here, as a peer or by foreign-device registration.
    pub async fn who_is_router_to_network_directed(
        &self,
        router: &[u8],
        network: Option<u16>,
    ) -> Result<(), Error> {
        self.send_who_is_router(Some(router), network).await
    }

    async fn send_who_is_router(
        &self,
        router: Option<&[u8]>,
        network: Option<u16>,
    ) -> Result<(), Error> {
        let mut payload = [0u8; 2];
        let len = match network {
            Some(network) => {
                check_network(network)?;
                Networks::write([network], &mut payload).map_err(network_list_refused)?
            }
            None => 0,
        };
        self.link_open()?;
        self.network
            .send_network_message(
                NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK,
                &payload[..len],
                router,
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

    /// The numbers of the networks this client's local broadcast reaches,
    /// as it knows them now: its own network's, once learned
    /// ([`Self::network_number`]), or none yet; and on a router's
    /// application port, which is on each of the router's networks, every
    /// one of them ([`TransportPort::attached_networks`], #2601). An
    /// answer whose SNET is one of these comes from this network:
    /// [`Broadcast::answered_from`](super::Broadcast::answered_from) takes
    /// this list.
    pub fn local_networks(&self) -> Vec<u16> {
        match self.network.transport().attached_networks() {
            [] => self.network_number().into_iter().collect(),
            attached => attached.to_vec(),
        }
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

    /// Broadcast a Who-Is-Router-To-Network for `network` (or every
    /// network), then collect the I-Am-Router-To-Network messages heard for
    /// `wait`: one entry per router MAC, in the order first heard. For a
    /// `network`, only the routers that list it are kept. No answer is an
    /// empty list, not an error. [`Self::discover_routers_directed`] asks
    /// one router.
    pub async fn discover_routers(
        &self,
        network: Option<u16>,
        wait: Duration,
    ) -> Result<Vec<DiscoveredRouter>, Error> {
        let (routers, _) = self.collect_routers(None, network, wait, false).await?;
        Ok(routers)
    }

    /// Ask the one router at `router`, its MAC on the local network, as
    /// [`Self::who_is_router_to_network_directed`] does, then collect for
    /// `wait` as [`Self::discover_routers`] does, keeping only the reports
    /// from `router`'s own MAC that no other router passed on: at most one
    /// entry. A router asked about a network it has no entry for may ask
    /// further before it answers.
    ///
    /// The router answers with a local broadcast (Clause 6.6.3.3), so a
    /// B/IP router on another IP subnet is heard only when a BBMD forwards
    /// its answer here, as a peer or by foreign-device registration; with
    /// no BBMD, an empty list is the expected result. A `router` that
    /// names no one station is refused as
    /// [`Self::who_is_router_to_network_directed`] refuses it.
    pub async fn discover_routers_directed(
        &self,
        router: &[u8],
        network: Option<u16>,
        wait: Duration,
    ) -> Result<Vec<DiscoveredRouter>, Error> {
        let (routers, _) = self
            .collect_routers(Some(router), network, wait, true)
            .await?;
        Ok(routers)
    }

    /// Ask `router` (or broadcast, with `None`) and collect the
    /// I-Am-Router-To-Network reports heard for `wait`, with the number of
    /// reports lost to a lagging receiver. A directed ask keeps only
    /// `router`'s own reports; `local_only` keeps only reports no other
    /// router passed on.
    pub(super) async fn collect_routers(
        &self,
        router: Option<&[u8]>,
        network: Option<u16>,
        wait: Duration,
        local_only: bool,
    ) -> Result<(Vec<DiscoveredRouter>, u64), Error> {
        // Subscribe first, so no answer can arrive before the receiver.
        let mut rx = self.network_reports();
        self.send_who_is_router(router, network).await?;
        let deadline = tokio::time::Instant::now() + wait;
        let mut routers: Vec<DiscoveredRouter> = Vec::new();
        let lost = collect_until(&mut rx, deadline, "I-Am-Router-To-Network", |received| {
            let NetworkReport::IAmRouterToNetwork(listed) = received.report else {
                return;
            };
            if (local_only && received.source_network.is_some())
                || router.is_some_and(|peer| received.source_mac.as_slice() != peer)
                || network.is_some_and(|wanted| !listed.contains(&wanted))
            {
                return;
            }
            record(&mut routers, received.source_mac, listed);
        })
        .await;
        Ok((routers, lost))
    }
}

#[cfg(test)]
#[path = "network_reports_tests.rs"]
mod tests;
