//! `bacnet_alarms`: a device's active alarms and unacknowledged events
//! (GetEventInformation), and optionally its alarm and enrollment
//! summaries, decoded by `bacnet_ops::format::alarms`.

use bacnet_ops::format::alarms::{
    AlarmSummaryData, EnrollmentSummaryData, EventSummaryData, read_event_information_until,
};
use bacnet_services::enrollment_summary::GetEnrollmentSummaryRequest;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::AcknowledgmentFilter;
use bacnet_types::primitives::ObjectIdentifier;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Spec, next, schema, start};
use crate::context::{Call, Ctx, DeviceArg};
use crate::errors::ToolError;
use crate::value::enveloped;

pub(super) const NAME: &str = "bacnet_alarms";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "Read a device's alarms",
    description: "Read one device's event summaries (GetEventInformation): every object in \
                  alarm or fault, or with a transition not yet acknowledged, with its event \
                  state, acknowledged transitions, enabled transitions, notify type, the time \
                  of each transition's last event and their priorities, all decoded and \
                  named. Paged across the device's answers; at most max a call (default 100), \
                  then next_cursor. summaries: true also reads the alarm summary and the \
                  enrollment summary on the first page (a device may not support them; that \
                  shows as an error beside them, not a failed call). Read-only.",
    input,
    output,
    idempotent: true,
    open_world: true,
    passive: false,
};

/// Event summaries one call returns, at most and by default.
const MAX_EVENTS: usize = 100;

fn input() -> Value {
    schema::input(
        json!({
            "device": schema::device(),
            "max": { "type": "integer", "minimum": 1, "maximum": MAX_EVENTS, "default": MAX_EVENTS },
            "summaries": {
                "type": "boolean",
                "default": false,
                "description": "Also read GetAlarmSummary and GetEnrollmentSummary",
            },
            "cursor": schema::cursor(),
        }),
        &["device"],
    )
}

/// An object of `fields`, each a typed value.
fn typed_fields(fields: &[&str]) -> Value {
    let properties: Map<String, Value> = fields
        .iter()
        .map(|field| ((*field).to_string(), schema::typed()))
        .collect();
    json!({ "type": "object", "properties": properties, "required": fields })
}

fn output() -> Value {
    let transitions = typed_fields(&["to_offnormal", "to_fault", "to_normal"]);
    let mut event = typed_fields(&[
        "object",
        "event_state",
        "acknowledged_transitions",
        "event_enable",
        "notify_type",
    ]);
    event["properties"]["event_timestamps"] = transitions.clone();
    event["properties"]["event_priorities"] = transitions;
    let summaries = |item: Value| json!({ "type": ["array", "null"], "items": item });
    schema::output(json!({
        "device": { "type": "integer" },
        "events": { "type": "array", "items": event },
        "more_events": {
            "type": "boolean",
            "description": "The device holds more: call again with next_cursor",
        },
        "alarm_summary": summaries(typed_fields(&[
            "object",
            "alarm_state",
            "acknowledged_transitions",
        ])),
        "enrollment_summary": summaries(typed_fields(&[
            "object",
            "event_type",
            "event_state",
            "priority",
            "notification_class",
        ])),
        "summary_errors": {
            "type": "object",
            "description": "Why a summary couldn't be read, by name",
        },
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Args {
    device: DeviceArg,
    max: Option<usize>,
    #[serde(default)]
    summaries: bool,
    cursor: Option<String>,
}

impl Args {
    /// The first page of `device`'s events alone, for its resource.
    pub(crate) fn of(device: u32) -> Self {
        Self {
            device: DeviceArg::Number(device),
            max: None,
            summaries: false,
            cursor: None,
        }
    }
}

/// The object a page resumes after, packed into a cursor position: 0 for
/// the first page, else the object's encoding plus one.
fn after(position: u64) -> Result<Option<ObjectIdentifier>, ToolError> {
    let unusable = || ToolError::invalid("cursor", "not a usable cursor");
    match position.checked_sub(1) {
        None => Ok(None),
        Some(raw) => {
            let raw = u32::try_from(raw).map_err(|_| unusable())?;
            ObjectIdentifier::decode(&raw.to_be_bytes())
                .map(Some)
                .map_err(|_| unusable())
        }
    }
}

/// The cursor position resuming after `object`.
fn position(object: ObjectIdentifier) -> u64 {
    u64::from(u32::from_be_bytes(object.encode())) + 1
}

pub(crate) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    args: Args,
    raw: &Map<String, Value>,
) -> Result<Value, ToolError> {
    let max = args.max.unwrap_or(MAX_EVENTS);
    if !(1..=MAX_EVENTS).contains(&max) {
        return Err(ToolError::invalid("max", format!("1 to {MAX_EVENTS}")));
    }
    let device = ctx.device(call, &args.device).await?;
    let engine = ctx.engine()?;
    let resume = after(start(ctx, NAME, args.cursor.as_deref(), raw)?)?;
    let instance = device.instance;
    // The first answer continues from the cursor's object; later ones
    // from the last object of the answer before. Once the call's budget is
    // spent no further answer is asked for: what was read comes back with
    // more_events set and a cursor after its last object.
    let (summaries, more) = read_event_information_until(
        max,
        || call.out_of_time(),
        |next_after| engine.alarms(instance, next_after.or(resume)),
    )
    .await
    .map_err(|e| ToolError::device(instance, &e))?;
    let cap = ctx.policy.caps.text;
    let source = format!("device {instance} event-information");
    let events: Vec<Value> = summaries
        .iter()
        .map(|summary| enveloped(&EventSummaryData::new(summary), &source, cap))
        .collect();
    let last = summaries.last().map(|summary| summary.object_identifier);
    let mut body = json!({
        "device": instance,
        "events": events,
        "more_events": more,
        "alarm_summary": null,
        "enrollment_summary": null,
        "truncated": more,
        "next_cursor": last
            .filter(|_| more)
            .map(|object| next(ctx, NAME, raw, position(object))),
        "limits": ctx.limits(Some(max)),
    });
    // The summaries come only when the events left time for them: a call
    // whose budget went on the events sends neither, and says so under
    // summary_errors, so the agent can ask for them on their own.
    if args.summaries && args.cursor.is_none() && call.out_of_time() {
        let skipped = json!({
            "skipped": "budget",
            "message": "the call's budget went on the event summaries; nothing was sent for \
                        this summary: call again with summaries true and max smaller",
        });
        body["summary_errors"] = json!({ "alarm": skipped, "enrollment": skipped });
    } else if args.summaries && args.cursor.is_none() {
        let mut errors = Map::new();

        match engine.alarm_summary(instance).await {
            Ok(ack) => {
                let source = format!("device {instance} alarm-summary");
                let entries: Vec<Value> = ack
                    .entries
                    .iter()
                    .map(|entry| enveloped(&AlarmSummaryData::new(entry), &source, cap))
                    .collect();
                body["alarm_summary"] = json!(entries);
            }
            Err(error) => {
                errors.insert("alarm".into(), ToolError::device(instance, &error).json());
            }
        }
        let request = GetEnrollmentSummaryRequest {
            acknowledgment_filter: AcknowledgmentFilter::ALL,
            enrollment_filter: None,
            event_state_filter: None,
            event_type_filter: None,
            priority_filter: None,
            notification_class_filter: None,
        };
        match engine.enrollment_summary(instance, &request).await {
            Ok(ack) => {
                let source = format!("device {instance} enrollment-summary");
                let entries: Vec<Value> = ack
                    .entries
                    .iter()
                    .map(|entry| enveloped(&EnrollmentSummaryData::new(entry), &source, cap))
                    .collect();
                body["enrollment_summary"] = json!(entries);
            }
            Err(error) => {
                errors.insert(
                    "enrollment".into(),
                    ToolError::device(instance, &error).json(),
                );
            }
        }
        if !errors.is_empty() {
            body["summary_errors"] = Value::Object(errors);
        }
    }
    Ok(body)
}
