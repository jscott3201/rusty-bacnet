//! `bacnet_watch`: the tool-only fallback for hosts without subscriptions
//! (Claude Desktop's remote connectors, older clients). It puts points on
//! the watch hub, as a subscription would, and answers "what changed since
//! my last call": the cursor holds the hub's sequence number at that call,
//! so a point unchanged since then comes back as `changed: false` without
//! values, and reading it sent nothing. A point stays watched while calls
//! keep asking for it, and goes [`TOOL_IDLE`] after the last.

use std::time::Duration;

use bacnet_ops::parse::{parse_object_specifier, parse_property};
use bacnet_ops::watch::{Point, State, Watched};
use bacnet_transport::port::TransportPort;
use bacnet_types::primitives::ObjectIdentifier;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::time::Instant;

use super::{Spec, next, schema, start};
use crate::context::{Call, Ctx, DeviceArg};
use crate::errors::ToolError;
use crate::listen::{SHUTTING_DOWN, TOOL_IDLE};
use crate::resources::point_body;
use crate::resources::uri::of_point;

pub(super) const NAME: &str = "bacnet_watch";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "Watch BACnet points",
    description: "Follow points and see what changed since the last call, for hosts without \
                  resource subscriptions. The first call starts a watch on each point (a COV \
                  subscription, or polling every 10 s when the device refuses) and returns \
                  every value; call again with next_cursor and the same points to get only \
                  what changed (changed: false, no values, for the rest). Reading a watched \
                  point sends nothing. A point stays watched while calls ask for it, and \
                  stops 10 minutes after the last. Each point is an object (its point facts) \
                  or one property of it. Read-only: subscriptions are finite and cancelled \
                  when the watch ends.",
    input,
    output,
    idempotent: false,
    open_world: true,
    passive: false,
};

/// Points one call may name.
const MAX_POINTS: usize = 32;

/// How long a first call waits for its points' first reads.
const FIRST_READ: Duration = Duration::from_secs(10);

fn input() -> Value {
    schema::input(
        json!({
            "points": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_POINTS,
                "items": {
                    "type": "object",
                    "properties": {
                        "device": schema::device(),
                        "object": schema::object(),
                        "property": {
                            "type": "string",
                            "description": "One property (kebab-case name or number, [index] \
                                            for an element); left out, the object's point facts",
                        },
                    },
                    "required": ["device", "object"],
                    "additionalProperties": false,
                },
            },
            "cursor": schema::cursor(),
        }),
        &["points"],
    )
}

fn output() -> Value {
    schema::output(json!({
        "points": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "uri": { "type": "string" },
                    "device": { "type": "integer" },
                    "object": { "type": "string" },
                    "property": { "type": "string" },
                    "changed": { "type": "boolean" },
                    "watched": { "type": "boolean" },
                    "source": { "enum": ["cov", "poll", "starting"] },
                    "state": { "enum": ["starting", "live", "failed", "unreachable", "stopped"] },
                    "note": { "type": ["string", "null"] },
                    "seq": { "type": "integer" },
                    "age_ms": { "type": ["integer", "null"] },
                    "updates": { "type": "object" },
                    "values": {
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
                    "error": { "type": "object" },
                },
                "required": ["uri"],
            },
        },
        "watching": { "type": "object" },
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    points: Vec<PointArg>,
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PointArg {
    device: DeviceArg,
    object: String,
    property: Option<String>,
}

/// What a point argument asks for, parsed: nothing is sent for a bad one.
fn watched(arg: &PointArg) -> Result<(ObjectIdentifier, Watched), ToolError> {
    let (object_type, instance) =
        parse_object_specifier(&arg.object).map_err(|e| ToolError::invalid("object", e))?;
    let object = ObjectIdentifier::new(object_type, instance)
        .map_err(|e| ToolError::invalid("object", e.to_string()))?;
    let watched = match &arg.property {
        None => Watched::Object,
        Some(text) => {
            let (property, index) =
                parse_property(text).map_err(|e| ToolError::invalid("property", e))?;
            Watched::Property { property, index }
        }
    };
    Ok((object, watched))
}

pub(super) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    args: Args,
    raw: &Map<String, Value>,
) -> Result<Value, ToolError> {
    ctx.engine()?;
    let Some(hub) = ctx.hub.as_ref() else {
        return Err(ToolError::refused("passive", "the server is passive"));
    };
    let max = ctx.policy.caps.watches;
    if args.points.is_empty() || args.points.len() > MAX_POINTS {
        return Err(ToolError::invalid(
            "points",
            format!("1 to {MAX_POINTS} points"),
        ));
    }
    let parsed = args
        .points
        .iter()
        .map(watched)
        .collect::<Result<Vec<_>, _>>()?;
    // Position 0 is a first call; a later one holds the sequence number
    // plus one, so a hub with no change yet still reads as "since".
    let since = start(ctx, NAME, args.cursor.as_deref(), raw)?.checked_sub(1);
    ctx.subs.expire_tool(TOOL_IDLE);
    let mut rows: Vec<Result<Point, (String, Value)>> = Vec::new();
    for (arg, (object, watched)) in args.points.iter().zip(parsed) {
        let row = match ctx.device(call, &arg.device).await {
            Ok(device) => {
                let point = Point {
                    device: device.instance,
                    object,
                    watched,
                };
                match ctx.subs.tool_watch(ctx, point) {
                    Ok(()) => Ok(point),
                    Err(SHUTTING_DOWN) => {
                        let error =
                            ToolError::refused(SHUTTING_DOWN, "the server is shutting down");
                        Err((of_point(&point), error.json()))
                    }
                    Err(cap) => {
                        let error = ToolError::refused(
                            "watch-cap",
                            format!("no room for another watched point ({cap}); max {max}"),
                        );
                        Err((of_point(&point), error.json()))
                    }
                }
            }
            Err(error) => {
                let object =
                    crate::value::object_text(object.object_type(), object.instance_number());
                let uri = format!("bacnet://device/{}/object/{object}", arg.device.text());
                Err((uri, error.json()))
            }
        };
        rows.push(row);
    }
    // A new point's first read and subscription take a moment: wait for
    // them, within the budget.
    let deadline = (Instant::now() + FIRST_READ).min(call.deadline);
    while Instant::now() < deadline
        && rows.iter().flatten().any(|point| {
            hub.peek(point).is_some_and(|snapshot| {
                snapshot.source.is_none() && matches!(snapshot.state, State::Starting | State::Live)
            })
        })
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Read before the points: a change landing in between shows again next
    // time, never not at all.
    let now_seq = hub.seq();
    let points: Vec<Value> = rows
        .into_iter()
        .map(|row| match row {
            Ok(point) => {
                let Some(snapshot) = hub.peek(&point) else {
                    return json!({ "uri": of_point(&point), "error": { "error": "failed",
                        "message": "the watch ended" } });
                };
                let changed = since.is_none_or(|since| snapshot.seq > since);
                let rows = if changed { &snapshot.rows[..] } else { &[] };
                let mut body = point_body(ctx, &point, rows, Some(&snapshot));
                body["uri"] = json!(of_point(&point));
                body["changed"] = json!(changed);
                if !changed && let Some(map) = body.as_object_mut() {
                    map.remove("values");
                }
                body
            }
            Err((uri, error)) => json!({ "uri": uri, "error": error }),
        })
        .collect();
    Ok(json!({
        "points": points,
        "watching": {
            "held": ctx.subs.held(),
            "max": max,
            "idle_s": TOOL_IDLE.as_secs(),
        },
        "truncated": false,
        "next_cursor": next(ctx, NAME, raw, now_seq + 1),
        "limits": ctx.limits(None),
    }))
}
