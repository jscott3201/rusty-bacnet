//! The B/IP mode a node runs in, as one choice (#938): a plain node, a
//! foreign device or a BBMD. The client, server and endpoint builders take
//! it, so a BBMD that also registers as a foreign device can't be built.

use std::path::PathBuf;

use super::{BipTransport, FanoutPolicy, ForeignDeviceConfig};
use crate::bbmd::{BdtEntry, ForeignDevicePolicy, NatTraversal};

/// How a B/IP node takes part in its network (Annex J).
///
/// ```
/// use std::net::Ipv4Addr;
/// use std::time::Duration;
/// use bacnet_transport::bbmd::ForeignDevicePolicy;
/// use bacnet_transport::bip::{BbmdOptions, BipMode, ForeignDeviceConfig};
///
/// // A foreign device behind NAT: a long lease, renewed every 20 s so the
/// // NAT mapping the BBMD's replies come back on stays open.
/// let foreign = BipMode::Foreign(ForeignDeviceConfig {
///     bbmd_ip: Ipv4Addr::new(192, 0, 2, 10),
///     bbmd_port: 47808,
///     ttl: 600,
///     renewal_interval: Some(Duration::from_secs(20)),
/// });
/// // A BBMD that takes foreign-device registrations.
/// let bbmd = BipMode::Bbmd(BbmdOptions {
///     foreign_device_policy: Some(ForeignDevicePolicy::default()),
///     ..BbmdOptions::default()
/// });
/// # let _ = (foreign, bbmd);
/// ```
// A mode is built once, when a transport is configured, so the BBMD
// variant's size costs nothing worth a box.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Default)]
pub enum BipMode {
    /// A node whose broadcasts reach its own subnet only. The default.
    #[default]
    Normal,
    /// A foreign device: it registers with a BBMD on another subnet, renews
    /// the registration, and broadcasts through that BBMD. How the
    /// registration goes, a refusal included, shows in the
    /// [`ForeignRegistrationSnapshot`](super::ForeignRegistrationSnapshot)
    /// the client, server and endpoint report.
    Foreign(ForeignDeviceConfig),
    /// A BBMD: it passes broadcasts between its subnet, the BBMDs in its
    /// BDT and the foreign devices registered with it.
    Bbmd(BbmdOptions),
}

/// A BBMD's set-up: what [`BipTransport::enable_bbmd`] and its BBMD setters
/// take, in one value. The default is a BBMD with only its own BDT row,
/// refusing foreign-device registrations and Delete-FDT-Entry requests.
#[derive(Debug, Clone, Default)]
pub struct BbmdOptions {
    /// The BDT it starts with; its own row is added when missing. See
    /// [`BipTransport::enable_bbmd`] for how a `0.0.0.0` BBMD finds its own
    /// address in it.
    pub bdt: Vec<BdtEntry>,
    /// The terms on which it accepts foreign-device registrations. `None`
    /// refuses every one.
    pub foreign_device_policy: Option<ForeignDevicePolicy>,
    /// The source addresses allowed to delete Foreign Device Table entries.
    /// Empty denies every source.
    pub management_acl: Vec<[u8; 4]>,
    /// A file holding a BDT to start with instead of `bdt`, when it loads,
    /// and where an activated BDT is saved
    /// ([`BipTransport::set_bdt_persist_path`]).
    pub bdt_persist_path: Option<PathBuf>,
    /// The forwarding queue and rate limits.
    pub fanout_policy: FanoutPolicy,
    /// Whether it is the only device of its B/IP network on its subnet, as
    /// a BBMD behind a NAT router often is: it then skips the local
    /// rebroadcast of forwarded and distributed broadcasts. Off by default;
    /// see [`BipTransport::set_bbmd_alone_on_subnet`].
    pub alone_on_subnet: bool,
    /// How it is reached through a NAT router, when it sits behind one
    /// (Annex J.7.5): its global address and any extra private ranges. The
    /// BDT must then list the global address as this BBMD's own row, with
    /// every mask all ones, or `start()` fails. `None` by default; see
    /// [`BipTransport::set_bbmd_nat_traversal`].
    pub nat: Option<NatTraversal>,
}

impl BipTransport {
    /// Run this BBMD behind a NAT router, reached at `nat`'s global address,
    /// or, with `None`, not (Annex J.7.5). The router must port-forward the
    /// global address's UDP port to this transport's socket. The BDT then
    /// names this BBMD by the global address: it must list it, and every row
    /// must have an all-ones mask (two-hop), or `start()` fails. The
    /// Forwarded-NPDUs the BBMD builds carry the global address as their
    /// origin, except a public foreign device's broadcast, which keeps its
    /// own; one it passes on keeps the origin it came with. The socket, its
    /// subnet and its echo filter stay on the private side. The registered
    /// Network Port serves the setting as BACnet_IP_NAT_Traversal and
    /// BACnet_IP_Global_Address, which a ReinitializeDevice can change.
    /// Must be called after `enable_bbmd()` and before `start()`.
    pub fn set_bbmd_nat_traversal(&mut self, nat: Option<NatTraversal>) {
        if let Some(config) = &mut self.bbmd_config {
            config.nat = nat;
        } else if nat.is_some() {
            tracing::warn!(
                "set_bbmd_nat_traversal called before enable_bbmd(); setting will be ignored"
            );
        }
    }

    /// The persisted-BDT file's path, if one is set.
    #[cfg(test)]
    pub(super) fn bdt_persist_path(&self) -> Option<&std::path::Path> {
        self.bdt_file.as_deref().map(crate::bbmd::BdtFile::path)
    }

    /// How many saves of an activated BDT to the persisted-BDT file have
    /// failed ([`set_bdt_persist_path`](Self::set_bdt_persist_path)).
    pub fn bdt_persist_failures(&self) -> u64 {
        self.bdt_file.as_ref().map_or(0, |file| file.failures())
    }

    /// Set up this transport's B/IP mode, replacing the one set up before,
    /// with the setters below: [`enable_bbmd`](Self::enable_bbmd) and the
    /// BBMD setters for [`BipMode::Bbmd`], and
    /// [`register_as_foreign_device`](Self::register_as_foreign_device) for
    /// [`BipMode::Foreign`]. Everything a BBMD set up before goes back to
    /// its default, its fanout policy included. Call it before the first
    /// `start()`: a transport that has run as a BBMD keeps its tables and
    /// stays one.
    pub fn set_mode(&mut self, mode: BipMode) {
        self.bbmd_config = None;
        self.foreign_device = None;
        self.bdt_file = None;
        self.set_fanout_policy(FanoutPolicy::default());
        match mode {
            BipMode::Normal => {}
            BipMode::Foreign(config) => self.register_as_foreign_device(config),
            BipMode::Bbmd(BbmdOptions {
                bdt,
                foreign_device_policy,
                management_acl,
                bdt_persist_path,
                fanout_policy,
                alone_on_subnet,
                nat,
            }) => {
                self.enable_bbmd(bdt);
                if let Some(policy) = foreign_device_policy {
                    self.enable_foreign_device_registration(policy);
                }
                self.set_bbmd_management_acl(management_acl);
                if let Some(path) = bdt_persist_path {
                    self.set_bdt_persist_path(path);
                }
                self.set_fanout_policy(fanout_policy);
                self.set_bbmd_alone_on_subnet(alone_on_subnet);
                self.set_bbmd_nat_traversal(nat);
            }
        }
    }
}
