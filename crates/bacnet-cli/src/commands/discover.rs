//! Discovery commands: WhoIs and WhoHas.

use bacnet_client::client::{BACnetClient, ReceivedIHave};
use bacnet_ops::target::{self, AddressStyle};
use bacnet_ops::{DeviceAddress, Scope};
use bacnet_services::who_has::WhoHasObject;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use serde::Serialize;

use crate::output::{self, DeviceInfo, OutputFormat, device_info};

/// Collect device infos from the client's discovered device table,
/// optionally filtering by instance range.
async fn collect_devices<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    range: Option<DeviceInstanceRange>,
) -> Vec<DeviceInfo> {
    let devices = client.discovered_devices().await;
    devices
        .iter()
        .filter(|d| {
            let inst = d.object_identifier.instance_number();
            range.is_none_or(|range| range.contains(inst))
        })
        .map(|device| device_info(device, AddressStyle::of(client.transport())))
        .collect()
}

/// Where `discover`'s Who-Is goes (#2560): one station for `--target`,
/// one remote network for `--dnet`, every network for `--global`, and with
/// none of them this network alone.
pub async fn scope<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    target: Option<&str>,
    dnet: Option<u16>,
    global: bool,
) -> Result<Scope, Box<dyn std::error::Error>> {
    Ok(match (target, dnet) {
        (Some(target), _) => {
            let mac =
                target::address_mac(client.transport(), target, target::TARGET_NOT_AN_ADDRESS)
                    .await?;
            Scope::Directed(DeviceAddress::local(&mac)?)
        }
        (None, Some(network)) => Scope::Network(network),
        (None, None) if global => Scope::Global,
        (None, None) => Scope::Local,
    })
}

/// Send a Who-Is to `scope` and display the discovered devices after
/// waiting.
pub async fn discover<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    scope: &Scope,
    range: Option<DeviceInstanceRange>,
    wait_secs: u64,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    bacnet_ops::raw::who_is(client, scope, range).await?;
    output::print_waiting(wait_secs);
    tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
    output::print_devices(&collect_devices(client, range).await, format);
    Ok(())
}

/// One object an I-Have reported, as `find` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FoundObject {
    /// The instance of the device that has the object.
    pub device: u32,
    /// The object, as `TYPE:instance`.
    pub object: String,
    /// The object's name.
    pub name: String,
}

impl From<&ReceivedIHave> for FoundObject {
    fn from(reply: &ReceivedIHave) -> Self {
        let object = reply.i_have.object_identifier;
        Self {
            device: reply.i_have.device_identifier.instance_number(),
            object: format!("{}:{}", object.object_type(), object.instance_number()),
            name: reply.i_have.object_name.clone(),
        }
    }
}

/// Send a Who-Has for `name` and print the objects the I-Haves that answer
/// it within `wait_secs` report: a JSON array of `{device, object, name}`,
/// empty when nothing answered.
pub async fn find_by_name<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    name: &str,
    wait_secs: u64,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    output::print_waiting(wait_secs);
    let replies = client
        .who_has_replies(
            WhoHasObject::Name(name.to_string()),
            None,
            std::time::Duration::from_secs(wait_secs),
        )
        .await?;
    let found: Vec<FoundObject> = replies.iter().map(FoundObject::from).collect();
    print_found(&found, format);
    Ok(())
}

fn print_found(found: &[FoundObject], format: OutputFormat) {
    match format {
        OutputFormat::Table if found.is_empty() => println!("No objects found."),
        OutputFormat::Table => {
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["Device", "Object", "Name"]);
            for f in found {
                table.add_row(vec![
                    f.device.to_string(),
                    f.object.clone(),
                    output::screen(&f.name),
                ]);
            }
            println!("{table}");
        }
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(found).expect("serialize found objects")
        ),
    }
}

#[cfg(test)]
mod tests {
    use bacnet_client::client::ClientConfig;
    use bacnet_transport::test_transport::{BIP_LOCAL_MAC, TestTransport};

    use super::*;
    use crate::args::{Cli, Command};

    /// `bacnet discover` with `flags`, as `main` parses it.
    fn parsed(flags: &[&'static str]) -> Result<Option<Command>, clap::Error> {
        let argv: Vec<&'static str> = [&["bacnet", "discover"][..], flags].concat();
        crate::parse_on_large_stack(move || <Cli as clap::Parser>::try_parse_from(argv))
            .map(|cli| cli.command)
    }

    /// `discover` with no scope flag sends a local broadcast, which no
    /// router passes on: no destination network in its NPDU (#2560).
    /// `--dnet` and `--global` are the explicit broadcasts to one network
    /// and to every network, `--target` a unicast; one at a time.
    #[tokio::test]
    async fn discover_without_a_scope_flag_is_a_local_broadcast() {
        let port = TestTransport::builder().local_mac(&BIP_LOCAL_MAC).build();
        let sent = port.sent();
        let mut client = BACnetClient::start(ClientConfig::default(), port)
            .await
            .unwrap();
        let cases: [(&[&'static str], Option<u16>, bool); 4] = [
            (&[], None, true),
            (&["--dnet", "5"], Some(5), true),
            (&["--global"], Some(0xFFFF), true),
            (&["--target", "10.0.0.9"], None, false),
        ];
        for (flags, destination, broadcast) in cases {
            let Some(Command::Discover {
                target,
                dnet,
                global,
                ..
            }) = parsed(flags).unwrap()
            else {
                panic!("{flags:?} is no discover");
            };
            let scope = scope(&client, target.as_deref(), dnet, global)
                .await
                .unwrap();
            discover(&client, &scope, None, 0, OutputFormat::Json)
                .await
                .unwrap();
            let frames = sent.take();
            let [frame] = frames.as_slice() else {
                panic!("{flags:?}: one Who-Is, not {frames:?}");
            };
            let to = frame.decode_npdu().destination.map(|to| to.network);
            assert_eq!((to, frame.broadcast), (destination, broadcast), "{flags:?}");
        }
        for flags in [
            &["--global", "--dnet", "5"][..],
            &["--target", "10.0.0.9", "--global"],
            &["--target", "10.0.0.9", "--dnet", "5"],
        ] {
            assert!(parsed(flags).is_err(), "{flags:?} parsed");
        }
        // Network 0 and 65535 are refused when parsed, naming what to use.
        for (network, says) in [("0", "leave --dnet out"), ("65535", "--global")] {
            let refused = parsed(&["--dnet", network]).unwrap_err().to_string();
            assert!(refused.contains(says), "{network}: {refused}");
        }
        client.stop().await.unwrap();
    }
}
