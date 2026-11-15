//! `bacnet inventory` (#2560): a device's objects as point records, read
//! by `bacnet_ops::inventory` and printed as a table, or as the JSON or
//! CSV that module writes.

use std::sync::atomic::AtomicBool;

use bacnet_client::client::BACnetClient;
use bacnet_ops::inventory::{CsvOptions, Inventory, InventoryOptions, PointRecord};
use bacnet_ops::label::kebab;
use bacnet_ops::limits::RequestLimits;
use bacnet_ops::parse::parse_object_type;
use bacnet_ops::raw::{InventoryHalt, Reader, read_inventory};
use bacnet_ops::{OpsError, Value};
use bacnet_transport::port::TransportPort;

use crate::destination::Destination;
use crate::output::{OutputFormat, screen};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// How `bacnet inventory` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// A table for people, device text cleaned for the screen.
    Table,
    /// The inventory's JSON ([`Inventory::to_json`]).
    Json,
    /// The records as CSV ([`Inventory::to_csv`]).
    Csv,
}

impl Layout {
    /// `--format csv` is the CSV; otherwise the format every command
    /// resolves (`--json`, `--format`, or by whether stdout is a terminal).
    pub fn of(csv: bool, format: OutputFormat) -> Self {
        match (csv, format) {
            (true, _) => Self::Csv,
            (false, OutputFormat::Json) => Self::Json,
            (false, OutputFormat::Table) => Self::Table,
        }
    }
}

/// What `bacnet inventory` was asked for.
#[derive(Debug, Clone, Copy)]
pub struct InventoryArgs<'a> {
    /// The target device.
    pub target: &'a str,
    /// `--types`: the object types to read, all when empty.
    pub types: &'a [String],
    /// `--values`.
    pub values: bool,
    /// `--settings`.
    pub settings: bool,
    /// `--csv-exact`: the CSV without its formula guard.
    pub csv_exact: bool,
    /// How to print.
    pub layout: Layout,
}

/// Run `bacnet inventory`.
pub async fn inventory_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: InventoryArgs<'_>,
) -> Result<()> {
    // The options are checked before anything is sent.
    let mut options = InventoryOptions::new()
        .with_values(args.values)
        .with_settings(args.settings);
    if !args.types.is_empty() {
        let types = args
            .types
            .iter()
            .map(|name| parse_object_type(name))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        options = options.with_types(types);
    }
    // At the target's address; by instance, the one the discovery table
    // holds for it; by an address the table doesn't know, the instance is
    // read once from the device.
    let Destination { address, instance } = crate::resolve_destination(client, args.target).await?;
    // A one-shot command has the device to itself: no rate cap beyond the
    // client's --min-interval-ms, one request at a time.
    let limits = RequestLimits::new(0);
    let never = AtomicBool::new(false);
    let mut reader = Reader {
        client,
        device: address,
        instance,
        rate: &limits,
        cancel: &never,
        rpm: true,
    };
    let inventory = match read_inventory(&mut reader, &options).await {
        Ok(inventory) => inventory,
        Err(InventoryHalt::Device(error)) => return Err(error.into()),
        // A list read element by element that failed: say which element.
        Err(halt) => return Err(halt.to_string().into()),
    };
    // Stdout carries only the data; the notes go to stderr in every format.
    match args.layout {
        Layout::Json => println!("{}", inventory.to_json()),
        Layout::Csv => {
            let options = CsvOptions::new().with_exact(args.csv_exact);
            print!("{}", inventory.to_csv(&options));
        }
        Layout::Table => {
            if !inventory.points.is_empty() {
                println!("{}", table(&inventory, &options));
            }
        }
    }
    let device = inventory.device.device;
    eprintln!("{}", note(&inventory));
    if inventory.points.is_empty() && inventory.stopped.is_none() {
        let which = if args.types.is_empty() {
            "no object"
        } else {
            "no object of the selected types"
        };
        eprintln!("Device {device} has {which}.");
    }
    match &inventory.stopped {
        Some(why) => Err(stopped(&inventory, why).into()),
        None => Ok(()),
    }
}

/// The line that says what was read, for stderr.
fn note(inventory: &Inventory) -> String {
    let device = inventory.device.device;
    let name = inventory
        .device
        .name()
        .map(|name| format!(" ({})", screen(&name)))
        .unwrap_or_default();
    let troubled = inventory
        .points
        .iter()
        .filter(|point| !point.problems.is_empty())
        .count();
    format!(
        "Read {} objects of device {device}{name} at {}; {troubled} with a property that couldn't \
         be read.",
        inventory.points.len(),
        inventory.address,
    )
}

/// Why the command fails after printing what was read.
fn stopped(inventory: &Inventory, why: &OpsError) -> String {
    let device = inventory.device.device;
    format!(
        "the inventory of device {device} stopped early ({}); {} objects were not read",
        screen(&why.to_string()),
        inventory.unread.len()
    )
}

/// A value's text, cleaned for the screen; empty for none.
fn shown(value: &Option<Value>) -> String {
    value
        .as_ref()
        .map(|value| screen(&value.text))
        .unwrap_or_default()
}

/// A record's object as a command takes it: `analog-input:1`.
fn object(record: &PointRecord) -> String {
    format!(
        "{}:{}",
        kebab(record.object.object_type()),
        record.object.instance_number()
    )
}

/// The table: one row a record, with the values' and the settings' columns
/// when they were read, and a problems column when any record has one.
/// Device text is cleaned for the screen.
fn table(inventory: &Inventory, options: &InventoryOptions) -> comfy_table::Table {
    let problems = inventory
        .points
        .iter()
        .any(|point| !point.problems.is_empty());
    let mut header = vec!["Object", "Name", "Units"];
    if options.values {
        header.extend(["Value", "Status"]);
    }
    header.push("Commandable");
    if options.settings {
        header.extend(["Relinquish default", "COV increment", "Min", "Max"]);
    }
    header.extend(["States", "Description"]);
    if problems {
        header.push("Problems");
    }
    let mut table = comfy_table::Table::new();
    table.set_header(header);
    for point in &inventory.points {
        let text = |text: &Option<String>| text.as_deref().map(screen).unwrap_or_default();
        let mut row = vec![
            object(point),
            text(&point.name),
            point.units.map(kebab).unwrap_or_default(),
        ];
        if options.values {
            row.extend([shown(&point.present_value), shown(&point.status_flags)]);
        }
        row.push(if point.commandable { "yes" } else { "no" }.into());
        if options.settings {
            row.extend([
                shown(&point.relinquish_default),
                shown(&point.cov_increment),
                shown(&point.min_present_value),
                shown(&point.max_present_value),
            ]);
        }
        row.extend([states(point), text(&point.description)]);
        if problems {
            let list: Vec<String> = point
                .problems
                .iter()
                .map(|problem| format!("{}: {}", kebab(problem.property), problem.error))
                .collect();
            row.push(list.join("; "));
        }
        table.add_row(row);
    }
    table
}

/// A record's state texts for the table: a binary object's inactive and
/// active texts, or a multi-state object's from state 1, each cleaned.
fn states(record: &PointRecord) -> String {
    let texts = &record.state_texts;
    let clean = |text: &Option<String>| text.as_deref().map(screen).unwrap_or_default();
    if texts.active.is_some() || texts.inactive.is_some() {
        return format!("{} / {}", clean(&texts.inactive), clean(&texts.active));
    }
    let states: Vec<String> = texts.states.iter().map(|text| screen(text)).collect();
    states.join(" / ")
}
