//! Resources and templates (design §6). Names and titles are built from
//! identifiers only: hosts show resource lists to the model and the user.
//!
//! A read runs like a tool call: under its own account and budget, every
//! request through the engine's limits, cancelled by dropping it. A watched
//! point is read from the watch hub, which sends nothing. Content is JSON,
//! with device text in envelopes and values as `{display, typed}`, the
//! tools' shapes, and a `cost` block.
//!
//! A read that is refused before anything is sent (a bad URI, a device not
//! discovered or out of reach, a passive server) is a JSON-RPC -32602
//! whose `data` is the tool error body; one that failed after sending (the
//! device answered with an error, or not at all) is a -32603 with the same
//! body. Under `--passive` only `bacnet://session` and `bacnet://ledger`
//! are listed, and the others are refused with `refused: passive`.

mod content;
pub(crate) mod uri;

use std::sync::Arc;
use std::time::Duration;

use bacnet_ops::limits::{Account, charged};
use bacnet_transport::port::TransportPort;
use rmcp::ErrorData;
use rmcp::model::{ReadResourceResult, Resource, ResourceContents, ResourceTemplate};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub(crate) use self::content::{Cache, point_body, refresh_alarms};
use self::uri::{ALARMS, DEVICES, LEDGER, SESSION, Uri};
use crate::context::{Call, Ctx};
use crate::errors::ToolError;

const JSON: &str = "application/json";

/// How long past its budget a read may run while a request finishes.
const GRACE: Duration = Duration::from_secs(15);

/// The fixed resources this server lists.
pub(crate) fn list<T: TransportPort + 'static>(ctx: &Ctx<T>) -> Vec<Resource> {
    let all = [
        (
            DEVICES,
            "devices",
            "Discovered devices",
            "The devices in reach in the discovery table; updated as devices arrive or are lost",
        ),
        (
            ALARMS,
            "alarms",
            "Active alarms",
            "Active and unacknowledged alarms of the devices in reach, polled every 60 s",
        ),
        (
            SESSION,
            "session",
            "MCP session",
            "The server's level, limits, rate and watch footprint",
        ),
        (
            LEDGER,
            "ledger",
            "Write ledger",
            "This principal's writes still in effect, with what undoes each",
        ),
    ];
    all.into_iter()
        .filter(|(uri, ..)| ctx.engine.is_some() || *uri == SESSION || *uri == LEDGER)
        .map(|(uri, name, title, description)| {
            Resource::new(uri, name)
                .with_title(title)
                .with_description(description)
                .with_mime_type(JSON)
        })
        .collect()
}

/// The resource templates; none in passive mode.
pub(crate) fn templates<T: TransportPort + 'static>(ctx: &Ctx<T>) -> Vec<ResourceTemplate> {
    if ctx.engine.is_none() {
        return Vec::new();
    }
    [
        (
            "bacnet://devices/after/{instance}",
            "devices-after",
            "More discovered devices",
            "The next page of the device list: the devices whose instance is above the one \
             given, as many as bacnet://devices holds; its `next` names the page after",
        ),
        (
            "bacnet://device/{instance}",
            "device",
            "A device",
            "One device's summary: vendor, model, revisions, services and object types",
        ),
        (
            "bacnet://device/{instance}/object/{object}",
            "point",
            "A point",
            "An object's point facts (present value, status, units, priorities); a \
             subscription watches it by COV, or by polling when the device refuses",
        ),
        (
            "bacnet://device/{instance}/object/{object}/property/{property}",
            "property",
            "A property",
            "One property of an object; a subscription watches it by COV for the property, \
             or by polling",
        ),
        (
            "bacnet://device/{instance}/alarms",
            "device-alarms",
            "A device's alarms",
            "One device's active and unacknowledged alarms, polled every 60 s",
        ),
    ]
    .into_iter()
    .map(|(template, name, title, description)| {
        ResourceTemplate::new(template, name)
            .with_title(title)
            .with_description(description)
            .with_mime_type(JSON)
    })
    .collect()
}

/// Read the resource at `uri`.
pub(crate) async fn read<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    text: &str,
    cancel: CancellationToken,
) -> Result<ReadResourceResult, ErrorData> {
    let parsed = uri::parse(text).map_err(|why| {
        let error = ToolError::invalid("uri", why);
        ErrorData::invalid_params(error.summary(), Some(error.json()))
    })?;
    let account = Arc::new(Account::new(Some(Arc::clone(&ctx.bucket))));
    let budget = ctx.policy.caps.budget;
    let call = Call::new(budget, Arc::clone(&account));
    let work = async {
        match parsed {
            Uri::Point(point) => content::point(ctx, &call, point).await,
            Uri::Devices => content::devices(ctx, None).await,
            Uri::DevicesAfter(after) => content::devices(ctx, Some(after)).await,
            Uri::Device(device) => content::device(ctx, &call, device).await,
            Uri::Alarms => content::alarms(ctx, &call).await,
            Uri::DeviceAlarms(device) => content::device_alarms(ctx, &call, device).await,
            Uri::Session => Ok(content::session(ctx)),
            Uri::Ledger => Ok(content::ledger(ctx)),
        }
    };
    let finished = tokio::select! {
        finished = tokio::time::timeout(budget + GRACE, charged(account, work)) => finished,
        () = cancel.cancelled() => {
            ctx.log_read("resources/read", &call, "cancelled");
            return Err(ErrorData::internal_error("the read was cancelled", None));
        }
    };
    let outcome = finished.unwrap_or(Err(ToolError::Timeout {
        device: None,
        after_ms: u64::try_from((budget + GRACE).as_millis()).unwrap_or(u64::MAX),
    }));
    match outcome {
        Ok(mut body) => {
            body["uri"] = Value::from(text);
            body["cost"] = call.cost();
            ctx.log_read("resources/read", &call, "ok");
            let contents = ResourceContents::text(body.to_string(), text).with_mime_type(JSON);
            Ok(ReadResourceResult::new(vec![contents]))
        }
        Err(error) => {
            ctx.log_read("resources/read", &call, &error.summary());
            let mut data = error.json();
            data["cost"] = call.cost();
            Err(match error {
                ToolError::Invalid { .. }
                | ToolError::Refused { .. }
                | ToolError::UnknownDevice { .. } => {
                    ErrorData::invalid_params(error.summary(), Some(data))
                }
                _ => ErrorData::internal_error(error.summary(), Some(data)),
            })
        }
    }
}
