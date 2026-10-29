//! `bacnet_read`: properties of one device's objects, by
//! ReadPropertyMultiple with the engine's ReadProperty fallback.

use bacnet_ops::engine::{DEVICE_PROPERTIES, ReadVia, Wanted, timed_out, unreachable};
use bacnet_ops::format::{self, RawRow};
use bacnet_ops::label::kebab;
use bacnet_ops::parse::{parse_object_specifier, parse_property};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{MAX_STRUCTURED, Spec, next, schema, start};
use crate::context::{Call, Ctx, DeviceArg};
use crate::errors::{ToolError, row_error_json};
use crate::value::{object_text, property_value};

pub(super) const NAME: &str = "bacnet_read";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "Read BACnet properties",
    description: "Read properties of one device's objects (ReadPropertyMultiple, falling back to \
                  ReadProperty). Each value comes as display text and a typed form with \
                  enumeration and bit names. preset \"point\" reads what an agent needs about a \
                  point (present value, status, units, priority array, ...); preset \"device\" \
                  reads the Device object's summary properties. A property the device can't \
                  give is an error row, not a failed call. Read-only.",
    input,
    output,
    idempotent: true,
    open_world: true,
    passive: false,
};

/// The properties `preset: "point"` reads: what an agent needs to reason
/// about a point, and what a write preview reads. A point resource and
/// its watch read the same.
const POINT: [PropertyIdentifier; 10] = bacnet_ops::watch::POINT_PROPERTIES;

/// Objects one call may list.
const MAX_READS: usize = 100;

fn input() -> Value {
    schema::input(
        json!({
            "device": schema::device(),
            "reads": {
                "type": "array",
                "maxItems": MAX_READS,
                "description": "The objects to read, each with the properties wanted",
                "items": {
                    "type": "object",
                    "properties": {
                        "object": schema::object(),
                        "properties": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "kebab-case names or numbers, with [index] for one \
                                            array element (priority-array[8]); \"all\" for every \
                                            property. Left out: the preset's, or present-value",
                        },
                    },
                    "required": ["object"],
                    "additionalProperties": false,
                },
            },
            "preset": {
                "enum": ["point", "device"],
                "description": "point: the point facts for each object without properties; \
                                device: the Device object's summary as well",
            },
            "format": { "enum": ["display", "typed", "both"], "default": "both" },
            "cursor": schema::cursor(),
        }),
        &["device"],
    )
}

fn output() -> Value {
    schema::output(json!({
        "device": { "type": "integer" },
        "via": { "enum": ["rpm", "rp", "mixed", "none"] },
        "results": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "object": { "type": "string" },
                    "property": { "type": "string" },
                    "index": { "type": ["integer", "null"] },
                    "value": schema::value(),
                    "error": { "type": "object" },
                },
                "required": ["object", "property"],
            },
        },
        "stopped": { "type": "object" },
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    device: DeviceArg,
    #[serde(default)]
    reads: Vec<ReadSpec>,
    preset: Option<Preset>,
    #[serde(default)]
    format: Format,
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadSpec {
    object: String,
    properties: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Preset {
    Point,
    Device,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Format {
    Display,
    Typed,
    #[default]
    Both,
}

/// A property as read: its array index, and its value or the error body.
type Row = (PropertyIdentifier, Option<u32>, Result<Vec<u8>, Value>);

/// One object's read, parsed.
struct Plan {
    object: ObjectIdentifier,
    /// Every property, by ALL.
    all: bool,
    /// Properties read together.
    list: Vec<PropertyIdentifier>,
    /// Array elements, read one by one.
    elements: Vec<(PropertyIdentifier, u32)>,
}

fn plans(device: u32, args: &Args) -> Result<Vec<Plan>, ToolError> {
    if args.reads.len() > MAX_READS {
        return Err(ToolError::invalid(
            "reads",
            format!("at most {MAX_READS} objects a call"),
        ));
    }
    let mut plans = Vec::new();
    if args.preset == Some(Preset::Device) {
        let object = ObjectIdentifier::new(ObjectType::DEVICE, device)
            .map_err(|e| ToolError::invalid("device", e.to_string()))?;
        plans.push(Plan {
            object,
            all: false,
            list: DEVICE_PROPERTIES.to_vec(),
            elements: Vec::new(),
        });
    }
    for spec in &args.reads {
        let (object_type, instance) =
            parse_object_specifier(&spec.object).map_err(|e| ToolError::invalid("object", e))?;
        let object = ObjectIdentifier::new(object_type, instance)
            .map_err(|e| ToolError::invalid("object", e.to_string()))?;
        let mut plan = Plan {
            object,
            all: false,
            list: Vec::new(),
            elements: Vec::new(),
        };
        match &spec.properties {
            None if args.preset == Some(Preset::Point) => plan.list = POINT.to_vec(),
            None => plan.list = vec![PropertyIdentifier::PRESENT_VALUE],
            Some(names) if names.is_empty() => {
                return Err(ToolError::invalid(
                    "properties",
                    "name at least one property",
                ));
            }
            Some(names) => {
                for name in names {
                    if name.eq_ignore_ascii_case("all") {
                        plan.all = true;
                        continue;
                    }
                    match parse_property(name).map_err(|e| ToolError::invalid("properties", e))? {
                        (property, Some(index)) => plan.elements.push((property, index)),
                        (property, None) => plan.list.push(property),
                    }
                }
            }
        }
        plans.push(plan);
    }
    if plans.is_empty() {
        return Err(ToolError::invalid(
            "reads",
            "name an object to read, or preset device",
        ));
    }
    Ok(plans)
}

pub(super) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    args: Args,
    raw: &Map<String, Value>,
) -> Result<Value, ToolError> {
    let device = ctx.device(call, &args.device).await?;
    let engine = ctx.engine()?;
    let plans = plans(device.instance, &args)?;
    let first = start(ctx, NAME, args.cursor.as_deref(), raw)? as usize;
    let cap = ctx.policy.caps.values;
    let mut results = Vec::new();
    let mut size = 0;
    let mut vias = Vec::new();
    let mut resume = None;
    let mut stopped = None;
    for (at, plan) in plans.iter().enumerate().skip(first) {
        let full = results.len() >= cap || size >= MAX_STRUCTURED;
        if at > first && (full || call.out_of_time()) {
            resume = Some(at);
            break;
        }
        let mut rows: Vec<Row> = Vec::new();
        let mut failed = None;
        if plan.all || !plan.list.is_empty() {
            let wanted = if plan.all {
                Wanted::All
            } else {
                Wanted::List(&plan.list)
            };
            match engine.read(device.instance, plan.object, wanted).await {
                Ok(read) => {
                    vias.push(read.via);
                    for (property, value) in read.rows {
                        rows.push((property, None, value.map_err(|e| row_error_json(&e))));
                    }
                }
                Err(error) => failed = Some(error),
            }
        }
        for &(property, index) in &plan.elements {
            if failed.is_some() {
                break;
            }
            match engine
                .read_property(device.instance, plan.object, property, Some(index))
                .await
            {
                Ok(data) => rows.push((property, Some(index), Ok(data))),
                Err(error) if unreachable(&error) => failed = Some(error),
                Err(error) => {
                    let body = ToolError::device(device.instance, &error).json();
                    rows.push((property, Some(index), Err(body)));
                }
            }
        }
        if let Some(error) = failed {
            if results.is_empty() {
                return Err(ToolError::device(device.instance, &error));
            }
            if timed_out(&error) {
                call.timed_out();
            }
            // Later objects would fail the same way: stop here, and say why.
            stopped = Some(ToolError::device(device.instance, &error).json());
            resume = Some(at);
            break;
        }
        let units = format::units(
            &rows
                .iter()
                .filter_map(|(p, i, v)| match (i, v) {
                    (None, Ok(data)) => Some((*p, Ok(data.clone()))),
                    _ => None,
                })
                .collect::<Vec<RawRow>>(),
        );
        for (property, index, value) in rows {
            let row = result_row(
                device.instance,
                plan.object,
                property,
                index,
                value,
                units,
                args.format,
                ctx.policy.caps.text,
            );
            size += row.to_string().len();
            results.push(row);
        }
    }
    let via = match (vias.contains(&ReadVia::Rpm), vias.contains(&ReadVia::Rp)) {
        (true, true) => "mixed",
        (true, false) => "rpm",
        (false, true) => "rp",
        (false, false) => "none",
    };
    let mut body = json!({
        "device": device.instance,
        "via": via,
        "results": results,
        "truncated": resume.is_some(),
        "next_cursor": resume.map(|at| next(ctx, NAME, raw, at as u64)),
        "limits": ctx.limits(Some(cap)),
    });
    if let Some(stopped) = stopped {
        body["stopped"] = stopped;
    }
    Ok(body)
}

/// One property's row.
#[allow(clippy::too_many_arguments)]
pub(crate) fn result_row(
    device: u32,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: Result<Vec<u8>, Value>,
    units: Option<bacnet_types::enums::EngineeringUnits>,
    format: Format,
    cap: usize,
) -> Value {
    let object_name = object_text(object.object_type(), object.instance_number());
    let property_name = kebab(property);
    let mut row = json!({ "object": object_name, "property": property_name, "index": index });
    match value {
        Ok(data) => {
            let source = format!("device {device} {object_name} {property_name}");
            let mut value = property_value(
                object.object_type(),
                property,
                index,
                &data,
                units,
                &source,
                cap,
            );
            match format {
                Format::Display => drop(value.as_object_mut().map(|v| v.remove("typed"))),
                Format::Typed => drop(value.as_object_mut().map(|v| v.remove("display"))),
                Format::Both => {}
            }
            row["value"] = value;
        }
        Err(error) => row["error"] = error,
    }
    row
}
