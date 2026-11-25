//! The network-layer messages a non-routing node's user sends and reads
//! (Clause 6.4): a send for a local network message, and the reports the
//! user acts on, read from a received control with the one parser each
//! payload has (#1794, #1818).

use super::*;
use crate::network_number::NumberControl;
use bacnet_npdu::{NetworkMessageType, RejectMessageToNetwork};
use bacnet_router_core::message::{NetworkNumberIs, Networks, RoutingEntries};
use bacnet_router_core::table::is_reserved;

/// The network messages [`NetworkLayer::send_network_message`] sends: the
/// two questions a non-routing node asks. The rest change routing tables,
/// flow control or connections, or announce routes this node doesn't
/// have, so each joins the list only deliberately, with its own review.
///
/// The routing-table query (Initialize-Routing-Table with a port count of
/// zero, Clauses 6.4.7 and 6.6.3.8) is not on this list: it has its own
/// function, [`NetworkLayer::send_routing_table_query`], which fixes the
/// payload, so the update form can't be expressed.
pub const SENDABLE_NETWORK_MESSAGES: [NetworkMessageType; 2] = [
    NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK,
    NetworkMessageType::WHAT_IS_NETWORK_NUMBER,
];

/// The data portion of every routing-table query: a port count of zero and
/// nothing else (Clause 6.4.7).
const ROUTING_TABLE_QUERY: [u8; 1] = [0];

/// One entry of a router's table, as an Initialize-Routing-Table-Ack lists
/// it (Clause 6.4.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingTableEntry {
    /// The network the entry is for.
    pub network: u16,
    /// The router's own identifier for the port that reaches it.
    pub port_id: u8,
    /// The port information octets, which the standard defines only as an
    /// octet string (typically dial-up details for a point-to-point link);
    /// empty when the entry has none.
    pub port_info: Vec<u8>,
}

/// A network message that tells a non-routing node something about the
/// internetwork, read from a [`ReceivedNetworkControl`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NetworkReport {
    /// I-Am-Router-To-Network (Clause 6.4.2): the sender routes to these
    /// networks, in the order it listed them.
    IAmRouterToNetwork(Vec<u16>),
    /// Network-Number-Is (Clause 6.4.20): the number of the local network,
    /// and whether the sender had it configured rather than learned.
    NetworkNumberIs {
        /// The local network's number, in 1..=65534.
        network: u16,
        /// Whether the sender's number was configured (flag 1).
        configured: bool,
    },
    /// Reject-Message-To-Network (Clause 6.4.4): a router refused a message
    /// this node sent, for the reason and network it names.
    RejectMessageToNetwork(RejectMessageToNetwork),
    /// Initialize-Routing-Table-Ack (Clause 6.4.8) that answers a table
    /// query: the entries this one message lists, possibly none. A long
    /// table comes as several messages, and nothing marks the last
    /// (Clause 6.6.3.9). The acknowledgement of a table update carries no
    /// data and is not a report.
    RoutingTable(Vec<RoutingTableEntry>),
}

/// A [`NetworkReport`] and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedNetworkReport {
    /// The link-layer peer that sent it: for an I-Am-Router-To-Network, the
    /// router to send through to reach the networks it lists.
    pub source_mac: MacAddr,
    /// SNET/SADR, when a router passed the message on from another network,
    /// as a reject from a router further along the path is.
    pub source_network: Option<NpduAddress>,
    /// What the message reports.
    pub report: NetworkReport,
}

impl ReceivedNetworkReport {
    /// Read `control` as a report, or `None` for any other message type or
    /// a payload its parser refuses.
    ///
    /// - I-Am-Router-To-Network: router-core's [`Networks`] view. Entries
    ///   0 and 0xFFFF name no network a router can reach and are dropped,
    ///   as router-core's table drops them ([`is_reserved`]); the message is
    ///   refused only when no other entry is left.
    /// - Network-Number-Is: [`NumberControl::parse`], the reader the local
    ///   Number controls use, so a routed, unicast or malformed one is
    ///   ignored here too (Clause 6.4.20).
    /// - Reject-Message-To-Network: [`RejectMessageToNetwork::decode`],
    ///   exactly three octets.
    /// - Initialize-Routing-Table-Ack: router-core's [`RoutingEntries`]
    ///   view; an empty payload (an update's acknowledgement) or one that
    ///   ends inside an entry is refused.
    pub fn parse(control: &ReceivedNetworkControl) -> Option<Self> {
        Self::parse_with(control, NumberControl::parse(control))
    }

    /// [`Self::parse`] for a caller that already read `control` with
    /// [`NumberControl::parse`] and passes that result as `number`, so a
    /// Network-Number-Is is read once.
    pub fn parse_with(
        control: &ReceivedNetworkControl,
        number: Option<NumberControl>,
    ) -> Option<Self> {
        Some(Self {
            source_mac: control.source_mac.clone(),
            source_network: control.npdu.source.clone(),
            report: NetworkReport::parse_with(&control.npdu, number)?,
        })
    }
}

impl NetworkReport {
    /// Parse an overheard NPDU using its logical broadcast/unicast delivery,
    /// without manufacturing transport provenance or starting a network layer.
    pub fn parse_npdu(npdu: &Npdu, link_layer_group: bool) -> Option<Self> {
        Self::parse_with(npdu, NumberControl::parse_npdu(npdu, link_layer_group))
    }

    fn parse_with(npdu: &Npdu, number: Option<NumberControl>) -> Option<Self> {
        Some(match npdu.message_type? {
            NetworkMessageType::I_AM_ROUTER_TO_NETWORK => {
                let networks = Networks::parse(&npdu.payload).ok()?;
                let listed: Vec<u16> = networks.iter().filter(|&n| !is_reserved(n)).collect();
                if listed.is_empty() {
                    return None;
                }
                NetworkReport::IAmRouterToNetwork(listed)
            }
            NetworkMessageType::NETWORK_NUMBER_IS => match number? {
                NumberControl::NumberIs { number, flag } => NetworkReport::NetworkNumberIs {
                    network: number,
                    configured: flag == NetworkNumberIs::CONFIGURED,
                },
                NumberControl::WhatIs { .. } => return None,
            },
            NetworkMessageType::REJECT_MESSAGE_TO_NETWORK => NetworkReport::RejectMessageToNetwork(
                RejectMessageToNetwork::decode(&npdu.payload).ok()?,
            ),
            NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK => {
                let entries = RoutingEntries::parse(&npdu.payload).ok()?;
                NetworkReport::RoutingTable(
                    entries
                        .iter()
                        .map(|entry| RoutingTableEntry {
                            network: entry.network,
                            port_id: entry.port_id,
                            port_info: entry.port_info.to_vec(),
                        })
                        .collect(),
                )
            }
            _ => return None,
        })
    }
}

impl<T: TransportPort + 'static> NetworkLayer<T> {
    /// Send a network-layer message on the local network, with no DNET:
    /// broadcast when `destination_mac` is `None`, else to that peer.
    ///
    /// The NPDU names no destination network, so routers act on it
    /// themselves rather than pass it on, as Who-Is-Router-To-Network and
    /// What-Is-Network-Number need (Clauses 6.4.1 and 6.4.19). A
    /// Who-Is-Router-To-Network sent to a `destination_mac` needs one
    /// station's own address ([`Self::is_local_unicast_station`]): any
    /// other is refused with [`Error::Transport`] of kind
    /// [`InvalidInput`](std::io::ErrorKind::InvalidInput) and nothing is
    /// sent, never a broadcast instead. What-Is-Network-Number also accepts
    /// the link's broadcast MAC as a local broadcast. Only the two questions in
    /// [`SENDABLE_NETWORK_MESSAGES`] are sent; any other type,
    /// Initialize-Routing-Table included (it has
    /// [`Self::send_routing_table_query`], which takes no payload), is
    /// refused before anything is sent.
    pub async fn send_network_message(
        &self,
        message_type: NetworkMessageType,
        payload: &[u8],
        destination_mac: Option<&[u8]>,
    ) -> Result<(), Error> {
        if !SENDABLE_NETWORK_MESSAGES.contains(&message_type) {
            return Err(Error::InvalidArgument(Refusal::text(format!(
                "network message type {message_type} is not one a non-routing node sends; \
                 only Who-Is-Router-To-Network and What-Is-Network-Number are"
            ))));
        }
        self.send_local_message(message_type, payload, false, destination_mac)
            .await
    }

    /// Send a routing-table query through one local unicast peer.
    /// An optional `destination` addresses a router on another network: an
    /// Initialize-Routing-Table with a port count of zero, which asks the
    /// router to return its whole table in Initialize-Routing-Table-Acks
    /// and change nothing (Clauses 6.4.7 and 6.6.3.8). The acks come as
    /// [`NetworkReport::RoutingTable`] through the network-control
    /// receiver.
    ///
    /// The data portion is fixed here: no caller supplies a payload or a
    /// port list, so the update form, which would rewrite the router's
    /// table, cannot be sent. The message goes out with the
    /// data-expecting-reply flag set, as Clause 6.4.7 requires; on MS/TP
    /// that is what lets the router answer inside the reply window
    /// instead of waiting for the token.
    ///
    /// `router_mac` is the immediate peer for this query and must be one
    /// router's own address ([`Self::is_local_unicast_station`]): any other,
    /// a broadcast, group or empty one among them, is refused with
    /// [`Error::Transport`] of kind
    /// [`InvalidInput`](std::io::ErrorKind::InvalidInput) before anything
    /// is sent, as is a `destination` on a reserved network or with an
    /// empty or oversized address.
    pub async fn send_routing_table_query(
        &self,
        router_mac: &[u8],
        destination: Option<&NpduAddress>,
    ) -> Result<(), Error> {
        if !self.is_local_unicast_station(router_mac).await? {
            return Err(not_one_station(
                "a routing-table query goes to one router's own address on this link, using unicast",
            ));
        }
        if destination.is_some_and(|address| {
            is_reserved(address.network)
                || address.mac_address.is_empty()
                || address.mac_address.len() > bacnet_npdu::MAX_MAC_LEN
        }) {
            return Err(not_one_station(
                "a routed table query needs one valid destination station",
            ));
        }
        let npdu = Npdu {
            message_type: Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE),
            payload: Bytes::from_static(&ROUTING_TABLE_QUERY),
            expecting_reply: true,
            destination: destination.cloned(),
            ..Npdu::default()
        };
        let mut buf = BytesMut::new();
        encode_npdu(&mut buf, &npdu)?;
        self.transport.send_unicast(&buf, router_mac).await
    }

    /// Whether `mac` is neither empty nor one of the link's local broadcast
    /// destinations: the first test of
    /// [`Self::is_local_unicast_station`].
    fn is_routing_table_target(&self, mac: &[u8]) -> bool {
        !mac.is_empty() && !self.local_broadcasts.contains(mac)
    }

    /// Whether a link destination addresses one local station, including the
    /// address form and group destinations of this transport. For B/IP this
    /// can look up local subnet masks without sending a packet.
    pub async fn is_local_unicast_station(&self, mac: &[u8]) -> Result<bool, Error> {
        if !self.is_routing_table_target(mac) || self.transport.is_group_destination(mac) {
            return Ok(false);
        }
        station_form(self.transport.data_link(), mac).await
    }

    /// Build and send one local network message. Private: its callers fix
    /// the type and, for the query, the payload.
    async fn send_local_message(
        &self,
        message_type: NetworkMessageType,
        payload: &[u8],
        expecting_reply: bool,
        destination_mac: Option<&[u8]>,
    ) -> Result<(), Error> {
        let npdu = Npdu {
            message_type: Some(message_type),
            payload: Bytes::copy_from_slice(payload),
            expecting_reply,
            ..Npdu::default()
        };
        let mut buf = BytesMut::with_capacity(3 + payload.len());
        encode_npdu(&mut buf, &npdu)?;
        match destination_mac {
            Some(mac) if message_type == NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK => {
                if !self.is_local_unicast_station(mac).await? {
                    return Err(not_one_station(
                        "directed router discovery needs one local station's unicast address",
                    ));
                }
                self.transport.send_unicast(&buf, mac).await
            }
            Some(mac) if !self.local_broadcasts.contains(mac) => {
                self.transport.send_unicast(&buf, mac).await
            }
            _ => self.transport.send_broadcast(&buf).await,
        }
    }
}

/// A destination refused, with nothing sent, because it names no one
/// station: the error kind a transport gives a MAC it cannot address.
fn not_one_station(why: &'static str) -> Error {
    Error::Transport(std::io::Error::new(std::io::ErrorKind::InvalidInput, why))
}

/// Check the form of a station's MAC on the sending link. In particular
/// six octets represent an IP and port only on B/IP.
async fn station_form(
    link: Option<bacnet_types::data_link::DataLink>,
    mac: &[u8],
) -> Result<bool, Error> {
    use bacnet_types::data_link::DataLink;

    Ok(match link {
        Some(DataLink::Bip) => bacnet_transport::bip::is_unicast_station(mac)
            .await
            .map_err(Error::Transport)?,
        Some(DataLink::Bip6) => mac.len() == 3,
        Some(DataLink::Mstp) => matches!(mac, [0..=254]),
        Some(DataLink::Sc) => mac.len() == 6 && mac != bacnet_transport::sc_frame::UNKNOWN_VMAC,
        Some(DataLink::Ethernet) => mac.len() == 6 && mac != [0; 6],
        _ => !mac.is_empty(),
    })
}

#[cfg(test)]
#[path = "layer_network_message_tests.rs"]
mod tests;
