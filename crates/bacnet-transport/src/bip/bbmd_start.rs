//! BBMD set-up in `start()`: the starting BDT and this BBMD's own B/IP
//! address (#937).
//!
//! The own address is the originating address of the BBMD's own forwarded
//! broadcasts, the BDT row it never forwards to, and the source its receive
//! loop drops as its own echo. Bound to one interface, it is that interface's
//! address and the bound port. Bound to `0.0.0.0`, the transport cannot tell
//! which local address its peers know it by, so it reads it from the BDT it
//! runs with:
//!
//! - the one BDT row whose IP is a local IPv4 address and whose port is the
//!   bound port;
//! - with several such rows, `start()` fails and asks for an explicit
//!   interface;
//! - with none, the address the host uses toward its default route, but only
//!   when that is a local, non-loopback address; otherwise `start()` fails.
//!
//! The local IPv4 addresses are the ones `local_addresses` lists, on every OS;
//! when they cannot be listed or none is usable, a wildcard `start()` fails
//! before it gets here.
//!
//! The BDT is the persisted one when it loads, else the configured one; a
//! persisted NAT setting (#940) comes with it, the same way. A
//! persisted BDT that loads is authoritative: when no own address can be
//! chosen from it, `start()` fails rather than retrying with the configured
//! BDT. Only a self row that would overflow the persisted BDT still falls back
//! to the configured BDT, with a warning.
//!
//! Every start of a wildcard-bound BBMD repeats the choice, so a restart
//! follows address changes.

use std::fmt;
use std::net::Ipv4Addr;
use std::path::Path;

use tracing::{debug, warn};

use crate::bbmd::{self, BbmdState, BdtEntry, ForeignDevicePolicy, NatTraversal, PersistedNat};
use bacnet_types::error::Error;

/// Pre-start configuration for BBMD mode.
pub(super) struct BbmdConfig {
    pub(super) initial_bdt: Vec<BdtEntry>,
    pub(super) management_acl: Vec<[u8; 4]>,
    pub(super) foreign_device_policy: Option<ForeignDevicePolicy>,
    pub(super) alone_on_subnet: bool,
    pub(super) nat: Option<NatTraversal>,
}

/// The BDT a wildcard-bound BBMD reads its own address from, for messages.
#[derive(Clone, Copy)]
pub(super) enum BdtSource<'a> {
    /// The BDT given to `enable_bbmd`.
    Configured,
    /// The BDT loaded from `set_bdt_persist_path`.
    Persisted(&'a Path),
    /// The BDT of an earlier start, on a restart.
    Current,
}

impl fmt::Display for BdtSource<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configured => f.write_str("the configured BDT"),
            Self::Persisted(path) => write!(f, "the persisted BDT {}", path.display()),
            Self::Current => f.write_str("the BDT"),
        }
    }
}

/// What `start()` knows about the local addresses once the socket is bound.
pub(super) struct OwnAddressContext<'a> {
    /// The configured interface; `0.0.0.0` for a wildcard bind.
    pub(super) interface: Ipv4Addr,
    /// The bound UDP port.
    pub(super) port: u16,
    /// The host's IPv4 addresses (wildcard bind only).
    pub(super) host: &'a [Ipv4Addr],
    /// The local address toward the default route, if any (wildcard bind only).
    pub(super) route_ip: Option<Ipv4Addr>,
}

impl OwnAddressContext<'_> {
    fn own_ip(
        &self,
        rows: &[BdtEntry],
        source: BdtSource<'_>,
        behind_nat: bool,
    ) -> Result<Ipv4Addr, Error> {
        if self.interface.is_unspecified() {
            select_wildcard_bbmd_ip(
                rows,
                self.port,
                self.host,
                self.route_ip,
                source,
                behind_nat,
            )
        } else {
            Ok(self.interface)
        }
    }
}

fn own_address_error(message: String) -> Error {
    Error::Transport(std::io::Error::new(
        std::io::ErrorKind::AddrNotAvailable,
        message,
    ))
}

fn route_text(route_ip: Option<Ipv4Addr>) -> String {
    route_ip.map_or_else(|| "none".to_owned(), |ip| ip.to_string())
}

/// The own IP of a BBMD bound to `0.0.0.0` at `port` (rules in the module
/// docs). `rows` must already be canonical. Only rows at `port` are checked
/// against `host`. Behind a NAT router (`behind_nat`) the rows are global
/// addresses, so only the default route can name it.
pub(super) fn select_wildcard_bbmd_ip(
    rows: &[BdtEntry],
    port: u16,
    host: &[Ipv4Addr],
    route_ip: Option<Ipv4Addr>,
    source: BdtSource<'_>,
    behind_nat: bool,
) -> Result<Ipv4Addr, Error> {
    let mut own: Vec<Ipv4Addr> = rows
        .iter()
        .filter(|row| row.port == port)
        .map(|row| Ipv4Addr::from(row.ip))
        .filter(|ip| host.contains(ip))
        .collect();
    own.sort_unstable();
    own.dedup();
    match own.as_slice() {
        [ip] => Ok(*ip),
        [] => route_ip
            .filter(|ip| !ip.is_loopback() && host.contains(ip))
            .ok_or_else(|| {
                if behind_nat {
                    return own_address_error(format!(
                        "BBMD behind a NAT router and bound to 0.0.0.0 cannot determine its \
                         own local B/IP address: its BDT lists global addresses, and the \
                         default-route address ({}) is not a local non-loopback address; \
                         bind an explicit interface address",
                        route_text(route_ip)
                    ));
                }
                own_address_error(format!(
                    "BBMD bound to 0.0.0.0 cannot determine its own B/IP address: {source} \
                     has no row for a local IPv4 address at port {port}, and the \
                     default-route address ({}) is not a local non-loopback address; \
                     bind an explicit interface address or add this BBMD's own row to the BDT",
                    route_text(route_ip)
                ))
            }),
        several => {
            let rows: Vec<String> = several.iter().map(|ip| format!("{ip}:{port}")).collect();
            Err(own_address_error(format!(
                "BBMD bound to 0.0.0.0 cannot choose its own B/IP address: {source} has \
                 rows for several local IPv4 addresses at port {port} ({}); bind an \
                 explicit interface address",
                rows.join(", ")
            )))
        }
    }
}

/// Warn, and return `true`, when a BBMD bound to `0.0.0.0` with own address
/// `own_ip` broadcasts to 255.255.255.255 and `own_ip` is not the
/// default-route address. The kernel may then send each broadcast from
/// another interface, so its echo comes back from an address that is not the
/// BBMD's own and is forwarded again.
pub(super) fn warn_if_broadcast_may_leave_another_interface(
    ctx: &OwnAddressContext<'_>,
    own_ip: Ipv4Addr,
    broadcast: Ipv4Addr,
) -> bool {
    let risky = ctx.interface.is_unspecified()
        && broadcast == Ipv4Addr::BROADCAST
        && ctx.route_ip != Some(own_ip);
    if risky {
        warn!(
            %own_ip,
            route_ip = %route_text(ctx.route_ip),
            "BBMD bound to 0.0.0.0 broadcasts to 255.255.255.255, but its own B/IP address \
             is not the default-route address: the kernel may send these broadcasts from \
             another interface, so their echo is not recognised as the BBMD's own and can \
             be forwarded again; bind an explicit interface address and use that subnet's \
             broadcast address"
        );
    }
    risky
}

/// The mask of the subnet `ip`, this BBMD's own address, is on, as the
/// host reports it; `None`, logged at debug, when it can't say. A peer's
/// Forwarded-NPDU sent from that subnet at the BBMD's own port isn't
/// broadcast on it again (Annex J.4.5). An all-zero mask would put every sender on the subnet, so it
/// counts as unknown. Listing adapters can be slow, so it runs on a
/// blocking thread.
pub(super) async fn subnet_mask_of(ip: Ipv4Addr) -> Option<Ipv4Addr> {
    let lookup = tokio::task::spawn_blocking(move || crate::local_addresses::interface_of(ip));
    let mask = match lookup.await {
        Ok(Ok(found)) => found.and_then(|local| local.netmask),
        Ok(Err(error)) => {
            debug!(%ip, %error, "Could not look up the BBMD's subnet");
            None
        }
        Err(error) => {
            debug!(%ip, %error, "Could not look up the BBMD's subnet");
            None
        }
    };
    let mask = mask.filter(|mask| !mask.is_unspecified());
    if mask.is_none() {
        debug!(%ip, "BBMD subnet mask unknown: a forward from its own subnet is rebroadcast");
    }
    mask
}

/// Read and validate the persisted BDT, and the NAT setting saved with it
/// when the file has one. `None`, after a warning when the file is
/// unusable, means the configured BDT is used.
fn load_persisted_bdt(path: &Path) -> Option<(Vec<BdtEntry>, Option<PersistedNat>)> {
    let data = std::fs::read(path).ok()?;
    let (entries, nat) = match bbmd::decode_file(&data) {
        Ok(decoded) => decoded,
        Err(e) => {
            warn!(error = %e, "Failed to decode persisted BDT, using config");
            return None;
        }
    };
    match bbmd::canonical_bdt(entries) {
        Ok(rows) => Some((rows, nat)),
        Err(e) => {
            warn!(error = %e, "Persisted BDT invalid, using config");
            None
        }
    }
}

/// An empty BBMD state at `ip:port`, behind NAT when `config` says so, and
/// serving a multicast group when `group` says so, so the BDT set next is
/// checked under NAT's rules (Annex J.7.5) and the group's (J.8.3).
fn new_state(
    config: &BbmdConfig,
    group: bool,
    ip: Ipv4Addr,
    port: u16,
) -> Result<BbmdState, Error> {
    let mut state = BbmdState::new(ip.octets(), port);
    state.set_nat_traversal(config.nat.clone())?;
    state.set_serves_group(group)?;
    Ok(state)
}

/// An empty BBMD state at `ip:port` with the NAT setting a persisted file
/// saved, which an activation made, so it wins over the configured one, as
/// the persisted BDT does; the private ranges are the configuration's.
fn persisted_state(
    config: &BbmdConfig,
    group: bool,
    nat: PersistedNat,
    ip: Ipv4Addr,
    port: u16,
) -> Result<BbmdState, Error> {
    let ranges = config
        .nat
        .as_ref()
        .map(|nat| nat.private_ranges.clone())
        .unwrap_or_default();
    let mut state = BbmdState::new(ip.octets(), port);
    state.set_persisted_nat(nat, ranges)?;
    state.set_serves_group(group)?;
    Ok(state)
}

/// Warn when the persisted NAT setting now in effect isn't the configured
/// one, so a changed builder setting that a saved one overrides is visible.
fn warn_if_overridden(config: &BbmdConfig, nat: PersistedNat) {
    let configured = PersistedNat {
        enabled: config.nat.is_some(),
        global: config.nat.as_ref().map(|nat| nat.global_address),
    };
    if nat != configured {
        warn!(
            persisted_nat = nat.enabled,
            persisted_global = ?nat.global,
            configured_nat = configured.enabled,
            configured_global = ?configured.global,
            "Using the persisted NAT setting, not the configured one; \
             delete the persisted-BDT file to apply the configuration"
        );
    }
}

/// The BBMD state for a first start: the persisted BDT when it loads, else the
/// configured one, and this BBMD's own address chosen with that BDT. An
/// invalid configured BDT or an undeterminable own address fails the start.
/// An own address that cannot be chosen from a loaded persisted BDT fails it
/// too, without trying the configured BDT. A BBMD that serves a B/IP-M
/// group (`group`) needs a two-hop own row (Annex J.8.3): a persisted BDT
/// without one falls back to the configured BDT, with a warning, and a
/// configured one without one fails the start.
pub(super) fn initial_bbmd_state(
    config: &BbmdConfig,
    group: bool,
    persist_path: Option<&Path>,
    ctx: &OwnAddressContext<'_>,
) -> Result<BbmdState, Error> {
    let mut loaded = None;
    let persisted = persist_path.and_then(|path| Some((path, load_persisted_bdt(path)?)));
    if let Some((path, (rows, nat))) = persisted {
        let behind_nat = nat.map_or(config.nat.is_some(), |nat| nat.enabled);
        let ip = ctx.own_ip(&rows, BdtSource::Persisted(path), behind_nat)?;
        let state = match nat {
            Some(nat) => persisted_state(config, group, nat, ip, ctx.port),
            None => new_state(config, group, ip, ctx.port),
        };
        // The rows are canonical, so only the self row can still overflow,
        // or, behind NAT, the NAT rules refuse them, or the group's rule a
        // one-hop own row.
        match state.and_then(|mut state| state.set_bdt(rows).map(|()| state)) {
            Ok(state) => {
                if let Some(nat) = nat {
                    warn_if_overridden(config, nat);
                }
                debug!(
                    path = %path.display(),
                    entries = state.bdt().len(),
                    nat = state.nat_traversal(),
                    "Loaded persisted BDT"
                );
                loaded = Some(state);
            }
            Err(e) => warn!(error = %e, "Persisted BDT invalid, using config"),
        }
    }
    let mut state = match loaded {
        Some(state) => state,
        None => {
            let config_error = |e: Error| Error::Encoding(format!("BDT configuration error: {e}"));
            let rows = bbmd::canonical_bdt(config.initial_bdt.clone()).map_err(config_error)?;
            let ip = ctx.own_ip(&rows, BdtSource::Configured, config.nat.is_some())?;
            let mut state = new_state(config, group, ip, ctx.port)?;
            state.set_bdt(rows).map_err(config_error)?;
            state
        }
    };
    state.set_management_acl(config.management_acl.clone());
    state.set_foreign_device_policy(config.foreign_device_policy.clone());
    state.set_alone_on_subnet(config.alone_on_subnet);
    Ok(state)
}

/// Warn about each BDT peer at this BBMD's own port whose IP is on its own
/// IP subnet (by `mask`), when the BBMD serves a B/IP-M group. B/IP-M and
/// plain devices don't share a subnet (Annex J.8.1): such a peer's
/// forwards still reach the group, but it takes this BBMD's forwards as
/// heard on its subnet already (J.4.5), so its own subnet never gets them.
/// Returns how many it warned about.
pub(super) fn warn_about_peers_on_the_group_subnet(
    state: &BbmdState,
    mask: Option<Ipv4Addr>,
) -> usize {
    let Some(mask) = mask.filter(|_| state.serves_group()) else {
        return 0;
    };
    let own = state.local_address();
    let on_subnet = |row: &BdtEntry| {
        let ip = Ipv4Addr::from(row.ip);
        row.port == own.port()
            && ip != *own.ip()
            && (ip.to_bits() ^ own.ip().to_bits()) & mask.to_bits() == 0
    };
    let mut warned = 0;
    for row in state.bdt().iter().filter(|row| on_subnet(row)) {
        warn!(
            peer = %Ipv4Addr::from(row.ip),
            port = row.port,
            "A BDT peer shares this multicast BBMD's IP subnet: B/IP-M and plain B/IP \
             devices must not share a subnet, and that peer won't pass this BBMD's \
             broadcasts on to its own subnet"
        );
        warned += 1;
    }
    warned
}

/// Choose the own address of a wildcard-bound BBMD again on a restart, from
/// the BDT without the self row it appended. The BDT and FDT are kept; an
/// appended self row follows the new address. On error the state is unchanged.
pub(super) fn refresh_own_address(
    state: &mut BbmdState,
    ctx: &OwnAddressContext<'_>,
) -> Result<(), Error> {
    let ip = ctx.own_ip(
        &state.configured_bdt(),
        BdtSource::Current,
        state.nat_traversal(),
    )?;
    let old = state.local_address();
    let (old_ip, old_port) = (old.ip().octets(), old.port());
    state.set_local_address(ip.octets(), ctx.port).map_err(|e| {
        Error::Encoding(format!(
            "BBMD restart moved its own B/IP address from {}:{old_port} to {ip}:{}, and a \
             self row for the new address would exceed the BDT limit of {} entries: {e}",
            Ipv4Addr::from(old_ip),
            ctx.port,
            BbmdState::MAX_BDT_ENTRIES
        ))
    })
}
