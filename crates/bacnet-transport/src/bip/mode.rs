//! The B/IP mode a node runs in, as one choice (#938): a plain node, a
//! foreign device or a BBMD. The client, server and endpoint builders take
//! it, so a BBMD that also registers as a foreign device can't be built.

use std::path::PathBuf;

use super::{BipTransport, FanoutPolicy, ForeignDeviceConfig};
use crate::bbmd::{BdtEntry, ForeignDevicePolicy};

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
    /// A file holding a BDT to start with instead of `bdt`, when it loads
    /// ([`BipTransport::set_bdt_persist_path`]).
    pub bdt_persist_path: Option<PathBuf>,
    /// The forwarding queue and rate limits.
    pub fanout_policy: FanoutPolicy,
}

impl BipTransport {
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
        self.bdt_persist_path = None;
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
            }
        }
    }
}
