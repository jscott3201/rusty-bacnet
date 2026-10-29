//! `bacnet_read_log_records`: an Event Log or Audit Log buffer, by
//! ReadRange, paged across the device's answers and across calls as
//! `bacnet_read_trend` pages a trend log (#2103). A sibling tool rather
//! than more of the trend tool: these records are notifications, not
//! samples, so they have no columns, and their summary counts states and
//! operations instead of averaging values.

use bacnet_transport::port::TransportPort;
use bacnet_types::enums::ObjectType;
use serde_json::{Map, Value, json};

use super::log_pages::{self, Args, Pager};
use super::log_records_data::{Summary, records};
use super::{Spec, schema};
use crate::context::{Call, Ctx};
use crate::errors::ToolError;
use crate::value::object_text;

pub(super) const NAME: &str = "bacnet_read_log_records";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "Read an event or audit log",
    description: "Read an Event Log's or Audit Log's records (ReadRange), oldest first: from \
                  (records logged after this local time, YYYY-MM-DDTHH:MM[:SS]), to (up to \
                  this time), or from the oldest; at most max_records a call (default 500), \
                  then next_cursor goes on. Each record has its time, sequence number and \
                  kind: an event notification (objects, states, event type and event values \
                  decoded and named), an audit notification (operation, source and target, \
                  values written and held, the error when it failed), a change in the log's \
                  own status, or a clock change; flagged marks a log status change, an event \
                  into a non-normal state and a failed operation. The result summarises the \
                  records it read: counts by kind, events by state entered, event type and \
                  notify type, or operations by name and how many failed. summary: true \
                  reads the whole window on the server (up to 20000 records) and returns \
                  only the summary. Each request is charged. Read-only.",
    input,
    output,
    idempotent: true,
    open_world: true,
    passive: false,
};

fn input() -> Value {
    log_pages::input("The log as type:instance: event-log:1, audit-log:2")
}

fn output() -> Value {
    let counts = |what: &str| {
        json!({
            "type": "object",
            "description": what,
            "additionalProperties": { "type": "integer" },
        })
    };
    // The notifications' fields are typed values, device text enveloped.
    let fields = json!({ "type": "object", "additionalProperties": schema::typed() });
    let mut properties = json!({
        "device": { "type": "integer" },
        "object": { "type": "string" },
        "records": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "time": { "type": "string" },
                    "seq": { "type": "integer" },
                    "kind": { "enum": ["notification", "audit", "log-status", "time-change"] },
                    "flagged": { "type": "boolean" },
                    "notification": fields,
                    "audit": {
                        "type": "object",
                        "properties": {
                            "result": {
                                "type": ["object", "null"],
                                "additionalProperties": schema::typed(),
                            },
                        },
                        "additionalProperties": schema::typed(),
                    },
                    "log_status": schema::typed(),
                    "time_change_s": { "type": ["number", "null"] },
                },
                "required": ["time", "kind", "flagged"],
            },
        },
        "summary": {
            "type": "object",
            "description": "Of the records this call read",
            "properties": {
                "records": { "type": "integer" },
                "flagged": { "type": "integer" },
                "log_status": { "type": "integer" },
                "time_changes": { "type": "integer" },
                "first_time": { "type": ["string", "null"] },
                "last_time": { "type": ["string", "null"] },
                "notifications": { "type": "integer" },
                "by_to_state": counts("Event notifications by the state they entered"),
                "by_event_type": counts("Event notifications by event type"),
                "by_notify_type": counts("Event notifications by notify type"),
                "audits": { "type": "integer" },
                "failed": { "type": "integer", "description": "Audited operations that failed" },
                "by_operation": counts("Audit notifications by operation"),
            },
        },
    });
    if let Some(map) = properties.as_object_mut() {
        log_pages::paging_output(map);
    }
    schema::output(properties)
}

pub(super) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    args: Args,
    raw: &Map<String, Value>,
) -> Result<Value, ToolError> {
    let log = args.log(
        &[ObjectType::EVENT_LOG, ObjectType::AUDIT_LOG],
        "an event-log or audit-log object (trend logs: bacnet_read_trend)",
    )?;
    // Every argument is checked before the device is looked up, so a bad
    // one costs nothing.
    let to = args.to()?;
    let limit = args.limit()?;
    let cursor = args.cursor(ctx, NAME, raw)?;
    let device = ctx.device(call, &args.device).await?;
    let engine = ctx.engine()?;
    let object = object_text(log.object_type(), log.instance_number());
    let source = format!("device {} {object} log-buffer", device.instance);
    let cap = ctx.policy.caps.text;
    let mut summary = Summary::default();
    let mut pager = Pager::new(device.instance, log, cursor, limit);
    while let Some(page) = pager.next(engine, call).await? {
        for (i, record) in records(&page).iter().enumerate() {
            let Some(seq) = pager.admit(&page, i, &record.time, to.as_deref()) else {
                break;
            };
            summary.add(record);
            if !args.summary {
                pager.push(record.json(seq, &source, cap));
            }
        }
        if !pager.read_on(&page) {
            break;
        }
    }
    let mut body = json!({
        "device": device.instance,
        "object": object,
        "summary": summary.json(log.object_type() == ObjectType::AUDIT_LOG),
    });
    pager.finish(ctx, NAME, raw, &mut body);
    Ok(body)
}
