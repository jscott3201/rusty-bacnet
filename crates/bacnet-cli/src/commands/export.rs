//! `bacnet export 223-experimental` (#2436): a device's objects as an
//! experimental Standard 223 point list in Turtle, read and written by
//! `bacnet_ops::export::s223`.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::AtomicBool;

use bacnet_client::client::BACnetClient;
use bacnet_ops::destination::Destination;
use bacnet_ops::engine::{Halt, Reader};
use bacnet_ops::export::s223::{
    InventoryOptions, TurtleOptions, WritePriority, read_inventory, write_turtle,
};
use bacnet_ops::format::decode_all;
use bacnet_ops::limits::RequestLimits;
use bacnet_ops::parse::parse_object_type;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

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
    let device = match crate::resolve_destination(client, target).await? {
        Destination::Device(instance) => instance,
        Destination::Mac(mac) => device_at(client, &mac).await?,
    };
    // A one-shot command has the device to itself: no rate cap beyond the
    // client's --min-interval-ms, one request at a time.
    let limits = RequestLimits::new(0);
    let never = AtomicBool::new(false);
    let mut reader = Reader {
        client,
        device,
        rate: &limits,
        cancel: &never,
        rpm: true,
    };
    let inventory = match read_inventory(&mut reader, &options).await {
        Ok(inventory) => inventory,
        Err(Halt::Device(error)) => return Err(error.into()),
        Err(Halt::Cancelled) => return Err("the export was stopped".into()),
    };
    let mut text = String::new();
    let counts = write_turtle(&inventory, &turtle, &mut text)?;
    match output {
        Some(path) => {
            let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(text.as_bytes())?;
            file.flush()?;
            eprintln!(
                "Wrote {} points of device {device} to {} ({} skipped, {} with an unmapped unit, \
                 {} identifiers left out for instance 0).",
                counts.exported,
                path.display(),
                counts.skipped,
                counts.unmapped_units,
                counts.instance_zero,
            );
        }
        None => print!("{text}"),
    }
    match inventory.stopped {
        Some(why) => Err(format!(
            "device {device} stopped answering ({}); the export holds what was read before",
            screen(&why)
        )
        .into()),
        None => Ok(()),
    }
}

/// The instance of the device at `mac`, from its Device object read by the
/// wildcard instance, which a device answers as its own. The client's
/// discovery table gets
/// the device when it doesn't hold it yet, since the export reads by
/// instance; one it holds at another address is refused.
async fn device_at<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
) -> Result<u32> {
    let any = ObjectIdentifier::new(ObjectType::DEVICE, ObjectIdentifier::WILDCARD_INSTANCE)?;
    let ack = client
        .read_property(mac, any, PropertyIdentifier::OBJECT_IDENTIFIER, None)
        .await?;
    let instance = match decode_all(&ack.property_value).as_deref() {
        Some([PropertyValue::ObjectIdentifier(device)])
            if device.object_type() == ObjectType::DEVICE =>
        {
            device.instance_number()
        }
        _ => return Err("the device didn't name its Device object".into()),
    };
    match client.get_device(instance).await {
        None => client.add_device(instance, mac).await?,
        Some(known) if known.source_network.is_none() && known.mac_address.as_slice() == mac => {}
        Some(_) => {
            return Err(format!(
                "device {instance} answers at this address, but the discovery table holds it \
                 at another; export it by instance"
            )
            .into());
        }
    }
    Ok(instance)
}
