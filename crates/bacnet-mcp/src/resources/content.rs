//! What each resource holds.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bacnet_ops::engine::{Wanted, unreachable};
use bacnet_ops::format::{self, RawRow, RowError};
use bacnet_ops::label::kebab;
use bacnet_ops::limits::{Account, charged};
use bacnet_ops::watch::{Point, Snapshot, Source, State, Watched};
use bacnet_transport::port::TransportPort;
use serde_json::{Map, Value, json};
use tokio::time::Instant;

use super::uri::{self, Uri};
use crate::context::{Call, Ctx, DeviceArg};
use crate::errors::{ToolError, row_error_json};
use crate::listen::ALARMS_EVERY;
use crate::tools::read::{Format, result_row};
use crate::tools::{alarms, describe, discover};
use crate::value::object_text;

/// How long a device summary is served from the cache.
const DEVICE_TTL: Duration = Duration::from_secs(300);

/// The fewest devices whose summaries or alarms are kept; a policy whose
/// device page is larger keeps that many, so a full alarm sweep fits.
const CACHE_CAP: usize = 64;

/// A cache of resource bodies by device, at most [`CACHE_CAP`] or the
/// device page of them.
pub(crate) type Cache = Mutex<HashMap<u32, (Instant, Value)>>;

fn cached(cache: &Cache, device: u32, ttl: Duration) -> Option<Value> {
    let cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    let (at, body) = cache.get(&device)?;
    (at.elapsed() < ttl).then(|| body.clone())
}

fn keep(cache: &Cache, cap: usize, device: u32, body: &Value) {
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    if cache.len() >= cap.max(CACHE_CAP)
        && !cache.contains_key(&device)
        && let Some(oldest) = cache.iter().min_by_key(|(_, (at, _))| *at).map(|(d, _)| *d)
    {
        cache.remove(&oldest);
    }
    cache.insert(device, (Instant::now(), body.clone()));
}

/// A point: from the watch hub when it is watched (nothing is sent),
/// else read once.
pub(super) async fn point<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    point: Point,
) -> Result<Value, ToolError> {
    ctx.device(call, &DeviceArg::Number(point.device)).await?;
    if let Some(snapshot) = ctx.hub.as_ref().and_then(|hub| hub.peek(&point)) {
        return Ok(point_body(ctx, &point, &snapshot.rows, Some(&snapshot)));
    }
    let engine = ctx.engine()?;
    let failed = |error| ToolError::device(point.device, &error);
    let rows: Vec<RawRow> = match point.watched {
        Watched::Object => {
            engine
                .read(
                    point.device,
                    point.object,
                    Wanted::List(&bacnet_ops::watch::POINT_PROPERTIES),
                )
                .await
                .map_err(failed)?
                .rows
        }
        Watched::Property { property, index } => {
            match engine
                .read_property(point.device, point.object, property, index)
                .await
            {
                Ok(data) => vec![(property, Ok(data))],
                Err(error) if unreachable(&error) => return Err(failed(error)),
                Err(error) => vec![(property, Err(RowError::from(error)))],
            }
        }
    };
    Ok(point_body(ctx, &point, &rows, None))
}

/// A point's content: its values as the tools give them, and how its watch
/// goes (`source` is `cov` or `poll` for a watched point, `read` for one
/// read just now).
pub(crate) fn point_body<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    point: &Point,
    rows: &[RawRow],
    watch: Option<&Snapshot>,
) -> Value {
    let units = format::units(
        &rows
            .iter()
            .filter(|(_, v)| v.is_ok())
            .cloned()
            .collect::<Vec<RawRow>>(),
    );
    let index = match point.watched {
        Watched::Object => None,
        Watched::Property { index, .. } => index,
    };
    let values: Vec<Value> = rows
        .iter()
        .map(|(property, value)| {
            result_row(
                point.device,
                point.object,
                *property,
                index,
                value.clone().map_err(|e| row_error_json(&e)),
                units,
                Format::Both,
                ctx.policy.caps.text,
            )
        })
        .collect();
    let mut body = json!({
        "device": point.device,
        "object": object_text(point.object.object_type(), point.object.instance_number()),
        "watched": watch.is_some(),
        "values": values,
    });
    if let Watched::Property { property, .. } = point.watched {
        body["property"] = json!(kebab(property));
    }
    match watch {
        None => body["source"] = json!("read"),
        Some(snapshot) => {
            body["source"] = json!(match snapshot.source {
                Some(Source::Cov) => "cov",
                Some(Source::Poll) => "poll",
                None => "starting",
            });
            body["state"] = json!(state_name(snapshot.state));
            body["note"] = json!(snapshot.note);
            body["seq"] = json!(snapshot.seq);
            body["age_ms"] = json!(
                snapshot
                    .updated
                    .map(|at| u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX))
            );
            body["updates"] = json!({
                "notifications": snapshot.notifications,
                "coalesced": snapshot.coalesced,
                "polls": snapshot.polls,
                "renewals": snapshot.renewals,
            });
        }
    }
    body
}

pub(crate) fn state_name(state: State) -> &'static str {
    match state {
        State::Starting => "starting",
        State::Live => "live",
        // The hub gives a point up only under `MissingDevice::Fail`, which
        // the server doesn't use; it would be a failed watch.
        State::Failed | State::Gone => "failed",
        State::Unreachable => "unreachable",
        State::Stopped => "stopped",
    }
}

/// A page of the discovery table, in reach: no device text, nothing sent.
/// Devices are sorted by instance, and `after` starts the page past that
/// instance. The cursor is an instance, not a position, so a device found
/// between two reads shifts nothing already listed. `total` counts every
/// device in reach, `truncated` says whether more follow this page, and
/// `next` names the page that holds them.
pub(super) async fn devices<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    after: Option<u32>,
) -> Result<Value, ToolError> {
    let engine = ctx.engine()?;
    let mut devices = engine.devices().await;
    devices.retain(|found| {
        ctx.policy.allows_network(found.source_network.unwrap_or(0))
            && ctx
                .policy
                .allows_device(found.object_identifier.instance_number())
    });
    devices.sort_by_key(|found| found.object_identifier.instance_number());
    let total = devices.len();
    let start = after.map_or(0, |after| {
        devices.partition_point(|found| found.object_identifier.instance_number() <= after)
    });
    let page = ctx.policy.caps.devices;
    let shown = &devices[start..total.min(start.saturating_add(page))];
    let rows: Vec<Value> = shown
        .iter()
        .map(|found| {
            ctx.remember(found);
            discover::device_row(ctx, found)
        })
        .collect();
    let truncated = start + shown.len() < total;
    let mut body = json!({
        "devices": rows,
        "total": total,
        "truncated": truncated,
        "hint": "bacnet_discover finds more; this lists only devices already heard",
    });
    if truncated && let Some(last) = shown.last() {
        body["next"] = json!(uri::devices_after(last.object_identifier.instance_number()));
    }
    Ok(body)
}

/// A device's summary, served from a cache for five minutes.
pub(super) async fn device<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    device: u32,
) -> Result<Value, ToolError> {
    ctx.device(call, &DeviceArg::Number(device)).await?;
    if let Some(mut body) = cached(&ctx.described, device, DEVICE_TTL) {
        body["cached"] = json!(true);
        return Ok(body);
    }
    let mut body = describe::run(ctx, call, describe::Args::of(device)).await?;
    keep(&ctx.described, ctx.policy.caps.devices, device, &body);
    body["cached"] = json!(false);
    Ok(body)
}

/// A device's alarms, read at most once a minute.
pub(super) async fn device_alarms<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    device: u32,
) -> Result<Value, ToolError> {
    ctx.device(call, &DeviceArg::Number(device)).await?;
    alarms_of(ctx, call, device, ALARMS_EVERY).await
}

async fn alarms_of<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    device: u32,
    ttl: Duration,
) -> Result<Value, ToolError> {
    if let Some(body) = cached(&ctx.alarms, device, ttl) {
        return Ok(body);
    }
    let mut body = alarms::run(ctx, call, alarms::Args::of(device), &Map::new()).await?;
    if let Some(map) = body.as_object_mut() {
        // A cursor belongs to the tool; the resource holds the first page.
        for key in [
            "next_cursor",
            "alarm_summary",
            "enrollment_summary",
            "limits",
        ] {
            map.remove(key);
        }
    }
    keep(&ctx.alarms, ctx.policy.caps.devices, device, &body);
    Ok(body)
}

/// The alarms of every device in reach, each read at most once a minute,
/// as far as the call's budget goes.
pub(super) async fn alarms<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
) -> Result<Value, ToolError> {
    let engine = ctx.engine()?;
    let mut devices: Vec<u32> = engine
        .devices()
        .await
        .into_iter()
        .filter(|found| {
            ctx.policy.allows_network(found.source_network.unwrap_or(0))
                && ctx
                    .policy
                    .allows_device(found.object_identifier.instance_number())
        })
        .map(|found| found.object_identifier.instance_number())
        .collect();
    devices.sort_unstable();
    devices.truncate(ctx.policy.caps.devices);
    let mut read = Vec::new();
    let mut unread = Vec::new();
    let mut errors = Map::new();
    for device in devices {
        if call.out_of_time() && cached(&ctx.alarms, device, ALARMS_EVERY).is_none() {
            unread.push(device);
            continue;
        }
        match alarms_of(ctx, call, device, ALARMS_EVERY).await {
            Ok(body) => read.push(body),
            Err(error) => {
                errors.insert(device.to_string(), error.json());
            }
        }
    }
    Ok(json!({ "devices": read, "unread": unread, "errors": errors }))
}

/// Read a subscribed alarm resource again if its last read is half a
/// minute old; true when its events changed.
pub(crate) async fn refresh_alarms<T: TransportPort + 'static>(ctx: &Ctx<T>, text: &str) -> bool {
    let devices: Vec<u32> = match uri::parse(text) {
        Ok(Uri::DeviceAlarms(device)) => vec![device],
        Ok(Uri::Alarms) => {
            let mut devices = ctx.known_in_reach();
            devices.truncate(ctx.policy.caps.devices);
            devices
        }
        _ => return false,
    };
    let account = Arc::new(Account::new(Some(Arc::clone(&ctx.bucket))));
    let call = Call::new(ctx.policy.caps.budget, Arc::clone(&account));
    let events =
        |device: u32| cached(&ctx.alarms, device, Duration::MAX).map(|body| body["events"].clone());
    let work = async {
        let mut changed = false;
        for device in devices {
            if call.out_of_time() {
                break;
            }
            // A device not read before (or evicted) is unknown, not changed:
            // the subscriber reads the resource itself.
            let before = events(device);
            if alarms_of(ctx, &call, device, ALARMS_EVERY / 2)
                .await
                .is_ok()
                && let Some(before) = before
            {
                changed |= Some(before) != events(device);
            }
        }
        changed
    };
    let changed = charged(account, work).await;
    ctx.log_read("alarms refresh", &call, "ok");
    changed
}

/// The session: the server's level, limits, rate and watch footprint.
pub(super) fn session<T: TransportPort + 'static>(ctx: &Ctx<T>) -> Value {
    let caps = &ctx.policy.caps;
    let policy = &ctx.policy.writes;
    let offered = policy.offered() && !ctx.policy.passive;
    let level = if ctx.policy.passive {
        "passive"
    } else {
        ctx.writes.level(policy).audit_name()
    };
    let writes = if offered {
        json!({
            "mode": "elicit",
            "arm": match policy.arm {
                crate::policy::ArmMode::Never => "never",
                crate::policy::ArmMode::Elicit => "elicit",
            },
            "client_confirms": "T1 only",
        })
    } else {
        json!("not offered")
    };
    let grants: Vec<Value> = policy
        .plan
        .iter()
        .filter(|_| offered)
        .map(|plan| {
            json!({
                "kind": "plan",
                "in_force": ctx.writes.plan(policy).is_some(),
                "used": ctx.writes.plan_used(),
                "max_writes": plan.max_writes,
                "expires_unix": plan.expires_unix,
            })
        })
        .collect();
    let mut body = json!({
        "level": level,
        "writes": writes,
        "expires": ctx.writes.armed_for().map(|s| json!({ "in_s": s })),
        "grants": grants,
        "limits": ctx.limits(None),
        "watches": {
            "held": ctx.subs.held(),
            "max": caps.watches,
            "streams": *ctx.listening.borrow(),
            "legacy": ctx.subs.legacy_count(),
        },
    });
    if let Some(engine) = &ctx.engine {
        body["rate"] = json!({ "requests_last_10_s": engine.limits().recent() });
    }
    if let Some(hub) = &ctx.hub {
        let stats = hub.stats();
        body["cov"] = json!({
            "points": stats.points,
            "running": stats.tasks,
            "subscribed": stats.cov,
            "polled": stats.polled,
            "max": stats.max_points,
            "requests": stats.requests,
            "throttled_ms": u64::try_from(stats.throttled.as_millis()).unwrap_or(u64::MAX),
            "lagged": stats.lagged,
            "lifetime_s": hub.options().lifetime.secs(),
        });
    }
    body
}

/// This principal's ledger entries in effect, read from the workspace's
/// ledger (which the TUI shares): none when the server can't write.
pub(super) fn ledger<T: TransportPort + 'static>(ctx: &Ctx<T>) -> Value {
    let Some(workspace) = &ctx.policy.writes.workspace else {
        return json!({
            "entries": [],
            "note": "this server can't write, so its ledger is empty",
        });
    };
    let entries: Vec<Value> = bacnet_ops::safety::ledger::Ledger::open(&workspace.dir)
        .map(|ledger| ledger.entries().to_vec())
        .unwrap_or_default()
        .into_iter()
        .filter(|e| {
            e.origin.front_end == bacnet_ops::safety::FrontEnd::Mcp
                && e.origin.principal.as_deref() == Some(ctx.principal.as_str())
        })
        .map(|e| {
            json!({
                "ledger_id": e.id,
                "target": e.label(),
                "written": crate::untrusted::clean(&e.written.text, ctx.policy.caps.text).0,
                "undo": e.undo().map(|u| crate::untrusted::clean(&u.text, ctx.policy.caps.text).0),
                "state": e.state,
                "at": e.at,
            })
        })
        .collect();
    json!({ "entries": entries })
}
