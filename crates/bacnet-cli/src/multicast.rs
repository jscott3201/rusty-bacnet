//! `--bip-multicast`: run the CLI's own B/IP transport as a B/IP-M node
//! (Annex J.8, #941), so its broadcasts go to a multicast group and only
//! the group's broadcasts come back.

use std::net::Ipv4Addr;

use bacnet_transport::bip::BipMulticast;

/// Parse a `--bip-multicast` group: a class D IPv4 address.
pub(crate) fn parse_group_arg(value: &str) -> Result<Ipv4Addr, String> {
    let group = value
        .parse::<Ipv4Addr>()
        .map_err(|_| format!("'{value}' is not an IPv4 address"))?;
    if !group.is_multicast() {
        return Err(format!(
            "'{value}' is not a multicast group (224.0.0.0 to 239.255.255.255)"
        ));
    }
    Ok(group)
}

/// A group with its TTL and, when set, its own port, checked as the
/// transport checks it.
pub(crate) fn group(group: Ipv4Addr, ttl: u8, port: Option<u16>) -> Result<BipMulticast, String> {
    let multicast = BipMulticast::new(group).and_then(|multicast| multicast.with_ttl(ttl));
    match port {
        Some(port) => multicast.and_then(|multicast| multicast.with_port(port)),
        None => multicast,
    }
    .map_err(|e| e.to_string())
}

/// The group `--bip-multicast`, `--bip-multicast-ttl` and
/// `--bip-multicast-port` ask for: B/IP only, in normal mode, on an
/// explicit interface, with a port for the group.
pub(crate) fn from_cli(
    cli: &crate::args::Cli,
    interface: Ipv4Addr,
) -> Result<Option<BipMulticast>, String> {
    let Some(group_ip) = cli.bip_multicast else {
        for (id, flag) in [
            ("bip_multicast_ttl", "--bip-multicast-ttl"),
            ("bip_multicast_port", "--bip-multicast-port"),
        ] {
            if cli.explicit_args.iter().any(|arg| arg == id) {
                return Err(format!("{flag} requires --bip-multicast"));
            }
        }
        return Ok(None);
    };
    if cli.sc || cli.ipv6 {
        return Err(
            "--bip-multicast requires BACnet/IP transport (do not use --sc or --ipv6)".into(),
        );
    }
    if cli.foreign_bbmd.is_some() {
        return Err("--bip-multicast can't be combined with --foreign-bbmd".into());
    }
    if interface.is_unspecified() {
        return Err("--bip-multicast needs --interface: the group is joined there".into());
    }
    if cli.port == 0 && cli.bip_multicast_port.is_none() {
        return Err("--bip-multicast with --port 0 needs --bip-multicast-port".into());
    }
    group(group_ip, cli.bip_multicast_ttl, cli.bip_multicast_port).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::Cli;
    use crate::transport::TransportArgs;

    fn parse<const N: usize>(args: [&'static str; N]) -> Result<Cli, clap::Error> {
        crate::parse_on_large_stack(move || {
            use clap::{CommandFactory, FromArgMatches};
            let mut matches = Cli::command().try_get_matches_from(args)?;
            let explicit_args = crate::args::capture_sources(&matches)?;
            let mut cli = Cli::from_arg_matches_mut(&mut matches)?;
            cli.explicit_args = explicit_args;
            Ok(cli)
        })
    }

    fn transport(cli: &Cli) -> Result<TransportArgs, String> {
        let interface = cli.interface.unwrap_or(Ipv4Addr::UNSPECIFIED);
        TransportArgs::from_cli(cli, interface, Ipv4Addr::BROADCAST)
    }

    #[test]
    fn the_multicast_flags_reach_the_transport() {
        let cli = parse([
            "bacnet",
            "--interface",
            "192.0.2.20",
            "--bip-multicast",
            "239.255.66.1",
            "--bip-multicast-ttl",
            "3",
            "discover",
        ])
        .unwrap();
        let multicast = transport(&cli).unwrap().multicast.unwrap();
        assert_eq!(
            (multicast.group(), multicast.ttl()),
            (Ipv4Addr::new(239, 255, 66, 1), 3)
        );
        let cli = parse(["bacnet", "discover"]).unwrap();
        assert!(transport(&cli).unwrap().multicast.is_none());
    }

    #[test]
    fn the_multicast_flags_are_refused_where_they_cant_apply() {
        let refused = |cli: Cli, why: &str| {
            let error = transport(&cli).err().expect("refused");
            assert!(error.contains(why), "{error}");
        };
        let group = ["--bip-multicast", "239.255.66.1"];
        refused(
            parse(["bacnet", group[0], group[1], "discover"]).unwrap(),
            "--interface",
        );
        refused(
            parse([
                "bacnet",
                "--interface",
                "192.0.2.20",
                group[0],
                group[1],
                "--ipv6",
                "discover",
            ])
            .unwrap(),
            "BACnet/IP",
        );
        refused(
            parse([
                "bacnet",
                "--interface",
                "192.0.2.20",
                group[0],
                group[1],
                "--foreign-bbmd",
                "192.0.2.10",
                "discover",
            ])
            .unwrap(),
            "--foreign-bbmd",
        );
        refused(
            parse(["bacnet", "--bip-multicast-ttl", "2", "discover"]).unwrap(),
            "requires --bip-multicast",
        );
        assert!(parse(["bacnet", "--bip-multicast", "10.0.0.1", "discover"]).is_err());
        assert!(parse(["bacnet", "--bip-multicast-ttl", "0", "discover"]).is_err());
    }

    #[test]
    fn only_a_class_d_group_is_taken() {
        assert_eq!(
            parse_group_arg("239.255.66.1"),
            Ok(Ipv4Addr::new(239, 255, 66, 1))
        );
        for bad in ["10.0.0.1", "255.255.255.255", "group"] {
            assert!(parse_group_arg(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_group_port_flag_reaches_the_transport() {
        let cli = parse([
            "bacnet",
            "--interface",
            "127.0.0.1",
            "--port",
            "0",
            "--bip-multicast",
            "239.255.66.1",
            "--bip-multicast-port",
            "47809",
            "discover",
        ])
        .unwrap();
        let multicast = transport(&cli).unwrap().multicast.unwrap();
        assert_eq!(multicast.port(), Some(47809));
        // A picked port has none the group could share.
        let cli = parse([
            "bacnet",
            "--interface",
            "127.0.0.1",
            "--port",
            "0",
            "--bip-multicast",
            "239.255.66.1",
            "discover",
        ])
        .unwrap();
        let error = transport(&cli).err().unwrap();
        assert!(error.contains("--bip-multicast-port"), "{error}");
        let cli = parse(["bacnet", "--bip-multicast-port", "47809", "discover"]).unwrap();
        let error = transport(&cli).err().unwrap();
        assert!(error.contains("requires --bip-multicast"), "{error}");
        assert!(parse(["bacnet", "--bip-multicast-port", "0", "discover"]).is_err());
    }
}
