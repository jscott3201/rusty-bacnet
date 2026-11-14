//! A write's requests and their audit fields: sending a confirmed preview
//! to the address it was planned at, taking the turn there once the device
//! there is still the one planned for, and finding a device again before a
//! revert.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ConfirmedServiceChoice, PropertyIdentifier};
use bacnet_types::error::{Error, ErrorDetail, RemoteError};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;
use serde_json::{Value, json};

use super::{Outcome, Planned, Preview};
use bacnet_ops::DeviceAddress;
use bacnet_ops::client::{Halt, Reader};
use bacnet_ops::limits::{RequestLimits, Turn};
use bacnet_ops::octets::hex;

/// How long a revert waits for a device it had to look for.
const FIND_FOR: Duration = Duration::from_secs(3);

/// Send the preview as one WriteProperty or one WritePropertyMultiple, and
/// give each write its outcome. A WritePropertyMultiple error names the
/// first write that failed: the ones before it took effect and the rest
/// were not tried (Clause 15.10.2). A batch names each property once (the
/// plan and the worker both refuse a repeat), so that name is unambiguous.
pub(super) async fn send<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    preview: &Preview,
) -> Vec<Outcome> {
    let items = &preview.items;
    if !preview.batch {
        let Some(item) = items.first() else {
            return Vec::new();
        };
        let spec = &item.spec;
        let result = write_property(
            client,
            &preview.address,
            (spec.object, spec.property, spec.index),
            item.value.bytes.clone(),
            item.priority,
        )
        .await;
        return vec![Outcome::of(&result)];
    }
    let mut specs: Vec<WriteAccessSpecification> = Vec::new();
    for item in items {
        let value = BACnetPropertyValue {
            property_identifier: item.spec.property,
            property_array_index: item.spec.index,
            value: item.value.bytes.clone(),
            priority: item.priority,
        };
        match specs.last_mut() {
            Some(last) if last.object_identifier == item.spec.object => {
                last.list_of_properties.push(value);
            }
            _ => specs.push(WriteAccessSpecification {
                object_identifier: item.spec.object,
                list_of_properties: vec![value],
            }),
        }
    }
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: specs,
    };
    let mut buf = BytesMut::new();
    let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
    let result = match request.encode(&mut buf) {
        Ok(()) => confirmed(client, &preview.address, service, &buf).await,
        Err(error) => Err(error),
    };
    batch_outcomes(items, &result)
}

/// One WriteProperty of `value` to `target` (object, property and index)
/// at `address`.
pub(super) async fn write_property<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    address: &DeviceAddress,
    target: (ObjectIdentifier, PropertyIdentifier, Option<u32>),
    value: Vec<u8>,
    priority: Option<u8>,
) -> Result<(), Error> {
    let (object_identifier, property_identifier, property_array_index) = target;
    let request = WritePropertyRequest {
        object_identifier,
        property_identifier,
        property_array_index,
        property_value: value,
        priority,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf)?;
    confirmed(
        client,
        address,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        &buf,
    )
    .await
}

/// One confirmed request to `address`, its acknowledgement's data dropped.
async fn confirmed<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    address: &DeviceAddress,
    service: ConfirmedServiceChoice,
    data: &[u8],
) -> Result<(), Error> {
    client
        .confirmed_request_to(&address.into(), service, data)
        .await
        .map(drop)
}

/// Each write's outcome from one WritePropertyMultiple result: the ones
/// before the write the device names took effect, the rest weren't tried.
pub(crate) fn batch_outcomes(items: &[Planned], result: &Result<(), Error>) -> Vec<Outcome> {
    let failed_at = match result {
        Err(Error::Structured { detail, .. }) => match detail.as_ref() {
            ErrorDetail::FirstFailedWriteAttempt(at) => items.iter().position(|item| {
                item.spec.object == at.object_identifier
                    && item.spec.property.to_raw() == at.property_identifier
                    && item.spec.index == at.property_array_index
            }),
            _ => None,
        },
        _ => None,
    };
    let outcome = match (result, failed_at) {
        // An Error PDU without a failed write we can place leaves which
        // ones took effect unknown.
        (
            Err(error @ (Error::Remote(RemoteError::Error { .. }) | Error::Structured { .. })),
            None,
        ) => Outcome::Failed(error.to_string()),
        _ => Outcome::of(result),
    };
    (0..items.len())
        .map(|i| match failed_at {
            Some(at) if i < at => Outcome::Ack,
            Some(at) if i > at => Outcome::NotSent("an earlier write in the batch failed".into()),
            _ => outcome.clone(),
        })
        .collect()
}

/// Why a change was not sent to the address it was planned at (#2560).
/// Nothing went to the device for any of them.
#[derive(Debug)]
pub enum Unsent {
    /// The client has no way there, or failed to find out what is there:
    /// a routed address the discovery table has no row for (so no
    /// router), an identity read with no answer, or a device a Who-Is
    /// found no row for.
    Client(Error),
    /// Another device is at the address now: the discovery table's row
    /// there, or the device's own answer to a read of its Device object's
    /// Object_Identifier, names another instance than the one the change
    /// was confirmed for.
    OtherDevice {
        /// Where the change was to go.
        address: DeviceAddress,
        /// The instance it was planned and confirmed for.
        expected: u32,
        /// The instance at the address now.
        found: u32,
    },
    /// A revert of an entry kept without an address (written before
    /// #2560), whose instance two or more addresses claim now, so which
    /// device it changed is unknown.
    Contested {
        /// The entry's device instance.
        instance: u32,
        /// Every address claiming it, ordered.
        claims: Vec<DeviceAddress>,
    },
    /// A revert whose caller sends no Who-Is ([`Caller::may_look`]) of a
    /// device the discovery table can't place without one: a routed
    /// address with no row, or an entry without an address whose instance
    /// no row claims.
    ///
    /// [`Caller::may_look`]: super::desk::Caller::may_look
    NotLooking {
        /// The device as the entry names it: its address, or its instance.
        device: String,
    },
}

impl std::fmt::Display for Unsent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Client(error) => write!(f, "{error}"),
            Self::OtherDevice {
                address,
                expected,
                found,
            } => write!(
                f,
                "{address} is device {found} now, not device {expected}: nothing was sent there"
            ),
            Self::Contested { instance, claims } => {
                let claims: Vec<String> = claims.iter().map(ToString::to_string).collect();
                write!(
                    f,
                    "device {instance} is claimed by {}, and this entry predates the ledger \
                     keeping addresses, so which one it changed is unknown: revert it again \
                     once one address alone claims {instance}, or write its old value back by \
                     hand at the right one",
                    claims.join(" and ")
                )
            }
            Self::NotLooking { device } => write!(
                f,
                "{device} is not in the discovery table, and this revert sends no Who-Is to \
                 find it: discover the device, then revert again"
            ),
        }
    }
}

impl std::error::Error for Unsent {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Client(error) => Some(error),
            _ => None,
        }
    }
}

impl From<Error> for Unsent {
    fn from(error: Error) -> Self {
        Self::Client(error)
    }
}

/// The turn at the gate of `address`, the one its reads take too (#2560),
/// for the requests of one change to the device `instance` sent there
/// after it: plan, gate and send share the address, so no write is gated
/// at one device and sent to another. The discovery table's row at the
/// address is checked before the turn is taken and again once it is held:
/// one claiming another instance (the address re-announced as another
/// device) refuses the change. An address with no row is not checked here;
/// the desk's revert asks the device which it is first. Sends nothing.
///
/// # Errors
///
/// [`Unsent::OtherDevice`] when the row at the address claims another
/// instance, and [`Unsent::Client`] with [`Error::UnknownDevice`] for a
/// routed address the table has no row for, so no route. The caller
/// reports the change as not sent.
pub async fn turn<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RequestLimits,
    address: &DeviceAddress,
    instance: u32,
) -> Result<Turn, Unsent> {
    let never = AtomicBool::new(false);
    let reader = Reader {
        client,
        device: address.clone(),
        instance: Some(instance),
        rate,
        cancel: &never,
        rpm: true,
    };
    // Before the wait, so a refusal takes no slot; and again once the turn
    // is held, since the row may change while it waits.
    claimed(client, address, instance).await?;
    let held = reader.turn().await.map_err(|halt| match halt {
        Halt::Device(error) => error,
        Halt::Cancelled => Error::Encoding("the turn was cancelled".into()),
    })?;
    claimed(client, address, instance).await?;
    Ok(held)
}

/// The discovery table's row at `address` (localized, as the table keys
/// it), if any.
async fn row_at<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    address: &DeviceAddress,
) -> Option<DiscoveredDevice> {
    let at = address.clone().localized(client.network_number());
    client.get_device_at(at.network(), at.mac()).await
}

/// Refuse when the discovery table's row at `address` claims another
/// instance than `instance`. No row passes.
async fn claimed<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    address: &DeviceAddress,
    instance: u32,
) -> Result<(), Unsent> {
    match row_at(client, address).await {
        Some(row) if row.object_identifier.instance_number() != instance => {
            Err(Unsent::OtherDevice {
                address: address.clone(),
                expected: instance,
                found: row.object_identifier.instance_number(),
            })
        }
        _ => Ok(()),
    }
}

/// Where `entry` is reverted, and the turn there.
///
/// An entry with an address goes there. With no row at it, a local one is
/// asked which device it is first: one ReadProperty of its Device object's
/// Object_Identifier by the wildcard instance, refused unless it answers
/// with the entry's instance. A routed one has no route without a row: a
/// caller that may look ([`Caller::may_look`]) asks its network with a
/// Who-Is for the instance first; one that may not reports it.
///
/// An entry written before #2560 kept no address: its device is the one
/// row claiming its instance, after a Who-Is for it when none does and the
/// caller may look.
///
/// [`Caller::may_look`]: super::desk::Caller::may_look
pub(super) async fn revert_at<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RequestLimits,
    entry: &super::ledger::Entry,
    look: bool,
) -> Result<(DeviceAddress, Turn), Unsent> {
    let instance = entry.device;
    let address = match &entry.address {
        Some(address) => {
            let local = address
                .clone()
                .localized(client.network_number())
                .is_local();
            if local {
                if row_at(client, address).await.is_none() {
                    identify(client, rate, address, instance).await?;
                }
            } else if look {
                reach(client, address, instance).await;
            } else if row_at(client, address).await.is_none() {
                return Err(Unsent::NotLooking {
                    device: address.to_string(),
                });
            }
            address.clone()
        }
        None => {
            if look {
                look_for(client, instance).await;
            }
            match DeviceAddress::claiming(client, instance).await {
                Ok(address) => address,
                Err(Error::AmbiguousDevice { candidates, .. }) => {
                    let mut claims: Vec<DeviceAddress> = candidates
                        .iter()
                        .filter_map(|c| DeviceAddress::new(c.network_number, &c.mac_address).ok())
                        .collect();
                    claims.sort();
                    return Err(Unsent::Contested { instance, claims });
                }
                Err(Error::UnknownDevice { .. }) if !look => {
                    return Err(Unsent::NotLooking {
                        device: format!("device {instance}"),
                    });
                }
                Err(error) => return Err(Unsent::Client(error)),
            }
        }
    };
    let held = turn(client, rate, &address, instance).await?;
    Ok((address, held))
}

/// Ask the device at `address` which it is, by the wildcard instance (the
/// read part A of #2560 gave the device handles), and refuse unless it is
/// `instance`: one unicast, no Who-Is.
async fn identify<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RequestLimits,
    address: &DeviceAddress,
    instance: u32,
) -> Result<(), Unsent> {
    let never = AtomicBool::new(false);
    let mut reader = Reader {
        client,
        device: address.clone(),
        instance: None,
        rate,
        cancel: &never,
        rpm: true,
    };
    match reader.instance().await {
        Ok(found) if found == instance => Ok(()),
        Ok(found) => Err(Unsent::OtherDevice {
            address: address.clone(),
            expected: instance,
            found,
        }),
        Err(Halt::Device(error)) => Err(Unsent::Client(error)),
        Err(Halt::Cancelled) => Err(Unsent::Client(Error::Encoding(
            "the identity read was cancelled".into(),
        ))),
    }
}

/// Make sure the client has a route to `address` before a write to the
/// device `instance` there: a revert after a restart may come before any
/// Who-Is. A local address needs none, nor does one the discovery table
/// has a row for, so nothing is sent for them. A routed address without one
/// asks its network with a Who-Is for that one instance, and waits a
/// little for the device at that address to answer; another address
/// answering is never taken for it.
pub async fn reach<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    address: &DeviceAddress,
    instance: u32,
) {
    let local = address
        .clone()
        .localized(client.network_number())
        .is_local();
    let heard = Heard::At(address);
    if local || heard.holds(client).await {
        return;
    }
    let Ok(range) = DeviceInstanceRange::new(instance, instance) else {
        return;
    };
    if let Err(error) = client.who_is_network(address.network(), Some(range)).await {
        tracing::warn!("looking for device {instance} at {address}: {error}");
        return;
    }
    if !heard.wait(client).await {
        tracing::warn!("device {instance} at {address} did not answer a Who-Is");
    }
}

/// For a ledger entry written before #2560, which kept no address: when no
/// row claims the device `instance`, a Who-Is for that instance, and a
/// little wait for an answer.
async fn look_for<T: TransportPort + 'static>(client: &BACnetClient<T>, instance: u32) {
    let heard = Heard::Claims(instance);
    if heard.holds(client).await {
        return;
    }
    let Ok(range) = DeviceInstanceRange::new(instance, instance) else {
        return;
    };
    match client.who_is(Some(range)).await {
        Ok(()) => {
            if !heard.wait(client).await {
                tracing::warn!("device {instance} did not answer a Who-Is");
            }
        }
        Err(error) => tracing::warn!("looking for device {instance}: {error}"),
    }
}

/// What a Who-Is to find a device waits to hear of.
enum Heard<'a> {
    /// A row at this address.
    At(&'a DeviceAddress),
    /// A row claiming this instance.
    Claims(u32),
}

impl Heard<'_> {
    /// True when the discovery table holds it now.
    async fn holds<T: TransportPort + 'static>(&self, client: &BACnetClient<T>) -> bool {
        match self {
            Self::At(address) => client
                .get_device_at(address.network(), address.mac())
                .await
                .is_some(),
            Self::Claims(instance) => client
                .discovered_devices()
                .await
                .iter()
                .any(|d: &DiscoveredDevice| d.object_identifier.instance_number() == *instance),
        }
    }

    /// Check every 100 ms for up to [`FIND_FOR`]; true once it holds.
    async fn wait<T: TransportPort + 'static>(&self, client: &BACnetClient<T>) -> bool {
        let step = Duration::from_millis(100);
        let mut waited = Duration::ZERO;
        while waited < FIND_FOR {
            tokio::time::sleep(step).await;
            waited += step;
            if self.holds(client).await {
                return true;
            }
        }
        false
    }
}

/// A planned write's audit fields.
pub fn item_json(item: &Planned) -> Value {
    let spec = &item.spec;
    json!({
        "target": {
            "device": spec.device,
            "address": spec.address.to_string(),
            "object": bacnet_ops::label::object_label(spec.object),
            "property": bacnet_ops::label::kebab(spec.property),
            "index": spec.index,
        },
        "params": { "value": item.value.text, "encoded": hex(&item.value.bytes) },
        "priority": item.priority,
        "pre_image": {
            "value": item.before.as_ref().map(|b| b.text.clone()),
            "encoded": item.before.as_ref().map(|b| hex(&b.bytes)),
            "out_of_service": item.out_of_service,
        },
    })
}
