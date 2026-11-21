//! Discovery commands: WhoIs and WhoHas.

use bacnet_client::client::{BACnetClient, MAX_I_HAVE_REPLIES, ReceivedIHave};
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

/// Where `discover`'s Who-Is (#2560) or `find`'s Who-Has (#2596) goes:
/// one station for `--target`, one remote network for `--dnet`, every
/// network for `--global`, and with none of them this network alone.
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
                    .await
                    .map_err(crate::destination::with_link_flag)?;
            Scope::Directed(DeviceAddress::local(&mac)?.into())
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
    /// Where the I-Have came from on this link, as `discover` writes an
    /// address: the device, or the router that passed it on (#2596).
    pub address: String,
    /// The device's network, when a router passed the I-Have on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<u16>,
    /// The device's routed station, in the target syntax the CLI reads
    /// back (`2:0x05`), when a router passed the I-Have on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
}

impl FoundObject {
    /// What `reply` reports, its source written in `style`, as
    /// [`device_info`] writes a discovered device's. With `own`, the
    /// client's network number once learned, a copy a router passed on from
    /// that network is the local station's, as the discovery table keeps it
    /// (#2572).
    fn of(reply: &ReceivedIHave, style: AddressStyle, own: Option<u16>) -> Self {
        let object = reply.i_have.object_identifier;
        let (mac, routed) = match &reply.source_network {
            Some(routed) if Some(routed.network) == own => (&routed.mac_address, None),
            routed => (&reply.source_mac, routed.as_ref()),
        };
        Self {
            device: reply.i_have.device_identifier.instance_number(),
            object: format!("{}:{}", object.object_type(), object.instance_number()),
            name: reply.i_have.object_name.clone(),
            address: style.format(mac),
            network: routed.map(|routed| routed.network),
            station: routed
                .and_then(|routed| target::routed_station(routed.network, &routed.mac_address)),
        }
    }
}

/// What `find` says on stderr when it stopped collecting at the cap: the
/// list may be cut short.
pub(crate) fn full_note() -> String {
    format!(
        "Note: stopped at {MAX_I_HAVE_REPLIES} answers, so more devices may have the object: \
         ask one network with --dnet, or one station with --target."
    )
}

/// What `find` says on stderr when a search of this network alone found
/// nothing (#2596): the miss is this network's, not the site's.
pub(crate) const LOCAL_MISS: &str = "Note: only this network was asked (a local broadcast, \
     which no router passes on). --dnet N asks one remote network, --global every network.";

/// The note for a search of `scope` that found `found`: [`LOCAL_MISS`]
/// for nothing found on this network alone, none otherwise.
pub(crate) fn miss_note(scope: &Scope, found: &[FoundObject]) -> Option<&'static str> {
    (found.is_empty() && *scope == Scope::Local).then_some(LOCAL_MISS)
}

/// Send a Who-Has for `name` to `scope` and print the objects the I-Haves
/// that answer it from that scope within `wait_secs` report: a JSON array
/// of `{device, object, name, address}` (with `network` and `station` for
/// a routed answer), empty when nothing answered. When nothing answered a
/// search of this network alone, stderr says so and how to widen it, in
/// either format, and so it does when collection stopped at the cap: the
/// JSON on stdout stays data.
pub async fn find_by_name<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    name: &str,
    scope: &Scope,
    wait_secs: u64,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    output::print_waiting(wait_secs);
    let replies = bacnet_ops::raw::who_has_replies(
        client,
        scope,
        &WhoHasObject::Name(name.to_string()),
        None,
        std::time::Duration::from_secs(wait_secs),
    )
    .await?;
    let style = AddressStyle::of(client.transport());
    let own = client.network_number();
    let found: Vec<FoundObject> = replies
        .iter()
        .map(|r| FoundObject::of(r, style, own))
        .collect();
    print_found(&found, format);
    if let Some(note) = miss_note(scope, &found) {
        eprintln!("{note}");
    }
    if found.len() >= MAX_I_HAVE_REPLIES {
        eprintln!("{}", full_note());
    }
    Ok(())
}

fn print_found(found: &[FoundObject], format: OutputFormat) {
    match format {
        OutputFormat::Table if found.is_empty() => println!("No objects found."),
        OutputFormat::Table => {
            // As `discover`'s table: with any routed answer, the link
            // address is the router's, and the station column says where
            // the device is.
            let routed = found.iter().any(|f| f.network.is_some());
            let mut table = comfy_table::Table::new();
            let mut header = vec!["Device", "Object", "Name"];
            header.extend(match routed {
                true => &["Router", "DNET", "Station"][..],
                false => &["Address"],
            });
            table.set_header(header);
            for f in found {
                let mut row = vec![
                    f.device.to_string(),
                    f.object.clone(),
                    output::screen(&f.name),
                    f.address.clone(),
                ];
                if routed {
                    let dash = || "-".to_string();
                    row.push(f.network.map_or_else(dash, |n| n.to_string()));
                    row.push(f.station.clone().unwrap_or_else(dash));
                }
                table.add_row(row);
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
        parsed_as("discover", flags)
    }

    /// `bacnet COMMAND` with `flags`, as `main` parses it.
    fn parsed_as(
        command: &'static str,
        flags: &[&'static str],
    ) -> Result<Option<Command>, clap::Error> {
        let argv: Vec<&'static str> = [&["bacnet", command][..], flags].concat();
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

    /// `find` takes the scope flags `discover` takes (#2596): with none,
    /// its Who-Has is a local broadcast, no destination network in its
    /// NPDU; `--dnet` and `--global` name one network and every network,
    /// `--target` one station; one at a time, and network 0 and 65535 are
    /// refused when parsed, naming the option to use.
    #[tokio::test]
    async fn find_without_a_scope_flag_is_a_local_broadcast() {
        use bacnet_encoding::apdu::Apdu;
        use bacnet_types::enums::UnconfirmedServiceChoice;

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
            let argv = [&["Zone Temp"][..], flags].concat();
            let Some(Command::Find {
                name,
                target,
                dnet,
                global,
                ..
            }) = parsed_as("find", &argv).unwrap()
            else {
                panic!("{flags:?} is no find");
            };
            let scope = scope(&client, target.as_deref(), dnet, global)
                .await
                .unwrap();
            find_by_name(&client, &name.unwrap(), &scope, 0, OutputFormat::Json)
                .await
                .unwrap();
            let frames = sent.take();
            let [frame] = frames.as_slice() else {
                panic!("{flags:?}: one Who-Has, not {frames:?}");
            };
            assert!(
                matches!(frame.apdu(), Apdu::UnconfirmedRequest(request)
                    if request.service_choice == UnconfirmedServiceChoice::WHO_HAS),
                "{flags:?}: {frame:?}"
            );
            let to = frame.decode_npdu().destination.map(|to| to.network);
            assert_eq!((to, frame.broadcast), (destination, broadcast), "{flags:?}");
        }
        for flags in [
            &["AHU-1", "--global", "--dnet", "5"][..],
            &["AHU-1", "--target", "10.0.0.9", "--global"],
            &["AHU-1", "--target", "10.0.0.9", "--dnet", "5"],
        ] {
            assert!(parsed_as("find", flags).is_err(), "{flags:?} parsed");
        }
        for (network, says) in [("0", "leave --dnet out"), ("65535", "--global")] {
            let refused = parsed_as("find", &["AHU-1", "--dnet", network])
                .unwrap_err()
                .to_string();
            assert!(refused.contains(says), "{network}: {refused}");
        }
        client.stop().await.unwrap();
    }

    /// Nothing found on this network alone carries the note that says so
    /// and how to widen the search; nothing found where the search named
    /// its scope, or something found, carries none.
    #[test]
    fn only_a_local_miss_carries_the_note() {
        let hit = [FoundObject {
            device: 1,
            object: "ANALOG_INPUT:1".into(),
            name: "Zone Temp".into(),
            address: "10.0.0.9:47808".into(),
            network: None,
            station: None,
        }];
        assert_eq!(miss_note(&Scope::Local, &[]), Some(LOCAL_MISS));
        assert!(LOCAL_MISS.contains("--dnet") && LOCAL_MISS.contains("--global"));
        let station = Scope::Directed(DeviceAddress::local(&BIP_LOCAL_MAC).unwrap().into());
        for scope in [Scope::Network(5), Scope::Global, station] {
            assert_eq!(miss_note(&scope, &[]), None, "{scope:?}");
        }
        assert_eq!(miss_note(&Scope::Local, &hit), None);
    }

    /// A row carries where its I-Have came from as `discover` writes a
    /// device (#2596): the station's address, or for a routed answer the
    /// router's, with the network and the routed station; a routed answer
    /// from this client's own network is the local station's.
    #[test]
    fn a_found_row_carries_its_address() {
        use bacnet_encoding::npdu::NpduAddress;
        use bacnet_services::who_has::IHaveRequest;
        use bacnet_types::MacAddr;
        use bacnet_types::enums::ObjectType;
        use bacnet_types::primitives::ObjectIdentifier;

        let id = |kind, instance| ObjectIdentifier::new(kind, instance).unwrap();
        let reply = |routed: Option<NpduAddress>| ReceivedIHave {
            i_have: IHaveRequest {
                device_identifier: id(ObjectType::DEVICE, 7),
                object_identifier: id(ObjectType::ANALOG_INPUT, 1),
                object_name: "Zone Temp".into(),
            },
            source_mac: MacAddr::from_slice(&[10, 0, 0, 9, 0xBA, 0xC0]),
            source_network: routed,
        };
        let local = FoundObject::of(&reply(None), AddressStyle::Bip, None);
        assert_eq!(
            serde_json::to_value(&local).unwrap(),
            serde_json::json!({ "device": 7, "object": "ANALOG_INPUT:1",
                                "name": "Zone Temp", "address": "10.0.0.9:47808" })
        );
        let routed = reply(Some(NpduAddress {
            network: 2,
            mac_address: MacAddr::from_slice(&[5]),
        }));
        let routed = FoundObject::of(&routed, AddressStyle::Bip, Some(1));
        assert_eq!(routed.address, "10.0.0.9:47808", "the router");
        assert_eq!(
            (routed.network, routed.station.as_deref()),
            (Some(2), Some("2:0x05"))
        );
        // A router's copy from this client's own network, once the client
        // knows its number, is the local station's, as `discover` shows it.
        let own = reply(Some(NpduAddress {
            network: 1,
            mac_address: MacAddr::from_slice(&[10, 0, 0, 7, 0xBA, 0xC0]),
        }));
        let own = FoundObject::of(&own, AddressStyle::Bip, Some(1));
        assert_eq!(own.address, "10.0.0.7:47808", "the station, not the router");
        assert_eq!((own.network, own.station), (None, None));
    }
}
