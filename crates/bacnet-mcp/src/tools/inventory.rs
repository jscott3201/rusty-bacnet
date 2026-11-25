//! `bacnet_inventory` (#2779): one device's objects as the inventory's
//! point records (`bacnet_ops::inventory`), with the field names of the
//! JSON `bacnet inventory` prints, a page a call.
//!
//! A page reads as `bacnet_export_223`'s does. It takes the device's
//! Object_List from `bacnet_list_objects`' cache (read whole, or element
//! by element and resumed across calls; while that read is incomplete a
//! page holds no record and its cursor goes on reading the list), then
//! reads the objects of the selected types in Object_List order from the
//! cursor's place, in batches, each one call of
//! [`Device::point_records`](bacnet_ops::Device::point_records): one
//! ReadPropertyMultiple while the device answers, as the CLI's inventory
//! reads them. No page reads an object another page reads, but, now and
//! then, the ones a page hands back.
//!
//! A page ends at [`PAGE`] objects (or the policy's object page), once
//! the call's budget is spent, or before its result would pass
//! [`MAX_TEXT`] bytes, so that the text rendering holds it whole: a batch
//! holds no more objects than are likely to fit (judged by the page's
//! largest record so far), and a record that passes it anyway goes back
//! to the next page with those after it, to be read again there. A record
//! that passes it with the page's other fields goes out alone, flagged
//! `oversized`, and the text block then leaves it out ([`oversized_text`]).
//! A page's later batch still being read
//! [`OBJECT_GRACE`](super::batches::OBJECT_GRACE) past the budget is let
//! go and read again on the next page, so the records before it come back
//! before the call's own timeout; a first batch still being read
//! [`FIRST_GRACE`](super::batches::FIRST_GRACE) past it fails the call,
//! naming it and the ways past it ([`stuck`](super::batches::stuck)).
//! Either way the next attempt asks the device for fewer objects, down to
//! one ([`super::batches`]). `ended` says which of these ended the page.
//! The cursor holds
//! the next object's place and a fingerprint of the matching objects (the
//! export's [`Position`]), so a list changed between pages is refused
//! rather than paged wrong.
//!
//! A property the device can't give is a problem on its record, never an
//! error. A device that stops answering partway through a page leaves the
//! records read before it, the error under `stopped`, and a cursor that
//! tries that object again.

use std::collections::{BTreeMap, HashSet};

use bacnet_ops::label::kebab;
use bacnet_ops::parse::parse_object_type;
use bacnet_ops::{InventoryOptions, OpsError};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use rmcp::model::{CallToolResult, ContentBlock};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::batches::{Batch, next_batch};
use super::export::{Position, fingerprint};
use super::inventory_data::{point, problem_schema, problems_of, record_json};
use super::list::listing;
use super::{MAX_TEXT, Spec, next, render, schema, start};
use crate::context::{Call, Ctx, Device, DeviceArg};
use crate::errors::ToolError;
use crate::value::object_text;

pub(super) const NAME: &str = "bacnet_inventory";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "Inventory a device's points",
    description: "List one device's objects as point records: each object's name, description, \
                  units, state texts and whether it takes commands, values decoded, the records \
                  `bacnet inventory --json` prints. values: true adds Present_Value, \
                  Status_Flags, Event_State, Reliability and the priority in control (4 more \
                  properties an object, 5 on an output); settings: true adds COV_Increment, \
                  Min_Pres_Value, Max_Pres_Value, Relinquish_Default and Polarity where the \
                  type has them (up to 4 more); details: true adds Device_Type and Profile_Name \
                  (2 more). Cost: the Object_List once (cached 5 minutes), then the objects \
                  in batches as the CLI's `bacnet inventory` reads them, one \
                  ReadPropertyMultiple for up to 16 objects or 32 properties, so the options \
                  add requests: a 200-object controller takes about 28 requests (58 with \
                  values, 104 with every option), about what the CLI takes. A device that \
                  refuses ReadPropertyMultiple costs one ReadProperty a property: 2 to 5 an \
                  object, up to 14 with every option. types filters before anything is \
                  read. Paged: up to 50 objects and 24,000 bytes a page, or what the call's \
                  budget allows; `ended` says why a page ended; call again with next_cursor. \
                  A property the device can't give is a problem on its record and in the \
                  page's problems summary, never a failed call. Read-only.",
    input,
    output,
    idempotent: true,
    open_world: true,
    passive: false,
};

/// Objects a page reads at most (the policy's object page, when smaller).
const PAGE: usize = 50;

/// Room kept under [`MAX_TEXT`] for the page's fields beside its records,
/// when judging whether another record fits.
const ROOM: usize = 4_000;

/// Room for the `cost` block every result gets after the tool returns.
const COST_ROOM: usize = 160;

/// The problems a page's summary lists; each record lists all of its own.
const FIRST_PROBLEMS: usize = 10;

fn input() -> Value {
    let flag = |description: &str| json!({ "type": "boolean", "default": false, "description": description });
    schema::input(
        json!({
            "device": schema::device(),
            "types": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Only these object types (ai, analog-output, msv, trend-log, ...); \
                                left out or empty, every object, a vendor's included",
            },
            "values": flag(
                "Also read each object's Present_Value, Status_Flags, Event_State, Reliability \
                 and priority in control: 4 more properties an object, 5 on an output",
            ),
            "settings": flag(
                "Also read COV_Increment, Min_Pres_Value, Max_Pres_Value, Relinquish_Default \
                 and Polarity where the type has them: up to 4 more properties an object",
            ),
            "details": flag(
                "Also read each object's Device_Type and Profile_Name: 2 more properties an \
                 object",
            ),
            "cursor": schema::cursor(),
        }),
        &["device"],
    )
}

fn output() -> Value {
    let count = |description: &str| json!({ "type": "integer", "description": description });
    let null_or_count =
        |description: &str| json!({ "type": ["integer", "null"], "description": description });
    schema::output(json!({
        "device": { "type": "integer" },
        "address": {
            "type": "string",
            "description": "Where the device answers, as a target",
        },
        "cov": {
            "enum": ["unknown", "not-offered", "offered", "accepted", "refused"],
            "description": "What is known of SubscribeCOV at the device, at no request of its \
                            own",
        },
        "listed": null_or_count("Objects in the Object_List; null until it is all read"),
        "matching": null_or_count(
            "Objects of the selected types, each once, which the pages go through in \
             Object_List order; null until the Object_List is all read",
        ),
        "by_type": {
            "type": "object",
            "additionalProperties": { "type": "integer" },
            "description": "The matching objects by type, on every page",
        },
        "start": count("Where this page starts among the matching objects, from 0"),
        "read": count("Records on this page"),
        "points": { "type": "array", "items": point() },
        "remaining": null_or_count(
            "Matching objects after this page, not read yet; null until the Object_List is all \
             read",
        ),
        "next_object": {
            "type": ["string", "null"],
            "description": "The first of those, where next_cursor goes on",
        },
        "problems": {
            "type": "object",
            "description": "This page's problems: properties that gave no usable value",
            "properties": {
                "count": count("Problems on this page"),
                "objects": count("Records on this page with a problem"),
                "first": {
                    "type": "array",
                    "items": problem_schema(true),
                    "description": "The first ten, in record order",
                },
            },
        },
        "ended": {
            "enum": ["complete", "page-full", "size", "budget", "device", "list"],
            "description": "Why the page ended: complete (no object left), page-full (50 \
                            objects, or the policy's page) or size (24,000 bytes): call again \
                            now; budget (the call's time ran out, the rates holding it back): \
                            call again, or slow down; device (it stopped answering, see \
                            stopped); list (the Object_List is still being read, element by \
                            element)",
        },
        "oversized": {
            "type": "boolean",
            "description": "The page holds one record, and with it the result passes 24,000 \
                            bytes: the text block then leaves the record out, and only the \
                            structured content holds it",
        },
        "stopped": {
            "type": "object",
            "description": "Set when the device stopped answering partway, through the objects \
                            or the Object_List: the error. The page holds what was read \
                            before; next_cursor tries again there",
        },
        "list": {
            "type": "object",
            "description": "How the Object_List was read, as bacnet_list_objects says; when \
                            not complete, the page holds no record and next_cursor goes on \
                            reading the list",
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
    values: bool,
    #[serde(default)]
    settings: bool,
    #[serde(default)]
    details: bool,
    cursor: Option<String>,
}

impl Args {
    /// What to read, checked before anything is sent.
    fn options(&self) -> Result<InventoryOptions, ToolError> {
        let options = InventoryOptions::new()
            .with_values(self.values)
            .with_settings(self.settings)
            .with_details(self.details);
        if self.types.is_empty() {
            return Ok(options);
        }
        let types = self
            .types
            .iter()
            .map(|name| parse_object_type(name))
            .collect::<Result<Vec<ObjectType>, _>>()
            .map_err(|e| ToolError::invalid("types", e))?;
        Ok(options.with_types(types))
    }
}

/// The objects `options` selects from `listed`, in order, each once, as
/// the inventory reads them.
fn matching(listed: &[ObjectIdentifier], options: &InventoryOptions) -> Vec<ObjectIdentifier> {
    let mut seen = HashSet::new();
    listed
        .iter()
        .copied()
        .filter(|object| options.reads(object.object_type()) && seen.insert(*object))
        .collect()
}

/// Why a page ended, as its `ended` says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Ended {
    /// No matching object is left.
    #[default]
    Complete,
    /// It holds the page's objects.
    PageFull,
    /// Another record would take it past [`MAX_TEXT`].
    Size,
    /// The call's budget is spent.
    Budget,
    /// The device stopped answering.
    Device,
    /// The Object_List is still being read, element by element.
    List,
}

impl Ended {
    fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::PageFull => "page-full",
            Self::Size => "size",
            Self::Budget => "budget",
            Self::Device => "device",
            Self::List => "list",
        }
    }
}

/// The records read for a page, in order, each with its JSON's size, and
/// how the page ended.
#[derive(Default)]
struct Page {
    points: Vec<(Value, usize)>,
    /// The records' JSON so far, in bytes.
    size: usize,
    /// The largest record so far.
    largest: usize,
    /// Where the next page goes on among the matching objects, if one does.
    resume: Option<usize>,
    ended: Ended,
    /// The device's error, when it stopped answering.
    stopped: Option<Value>,
}

impl Page {
    fn push(&mut self, point: Value) {
        let size = point.to_string().len() + 1;
        self.size += size;
        self.largest = self.largest.max(size);
        self.points.push((point, size));
    }

    /// End the page before the matching object at `at`, for `why`.
    fn end(&mut self, at: usize, why: Ended) {
        self.resume = Some(at);
        self.ended = why;
    }

    /// Hand the last record back to the next page, which starts with it;
    /// the page's records begin at `first` among the matching objects. A
    /// page the device ended keeps saying so, beside its `stopped`.
    fn hand_back(&mut self, first: usize) {
        if let Some((_, size)) = self.points.pop() {
            self.size -= size;
        }
        let why = match self.ended {
            Ended::Device => Ended::Device,
            _ => Ended::Size,
        };
        self.end(first + self.points.len(), why);
    }

    /// End the page before the matching object at `at` because the device
    /// stopped answering, or the way to it failed, with `error`.
    fn stop(&mut self, at: usize, device: u32, error: &OpsError, call: &Call) {
        if error.timed_out() {
            call.timed_out();
        }
        self.stopped = Some(ToolError::device(device, error).json());
        self.end(at, Ended::Device);
    }

    /// How many more records the page can take: up to `cap` in all, and,
    /// once it holds one, as many as fit under [`MAX_TEXT`] if each were
    /// as large as its largest so far. A page's first batch has no such
    /// guess, and takes at least one object, as [`full`](Self::full) has
    /// every page read one.
    fn room(&self, cap: usize) -> usize {
        let left = cap.saturating_sub(self.points.len());
        if self.points.is_empty() {
            return left.max(1);
        }
        let bytes = MAX_TEXT.saturating_sub(self.size + ROOM);
        left.min(bytes / self.largest.max(1))
    }

    /// Why the page ends before its next object, if it does: it holds
    /// `cap`, another as large as its largest wouldn't fit under
    /// [`MAX_TEXT`], or the call's budget is spent. A page's first object
    /// is always read.
    fn full(&self, cap: usize, call: &Call) -> Option<Ended> {
        let read = self.points.len();
        if read == 0 {
            None
        } else if read >= cap {
            Some(Ended::PageFull)
        } else if self.room(cap) == 0 {
            Some(Ended::Size)
        } else {
            call.out_of_time().then_some(Ended::Budget)
        }
    }
}

/// Read a page of `matching` from `first`, a batch a call of
/// [`point_records`](bacnet_ops::Device::point_records): as many objects
/// as one ReadPropertyMultiple holds
/// ([`point_batch`](bacnet_ops::Device::point_batch); one a call once the
/// device has refused RPM), and no more than the page can still take.
/// Each call's records go on the page before the next call starts, so a
/// page the budget or the device ends keeps every batch read whole.
async fn read_page<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    device: &Device<T>,
    matching: &[ObjectIdentifier],
    first: usize,
    options: &InventoryOptions,
) -> Result<Page, ToolError> {
    let cap = ctx.policy.caps.objects.min(PAGE);
    let mut page = Page::default();
    let mut at = first;
    while at < matching.len() {
        if let Some(why) = page.full(cap, call) {
            page.end(at, why);
            break;
        }
        let rest = &matching[at..at + page.room(cap).min(matching.len() - at)];
        let opening = page.points.is_empty();
        let (_, batch) = next_batch(ctx, call, device, rest, options, opening).await?;
        let read = match batch {
            Batch::Read(read) => read,
            // A later batch let go well past the budget: read again on the
            // next page.
            Batch::LetGo => {
                page.end(at, Ended::Budget);
                break;
            }
            Batch::Failed(error) if opening => {
                return Err(ToolError::device(device.instance, &error));
            }
            // Refused before its request went (the route gone, say):
            // later batches would fail the same way.
            Batch::Failed(error) => {
                page.stop(at, device.instance, &error, call);
                break;
            }
        };
        for record in &read.points {
            page.push(record_json(record, ctx.policy.caps.text));
            at += 1;
        }
        if let Some(error) = &read.stopped {
            if page.points.is_empty() {
                return Err(ToolError::device(device.instance, error));
            }
            // Later objects would fail the same way: stop here, say why,
            // and let the cursor try the first one not read again.
            page.stop(at, device.instance, error, call);
            break;
        }
    }
    Ok(page)
}

pub(super) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    args: Args,
    raw: &Map<String, Value>,
) -> Result<Value, ToolError> {
    let options = args.options()?;
    let device = ctx.device(call, &args.device).await?;
    let position = Position::from(start(ctx, NAME, args.cursor.as_deref(), raw)?);
    let first = position.index;
    let instance = device.instance;
    let listing = listing(ctx, call, &device).await?;
    let mut body = json!({
        "device": instance,
        "address": device.address.to_string(),
        "cov": device.handle.cov_support(),
        "start": first,
        "oversized": false,
        "list": listing.progress(),
        "limits": ctx.limits(Some(ctx.policy.caps.objects.min(PAGE))),
    });
    if !listing.complete() {
        // Not all read yet: no record on this page, and the same cursor,
        // which goes on reading the list.
        fill(&mut body, None, (first, None), Vec::new());
        body["truncated"] = json!(true);
        body["next_cursor"] = next(ctx, NAME, raw, position.into());
        body["ended"] = json!(Ended::List.as_str());
        if let Some(why) = listing.stopped() {
            body["stopped"] = ToolError::Failed(format!("the Object_List read, {why}")).json();
            body["ended"] = json!(Ended::Device.as_str());
        }
        return Ok(body);
    }
    let matching = matching(&listing.objects, &options);
    let list = fingerprint(&matching);
    if position.list.is_some_and(|cut| cut != list) || (first > 0 && first >= matching.len()) {
        // Pages cut from another list would repeat some objects and miss
        // others; nothing past the list read is sent.
        return Err(ToolError::Failed(format!(
            "device {instance}'s Object_List changed since this cursor's page (objects added, \
             removed or moved), so its pages no longer line up: start the inventory again \
             without a cursor"
        )));
    }
    let mut page = read_page(ctx, call, &device, &matching, first, &options).await?;
    body["listed"] = json!(listing.objects.len());
    body["by_type"] = by_type(&matching);
    if let Some(stopped) = page.stopped.take() {
        body["stopped"] = stopped;
    }
    // A record that took the page past the bound goes back to the next one,
    // unless it is the page's only record.
    loop {
        let points: Vec<Value> = page.points.iter().map(|(point, _)| point.clone()).collect();
        let cursor = page.resume.map(|index| {
            let position = Position {
                list: Some(list),
                index,
            };
            next(ctx, NAME, raw, position.into())
        });
        fill(&mut body, Some(&matching), (first, page.resume), points);
        body["truncated"] = json!(page.resume.is_some());
        body["next_cursor"] = json!(cursor);
        body["ended"] = json!(page.ended.as_str());
        let size = body.to_string().len() + COST_ROOM;
        if size <= MAX_TEXT || page.points.len() <= 1 {
            body["oversized"] = json!(size > MAX_TEXT && page.points.len() == 1);
            return Ok(body);
        }
        page.hand_back(first);
    }
}

/// The text block of an oversized page: the result without its one record,
/// which the structured content alone holds, so the text stays whole JSON
/// rather than being cut inside the record.
pub(super) fn oversized_text(result: &mut CallToolResult) {
    let Some(body) = &result.structured_content else {
        return;
    };
    if result.is_error == Some(true) || body["oversized"] != json!(true) {
        return;
    }
    let bytes = body["points"].to_string().len();
    let object = &body["points"][0]["object"];
    // The record's object, from its type's name and number, never from
    // device text.
    let object = format!(
        "{}:{}",
        object["type"].as_str().unwrap_or_default(),
        object["instance"]
    );
    let mut short = body.clone();
    short["points"] = json!(format!(
        "the record of {object} ({bytes} bytes) is too large for this text: it is whole in \
         this result's structured content, and bacnet_read reads its properties one at a time"
    ));
    if let Some(ContentBlock::Text(text)) = result.content.first_mut() {
        text.text = render(NAME, &short, None);
    }
}

/// Put a page's records and what they come to in `body`, given the
/// matching objects (`None` while the list is still being read), where the
/// page starts and where the next goes on, if one does.
fn fill(
    body: &mut Value,
    matching: Option<&[ObjectIdentifier]>,
    (first, resume): (usize, Option<usize>),
    points: Vec<Value>,
) {
    let problems: Vec<Value> = points.iter().flat_map(problems_of).collect();
    let troubled = points
        .iter()
        .filter(|point| point["problems"].as_array().is_some_and(|p| !p.is_empty()))
        .count();
    let next_object = resume
        .zip(matching)
        .and_then(|(at, matching)| matching.get(at))
        .map(|object| object_text(object.object_type(), object.instance_number()));
    let read = points.len();
    body["read"] = json!(read);
    body["points"] = Value::Array(points);
    body["problems"] = json!({
        "count": problems.len(),
        "objects": troubled,
        "first": problems.into_iter().take(FIRST_PROBLEMS).collect::<Vec<_>>(),
    });
    body["next_object"] = json!(next_object);
    if let Some(matching) = matching {
        body["matching"] = json!(matching.len());
        body["remaining"] = json!(matching.len().saturating_sub(first + read));
    } else {
        body["listed"] = Value::Null;
        body["matching"] = Value::Null;
        body["remaining"] = Value::Null;
        body["by_type"] = json!({});
    }
}

/// The matching objects by type, in kebab case.
fn by_type(matching: &[ObjectIdentifier]) -> Value {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for object in matching {
        *counts.entry(kebab(object.object_type())).or_default() += 1;
    }
    json!(counts)
}
