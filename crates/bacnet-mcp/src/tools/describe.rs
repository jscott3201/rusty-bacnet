//! `bacnet_describe_device`: what a device is, from its Device object.

use std::collections::BTreeMap;

use bacnet_ops::format::{RawRow, decode_all};
use bacnet_ops::label::kebab;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{PropertyIdentifier, ResolvedEnum, resolve_enum};
use bacnet_types::primitives::PropertyValue;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Spec, list::listing, schema};
use crate::context::{Call, Ctx, DeviceArg};
use crate::errors::{ToolError, row_error_json};
use crate::untrusted::envelope;
use crate::value::{bit_names, row_text};

pub(super) const NAME: &str = "bacnet_describe_device";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "Describe a BACnet device",
    description: "Summarise one device from its Device object: name, vendor, model, firmware \
                  and software versions, protocol revision, the services and object types it \
                  supports (by name), APDU size, segmentation, database revision, and where it \
                  is. count_objects: true also counts its objects by type. Read-only.",
    input,
    output,
    idempotent: true,
    open_world: true,
    passive: false,
};

fn input() -> Value {
    schema::input(
        json!({
            "device": schema::device(),
            "count_objects": {
                "type": "boolean",
                "default": false,
                "description": "Also read the Object_List and count objects by type",
            },
        }),
        &["device"],
    )
}

fn output() -> Value {
    let names = json!({ "type": "array", "items": { "type": "string" } });
    schema::output(json!({
        "device": { "type": "integer" },
        "address": { "type": "string" },
        "router": { "type": ["string", "null"] },
        "network": { "type": "integer" },
        "name": schema::envelope(),
        "vendor_name": schema::envelope(),
        "model_name": schema::envelope(),
        "firmware_revision": schema::envelope(),
        "application_software_version": schema::envelope(),
        "vendor_id": { "type": ["integer", "null"] },
        "protocol_version": { "type": ["integer", "null"] },
        "protocol_revision": { "type": ["integer", "null"] },
        "max_apdu": { "type": ["integer", "null"] },
        "segmentation": { "type": ["string", "null"] },
        "database_revision": { "type": ["integer", "null"] },
        "services_supported": names,
        "object_types_supported": names,
        "errors": { "type": "object", "description": "Properties the device couldn't give, with why" },
        "object_counts": {
            "type": ["object", "null"],
            "description": "Objects by type; null while a long Object_List is still being read \
                            (call again to go on)",
        },
        "object_list": { "type": "object" },
        "via": { "enum": ["rpm", "rp"] },
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Args {
    device: DeviceArg,
    #[serde(default)]
    count_objects: bool,
}

impl Args {
    /// The summary of `device` alone, for its resource.
    pub(crate) fn of(device: u32) -> Self {
        Self {
            device: DeviceArg::Number(device),
            count_objects: false,
        }
    }
}

/// The value of `property` among `rows`, decoded.
fn values(rows: &[RawRow], property: PropertyIdentifier) -> Option<Vec<PropertyValue>> {
    let (_, value) = rows.iter().find(|(p, _)| *p == property)?;
    decode_all(value.as_deref().ok()?)
}

fn unsigned(rows: &[RawRow], property: PropertyIdentifier) -> Option<u64> {
    match values(rows, property)?.as_slice() {
        [PropertyValue::Unsigned(n)] => Some(*n),
        _ => None,
    }
}

/// The names of a bit string's set bits (services, object types).
fn set_bits(rows: &[RawRow], property: PropertyIdentifier) -> Vec<String> {
    match values(rows, property).as_deref() {
        Some([bits @ PropertyValue::BitString { .. }]) => bit_names(property, bits),
        _ => Vec::new(),
    }
}

pub(crate) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    args: Args,
) -> Result<Value, ToolError> {
    let device = ctx.device(call, &args.device).await?;
    let engine = ctx.engine()?;
    let summary = engine
        .describe_device(device.instance)
        .await
        .map_err(|e| ToolError::device(device.instance, &e))?;
    let rows = &summary.rows;
    let cap = ctx.policy.caps.text;
    let text = |property: PropertyIdentifier| {
        let source = format!(
            "device {} device:{} {}",
            device.instance,
            device.instance,
            kebab(property)
        );
        row_text(rows, property).map_or(Value::Null, |text| envelope(&text, source, cap))
    };
    let segmentation = match values(rows, PropertyIdentifier::SEGMENTATION_SUPPORTED).as_deref() {
        Some([value @ PropertyValue::Enumerated(raw)]) => {
            match resolve_enum(PropertyIdentifier::SEGMENTATION_SUPPORTED, value) {
                Some(ResolvedEnum::Unknown(_)) | None => Some(raw.to_string()),
                Some(named) => Some(kebab(named)),
            }
        }
        _ => None,
    };
    let errors: Map<String, Value> = rows
        .iter()
        .filter_map(|(property, value)| {
            let error = value.as_ref().err()?;
            Some((kebab(property), row_error_json(error)))
        })
        .collect();
    let entry = &device.entry;
    let routed = entry.source_network.zip(entry.source_address.as_deref());
    let (address, router) = engine.address_style().locate(&entry.mac_address, routed);
    let mut body = json!({
        "device": device.instance,
        "address": address,
        "router": router,
        "network": device.network,
        "name": text(PropertyIdentifier::OBJECT_NAME),
        "vendor_name": text(PropertyIdentifier::VENDOR_NAME),
        "model_name": text(PropertyIdentifier::MODEL_NAME),
        "firmware_revision": text(PropertyIdentifier::FIRMWARE_REVISION),
        "application_software_version": text(PropertyIdentifier::APPLICATION_SOFTWARE_VERSION),
        "vendor_id": unsigned(rows, PropertyIdentifier::VENDOR_IDENTIFIER),
        "protocol_version": unsigned(rows, PropertyIdentifier::PROTOCOL_VERSION),
        "protocol_revision": unsigned(rows, PropertyIdentifier::PROTOCOL_REVISION),
        "max_apdu": unsigned(rows, PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED),
        "segmentation": segmentation,
        "database_revision": unsigned(rows, PropertyIdentifier::DATABASE_REVISION),
        "services_supported": set_bits(rows, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED),
        "object_types_supported": set_bits(rows, PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED),
        "errors": errors,
        "via": match summary.via {
            bacnet_ops::engine::ReadVia::Rpm => "rpm",
            bacnet_ops::engine::ReadVia::Rp => "rp",
        },
        "limits": ctx.limits(None),
    });
    if args.count_objects {
        let listing = listing(ctx, call, &device).await?;
        body["object_list"] = listing.progress();
        body["object_counts"] = if listing.complete() {
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for object in listing.objects.iter() {
                *counts.entry(kebab(object.object_type())).or_default() += 1;
            }
            json!(counts)
        } else {
            Value::Null
        };
    }
    Ok(body)
}
