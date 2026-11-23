//! Requests by address: to a station on this network by its MAC, and to one
//! behind a router through the router the low-level client's discovery
//! table holds for it (Clause 6.2.2), never through an instance. A routed
//! address the table has no row for has no route, and fails with
//! [`Error::UnknownDevice`] before anything is sent. The requests are the
//! client's own typed calls to the [`Station`] found here (#2588).

use bacnet_client::client::{BACnetClient, Station};
use bacnet_transport::port::TransportPort;
use bacnet_types::MacAddr;
use bacnet_types::error::Error;

use super::DeviceAddress;

/// The station a request to `address` goes to: its MAC for a local one
/// (one on `client`'s own network too, once it knows the number), and for
/// a routed one through the router its row in `client`'s discovery table
/// names. The route is found once, so the requests of one read or one page
/// all take it.
pub(crate) async fn path_to<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    address: &DeviceAddress,
) -> Result<Station, Error> {
    let address = address.clone().localized(client.network_number());
    match routed_from(client, &address).await? {
        None => Ok(Station::local(address.mac())),
        Some((router, network)) => {
            let station = Station::routed(&router, network, address.mac());
            // A MAC the mark lifts is only ever a marked address
            // (#2738): the client would refuse a confirmed request to it
            // otherwise.
            Ok(if address.is_one_station() {
                station.one_station()
            } else {
                station
            })
        }
    }
}

/// The router a request to `address` goes through, with the network behind
/// it: `None` for an address on `client`'s own network, and for a routed
/// one the router MAC its row in `client`'s discovery table names. A routed
/// address with no row fails with [`Error::UnknownDevice`].
pub(crate) async fn router_for<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    address: &DeviceAddress,
) -> Result<Option<(MacAddr, u16)>, Error> {
    let address = address.clone().localized(client.network_number());
    routed_from(client, &address).await
}

/// [`router_for`] for an address already localized.
async fn routed_from<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    address: &DeviceAddress,
) -> Result<Option<(MacAddr, u16)>, Error> {
    if address.is_local() {
        return Ok(None);
    }
    match client.get_device_at(address.network(), address.mac()).await {
        Some(row) => Ok(Some((row.mac_address, address.network()))),
        None => Err(Error::UnknownDevice {
            device: address.to_string(),
        }),
    }
}
