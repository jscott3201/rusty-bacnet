//! `bacnet_list_objects`: a device's Object_List, paged.

use std::sync::{Arc, PoisonError};

use bacnet_ops::engine::{MAX_OBJECTS, Wanted, unreachable};
use bacnet_ops::format::decode_all;
use bacnet_ops::parse::parse_object_type;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::time::Instant;

use super::{Spec, next, schema, start};
use crate::context::{CachedList, Call, Ctx, Device, DeviceArg, LIST_CAP};
use crate::errors::ToolError;
use crate::untrusted::envelope;
use crate::value::{object_text, row_text};

pub(super) const NAME: &str = "bacnet_list_objects";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "List a device's objects",
    description: "List one device's objects from its Object_List, paged (read whole, or element \
                  by element when it can't come in one answer, and kept for 5 minutes for the \
                  next pages). Filter by object type; names: true also reads each listed \
                  object's name, one request each, so its pages are smaller. Read-only.",
    input,
    output,
    idempotent: true,
    open_world: true,
    passive: false,
};

/// Objects on a page that reads names, one request each.
const NAMED_PAGE: usize = 50;

fn input() -> Value {
    schema::input(
        json!({
            "device": schema::device(),
            "types": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Only these object types (ai, analog-output, trend-log, ...)",
            },
            "names": {
                "type": "boolean",
                "default": false,
                "description": "Also read each listed object's Object_Name",
            },
            "cursor": schema::cursor(),
        }),
        &["device"],
    )
}

fn output() -> Value {
    schema::output(json!({
        "device": { "type": "integer" },
        "objects": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "object": { "type": "string" },
                    "name": schema::envelope(),
                },
                "required": ["object"],
            },
        },
        "total": {
            "type": ["integer", "null"],
            "description": "Objects matching the filter; null until the list is all read",
        },
        "listed": { "type": ["integer", "null"], "description": "Objects in the Object_List" },
        "list": {
            "type": "object",
            "description": "How the list was read; when not complete, call again with \
                            next_cursor to go on reading where it stopped",
            "properties": {
                "cached": { "type": "boolean" },
                "indexed": { "type": "boolean" },
                "complete": { "type": "boolean" },
                "read": { "type": "integer" },
                "elements": { "type": ["integer", "null"] },
                "next_element": { "type": ["integer", "null"] },
                "gaps": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Elements the device answered with an error, passed over",
                },
                "stopped": { "type": ["string", "null"] },
            },
        },
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    device: DeviceArg,
    #[serde(default)]
    types: Vec<String>,
    #[serde(default)]
    names: bool,
    cursor: Option<String>,
}

/// How the Object_List was had.
pub(super) struct Listing {
    pub(super) objects: Arc<Vec<ObjectIdentifier>>,
    cached: bool,
    /// Read element by element: the element count, and the next element.
    indexed: Option<(u32, u32)>,
    /// Elements the device answered with an error, skipped.
    gaps: Vec<u32>,
    /// Why an element read stopped early (the device stopped answering),
    /// if one did.
    stopped: Option<String>,
}

impl Listing {
    /// Whether every entry has been read.
    pub(super) fn complete(&self) -> bool {
        self.indexed.is_none_or(|(total, next)| next > total)
    }

    /// Why an element read stopped early, if the device stopped answering.
    pub(super) fn stopped(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    /// How far the read has got, for a result.
    pub(super) fn progress(&self) -> Value {
        let (total, next) = self.indexed.unwrap_or((0, 0));
        json!({
            "cached": self.cached,
            "indexed": self.indexed.is_some(),
            "complete": self.complete(),
            "read": self.objects.len(),
            "elements": self.indexed.map(|_| total),
            "next_element": (!self.complete()).then_some(next),
            "gaps": self.gaps,
            "stopped": self.stopped,
        })
    }

    fn from_cache(kept: &CachedList, cached: bool, stopped: Option<String>) -> Self {
        Self {
            objects: Arc::clone(&kept.objects),
            cached,
            indexed: kept.indexed,
            gaps: kept.gaps.clone(),
            stopped,
        }
    }
}

/// The object identifiers in an application-tagged value.
fn identifiers(data: &[u8]) -> Vec<ObjectIdentifier> {
    decode_all(data)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| match value {
            PropertyValue::ObjectIdentifier(object) => Some(object),
            _ => None,
        })
        .collect()
}

/// An element count as read from element 0.
fn count(data: &[u8]) -> Option<u32> {
    match decode_all(data)?.as_slice() {
        [PropertyValue::Unsigned(n)] => Some(u32::try_from(*n).unwrap_or(u32::MAX)),
        _ => None,
    }
}

/// `device`'s Object_List, as far as this call's budget gets it.
///
/// A fresh complete list comes from the cache. Otherwise the list is read
/// whole, or, when the device can't send it in one answer, element by
/// element (element 0 holds the count). Calls on one device take turns at
/// its lock, so a second call goes on from where the first stopped and the
/// read never goes back. An element read stops when the call's budget is
/// spent (after at least one element) or the device stops answering; an
/// element the device answers with an error is a gap, recorded and passed.
/// What was read is kept with where to go on. A read resumed checks the
/// count again and starts over if it changed, and one begun more than
/// [`PARTIAL_TTL`](crate::context::PARTIAL_TTL) ago starts over.
pub(super) async fn listing<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    device: &Device,
) -> Result<Listing, ToolError> {
    let engine = ctx.engine()?;
    let lock = {
        let mut locks = ctx
            .list_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        locks.retain(|_, lock| Arc::strong_count(lock) > 1);
        Arc::clone(locks.entry(device.instance).or_default())
    };
    let _turn = lock.lock().await;
    let failed = |e: &Error| ToolError::device(device.instance, e);
    let kept = {
        let lists = ctx.lists.lock().unwrap_or_else(PoisonError::into_inner);
        lists
            .get(&device.instance)
            .filter(|kept| kept.fresh())
            .cloned()
    };
    if let Some(kept) = &kept
        && kept.complete()
    {
        return Ok(Listing::from_cache(kept, true, None));
    }
    let list_object = ObjectIdentifier::new(ObjectType::DEVICE, device.instance)
        .map_err(|e| ToolError::invalid("device", e.to_string()))?;
    let list = PropertyIdentifier::OBJECT_LIST;
    let read_count = async |whole: Option<&Error>| -> Result<u32, ToolError> {
        let data = engine
            .read_property(device.instance, list_object, list, Some(0))
            .await
            .map_err(|e| failed(whole.unwrap_or(&e)))?;
        let total = count(&data).ok_or_else(|| {
            whole.map_or_else(
                || ToolError::Failed("Object_List count isn't a number".into()),
                failed,
            )
        })?;
        if total > MAX_OBJECTS {
            return Err(ToolError::Failed(format!(
                "Object_List claims {total} objects; at most {MAX_OBJECTS} are read"
            )));
        }
        Ok(total)
    };
    let (mut state, cached) = match kept {
        Some(kept) => {
            // Resuming: the count must be what it was, or start over.
            let (total, _) = kept.indexed.unwrap_or((0, 1));
            let now = read_count(None).await?;
            if now == total {
                (kept, true)
            } else {
                tracing::info!(
                    "device {}: Object_List count went from {total} to {now}; reading it again",
                    device.instance
                );
                (fresh_partial(now), false)
            }
        }
        None => match engine
            .read_property(device.instance, list_object, list, None)
            .await
        {
            Ok(data) => {
                let whole = CachedList {
                    at: Instant::now(),
                    objects: Arc::new(identifiers(&data)),
                    indexed: None,
                    gaps: Vec::new(),
                };
                store(ctx, device.instance, whole.clone());
                return Ok(Listing::from_cache(&whole, false, None));
            }
            Err(e) if unreachable(&e) => return Err(failed(&e)),
            Err(whole) => (fresh_partial(read_count(Some(&whole)).await?), false),
        },
    };
    let (total, mut next) = state.indexed.unwrap_or((0, 1));
    let mut objects = state.objects.as_ref().clone();
    let mut stopped = None;
    let mut read_here = 0;
    while next <= total {
        if read_here > 0 && call.out_of_time() {
            break;
        }
        match engine
            .read_property(device.instance, list_object, list, Some(next))
            .await
        {
            Ok(data) => objects.extend(identifiers(&data)),
            Err(e) if unreachable(&e) => {
                if read_here == 0 && objects.is_empty() {
                    return Err(failed(&e));
                }
                stopped = Some(format!("element {next} of {total}: {e}"));
                break;
            }
            // The device has no answer for this element: a gap, passed.
            Err(_) => state.gaps.push(next),
        }
        next += 1;
        read_here += 1;
    }
    state.objects = Arc::new(objects);
    state.indexed = Some((total, next));
    store(ctx, device.instance, state.clone());
    Ok(Listing::from_cache(&state, cached, stopped))
}

/// An element-by-element read of `total` elements, begun now.
fn fresh_partial(total: u32) -> CachedList {
    CachedList {
        at: Instant::now(),
        objects: Arc::new(Vec::new()),
        indexed: Some((total, 1)),
        gaps: Vec::new(),
    }
}

/// Keep what was read of `device`'s list, evicting the oldest list past
/// [`LIST_CAP`] devices.
fn store<T: TransportPort + 'static>(ctx: &Ctx<T>, device: u32, list: CachedList) {
    let mut lists = ctx.lists.lock().unwrap_or_else(PoisonError::into_inner);
    lists.insert(device, list);
    while lists.len() > LIST_CAP {
        let oldest = lists
            .iter()
            .min_by_key(|(_, kept)| kept.at)
            .map(|(device, _)| *device);
        match oldest {
            Some(oldest) => drop(lists.remove(&oldest)),
            None => break,
        }
    }
}

pub(super) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    args: Args,
    raw: &Map<String, Value>,
) -> Result<Value, ToolError> {
    let types = args
        .types
        .iter()
        .map(|name| parse_object_type(name).map_err(|e| ToolError::invalid("types", e)))
        .collect::<Result<Vec<ObjectType>, _>>()?;
    let device = ctx.device(call, &args.device).await?;
    let first = start(ctx, NAME, args.cursor.as_deref(), raw)? as usize;
    let listing = listing(ctx, call, &device).await?;
    if !listing.complete() {
        // Not all read yet: say how far it got, and hand back a cursor
        // for the same page, which goes on reading from there.
        return Ok(json!({
            "device": device.instance,
            "objects": [],
            "total": null,
            "listed": null,
            "list": listing.progress(),
            "truncated": true,
            "next_cursor": next(ctx, NAME, raw, first as u64),
            "limits": ctx.limits(None),
        }));
    }
    let matching: Vec<ObjectIdentifier> = listing
        .objects
        .iter()
        .copied()
        .filter(|object| types.is_empty() || types.contains(&object.object_type()))
        .collect();
    let page = if args.names {
        ctx.policy.caps.objects.min(NAMED_PAGE)
    } else {
        ctx.policy.caps.objects
    };
    let engine = ctx.engine()?;
    let cap = ctx.policy.caps.text;
    let mut objects = Vec::new();
    let mut resume = None;
    for (at, object) in matching.iter().enumerate().skip(first) {
        if at >= first + page || (args.names && at > first && call.out_of_time()) {
            resume = Some(at);
            break;
        }
        let text = object_text(object.object_type(), object.instance_number());
        let mut row = json!({ "object": text });
        if args.names {
            let name = engine
                .read(
                    device.instance,
                    *object,
                    Wanted::List(&[PropertyIdentifier::OBJECT_NAME]),
                )
                .await
                .ok()
                .and_then(|read| row_text(&read.rows, PropertyIdentifier::OBJECT_NAME));
            let source = format!("device {} {text} object-name", device.instance);
            row["name"] = name.map_or(Value::Null, |name| envelope(&name, source, cap));
        }
        objects.push(row);
    }
    Ok(json!({
        "device": device.instance,
        "objects": objects,
        "total": matching.len(),
        "listed": listing.objects.len(),
        "list": listing.progress(),
        "truncated": resume.is_some(),
        "next_cursor": resume.map(|at| next(ctx, NAME, raw, at as u64)),
        "limits": ctx.limits(Some(page)),
    }))
}
