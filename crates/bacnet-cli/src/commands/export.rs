//! `bacnet export 223-experimental` (#2436): a device's objects as an
//! experimental Standard 223 point list in Turtle, read and written by
//! `bacnet_ops::export::s223`.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::AtomicBool;

use crate::destination::Destination;
use bacnet_client::client::BACnetClient;
use bacnet_ops::DeviceAddress;
use bacnet_ops::engine::{Halt, Reader};
use bacnet_ops::export::s223::{
    InventoryOptions, TurtleOptions, WritePriority, read_inventory, write_turtle,
};
use bacnet_ops::limits::RequestLimits;
use bacnet_ops::parse::parse_object_type;
use bacnet_transport::port::TransportPort;

use crate::args::ExportFormat;
use crate::output::screen;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Run `bacnet export`.
pub async fn export_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    format: &ExportFormat,
) -> Result<()> {
    let ExportFormat::S223 {
        target,
        base_iri,
        types,
        write_priority,
        output,
    } = format;
    // Every option is checked before anything is sent.
    let priority = write_priority.map(WritePriority::new).transpose()?;
    let turtle = TurtleOptions::new(base_iri, priority)?;
    let options = if types.is_empty() {
        InventoryOptions::default()
    } else {
        let types = types
            .iter()
            .map(|name| parse_object_type(name))
            .collect::<std::result::Result<_, _>>()?;
        InventoryOptions::new(types)?
    };
    if let Some(path) = output
        && std::fs::symlink_metadata(path).is_ok()
    {
        return Err(format!("output file already exists: {}", path.display()).into());
    }
    // By instance, at the address the discovery table holds for it; by
    // address, at it, the instance read once from the device (#2560).
    let (address, instance) = match crate::resolve_destination(client, target).await? {
        Destination::Device(instance) => (
            DeviceAddress::claiming(client, instance).await?,
            Some(instance),
        ),
        Destination::Mac(mac) => (DeviceAddress::local(&mac)?, None),
    };
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
        Err(Halt::Device(error)) => return Err(error.into()),
        Err(Halt::Cancelled) => return Err("the export was stopped".into()),
    };
    let device = inventory.device.instance;
    let mut text = String::new();
    let counts = write_turtle(&inventory, &turtle, &mut text)?;
    let to = match output {
        Some(path) => {
            let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(text.as_bytes())?;
            file.flush()?;
            path.display().to_string()
        }
        None => {
            print!("{text}");
            "stdout".into()
        }
    };
    // Stdout carries only the Turtle; the summary goes to stderr either way.
    eprintln!(
        "Wrote {} points of device {device} to {to} ({} skipped, {} with an unmapped unit, \
         {} identifiers left out for instance 0).",
        counts.exported, counts.skipped, counts.unmapped_units, counts.instance_zero,
    );
    if inventory.entries.is_empty() && inventory.stopped.is_none() {
        eprintln!("Device {device} has no object of the selected types: the model is empty.");
    }
    match inventory.stopped {
        Some(why) => Err(format!(
            "the export of device {device} stopped early ({}); it holds what was read before",
            screen(&why)
        )
        .into()),
        None => Ok(()),
    }
}
