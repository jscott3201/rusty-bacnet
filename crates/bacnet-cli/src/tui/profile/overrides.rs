//! Explicit command-line values override profile values, including defaults.

use super::sc::{DeviceUuid, Vmac};
use super::{Profile, Transport};
use crate::args::Cli;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;

/// Only values with clap's CommandLine provenance are present.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Overrides {
    timeout_ms: Option<u64>,
    min_interval_ms: Option<u64>,
    bip: BipOverrides,
    sc: ScOverrides,
    mstp: MstpOverrides,
}

/// Flags that only an MS/TP profile takes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct MstpOverrides {
    port: Option<String>,
    baud: Option<u32>,
    mac: Option<u8>,
    max_master: Option<u8>,
    max_info_frames: Option<u8>,
}

/// Flags that only a B/IP profile takes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct BipOverrides {
    interface: Option<Ipv4Addr>,
    port: Option<u16>,
    broadcast: Option<Ipv4Addr>,
    bbmd: Option<SocketAddrV4>,
    ttl: Option<u16>,
    renewal_secs: Option<u64>,
    multicast: Option<Ipv4Addr>,
    multicast_ttl: Option<u8>,
    multicast_port: Option<u16>,
}

/// Flags that only an SC profile takes. `--sc` itself only says which
/// transport the command line meant.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ScOverrides {
    sc: bool,
    hub_url: Option<String>,
    failover_hub_url: Option<String>,
    /// `--sc-reconnect` turns reconnect on; there is no flag to turn it off.
    reconnect: bool,
    connect_timeout_ms: Option<u64>,
    ca: Option<PathBuf>,
    cert: Option<PathBuf>,
    key: Option<PathBuf>,
    vmac: Option<Vmac>,
    device_uuid: Option<DeviceUuid>,
}

/// A path flag made absolute against the working directory now, so the
/// effective path doesn't depend on where the coordinator reads it later.
fn absolute(path: &std::path::Path) -> Result<PathBuf, String> {
    std::path::absolute(path).map_err(|e| format!("cannot resolve {}: {e}", path.display()))
}

impl Overrides {
    pub(crate) fn from_cli(cli: &Cli) -> Result<Self, String> {
        let has = |key: &str| cli.explicit_args.iter().any(|arg| arg == key);
        if ["ipv6", "ipv6_interface", "device_instance"]
            .iter()
            .any(|key| has(key))
        {
            return Err(
                "named profiles support BACnet/IP, BACnet/SC and MS/TP only; remove IPv6 flags or use --no-profile"
                    .into(),
            );
        }
        let path = |key: &str, value: &Option<PathBuf>| -> Result<Option<PathBuf>, String> {
            value
                .as_deref()
                .filter(|_| has(key))
                .map(absolute)
                .transpose()
        };
        Ok(Self {
            timeout_ms: has("timeout").then_some(cli.timeout),
            min_interval_ms: has("min_interval_ms").then_some(cli.min_interval_ms),
            bip: BipOverrides {
                interface: cli.interface.filter(|_| has("interface")),
                port: has("port").then_some(cli.port),
                broadcast: has("broadcast").then_some(cli.broadcast),
                bbmd: cli.foreign_bbmd.filter(|_| has("foreign_bbmd")),
                ttl: has("ttl").then_some(cli.ttl),
                renewal_secs: cli.renewal.filter(|_| has("renewal")),
                multicast: cli.bip_multicast.filter(|_| has("bip_multicast")),
                multicast_ttl: has("bip_multicast_ttl").then_some(cli.bip_multicast_ttl),
                multicast_port: cli.bip_multicast_port.filter(|_| has("bip_multicast_port")),
            },
            sc: ScOverrides {
                sc: has("sc") && cli.sc,
                hub_url: cli.sc_url.clone().filter(|_| has("sc_url")),
                failover_hub_url: cli
                    .sc_failover_url
                    .clone()
                    .filter(|_| has("sc_failover_url")),
                reconnect: has("sc_reconnect") && cli.sc_reconnect,
                connect_timeout_ms: cli.sc_connect_timeout.filter(|_| has("sc_connect_timeout")),
                ca: path("sc_ca", &cli.sc_ca)?,
                cert: path("sc_cert", &cli.sc_cert)?,
                key: path("sc_key", &cli.sc_key)?,
                vmac: cli.sc_vmac.filter(|_| has("sc_vmac")).map(Vmac),
                device_uuid: cli
                    .sc_device_uuid
                    .filter(|_| has("sc_device_uuid"))
                    .map(DeviceUuid),
            },
            mstp: MstpOverrides {
                port: cli.mstp.clone().filter(|_| has("mstp")),
                baud: has("mstp_baud").then_some(cli.mstp_baud),
                mac: cli.mstp_mac.filter(|_| has("mstp_mac")),
                max_master: has("mstp_max_master").then_some(cli.mstp_max_master),
                max_info_frames: has("mstp_max_info_frames").then_some(cli.mstp_max_info_frames),
            },
        })
    }

    /// Apply the explicit flags, then validate the actual connection settings.
    /// A flag for the other transport is an error, not silently dropped.
    pub(crate) fn resolve(&self, profile: &Profile) -> Result<Profile, String> {
        profile.validate()?;
        let mut resolved = profile.clone();
        match profile.transport {
            _ if profile.transport != Transport::Mstp && self.mstp != MstpOverrides::default() => {
                return Err("MS/TP flags (--mstp, --mstp-*) apply only to an MS/TP profile".into());
            }
            Transport::Mstp => {
                if self.bip != BipOverrides::default() || self.sc != ScOverrides::default() {
                    return Err("B/IP and SC flags don't apply to an MS/TP profile".into());
                }
                let m = resolved.mstp.as_mut().expect("validated MS/TP profile");
                let o = &self.mstp;
                set(&mut m.port, o.port.clone());
                set(&mut m.baud, o.baud);
                set(&mut m.mac, o.mac);
                set(&mut m.max_master, o.max_master);
                set(&mut m.max_info_frames, o.max_info_frames);
                set(&mut m.timeout_ms, self.timeout_ms);
                set(&mut m.min_interval_ms, self.min_interval_ms);
            }
            Transport::Bip => {
                if self.sc != ScOverrides::default() {
                    return Err("SC flags (--sc, --sc-*) don't apply to a B/IP profile".into());
                }
                let b = resolved.bip.as_mut().expect("validated B/IP profile");
                let o = &self.bip;
                set(&mut b.interface, o.interface);
                set(&mut b.port, o.port);
                set(&mut b.broadcast, o.broadcast);
                set(&mut b.timeout_ms, self.timeout_ms);
                set(&mut b.min_interval_ms, self.min_interval_ms);
                if let Some(v) = o.bbmd {
                    b.bbmd = Some(v);
                }
                set(&mut b.ttl, o.ttl);
                if let Some(v) = o.renewal_secs {
                    b.renewal_secs = Some(v);
                }
                if let Some(v) = o.multicast {
                    b.multicast = Some(v);
                }
                set(&mut b.multicast_ttl, o.multicast_ttl);
                if let Some(v) = o.multicast_port {
                    b.multicast_port = Some(v);
                }
            }
            Transport::Sc => {
                if self.bip != BipOverrides::default() {
                    return Err(
                        "B/IP flags (interface, port, broadcast, BBMD) don't apply to an SC profile"
                            .into(),
                    );
                }
                let s = resolved.sc.as_mut().expect("validated SC profile");
                let o = &self.sc;
                set(&mut s.hub_url, o.hub_url.clone());
                if let Some(v) = &o.failover_hub_url {
                    s.failover_hub_url = Some(v.clone());
                }
                // As on the command line, a failover hub brings reconnect.
                if o.reconnect || o.failover_hub_url.is_some() {
                    s.reconnect = true;
                }
                set(&mut s.connect_timeout_ms, o.connect_timeout_ms);
                set(&mut s.ca, o.ca.clone());
                set(&mut s.cert, o.cert.clone());
                set(&mut s.key, o.key.clone());
                if let Some(v) = o.vmac {
                    s.vmac = Some(v);
                }
                if let Some(v) = o.device_uuid {
                    s.device_uuid = Some(v);
                }
                set(&mut s.timeout_ms, self.timeout_ms);
                set(&mut s.min_interval_ms, self.min_interval_ms);
            }
        }
        resolved.validate()?;
        Ok(resolved)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

fn set<T>(slot: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *slot = value;
    }
}

/// Explicit network flags retain the existing nonpersistent entry path.
pub(crate) fn has_transport_flags(cli: &Cli) -> bool {
    cli.explicit_args.iter().any(|id| {
        matches!(
            id.as_str(),
            "interface"
                | "port"
                | "broadcast"
                | "timeout"
                | "min_interval_ms"
                | "foreign_bbmd"
                | "ttl"
                | "renewal"
                | "bip_multicast"
                | "bip_multicast_ttl"
                | "bip_multicast_port"
                | "ipv6"
                | "ipv6_interface"
                | "device_instance"
                | "sc"
                | "sc_url"
                | "sc_failover_url"
                | "sc_reconnect"
                | "sc_connect_timeout"
                | "sc_ca"
                | "sc_cert"
                | "sc_key"
                | "sc_vmac"
                | "sc_device_uuid"
                | "mstp"
                | "mstp_baud"
                | "mstp_mac"
                | "mstp_max_master"
                | "mstp_max_info_frames"
        )
    })
}
