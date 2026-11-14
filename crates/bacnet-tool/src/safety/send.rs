//! A write's requests and their audit fields: sending a confirmed preview
//! to the address it was planned at, taking the turn there, and finding a
//! device again before a revert.

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

/// The turn at the gate of `address`, the one its reads take too (#2560),
/// for the requests of one change sent there after it: plan, gate and send
/// share the address, so no write is gated at one device and sent to
/// another.
///
/// # Errors
///
/// [`Error::UnknownDevice`] for a routed address the discovery table has
/// no row for, so no route: the caller reports the write as not sent and
/// sends nothing.
pub async fn turn<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RequestLimits,
    address: &DeviceAddress,
) -> Result<Turn, Error> {
    let never = AtomicBool::new(false);
    let reader = Reader {
        client,
        device: address.clone(),
        instance: None,
        rate,
        cancel: &never,
        rpm: true,
    };
    reader.turn().await.map_err(|halt| match halt {
        Halt::Device(error) => error,
        Halt::Cancelled => Error::Encoding("the turn was cancelled".into()),
    })
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
    let local = address.is_local() || client.network_number() == Some(address.network());
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

/// Where the device `instance` is, for a ledger entry written before
/// #2560, which kept no address: the one row claiming it, after a Who-Is
/// for that instance when no row does yet.
///
/// # Errors
///
/// [`Error::AmbiguousDevice`] when two or more addresses claim it, and
/// [`Error::UnknownDevice`] when none does even after the Who-Is.
pub(super) async fn locate<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    instance: u32,
) -> Result<DeviceAddress, Error> {
    let heard = Heard::Claims(instance);
    if !heard.holds(client).await
        && let Ok(range) = DeviceInstanceRange::new(instance, instance)
    {
        match client.who_is(Some(range)).await {
            Ok(()) => {
                if !heard.wait(client).await {
                    tracing::warn!("device {instance} did not answer a Who-Is");
                }
            }
            Err(error) => tracing::warn!("looking for device {instance}: {error}"),
        }
    }
    DeviceAddress::claiming(client, instance).await
}

/// What a revert's Who-Is waits to hear of.
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
