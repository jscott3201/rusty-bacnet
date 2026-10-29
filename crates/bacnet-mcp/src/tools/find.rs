//! `bacnet_find_object`: objects by name, description or type across the
//! discovered devices, paged.

use std::collections::HashMap;
use std::sync::PoisonError;

use bacnet_ops::engine::{Wanted, timed_out, unreachable};
use bacnet_ops::parse::{parse_object_specifier, parse_object_type};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::time::Instant;

use super::discover::parse_range;
use super::list::listing;
use super::{Spec, next, schema, start};
use crate::context::{CachedNames, Call, Cleaned, Ctx, Device, LIST_CAP, Names};
use crate::errors::ToolError;
use crate::untrusted::{clean, envelope_cleaned};
use crate::value::{object_text, row_text};

pub(super) const NAME: &str = "bacnet_find_object";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "Find BACnet objects",
    description: "Find objects across the devices already discovered (call bacnet_discover \
                  first), within this server's reach: by name or description (a \
                  case-insensitive substring of the device-reported text), by object \
                  (type:instance) or by object types. Each device's Object_List is read once \
                  and kept 5 minutes; matching by name or description reads each candidate's \
                  name, one request an object, so narrow with types or range. Paged: call again \
                  with next_cursor. A device whose list couldn't be read whole is listed under \
                  incomplete, and one that failed under skipped. Read-only.",
    input,
    output,
    idempotent: true,
    open_world: true,
    passive: false,
};

/// The longest search text.
const MAX_QUERY: usize = 64;

/// Matches on one page, at most; the policy's object page may be smaller.
const PAGE: usize = 100;

fn page_size<T: TransportPort + 'static>(ctx: &Ctx<T>) -> usize {
    ctx.policy.caps.objects.min(PAGE)
}

fn input() -> Value {
    let text = |what: &str| {
        json!({
            "type": "string",
            "minLength": 1,
            "maxLength": MAX_QUERY,
            "description": format!("A case-insensitive substring of the object's {what}"),
        })
    };
    schema::input(
        json!({
            "name": text("Object_Name"),
            "description": text("Description"),
            "object": schema::object(),
            "types": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Only these object types (ai, analog-output, trend-log, ...)",
            },
            "range": {
                "type": "string",
                "pattern": "^[0-9]+(-[0-9]+)?$",
                "description": "Only devices whose instance is in low-high (or one instance)",
            },
            "cursor": schema::cursor(),
        }),
        &[],
    )
}

fn output() -> Value {
    schema::output(json!({
        "matches": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "device": { "type": "integer" },
                    "object": { "type": "string" },
                    "name": schema::envelope(),
                    "description": schema::envelope(),
                },
                "required": ["device", "object"],
            },
        },
        "devices": {
            "type": "integer",
            "description": "Devices in reach that the search covers",
        },
        "searched": {
            "type": "array",
            "items": { "type": "integer" },
            "description": "The devices this page searched",
        },
        "incomplete": {
            "type": "array",
            "description": "Devices whose Object_List couldn't be read whole: only the \
                            objects read were searched",
            "items": {
                "type": "object",
                "properties": {
                    "device": { "type": "integer" },
                    "read": { "type": "integer" },
                    "elements": { "type": ["integer", "null"] },
                    "why": { "type": "string" },
                },
                "required": ["device", "why"],
            },
        },
        "skipped": {
            "type": "array",
            "description": "Devices that failed, with the error",
            "items": {
                "type": "object",
                "properties": {
                    "device": { "type": "integer" },
                    "error": { "type": "object" },
                },
                "required": ["device", "error"],
            },
        },
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    name: Option<String>,
    description: Option<String>,
    object: Option<String>,
    #[serde(default)]
    types: Vec<String>,
    range: Option<String>,
    cursor: Option<String>,
}

/// What an object must be to match.
struct Query {
    object: Option<ObjectIdentifier>,
    types: Vec<ObjectType>,
    /// Lowercase.
    name: Option<String>,
    /// Lowercase.
    description: Option<String>,
}

impl Query {
    fn parse(args: &Args) -> Result<Self, ToolError> {
        let text = |field: &'static str, value: &Option<String>| {
            value
                .as_deref()
                .map(|text| {
                    let count = text.chars().count();
                    if count == 0 || count > MAX_QUERY {
                        return Err(ToolError::invalid(
                            field,
                            format!("1 to {MAX_QUERY} characters"),
                        ));
                    }
                    Ok(text.to_lowercase())
                })
                .transpose()
        };
        let object = args
            .object
            .as_deref()
            .map(|text| {
                let (kind, instance) =
                    parse_object_specifier(text).map_err(|e| ToolError::invalid("object", e))?;
                ObjectIdentifier::new(kind, instance)
                    .map_err(|e| ToolError::invalid("object", e.to_string()))
            })
            .transpose()?;
        let types = args
            .types
            .iter()
            .map(|name| parse_object_type(name).map_err(|e| ToolError::invalid("types", e)))
            .collect::<Result<Vec<_>, _>>()?;
        let query = Self {
            object,
            types,
            name: text("name", &args.name)?,
            description: text("description", &args.description)?,
        };
        if query.object.is_none()
            && query.types.is_empty()
            && query.name.is_none()
            && query.description.is_none()
        {
            return Err(ToolError::invalid(
                "name",
                "give a name, description, object or types to search for",
            ));
        }
        Ok(query)
    }

    /// Whether `object` is a candidate before any text is read.
    fn candidate(&self, object: &ObjectIdentifier) -> bool {
        self.object.is_none_or(|wanted| wanted == *object)
            && (self.types.is_empty() || self.types.contains(&object.object_type()))
    }

    fn reads_text(&self) -> bool {
        self.name.is_some() || self.description.is_some()
    }

    /// Whether device text `found` holds `wanted`. The text is kept
    /// cleaned, so hidden characters can't keep a match apart.
    fn holds(found: Option<&Cleaned>, wanted: Option<&String>) -> bool {
        match wanted {
            None => true,
            Some(wanted) => {
                found.is_some_and(|found| found.text.to_lowercase().contains(wanted.as_str()))
            }
        }
    }

    fn matches(&self, names: &Names) -> bool {
        Self::holds(names.name.as_ref(), self.name.as_ref())
            && Self::holds(
                names.description.as_ref().and_then(Option::as_ref),
                self.description.as_ref(),
            )
    }
}

/// Where a page resumes: a device instance and an index into its
/// candidates, packed into the cursor's position.
fn pack(device: u32, index: usize) -> u64 {
    (u64::from(device) << 32) | u64::try_from(index).unwrap_or(u64::from(u32::MAX))
}

fn unpack(position: u64) -> (u32, usize) {
    let device = u32::try_from(position >> 32).unwrap_or(u32::MAX);
    let index = usize::try_from(position & u64::from(u32::MAX)).unwrap_or(usize::MAX);
    (device, index)
}

/// The cached names of `object` on `device`, if fresh and holding what
/// the query needs.
fn cached<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    device: u32,
    object: &ObjectIdentifier,
    description: bool,
) -> Option<Names> {
    let names = ctx.names.lock().unwrap_or_else(PoisonError::into_inner);
    let kept = names
        .get(&device)
        .filter(|kept| kept.at.elapsed() < crate::context::LIST_TTL)?
        .objects
        .get(object)?;
    (!description || kept.description.is_some()).then(|| kept.clone())
}

/// Objects whose names are kept for one device; past it, names are read
/// again when needed.
const NAMES_PER_DEVICE: usize = 4_096;

/// Objects whose names are kept across every device; past it, the oldest
/// device's names go.
const NAMES_TOTAL: usize = 16_384;

/// Characters of a name or description kept for matching. Results show
/// only the policy's text cap of them, but a search matches against all
/// of these, so a description hit past the shown text still counts.
///
/// Worst case the cache holds [`NAMES_TOTAL`] objects with two strings of
/// this many characters each, about 134 million characters. A device sends
/// each string in one property answer, so real ones are far shorter.
const MATCH_CAP: usize = 4_096;

/// Kept text as an envelope, cut to `cap` characters for display.
fn shown(kept: &Cleaned, cap: usize, source: String) -> Value {
    match kept.text.char_indices().nth(cap) {
        Some((cut, _)) => envelope_cleaned(&kept.text[..cut], true, source),
        None => envelope_cleaned(&kept.text, kept.truncated, source),
    }
}

/// Keep what was read of `object`'s names, evicting the oldest device past
/// [`LIST_CAP`].
fn remember<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    device: u32,
    object: ObjectIdentifier,
    read: Names,
) {
    let mut names = ctx.names.lock().unwrap_or_else(PoisonError::into_inner);
    let entry = names.entry(device).or_insert_with(|| CachedNames {
        at: Instant::now(),
        objects: HashMap::new(),
    });
    if entry.at.elapsed() >= crate::context::LIST_TTL {
        *entry = CachedNames {
            at: Instant::now(),
            objects: HashMap::new(),
        };
    }
    if entry.objects.len() < NAMES_PER_DEVICE || entry.objects.contains_key(&object) {
        entry.objects.insert(object, read);
    }
    // Past the total, the oldest other devices go first; this device's
    // newest names stay.
    while names.values().map(|kept| kept.objects.len()).sum::<usize>() > NAMES_TOTAL {
        let oldest = names
            .iter()
            .filter(|(kept_device, _)| **kept_device != device)
            .min_by_key(|(_, kept)| kept.at)
            .map(|(device, _)| *device);
        match oldest {
            Some(oldest) => drop(names.remove(&oldest)),
            None => {
                if let Some(entry) = names.get_mut(&device) {
                    entry.objects.remove(&object);
                }
                break;
            }
        }
    }
    while names.len() > LIST_CAP {
        let oldest = names
            .iter()
            .min_by_key(|(_, kept)| kept.at)
            .map(|(device, _)| *device);
        match oldest {
            Some(oldest) => drop(names.remove(&oldest)),
            None => break,
        }
    }
}

/// What a page found, and where it stopped.
#[derive(Default)]
struct Page {
    matches: Vec<Value>,
    searched: Vec<u32>,
    incomplete: Vec<Value>,
    skipped: Vec<Value>,
    resume: Option<(u32, usize)>,
}

pub(super) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    args: Args,
    raw: &Map<String, Value>,
) -> Result<Value, ToolError> {
    let query = Query::parse(&args)?;
    let range = args.range.as_deref().map(parse_range).transpose()?;
    let (from_device, from_index) = unpack(start(ctx, NAME, args.cursor.as_deref(), raw)?);
    let engine = ctx.engine()?;
    let mut devices = engine.devices().await;
    devices.retain(|found| {
        let instance = found.object_identifier.instance_number();
        range.is_none_or(|(low, high)| (low..=high).contains(&instance))
            && ctx.policy.allows_network(found.source_network.unwrap_or(0))
            && ctx.policy.allows_device(instance)
    });
    devices.sort_by_key(|found| found.object_identifier.instance_number());
    let total = devices.len();
    let mut page = Page::default();
    for entry in devices {
        let instance = entry.object_identifier.instance_number();
        if instance < from_device {
            continue;
        }
        let first = if instance == from_device {
            from_index
        } else {
            0
        };
        // A full page, or a spent budget, ends here: the next device is
        // neither read nor charged until the next page.
        if page.matches.len() >= page_size(ctx) || (!page.searched.is_empty() && call.out_of_time())
        {
            page.resume = Some((instance, first));
            break;
        }
        let device = Device {
            instance,
            network: entry.source_network.unwrap_or(0),
            entry,
        };
        call.reached(instance);
        page.searched.push(instance);
        search(ctx, call, &query, &device, first, &mut page).await;
        if page.resume.is_some() {
            break;
        }
    }
    Ok(json!({
        "matches": page.matches,
        "devices": total,
        "searched": page.searched,
        "incomplete": page.incomplete,
        "skipped": page.skipped,
        "truncated": page.resume.is_some(),
        "next_cursor": page.resume.map(|(device, index)| next(ctx, NAME, raw, pack(device, index))),
        "limits": ctx.limits(Some(page_size(ctx))),
    }))
}

/// Search one device from its `first` candidate, adding to `page`; sets
/// `page.resume` when the page fills or the budget runs out.
async fn search<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    query: &Query,
    device: &Device,
    first: usize,
    page: &mut Page,
) {
    let instance = device.instance;
    let listing = match listing(ctx, call, device).await {
        Ok(listing) => listing,
        Err(error) => {
            page.skipped
                .push(json!({ "device": instance, "error": error.json() }));
            return;
        }
    };
    if !listing.complete() {
        match listing.stopped() {
            // The device stopped answering: search what was read, and say so.
            Some(why) => {
                let progress = listing.progress();
                page.incomplete.push(json!({
                    "device": instance,
                    "read": progress["read"],
                    "elements": progress["elements"],
                    "why": why,
                }));
            }
            // The budget ran out: the next page goes on reading the list.
            None => {
                page.resume = Some((instance, first));
                return;
            }
        }
    }
    let candidates: Vec<ObjectIdentifier> = listing
        .objects
        .iter()
        .copied()
        .filter(|object| query.candidate(object))
        .collect();
    let engine = match ctx.engine() {
        Ok(engine) => engine,
        Err(_) => return,
    };
    let cap = ctx.policy.caps.text;
    let mut properties = vec![PropertyIdentifier::OBJECT_NAME];
    if query.description.is_some() {
        properties.push(PropertyIdentifier::DESCRIPTION);
    }
    for (at, object) in candidates.iter().enumerate().skip(first) {
        if page.matches.len() >= page_size(ctx) {
            page.resume = Some((instance, at));
            return;
        }
        let mut names = cached(ctx, instance, object, query.description.is_some());
        if names.is_none() && query.reads_text() {
            if call.out_of_time() {
                page.resume = Some((instance, at));
                return;
            }
            match engine
                .read(instance, *object, Wanted::List(&properties))
                .await
            {
                Ok(read) => {
                    let kept = |property| {
                        row_text(&read.rows, property).map(|text| {
                            let (text, truncated) = clean(&text, MATCH_CAP);
                            Cleaned { text, truncated }
                        })
                    };
                    let found = Names {
                        name: kept(PropertyIdentifier::OBJECT_NAME),
                        description: query
                            .description
                            .is_some()
                            .then(|| kept(PropertyIdentifier::DESCRIPTION)),
                    };
                    remember(ctx, instance, *object, found.clone());
                    names = Some(found);
                }
                // The device stopped answering part way through its names:
                // it is reported incomplete and the search moves on to the
                // next device, with no cursor back to it, so a silent device
                // doesn't make every later page wait out its timeout again.
                // Search it alone (range) to try it again.
                Err(error) if unreachable(&error) => {
                    if timed_out(&error) {
                        call.timed_out();
                    }
                    page.incomplete.push(json!({
                        "device": instance,
                        "read": at,
                        "elements": candidates.len(),
                        "why": format!("stopped reading names: {error}"),
                    }));
                    return;
                }
                // The object has no name to give: no text match.
                Err(_) => names = Some(Names::default()),
            }
        }
        let names = names.unwrap_or_default();
        if query.reads_text() && !query.matches(&names) {
            continue;
        }
        let text = object_text(object.object_type(), object.instance_number());
        let source = |property: &str| format!("device {instance} {text} {property}");
        let mut row = json!({ "device": instance, "object": text });
        if let Some(name) = &names.name {
            row["name"] = shown(name, cap, source("object-name"));
        }
        if let Some(Some(description)) = &names.description {
            row["description"] = shown(description, cap, source("description"));
        }
        page.matches.push(row);
    }
}
