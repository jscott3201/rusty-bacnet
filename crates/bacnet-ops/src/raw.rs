//! The raw reading layer, for a front end that runs its own request jobs
//! (the TUI's browser and name reads, the tool's write desk, the exports):
//! a [`Reader`] for one device, with its own stop flag, whose requests
//! fail with the low-level client's [`Error`] in a [`Halt`]; the turn
//! hooks; and what classifies those errors. The [`Client`](crate::Client)
//! and [`Device`](crate::Device) calls are the high-level layer over it,
//! failing with [`OpsError`] instead, which answers the
//! same questions with its own methods.
//!
//! [`who_is`] sends a discovery's Who-Is straight from a low-level client,
//! for a front end that holds one and no [`Client`](crate::Client): it
//! checks its [`Scope`] as [`Client::discover`](crate::Client::discover)
//! does, but takes no rate slot and charges no account.

use bacnet_client::client::BACnetClient;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;

use crate::client::Scope;
use crate::error::OpsError;

pub use crate::client::read::{
    Halt, Reader, RpmFailure, rpm_failure, rpm_rows, timed_out, unreachable,
};
use crate::device::{DeviceAddress, Path};
use crate::limits::{RequestLimits, Turn};

/// The turn at `device`'s gate under `limits`, for a request the caller
/// sends itself: what [`Reader::turn`] takes, with no stop flag to halt it.
/// Hold it for the one request (see [`RequestLimits::turn`]).
///
/// # Errors
///
/// A routed address `client`'s discovery table has no row for has no path
/// ([`Error::UnknownDevice`]): it fails at once, taking no turn, slot or
/// charge.
pub async fn turn<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    limits: &RequestLimits,
    device: &DeviceAddress,
) -> Result<Turn, Error> {
    Path::to(client, device).await?;
    let gate = device.clone().localized(client.network_number());
    Ok(limits.turn(&gate).await)
}

/// Send one Who-Is to `scope` over `client`, for the devices in `range`
/// (all for `None`). The answers land in `client`'s discovery table as
/// they arrive. Unlike [`Client::discover`](crate::Client::discover) it
/// carries no rate cap: the caller paces it.
///
/// # Errors
///
/// Before anything is sent, [`OpsError::Parse`] for a scope that can't be
/// sent (see [`Scope`]); then [`OpsError::Client`] for a send the link
/// refused.
pub async fn who_is<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    scope: &Scope,
    range: Option<DeviceInstanceRange>,
) -> Result<(), OpsError> {
    let station = scope.station(client)?;
    scope
        .send(client, station.as_deref(), range)
        .await
        .map_err(OpsError::Client)
}
