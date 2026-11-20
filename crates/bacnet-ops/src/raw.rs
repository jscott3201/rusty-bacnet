//! The raw reading layer, for a front end that runs its own request jobs
//! (the TUI's browser and name reads, the tool's write desk, the exports):
//! a [`Reader`] for one device, with its own stop flag, whose requests
//! fail with the low-level client's [`Error`] in a [`Halt`]; the turn
//! hooks; and what classifies those errors. The [`Client`](crate::Client)
//! and [`Device`](crate::Device) calls are the high-level layer over it,
//! failing with [`OpsError`] instead, which answers the
//! same questions with its own methods.
//!
//! [`read_inventory`] and [`read_point_record`] are the layer under
//! [`Device::inventory`](crate::Device::inventory) and
//! [`Device::point_record`](crate::Device::point_record): the same reads
//! over a [`Reader`], for a caller that owns a low-level client.
//!
//! [`who_is`] sends a discovery's Who-Is straight from a low-level client,
//! for a front end that holds one and no [`Client`](crate::Client), and
//! [`who_has`] and [`who_has_replies`] a search's Who-Has: each checks its
//! [`Scope`] as [`Client::discover`](crate::Client::discover) and
//! [`Client::find`](crate::Client::find) do, but takes no rate slot and
//! charges no account.

use std::time::Duration;

use bacnet_client::client::{BACnetClient, ReceivedIHave};
use bacnet_services::who_has::WhoHasObject;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;

use crate::client::{Ask, Scope, find};
use crate::error::OpsError;

pub use crate::client::read::ListStop;
pub use crate::inventory::read::{InventoryHalt, read_inventory, read_point_record};

pub use crate::client::read::{
    Halt, Reader, RpmFailure, rpm_failure, rpm_rows, timed_out, unreachable,
};
use crate::device::{DeviceAddress, path_to};
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
    path_to(client, device).await?;
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
    let station = scope.station(client, Ask::WhoIs)?;
    scope
        .send(client, station, range)
        .await
        .map_err(OpsError::Client)
}

/// Send one Who-Has for `object` to `scope` over `client`, asking the
/// devices in `range` (all for `None`). The answers come through
/// `client`'s [`i_have_events`](BACnetClient::i_have_events). Unlike
/// [`Client::find`](crate::Client::find) it carries no rate cap: the
/// caller paces it.
///
/// # Errors
///
/// As [`who_is`]: [`OpsError::Parse`] for a scope that can't be sent,
/// before anything is; then [`OpsError::Client`] for a send the link
/// refused.
pub async fn who_has<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    scope: &Scope,
    object: &WhoHasObject,
    range: Option<DeviceInstanceRange>,
) -> Result<(), OpsError> {
    let station = find::station(client, scope)?;
    scope
        .send_who_has(client, station, object, range)
        .await
        .map_err(OpsError::Client)
}

/// Send one Who-Has as [`who_has`] does, then collect for `wait` the
/// I-Haves that answer it ([`ReceivedIHave::answers`]) from the scope asked:
/// an I-Have from elsewhere during the wait is no answer. One per device,
/// object and station (a station's routed copy is its local answer once
/// the client knows its network's number, by the end of the wait), in the
/// order they came, and at most
/// [`MAX_I_HAVE_REPLIES`](bacnet_client::client::MAX_I_HAVE_REPLIES), where
/// collection stops: a list of exactly that many may have been cut short.
/// No answer is an empty list.
///
/// # Errors
///
/// As [`who_has`].
pub async fn who_has_replies<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    scope: &Scope,
    object: &WhoHasObject,
    range: Option<DeviceInstanceRange>,
    wait: Duration,
) -> Result<Vec<ReceivedIHave>, OpsError> {
    let station = find::station(client, scope)?;
    find::replies(client, scope, station, object, range, wait)
        .await
        .map_err(OpsError::Client)
}
