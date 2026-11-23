//! The link peer and optional routed station selected for one table query.

use super::*;
use bacnet_encoding::npdu::NpduAddress;

#[derive(Clone, Debug)]
pub(in crate::client) struct QueryTarget {
    pub(super) hop: MacAddr,
    pub(super) station: Option<NpduAddress>,
}

impl QueryTarget {
    pub(in crate::client) fn answers(&self, hop: &MacAddr, source: Option<&NpduAddress>) -> bool {
        *hop == self.hop && source == self.station.as_ref()
    }

    pub(super) fn rejects(
        &self,
        hop: &MacAddr,
        source: Option<&NpduAddress>,
        reject: &RejectMessageToNetwork,
    ) -> bool {
        if *hop != self.hop {
            return false;
        }
        // Any router on the path that cannot reach the queried network
        // rejects it, and each router between relays that reject back
        // toward this client (Clause 6.6.3.5): it comes through the
        // selected hop, without an SNET from the hop itself and with the
        // rejecting router's network from one further on. Reason 4 is
        // never this query's: the query is nine octets, shorter than any
        // link's limit, so it answers a larger request to the same network.
        if let Some(station) = &self.station
            && reject.network == station.network
        {
            return reject.reason != RejectMessageReason::MESSAGE_TOO_LONG;
        }
        // A rejection by the destination itself must have its full identity.
        self.answers(hop, source)
            && ![
                RejectMessageReason::NOT_DIRECTLY_CONNECTED,
                RejectMessageReason::ROUTER_BUSY,
                RejectMessageReason::MESSAGE_TOO_LONG,
            ]
            .contains(&reject.reason)
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    pub(super) async fn table_query_target(
        &self,
        station: Station,
        wait: Duration,
    ) -> Result<QueryTarget, RoutingTableError> {
        let (network, mac) = station.address();
        Self::station_of(network, mac)
            .map_err(|e| RoutingTableError::InvalidArgument(e.to_string()))?;
        if network == 0 || self.network_number() == Some(network) {
            return Ok(QueryTarget {
                hop: MacAddr::from_slice(mac),
                station: None,
            });
        }
        let destination = NpduAddress {
            network,
            mac_address: MacAddr::from_slice(mac),
        };
        // A table query goes to one router, so a station named for a local
        // broadcast is asked through the router found for it, as one named
        // by its address alone is.
        let station = match station {
            Station::RoutedViaLocalBroadcast { network, mac, .. } => Station::remote(network, &mac),
            station => station,
        };
        let hop = match self.with_fixed_route(station) {
            Ok(Station::Routed { router, .. }) => router,
            Ok(Station::Local { mac }) => {
                return Ok(QueryTarget {
                    hop: mac,
                    station: None,
                });
            }
            // A fixed route always names its next hop; were it ever not to,
            // no hop is known.
            Ok(Station::Remote { .. } | Station::RoutedViaLocalBroadcast { .. }) => {
                return Err(RoutingTableError::RouteUnresolved {
                    network,
                    candidates: 0,
                });
            }
            Err(Error::UnknownDevice { .. }) => {
                let (routers, lost) = self
                    .collect_routers(None, Some(network), wait, true)
                    .await?;
                if lost != 0 {
                    return Err(RoutingTableError::DiscoveryIncomplete { network, lost });
                }
                if self.network_number() == Some(network) {
                    return Ok(QueryTarget {
                        hop: destination.mac_address,
                        station: None,
                    });
                }
                let mut selected = None;
                let mut candidates = 0;
                for router in routers {
                    if self.network.is_local_unicast_station(&router.mac).await? {
                        candidates += 1;
                        selected = Some(router.mac);
                    }
                }
                match selected {
                    Some(hop) if candidates == 1 => hop,
                    _ => {
                        return Err(RoutingTableError::RouteUnresolved {
                            network,
                            candidates,
                        });
                    }
                }
            }
            Err(other) => return Err(other.into()),
        };
        Ok(QueryTarget {
            hop,
            station: Some(destination),
        })
    }
}
