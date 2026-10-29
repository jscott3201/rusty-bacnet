//! `bacnet_read_trend`: a Trend Log or Trend Log Multiple buffer, by
//! ReadRange, paged across the device's answers and across calls.

use bacnet_client::log_reader::LogPage;
use bacnet_ops::format::LogColumns;
use bacnet_services::read_range::LogRecords;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::ObjectType;
use serde_json::{Map, Value, json};

pub(super) use super::log_pages::Args;
use super::log_pages::{self, Pager};
use super::trend_data::{Entry, Summary};
use super::{Spec, schema};
use crate::context::{Call, Ctx};
use crate::errors::ToolError;
use crate::value::object_text;

pub(super) const NAME: &str = "bacnet_read_trend";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "Read a trend log",
    description: "Read a Trend Log or Trend Log Multiple's records (ReadRange), oldest first: \
                  from (records logged after this local time, YYYY-MM-DDTHH:MM[:SS]), to (up \
                  to this time), or from the oldest; at most max_records a call (default \
                  500), then next_cursor goes on. Each record has its time, sequence number, \
                  values in typed form, and flagged when its status flags show a problem or \
                  it records a change in the log's own status. The result summarises the \
                  records it read: count, first and last time, and for each column min, max \
                  and mean of numbers and counts of states. Enumerations and bit strings \
                  are named by the property the log samples (its Log_DeviceObjectProperty, \
                  read once a call); a binary or multi-state object's states are counted \
                  by state, with the object's state texts (read once a call) in each \
                  column's state_texts. summary: true reads the whole window on the \
                  server (up to 20000 records) and returns only the summary. Each request is charged. \
                  Read-only.",
    input,
    output,
    idempotent: true,
    open_world: true,
    passive: false,
};

fn input() -> Value {
    log_pages::input("The log as type:instance: trend-log:1, trend-log-multiple:2")
}

fn output() -> Value {
    let column = json!({
        "type": "object",
        "properties": {
            "column": { "type": "integer" },
            "numeric": {
                "type": ["object", "null"],
                "properties": {
                    "count": { "type": "integer" },
                    "min": { "type": ["number", "null"] },
                    "max": { "type": ["number", "null"] },
                    "mean": { "type": ["number", "null"] },
                },
            },
            "states": { "type": ["object", "null"], "description": "Samples by state" },
            "state_texts": {
                "type": ["object", "null"],
                "description": "The monitored object's text for each key of states, untrusted",
                "additionalProperties": schema::envelope_object(),
            },
            "failures": { "type": "integer" },
            "other": { "type": "integer" },
        },
    });
    let value = json!({
        "anyOf": [
            schema::typed(),
            {
                "type": "object",
                "properties": { "error": { "type": "object" } },
                "required": ["error"],
                "additionalProperties": false,
            },
        ],
    });
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
                    "kind": { "enum": ["values", "log-status", "time-change"] },
                    "flagged": { "type": "boolean" },
                    "values": { "type": "array", "items": value },
                    "status_flags": schema::typed(),
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
                "columns": { "type": "array", "items": column },
            },
        },
    });
    if let Some(map) = properties.as_object_mut() {
        log_pages::paging_output(map);
    }
    schema::output(properties)
}

/// A page's records, oldest first.
fn entries(page: &LogPage) -> Vec<Entry> {
    match &page.records {
        LogRecords::TrendLog(records) => records.iter().map(Entry::trend).collect(),
        LogRecords::TrendLogMultiple(records) => records.iter().map(Entry::multiple).collect(),
        _ => Vec::new(),
    }
}

pub(super) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    args: Args,
    raw: &Map<String, Value>,
) -> Result<Value, ToolError> {
    let log = args.log(
        &[ObjectType::TREND_LOG, ObjectType::TREND_LOG_MULTIPLE],
        "a trend-log or trend-log-multiple object",
    )?;
    let to = args.to()?;
    let limit = args.limit()?;
    let device = ctx.device(call, &args.device).await?;
    let engine = ctx.engine()?;
    let cursor = args.cursor(ctx, NAME, raw)?;
    let object = object_text(log.object_type(), log.instance_number());
    let source = format!("device {} {object} log-buffer", device.instance);
    let cap = ctx.policy.caps.text;
    let mut columns = LogColumns::default();
    let mut summary = Summary::default();
    let mut pager = Pager::new(device.instance, log, cursor, limit);
    while let Some(page) = pager.next(engine, call).await? {
        if pager.pages == 1 {
            // What each column samples, to name its values: one charged
            // read a call, once the device has answered, so a silent one
            // costs a single timeout; a failure names nothing (#2091).
            let (named, by_reference) = engine.log_columns(device.instance, log).await;
            columns = named;
            // Then the state texts of the objects whose states columns
            // sample, one charged request an object, skipped once the
            // budget is spent (#2122).
            let by_texts = engine
                .log_state_texts(device.instance, &mut columns, || call.out_of_time())
                .await;
            // Naming never fails the read, but its timeouts count (#2131).
            for naming in [by_reference, by_texts] {
                if naming.timed_out() {
                    call.timed_out();
                }
            }
        }
        for (i, entry) in entries(&page).iter().enumerate() {
            let Some(seq) = pager.admit(&page, i, &entry.time, to.as_deref()) else {
                break;
            };
            summary.add(entry, &columns);
            if !args.summary {
                pager.push(entry.json(seq, &source, cap, &columns));
            }
        }
        if !pager.read_on(&page) {
            break;
        }
    }
    let mut body = json!({
        "device": device.instance,
        "object": object,
        "summary": summary.json(&columns, device.instance, cap),
    });
    pager.finish(ctx, NAME, raw, &mut body);
    Ok(body)
}
