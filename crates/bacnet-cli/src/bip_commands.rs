//! The one-shot commands only a BACnet/IP client runs: BBMD management, and
//! a `discover` that registers with a BBMD first. Each command's future is
//! boxed, as `execute_command`'s are (see [`crate::heap`]).

use bacnet_client::client::BACnetClient;
use bacnet_ops::target;
use bacnet_transport::bip::BipTransport;

use crate::args::Command;
use crate::commands;
use crate::core::range::parse_discover_range;
use crate::heap::boxed;
use crate::output::OutputFormat;
use crate::resolve_target_mac;

/// Try to execute a BIP-specific BBMD management command.
/// Returns `Ok(true)` if handled, `Ok(false)` if not a BIP-specific command.
pub(crate) async fn execute_bip_command(
    client: &BACnetClient<BipTransport>,
    cmd: &Command,
    ttl: u16,
    format: OutputFormat,
) -> Result<bool, Box<dyn std::error::Error>> {
    match cmd {
        Command::Discover {
            range,
            wait,
            target,
            bbmd: Some(bbmd_addr),
            dnet,
            global,
        } => {
            let bbmd_mac = boxed(|| {
                target::address_mac(
                    client.transport(),
                    bbmd_addr,
                    "--bbmd requires an IP address, not a device instance",
                )
            })
            .await?;
            boxed(|| client.register_foreign_device_bvlc(&bbmd_mac, ttl)).await?;
            eprintln!("Registered as foreign device with BBMD {bbmd_addr}");
            // Brief pause to allow registration to propagate.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let range = parse_discover_range(range.as_deref())?;
            let scope =
                boxed(|| commands::discover::scope(client, target.as_deref(), *dnet, *global))
                    .await?;
            boxed(|| commands::discover::discover(client, &scope, range, *wait, format)).await?;
        }
        Command::Bdt { target } => {
            let mac = resolve_target_mac(client, target).await?;
            boxed(|| commands::router::bdt_cmd(client, &mac, format)).await?;
        }
        Command::Fdt { target } => {
            let mac = resolve_target_mac(client, target).await?;
            boxed(|| commands::router::fdt_cmd(client, &mac, format)).await?;
        }
        Command::Register { target } => {
            let mac = resolve_target_mac(client, target).await?;
            boxed(|| commands::router::register_cmd(client, &mac, ttl, format)).await?;
        }
        Command::Unregister { target } => {
            let mac = resolve_target_mac(client, target).await?;
            boxed(|| commands::router::unregister_cmd(client, &mac, format)).await?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}
