//! The write worker's requests and their audit fields: sending a confirmed
//! preview, finding a device again before a revert, and comparing a fresh
//! read with what was previewed.

use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::{Error, ErrorDetail};
use serde_json::{json, Value};

use crate::tui::safety::{Gate, Outcome, Planned, Preview, NULL};

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
        let result = client
            .write_property_to_device(
                spec.device,
                spec.object,
                spec.property,
                spec.index,
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
        .write_property_multiple_to_device(preview.device, specs)
        .await;
    batch_outcomes(items, &result)
}

/// Each write's outcome from one WritePropertyMultiple result.
pub(super) fn batch_outcomes(items: &[Planned], result: &Result<(), Error>) -> Vec<Outcome> {
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
        (Err(error @ (Error::Protocol { .. } | Error::Structured { .. })), None) => {
            Outcome::Failed(error.to_string())
        }
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

/// Make sure the client knows where `device` is before writing to it: a
/// revert after a restart may come before any Who-Is. Asks with a Who-Is
/// for that one instance and waits a little for the I-Am.
pub(super) async fn find<T: TransportPort + 'static>(client: &BACnetClient<T>, device: u32) {
    let known = |devices: &[bacnet_client::discovery::DiscoveredDevice]| {
        devices
            .iter()
            .any(|d| d.object_identifier.instance_number() == device)
    };
    if known(&client.discovered_devices().await) {
        return;
    }
    let Ok(range) = DeviceInstanceRange::new(device, device) else {
        return;
    };
    if let Err(error) = client.who_is(Some(range)).await {
        tracing::warn!("looking for device {device}: {error}");
        return;
    }
    let step = Duration::from_millis(100);
    let mut waited = Duration::ZERO;
    while waited < FIND_FOR {
        tokio::time::sleep(step).await;
        waited += step;
        if known(&client.discovered_devices().await) {
            return;
        }
    }
    tracing::warn!("device {device} did not answer a Who-Is");
}

/// True when a fresh read plans the same writes as the confirmed preview:
/// the same pre-images, slot holders and priorities.
pub(super) fn unchanged(confirmed: &Preview, fresh: &Preview) -> bool {
    confirmed.items.len() == fresh.items.len()
        && confirmed.items.iter().zip(&fresh.items).all(|(a, b)| {
            (a.before == b.before)
                && (a.occupancy == b.occupancy)
                && (a.priority == b.priority)
                && (a.commandable == b.commandable)
        })
}

/// A relinquish: NULL to a slot, typed or from `n`.
pub(super) fn relinquishes(item: &Planned) -> bool {
    item.value.bytes == NULL
}

pub(super) fn gate_name(gate: &Gate) -> String {
    match gate {
        Gate::Allowed(tier) => format!("{tier:?}"),
        Gate::Blocked(_) => "blocked".into(),
    }
}

/// A planned write's audit fields.
pub(super) fn item_json(item: &Planned) -> Value {
    let spec = &item.spec;
    json!({
        "target": {
            "device": spec.device,
            "object": crate::tui::app::objects::object_label(spec.object),
            "property": crate::tui::app::objects::kebab(spec.property),
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
