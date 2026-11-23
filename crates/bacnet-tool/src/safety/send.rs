//! A write's requests and their audit fields: sending a confirmed preview
//! to the address it was planned at, in the turn [`super::place`] gives.

use bacnet_client::client::BACnetClient;
use bacnet_client::client::Station;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::{Error, ErrorDetail, RemoteError};
use bacnet_types::primitives::ObjectIdentifier;
use serde_json::{Value, json};

use super::{Outcome, Planned, Preview};
use bacnet_ops::DeviceAddress;
use bacnet_ops::octets::hex;

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
    let result = client
        .write_property_multiple(&Station::from(&preview.address), specs)
        .await;
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
    let (object, property, index) = target;
    client
        .write_property(
            &Station::from(address),
            object,
            property,
            index,
            value,
            priority,
        )
        .await
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
