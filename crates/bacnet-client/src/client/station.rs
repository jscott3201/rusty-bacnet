//! Requests addressed to one station by its network and MAC, the address
//! that identifies a device in the discovery table, rather than by a Device
//! instance (#2560). Two devices sharing an instance are two addresses, so
//! each is reachable this way while a lookup by the instance is refused.
//!
//! A local station is sent to at its MAC. A remote one goes through the
//! router the discovery table's row at its address names (Clause 6.2.2):
//! the table is the client's only record of a route, so a remote address
//! with no row fails before anything is sent. Both go out through the same
//! request path as every other call, so the row at the address, when there
//! is one, sizes the request and its answer keeps the row alive.

use super::*;
use bacnet_types::constructed::BACnetAddress;

/// How a request reaches a station.
enum Route {
    /// On this network, at the station's MAC.
    Local,
    /// Through the router at this MAC on this network.
    Via(MacAddr),
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Send a confirmed request to the station at `address` and wait for its
    /// answer: network 0 is this network, and on another network the MAC is
    /// the station's own there, never its router's.
    ///
    /// A local station is sent to as [`Self::confirmed_request`] sends; a
    /// remote one as [`Self::confirmed_request_routed`] sends, through the
    /// router the discovery table's row at `address` names. An address on
    /// this client's own network, once it has learned the number, is the
    /// local station at that MAC. The row at the address, when the table
    /// has one, bounds the request by the device's Max APDU Length Accepted
    /// and segmentation, and an answer from the device starts the row's age
    /// again; with no row the client's configured limits apply.
    ///
    /// # Errors
    ///
    /// Before anything is sent: [`Error::Encoding`] for an address that
    /// names no one station (an empty MAC, network 65535, or a MAC longer
    /// than [`BACnetAddress::MAX_MAC_LEN`]), and [`Error::UnknownDevice`]
    /// for a remote address the discovery table has no row for, since the
    /// row is the route. Then the errors of the request itself.
    pub async fn confirmed_request_to(
        &self,
        address: &BACnetAddress,
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<Bytes, Error> {
        let (network, mac) = (address.network_number, &address.mac_address[..]);
        match self.route_to(network, mac)? {
            Route::Local => {
                self.confirmed_request(mac, service_choice, service_data)
                    .await
            }
            Route::Via(router) => {
                self.confirmed_request_routed(&router, network, mac, service_choice, service_data)
                    .await
            }
        }
    }

    /// Send an unconfirmed request to the station at `address`, once, with
    /// no answer waited for: success means it was sent. A local station is
    /// sent to as [`Self::unconfirmed_request`] sends; a remote one through
    /// the router the discovery table's row at `address` names, with the
    /// station as the NPDU's destination. An address on this client's own
    /// network, once it has learned the number, is the local station at
    /// that MAC, which must then be one station, not a broadcast or group
    /// address.
    ///
    /// # Errors
    ///
    /// As [`Self::confirmed_request_to`] before sending, and the
    /// transport's error.
    pub async fn unconfirmed_request_to(
        &self,
        address: &BACnetAddress,
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        let (network, mac) = (address.network_number, &address.mac_address[..]);
        match self.route_to(network, mac)? {
            Route::Local if network != 0 && self.network.transport().is_group_destination(mac) => {
                Err(Error::Encoding(format!(
                    "{} names this network's broadcast or a group, not one station",
                    station_text(network, mac)
                )))
            }
            Route::Local => {
                self.unconfirmed_request(mac, service_choice, service_data)
                    .await
            }
            Route::Via(router) => {
                self.unconfirmed_request_routed(&router, network, mac, service_choice, service_data)
                    .await
            }
        }
    }

    /// An unconfirmed request through the router at `router` to the station
    /// at `mac` on `network`.
    pub(super) async fn unconfirmed_request_routed(
        &self,
        router: &[u8],
        network: u16,
        mac: &[u8],
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        self.seat_open()?;
        let apdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });
        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &apdu)?;
        self.ask_before(service_choice).await;
        self.network
            .send_apdu_routed(&buf, network, mac, router, false, NetworkPriority::Normal)
            .await
    }

    /// How a request reaches the station at `mac` on `network`: locally on
    /// network 0 or this client's own network, else through the router its
    /// discovery-table row names. Checked before any request state is
    /// taken.
    fn route_to(&self, network: u16, mac: &[u8]) -> Result<Route, Error> {
        if mac.is_empty() {
            return Err(Error::Encoding(format!(
                "network {network} with an empty MAC is a broadcast, not one station"
            )));
        }
        if network == 0 {
            return Ok(Route::Local);
        }
        check_routed_unicast(network, mac.len())?;
        if self.network.local_network_number().get() == Some(network) {
            return Ok(Route::Local);
        }
        match self.device_table.lock().get(network, mac) {
            Some(row) => Ok(Route::Via(MacAddr::from_slice(&row.mac_address))),
            None => Err(Error::UnknownDevice {
                device: station_text(network, mac),
            }),
        }
    }
}

/// `2:0x05`: a station as the CLI's target grammar writes it.
fn station_text(network: u16, mac: &[u8]) -> String {
    let hex: String = mac.iter().map(|octet| format!("{octet:02X}")).collect();
    format!("{network}:0x{hex}")
}

#[cfg(test)]
#[path = "station_tests.rs"]
mod tests;
