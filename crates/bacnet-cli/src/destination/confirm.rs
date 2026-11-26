//! A device named by its instance is reached only at an address whose
//! device answered as that instance to a question of this client's own
//! (#2824), placed as a `bacnet_ops` handle is: unless a row registered by
//! hand claims it, one Who-Is for it first (the settle time for its
//! answers, or the wait for a first answer when nothing claims it), then
//! every claim asked which device answers there, all at once
//! (`bacnet_ops::device::place_instance`). The refusal names each address
//! asked and what answered there.

use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_ops::device::{Look, place_instance};
use bacnet_ops::limits::RequestLimits;
use bacnet_ops::{DeviceAddress, OpsError, Scope};
use bacnet_transport::port::TransportPort;

use super::Destination;

/// The destination of device `instance`, on network `dnet` when the target
/// names one (a bare instance asks this network alone, as the high-level
/// client's default resolve scope does), at the address whose device
/// answers as it, its Who-Is waiting up to `wait` for a first answer when
/// nothing claims it.
pub(super) async fn confirmed<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    instance: u32,
    dnet: Option<u16>,
    wait: Duration,
) -> Result<Destination, String> {
    // A one-shot command has the device to itself: no rate cap beyond the
    // client's own.
    let limits = RequestLimits::new(0);
    let own = client.network_number();
    let scope = match dnet {
        Some(dnet) if Some(dnet) != own => Scope::Network(dnet),
        _ => Scope::Local,
    };
    let name = match dnet {
        Some(dnet) => format!("{dnet}:{instance}"),
        None => instance.to_string(),
    };
    // A claimant that answered from an address the one-station mark lifts
    // has no address of its own to ask: say so, and how to name it, with
    // nothing sent (#2738).
    let rows = client.discovered_devices().await;
    let rows: Vec<_> = rows
        .iter()
        .filter(|row| row.object_identifier.instance_number() == instance)
        .filter(|row| dnet.is_none_or(|dnet| row.source_network == Some(dnet)))
        .collect();
    if !rows.iter().any(|row| DeviceAddress::of(row).is_some())
        && let Some(form) = rows
            .iter()
            .find_map(|row| DeviceAddress::one_station_form(row))
    {
        return Err(format!(
            "device {name} answered from {form}, an address that needs the one-station \
             form: name it by that address"
        ));
    }
    let look = Look { scope, wait };
    let placement = place_instance(client, &limits, instance, dnet, Some(look)).await;
    if let Err(OpsError::UnknownDevice { asked, .. }) = &placement.placed {
        return Err(match asked {
            Some(Scope::Network(network)) => format!(
                "Device {name} not found: it did not answer a Who-Is on network {network} \
                 within {} ms",
                wait.as_millis()
            ),
            Some(_) => format!(
                "Device {name} not found: it did not answer a Who-Is on this network within \
                 {} ms",
                wait.as_millis()
            ),
            None => format!(
                "Device {name} not found: the discovery table holds no row for it, and the \
                 Who-Is for it couldn't be sent"
            ),
        });
    }
    if let Some(refusal) = placement.refusal(&name, &|at| at.to_string()) {
        return Err(refusal);
    }
    let address = placement.placed.map_err(|error| error.to_string())?;
    Ok(Destination {
        address,
        instance: Some(instance),
    })
}
