//! Constructor checks for B/IP-only options, and the B/IP mode options
//! (`bbmd=` and `foreign_device=`, #938; B/IPv6 `foreign_device=`, #1941).

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};
use std::path::PathBuf;
use std::time::Duration;

use bacnet_transport::bbmd::{BdtEntry, ForeignDevicePolicy, NatTraversal, PrivateRange};
use bacnet_transport::bip::{
    BbmdOptions, BipMode, BipMulticast, DEFAULT_BACNET_PORT, ForeignDeviceConfig,
    ForeignRegistrationOutcome, ForeignRegistrationSnapshot,
};
use bacnet_transport::bip6::Bip6ForeignDeviceConfig;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// `share_port_by_address` (#1538) binds a B/IP interface address, so it
/// means nothing on another transport: refuse it there rather than ignore it.
pub(crate) fn only_on_bip(share_port_by_address: bool, transport: &str) -> PyResult<()> {
    if share_port_by_address && transport != "bip" {
        return Err(PyValueError::new_err(
            "share_port_by_address applies only to transport='bip'",
        ));
    }
    Ok(())
}

/// The shortest renewal interval, in seconds: the transport's pacing floor.
const MIN_RENEWAL_SECS: f64 = 0.1;

/// A foreign-device registration, for the `foreign_device=` keyword (#938,
/// #1941).
///
/// `bbmd` names a B/IP BBMD as `"ip"` or `"ip:port"`, or a B/IPv6 one as
/// `"ip"`, `"[ip]"` or `"[ip]:port"` (port 47808 by default; a port needs
/// the brackets, since `"2001:db8::10:4780"` is a bare address); the owner's
/// transport must match. A B/IPv6 BBMD must be a unicast host: not unspecified,
/// multicast, link-local, IPv4-mapped or IPv4-compatible. `ttl`, at least
/// 1, is how many seconds the BBMD keeps the entry. `renewal` (B/IP only)
/// is the seconds between registrations, at least 0.1 and shorter than
/// `ttl`; `None` renews at half the TTL, which is what a B/IPv6 foreign
/// device always does (but never more often than every 30 s). Behind NAT, renew often so the
/// mapping the BBMD's replies come back on stays open, while the TTL stays
/// long. A bad value raises ValueError, or OverflowError for an integer out
/// of range.
#[pyclass(
    name = "ForeignDevice",
    module = "rusty_bacnet",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyForeignDevice {
    registration: Registration,
}

/// The registration a [`PyForeignDevice`] holds: with a B/IP BBMD or a
/// B/IPv6 one.
#[derive(Clone)]
enum Registration {
    Bip(ForeignDeviceConfig),
    Bip6(Bip6ForeignDeviceConfig),
}

#[pymethods]
impl PyForeignDevice {
    /// Check and keep the registration's settings.
    #[new]
    #[pyo3(signature = (bbmd, ttl=300, renewal=None))]
    fn new(bbmd: &str, ttl: u16, renewal: Option<f64>) -> PyResult<Self> {
        if let Some(address) = parse_ipv6_address(bbmd)? {
            return Self::bip6(bbmd, address, ttl, renewal);
        }
        if bbmd.parse::<SocketAddrV4>().is_err() && bbmd.parse::<Ipv4Addr>().is_err() {
            return Err(PyValueError::new_err(format!(
                "bbmd: '{bbmd}' is not an IPv4 or IPv6 address ('ip' or 'ip:port' for B/IP, \
                 'ip' or '[ip]:port' for B/IPv6)"
            )));
        }
        let address = parse_address(bbmd, "bbmd")?;
        if ttl == 0 {
            return Err(PyValueError::new_err("ttl must be at least 1 second"));
        }
        let renewal_interval = renewal
            .map(|seconds| {
                // The transport renews no faster than every 100 ms.
                if !(seconds >= MIN_RENEWAL_SECS && seconds < f64::from(ttl)) {
                    return Err(PyValueError::new_err(
                        "renewal must be at least 0.1 and less than ttl seconds",
                    ));
                }
                Ok(Duration::from_secs_f64(seconds))
            })
            .transpose()?;
        Ok(Self {
            registration: Registration::Bip(ForeignDeviceConfig {
                bbmd_ip: *address.ip(),
                bbmd_port: address.port(),
                ttl,
                renewal_interval,
            }),
        })
    }

    /// The BBMD's address: `"ip:port"`, or `"[ip]:port"` for B/IPv6.
    #[getter]
    fn bbmd(&self) -> String {
        match &self.registration {
            Registration::Bip(config) => {
                SocketAddrV4::new(config.bbmd_ip, config.bbmd_port).to_string()
            }
            Registration::Bip6(config) => {
                SocketAddrV6::new(config.bbmd_ip, config.bbmd_port, 0, 0).to_string()
            }
        }
    }

    /// The time-to-live, in seconds.
    #[getter]
    fn ttl(&self) -> u16 {
        match &self.registration {
            Registration::Bip(config) => config.ttl,
            Registration::Bip6(config) => config.ttl,
        }
    }

    /// The seconds between registrations, or `None` for half the TTL.
    #[getter]
    fn renewal(&self) -> Option<f64> {
        match &self.registration {
            Registration::Bip(config) => config.renewal_interval.map(|d| d.as_secs_f64()),
            Registration::Bip6(_) => None,
        }
    }

    fn __repr__(&self) -> String {
        let renewal = self
            .renewal()
            .map_or_else(|| "None".to_string(), |s| s.to_string());
        format!(
            "ForeignDevice(bbmd='{}', ttl={}, renewal={renewal})",
            self.bbmd(),
            self.ttl()
        )
    }
}

impl PyForeignDevice {
    /// A registration with the B/IPv6 BBMD at `address`, checked as the
    /// transport checks it.
    fn bip6(text: &str, address: SocketAddrV6, ttl: u16, renewal: Option<f64>) -> PyResult<Self> {
        if ttl == 0 {
            return Err(PyValueError::new_err("ttl must be at least 1 second"));
        }
        if renewal.is_some() {
            return Err(PyValueError::new_err(
                "renewal applies only to a B/IP BBMD: a B/IPv6 foreign device renews at \
                 half the ttl, but never more often than every 30 seconds",
            ));
        }
        let config = Bip6ForeignDeviceConfig {
            bbmd_ip: *address.ip(),
            bbmd_port: address.port(),
            ttl,
        };
        config.check().map_err(|refusal| {
            PyValueError::new_err(format!(
                "bbmd must be an IPv6 unicast host with a nonzero port, got '{text}': {refusal}"
            ))
        })?;
        Ok(Self {
            registration: Registration::Bip6(config),
        })
    }
}

/// `"ip"`, `"[ip]"` or `"[ip]:port"` as a B/IPv6 address, port 47808 by default;
/// `None` for any other text, which the B/IP parser then reads. A zone or
/// flow label is refused: neither travels with a B/IPv6 address.
fn parse_ipv6_address(text: &str) -> PyResult<Option<SocketAddrV6>> {
    let address = match text.parse::<SocketAddrV6>() {
        Ok(address) => address,
        Err(_) => {
            // "ip" or "[ip]", both with the default port.
            let bare = text
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .unwrap_or(text);
            match bare.parse::<Ipv6Addr>() {
                Ok(ip) => SocketAddrV6::new(ip, DEFAULT_BACNET_PORT, 0, 0),
                Err(_) => return Ok(None),
            }
        }
    };
    if address.scope_id() != 0 || address.flowinfo() != 0 {
        return Err(PyValueError::new_err(format!(
            "bbmd must be an IPv6 unicast host with a nonzero port, got '{text}': \
             a zone index or flow label doesn't travel with a B/IPv6 address"
        )));
    }
    Ok(Some(address))
}

/// A BBMD set-up, for the `bbmd=` keyword (#938).
///
/// `bdt` lists the BBMDs of the other subnets, each `"ip:port"` (its
/// broadcasts forwarded by unicast) or `("ip:port", "mask")`; this BBMD's
/// own row is added when missing. `accept_foreign_devices` lets foreign
/// devices register (default False: every registration is refused).
/// `management_acl` lists the IPv4 addresses allowed to delete Foreign
/// Device Table entries (default: none). `bdt_persist_path` names a file
/// holding a BDT to start with instead, when it loads, and where a BDT
/// activated through the registered Network Port is saved. `alone_on_subnet`
/// says this BBMD is the only device of its B/IP network on its subnet, as
/// behind a NAT router: it then skips the local rebroadcast of forwarded and
/// distributed broadcasts (default False). `nat_global_address`, `"ip"` or
/// `"ip:port"`, runs it behind a NAT router that port-forwards that public
/// address to it (Annex J.7.5): the BDT must then list that address, and
/// every row must be two-hop, or `ValueError` is raised (an empty BDT, a
/// BBMD serving foreign devices only, is fine). `nat_private_ranges` lists
/// up to four `"a.b.c.d/n"` ranges to count as private besides RFC 1918;
/// `"0.0.0.0/0"` counts every foreign device as private, so all their
/// broadcasts go on from the global address.
#[pyclass(name = "Bbmd", module = "rusty_bacnet", frozen, skip_from_py_object)]
pub(crate) struct PyBbmd {
    options: BbmdOptions,
}

#[pymethods]
impl PyBbmd {
    /// Check and keep the BBMD's settings.
    #[new]
    #[pyo3(signature = (bdt=None, *, accept_foreign_devices=false, management_acl=None, bdt_persist_path=None, alone_on_subnet=false, nat_global_address=None, nat_private_ranges=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        bdt: Option<Vec<Bound<'_, PyAny>>>,
        accept_foreign_devices: bool,
        management_acl: Option<Vec<String>>,
        bdt_persist_path: Option<PathBuf>,
        alone_on_subnet: bool,
        nat_global_address: Option<String>,
        nat_private_ranges: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let bdt = bdt
            .unwrap_or_default()
            .iter()
            .map(bdt_entry)
            .collect::<PyResult<Vec<_>>>()?;
        let management_acl = management_acl
            .unwrap_or_default()
            .iter()
            .map(|ip| parse_ip(ip, "management_acl").map(|ip| ip.octets()))
            .collect::<PyResult<Vec<_>>>()?;
        let nat = nat_options(nat_global_address, nat_private_ranges)?;
        if let Some(nat) = &nat {
            check_nat_bdt(&bdt, nat.global_address)?;
        }
        Ok(Self {
            options: BbmdOptions {
                bdt,
                foreign_device_policy: accept_foreign_devices.then(ForeignDevicePolicy::default),
                management_acl,
                bdt_persist_path,
                alone_on_subnet,
                nat,
                ..BbmdOptions::default()
            },
        })
    }

    /// The BDT rows given, as `("ip:port", "mask")`.
    #[getter]
    fn bdt(&self) -> Vec<(String, String)> {
        self.options.bdt.iter().map(bdt_row).collect()
    }

    /// Whether foreign devices may register.
    #[getter]
    fn accept_foreign_devices(&self) -> bool {
        self.options.foreign_device_policy.is_some()
    }

    /// The addresses allowed to delete Foreign Device Table entries.
    #[getter]
    fn management_acl(&self) -> Vec<String> {
        let acl = &self.options.management_acl;
        acl.iter()
            .map(|ip| Ipv4Addr::from(*ip).to_string())
            .collect()
    }

    /// The persisted-BDT file, if any.
    #[getter]
    fn bdt_persist_path(&self) -> Option<PathBuf> {
        self.options.bdt_persist_path.clone()
    }

    /// Whether this BBMD is the only device of its B/IP network on its
    /// subnet.
    #[getter]
    fn alone_on_subnet(&self) -> bool {
        self.options.alone_on_subnet
    }

    /// The NAT router's public `"ip:port"` this BBMD runs behind, if any.
    #[getter]
    fn nat_global_address(&self) -> Option<String> {
        self.options
            .nat
            .as_ref()
            .map(|nat| nat.global_address.to_string())
    }

    /// The ranges counted as private besides RFC 1918, as `"a.b.c.d/n"`.
    #[getter]
    fn nat_private_ranges(&self) -> Vec<String> {
        self.options.nat.as_ref().map_or_else(Vec::new, |nat| {
            nat.private_ranges.iter().map(ToString::to_string).collect()
        })
    }

    fn __repr__(&self) -> String {
        let python = |flag: bool| if flag { "True" } else { "False" };
        let nat = self
            .nat_global_address()
            .map_or_else(|| "None".to_string(), |global| format!("'{global}'"));
        format!(
            "Bbmd(bdt={} rows, accept_foreign_devices={}, alone_on_subnet={}, nat_global_address={nat})",
            self.options.bdt.len(),
            python(self.accept_foreign_devices()),
            python(self.options.alone_on_subnet),
        )
    }
}

/// A B/IP multicast group, for the `multicast=` keyword (Annex J.8, #941).
///
/// `group` is a class D IPv4 address outside 224.0.0.0/24, the block
/// reserved for local network control. The node's broadcasts then go to the
/// group, and it takes broadcasts only from the group, never a subnet
/// broadcast. `ttl`, 1 to 255 (default 1), is the multicast TTL: 1 keeps
/// the group's traffic on the local network, so raise it for a group that
/// spans IP routers. `port` is the group's UDP port, by default the node's
/// own; members on one host at one address need their own ports and so a
/// shared group port. Multicast needs an explicit `interface` (not
/// "0.0.0.0") and can't be combined with `foreign_device=` or
/// `share_port_by_address=True`; with `bbmd=` the owner is the BBMD that
/// serves the group (Annex J.8.3). A bad value raises ValueError, or
/// OverflowError for an integer out of range.
#[pyclass(
    name = "Multicast",
    module = "rusty_bacnet",
    frozen,
    eq,
    hash,
    skip_from_py_object
)]
#[derive(PartialEq, Eq, Hash)]
pub(crate) struct PyMulticast {
    multicast: BipMulticast,
}

#[pymethods]
impl PyMulticast {
    /// Check and keep the group.
    #[new]
    #[pyo3(signature = (group, *, ttl=1, port=None))]
    fn new(group: &str, ttl: u8, port: Option<u16>) -> PyResult<Self> {
        let ip = parse_ip(group, "group")?;
        let invalid = |e: bacnet_types::error::Error| PyValueError::new_err(e.to_string());
        let mut multicast = BipMulticast::new(ip).map_err(invalid)?;
        multicast = multicast.with_ttl(ttl).map_err(invalid)?;
        if let Some(port) = port {
            multicast = multicast.with_port(port).map_err(invalid)?;
        }
        Ok(Self { multicast })
    }

    /// The group address.
    #[getter]
    fn group(&self) -> String {
        self.multicast.group().to_string()
    }

    /// The multicast TTL.
    #[getter]
    fn ttl(&self) -> u8 {
        self.multicast.ttl()
    }

    /// The group's UDP port, or None for the node's own.
    #[getter]
    fn port(&self) -> Option<u16> {
        self.multicast.port()
    }

    fn __repr__(&self) -> String {
        let port = self
            .port()
            .map_or_else(|| "None".to_string(), |p| p.to_string());
        format!(
            "Multicast(group='{}', ttl={}, port={port})",
            self.group(),
            self.ttl()
        )
    }

    /// Copy and pickle as a call to the constructor with the same settings,
    /// keywords included, so a copy is equal and checked again.
    fn __reduce__<'py>(slf: &Bound<'py, Self>) -> PyResult<(Bound<'py, PyAny>, ())> {
        let py = slf.py();
        let this = slf.get();
        let kwargs = PyDict::new(py);
        kwargs.set_item("ttl", this.ttl())?;
        kwargs.set_item("port", this.port())?;
        let rebuild = py
            .import("functools")?
            .getattr("partial")?
            .call((slf.get_type(), this.group()), Some(&kwargs))?;
        Ok((rebuild, ()))
    }
}

/// The group the `multicast=` keyword asks for: only on B/IP, with an
/// explicit interface, and not for a foreign device. A BBMD with a group
/// serves it (Annex J.8.3).
pub(crate) fn bip_multicast(
    transport: &str,
    interface: &str,
    port: u16,
    share_port_by_address: bool,
    mode: &BipMode,
    multicast: Option<PyRef<'_, PyMulticast>>,
) -> PyResult<Option<BipMulticast>> {
    let Some(multicast) = multicast else {
        return Ok(None);
    };
    if transport != "bip" {
        return Err(PyValueError::new_err(
            "multicast applies only to transport='bip'",
        ));
    }
    if matches!(mode, BipMode::Foreign(_)) {
        return Err(PyValueError::new_err(
            "multicast can't be combined with foreign_device: a foreign device broadcasts \
             through its BBMD",
        ));
    }
    if parse_ip(interface, "interface")?.is_unspecified() {
        return Err(PyValueError::new_err(
            "multicast needs an explicit interface address, not 0.0.0.0",
        ));
    }
    if share_port_by_address {
        return Err(PyValueError::new_err(
            "multicast can't be combined with share_port_by_address; give each owner on \
             the host its own port and the group a shared one (Multicast(port=...))",
        ));
    }
    if port == 0 && multicast.multicast.port().is_none() {
        return Err(PyValueError::new_err(
            "multicast with port=0 needs the group's own port: Multicast(group, port=...)",
        ));
    }
    Ok(Some(multicast.multicast))
}

/// The modes the `bbmd=` and `foreign_device=` keywords ask for.
pub(crate) struct IpModes {
    /// The B/IP mode: normal unless the transport is B/IP.
    pub(crate) bip: BipMode,
    /// A B/IPv6 foreign-device registration (#1941).
    pub(crate) bip6_foreign: Option<Bip6ForeignDeviceConfig>,
}

/// The modes the `bbmd=` and `foreign_device=` keywords ask for: one of
/// them at most; `bbmd` only on B/IP, and `foreign_device` on B/IP or
/// B/IPv6 with a BBMD of the transport's family.
pub(crate) fn ip_modes(
    transport: &str,
    bbmd: Option<PyRef<'_, PyBbmd>>,
    foreign_device: Option<PyRef<'_, PyForeignDevice>>,
) -> PyResult<IpModes> {
    if bbmd.is_some() && transport != "bip" {
        return Err(PyValueError::new_err(
            "bbmd applies only to transport='bip'",
        ));
    }
    if foreign_device.is_some() && !matches!(transport, "bip" | "ipv6") {
        return Err(PyValueError::new_err(
            "foreign_device applies only to transport='bip' or 'ipv6'",
        ));
    }
    let bip = match (bbmd, foreign_device) {
        (Some(_), Some(_)) => {
            return Err(PyValueError::new_err(
                "bbmd and foreign_device are mutually exclusive: a B/IP node can't be both",
            ));
        }
        (Some(bbmd), None) => BipMode::Bbmd(bbmd.options.clone()),
        (None, Some(foreign)) => match (&foreign.registration, transport) {
            (Registration::Bip(config), "bip") => BipMode::Foreign(config.clone()),
            (Registration::Bip6(config), "ipv6") => {
                return Ok(IpModes {
                    bip: BipMode::Normal,
                    bip6_foreign: Some(config.clone()),
                });
            }
            (Registration::Bip(_), _) => {
                return Err(PyValueError::new_err(format!(
                    "transport='ipv6' needs a B/IPv6 BBMD ('ip' or '[ip]:port'), not {}",
                    foreign.bbmd()
                )));
            }
            (Registration::Bip6(_), _) => {
                return Err(PyValueError::new_err(format!(
                    "transport='bip' needs an IPv4 BBMD ('ip' or 'ip:port'), not {}",
                    foreign.bbmd()
                )));
            }
        },
        (None, None) => BipMode::Normal,
    };
    Ok(IpModes {
        bip,
        bip6_foreign: None,
    })
}

/// The B/IP mode for a B/IP-only owner (an endpoint or a router port): see
/// [`ip_modes`].
pub(crate) fn bip_mode(
    transport: &str,
    bbmd: Option<PyRef<'_, PyBbmd>>,
    foreign_device: Option<PyRef<'_, PyForeignDevice>>,
) -> PyResult<BipMode> {
    ip_modes(transport, bbmd, foreign_device).map(|modes| modes.bip)
}

/// `registration` as the dict `foreign_registration()` returns.
pub(crate) fn registration_dict(registration: ForeignRegistrationSnapshot) -> PyResult<Py<PyAny>> {
    let seconds = |d: Option<Duration>| d.map(|d| d.as_secs_f64());
    let outcome = registration.last_outcome.map(|outcome| match outcome {
        ForeignRegistrationOutcome::Pending => "pending",
        ForeignRegistrationOutcome::Accepted => "accepted",
        ForeignRegistrationOutcome::Refused => "refused",
        ForeignRegistrationOutcome::TimedOut => "timed_out",
        ForeignRegistrationOutcome::LocalError => "local_error",
        ForeignRegistrationOutcome::Cancelled => "cancelled",
    });
    crate::py_async::attach(|py| {
        let dict = PyDict::new(py);
        dict.set_item("registered", registration.registered)?;
        dict.set_item("lease_remaining", seconds(registration.lease_remaining))?;
        dict.set_item("next_attempt_in", seconds(registration.next_attempt_in))?;
        dict.set_item("last_bbmd", registration.last_bbmd.map(|a| a.to_string()))?;
        dict.set_item("last_result", registration.last_result.map(|c| c.to_raw()))?;
        dict.set_item("last_outcome", outcome)?;
        dict.set_item("attempts", registration.attempts)?;
        dict.set_item("accepted", registration.accepted)?;
        dict.set_item("refused", registration.refused)?;
        dict.set_item("lapses", registration.lapses)?;
        dict.set_item("renewal_hints", registration.renewal_hints)?;
        Ok(dict.into_any().unbind())
    })
}

/// The NAT setting the `nat_global_address` and `nat_private_ranges`
/// keywords ask for: none without a global address, and ranges only with
/// one.
fn nat_options(
    global: Option<String>,
    ranges: Option<Vec<String>>,
) -> PyResult<Option<NatTraversal>> {
    let Some(global) = global else {
        if ranges.is_some_and(|ranges| !ranges.is_empty()) {
            return Err(PyValueError::new_err(
                "nat_private_ranges needs nat_global_address",
            ));
        }
        return Ok(None);
    };
    let address = parse_address(&global, "nat_global_address")?;
    if address.ip().is_loopback() {
        return Err(PyValueError::new_err(format!(
            "nat_global_address must be a non-loopback address, got '{global}'"
        )));
    }
    let mut nat = NatTraversal::new(address);
    for range in ranges.unwrap_or_default() {
        nat.private_ranges.push(parse_range(&range)?);
    }
    if nat.private_ranges.len() > NatTraversal::MAX_PRIVATE_RANGES {
        return Err(PyValueError::new_err(format!(
            "nat_private_ranges holds at most {} ranges",
            NatTraversal::MAX_PRIVATE_RANGES
        )));
    }
    Ok(Some(nat))
}

/// Behind NAT, a BDT with rows must list the global address, and every row
/// must be two-hop (Annex J.7.5); an empty one serves foreign devices only.
fn check_nat_bdt(bdt: &[BdtEntry], global: SocketAddrV4) -> PyResult<()> {
    if bdt.iter().any(|row| row.broadcast_mask != [255; 4]) {
        return Err(PyValueError::new_err(
            "behind NAT every bdt row must be two-hop ('ip:port', mask 255.255.255.255)",
        ));
    }
    let lists_global = bdt
        .iter()
        .any(|row| SocketAddrV4::new(Ipv4Addr::from(row.ip), row.port) == global);
    if !bdt.is_empty() && !lists_global {
        return Err(PyValueError::new_err(format!(
            "behind NAT the bdt must list nat_global_address {global} as this BBMD's own row"
        )));
    }
    Ok(())
}

/// `"a.b.c.d/n"`, a prefix of at most 32 bits.
fn parse_range(text: &str) -> PyResult<PrivateRange> {
    let refused = || {
        PyValueError::new_err(format!(
            "nat_private_ranges entries are 'a.b.c.d/n', got '{text}'"
        ))
    };
    let (network, prefix) = text.split_once('/').ok_or_else(refused)?;
    let network = network.parse::<Ipv4Addr>().map_err(|_| refused())?;
    let prefix = prefix.parse::<u8>().map_err(|_| refused())?;
    PrivateRange::new(network, prefix).map_err(|_| refused())
}

/// A BDT row as Python reads it: `("ip:port", "mask")`.
pub(crate) fn bdt_row(row: &BdtEntry) -> (String, String) {
    let address = SocketAddrV4::new(Ipv4Addr::from(row.ip), row.port);
    (
        address.to_string(),
        Ipv4Addr::from(row.broadcast_mask).to_string(),
    )
}

/// A BDT row: `"ip[:port]"`, its broadcasts forwarded by unicast, or
/// `("ip[:port]", "mask")`.
pub(crate) fn bdt_entry(row: &Bound<'_, PyAny>) -> PyResult<BdtEntry> {
    let (address, mask) = match row.extract::<String>() {
        Ok(address) => (address, Ipv4Addr::BROADCAST),
        Err(_) => {
            let (address, mask) = row.extract::<(String, String)>().map_err(|_| {
                PyValueError::new_err("each bdt row is 'ip:port' or ('ip:port', 'mask')")
            })?;
            (address, parse_ip(&mask, "bdt mask")?)
        }
    };
    let address = parse_address(&address, "bdt")?;
    Ok(BdtEntry {
        ip: address.ip().octets(),
        port: address.port(),
        broadcast_mask: mask.octets(),
    })
}

/// `"ip"` or `"ip:port"`, a unicast IPv4 address with a nonzero port.
pub(crate) fn parse_address(text: &str, what: &str) -> PyResult<SocketAddrV4> {
    let address = match text.parse::<SocketAddrV4>() {
        Ok(address) => address,
        Err(_) => SocketAddrV4::new(parse_ip(text, what)?, DEFAULT_BACNET_PORT),
    };
    let ip = address.ip();
    if address.port() == 0 || ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() {
        return Err(PyValueError::new_err(format!(
            "{what} must be a unicast IPv4 address with a nonzero port, got '{text}'"
        )));
    }
    Ok(address)
}

pub(crate) fn parse_ip(text: &str, what: &str) -> PyResult<Ipv4Addr> {
    text.parse()
        .map_err(|_| PyValueError::new_err(format!("{what}: '{text}' is not an IPv4 address")))
}
