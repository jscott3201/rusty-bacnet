//! The size limits of the routed paths the low-level client sends through
//! (#2667), as the high-level client shows and sets them.
//!
//! A path is a router on this client's own link and a destination network.
//! The low-level client sizes every request through a router to its path's
//! limit: 228 NPDU octets, the least every router must carry (Table 6-1),
//! unless one is set by hand or a router's too-long reject lowered it. It
//! does not believe such a reject for a request of 228 octets or fewer, so a
//! router that really carries less shows as timeouts until its path's limit
//! is set. [`Client::routed_paths`] shows the limits with where each comes
//! from and the two counts of rejects the client did not believe;
//! [`Client::set_routed_path_limit`] and [`Client::clear_routed_path_limit`]
//! change one, and [`Device::routed_path`] names the path a device behind a
//! router is reached by. None of them sends anything to a device, and a
//! change applies at once, also to a request waiting out the hold a request
//! with no answer left on the path. The client does not shrink a request by
//! itself after a timeout: setting a limit is the caller's step.

use bacnet_client::client::{BACnetClient, RoutedPathLimitSource};
use bacnet_transport::port::TransportPort;
use bacnet_types::MacAddr;
use bacnet_types::error::Error;
use serde::Serialize;

use super::Client;
use crate::device::Device;
use crate::error::OpsError;
use crate::target::{AddressStyle, address_mac};

/// The least every router must carry, in NPDU octets (Table 6-1): the limit
/// of a path nothing has been set or learned for.
pub const DEFAULT_ROUTED_PATH_MAX_NPDU: u16 = 228;

/// A routed path: the router this client sends through, on its own link,
/// and the destination network behind it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct RoutedPath {
    /// The router's address on this client's link, in the link's own form
    /// (`192.168.1.20:47808`, `mstp:5`, `bip6:0x0004D2`, or hex octets).
    pub router: String,
    /// The router's MAC on this client's link.
    #[serde(skip)]
    pub router_mac: MacAddr,
    /// The destination network, 1 to 65534.
    pub network: u16,
}

impl RoutedPath {
    /// The path through the router at `router_mac` to `network`, its router
    /// shown in `style`.
    pub fn new(style: AddressStyle, router_mac: &[u8], network: u16) -> Self {
        Self {
            router: style.format(router_mac),
            router_mac: MacAddr::from_slice(router_mac),
            network,
        }
    }
}

impl std::fmt::Display for RoutedPath {
    /// `ROUTER/NETWORK`, as `--routed-path-limit` takes it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.router, self.network)
    }
}

/// One routed path's size limit, as the low-level client holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoutedPathStatus {
    /// The path.
    #[serde(flatten)]
    pub path: RoutedPath,
    /// The limit requests on the path are sized to, in NPDU octets.
    pub max_npdu: u16,
    /// Where [`max_npdu`](Self::max_npdu) comes from.
    #[serde(serialize_with = "source_name")]
    pub source: RoutedPathLimitSource,
    /// The limit set by hand, if one is.
    pub configured_max_npdu: Option<u16>,
    /// The limit learned from a router's too-long reject, if one was: one
    /// octet under the smallest request refused. Never below 228.
    pub learned_max_npdu: Option<u16>,
    /// Whether a too-long reject on this path was not believed, since its
    /// limit was last set or cleared, because the request sent nothing above
    /// 228 octets. With timeouts to the network, the sign of a router that
    /// carries less than every router must, whose limit needs setting.
    pub below_minimum_reject_seen: bool,
}

fn source_name<S: serde::Serializer>(
    source: &RoutedPathLimitSource,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(source.name())
}

/// The client's routed paths and its two counts of too-long rejects it did
/// not believe ([`Client::routed_paths`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoutedPaths {
    /// Each path holding more than the default (a limit set or learned, or
    /// a below-minimum reject seen), by router MAC and then network.
    pub paths: Vec<RoutedPathStatus>,
    /// Too-long rejects dropped since start because they could answer a
    /// larger frame of an earlier request on the same path. These clear by
    /// themselves.
    pub too_long_reject_drops: u64,
    /// Too-long rejects dropped since start because the request sent nothing
    /// above 228 octets. A rise, with timeouts to a network, is the sign of
    /// a router that needs its path's limit set.
    pub below_minimum_reject_drops: u64,
}

impl RoutedPaths {
    /// What `client` holds now, decoded for its link: for a front end that
    /// has the low-level client alone. Sends nothing.
    pub fn of<T: TransportPort + 'static>(client: &BACnetClient<T>) -> Self {
        let style = AddressStyle::of(client.transport());
        let paths = client
            .routed_path_limits()
            .into_iter()
            .map(|limit| RoutedPathStatus {
                path: RoutedPath::new(style, &limit.router_mac, limit.dnet),
                max_npdu: limit.max_npdu(),
                source: limit.source(),
                configured_max_npdu: limit.configured_max_npdu,
                learned_max_npdu: limit.learned_max_npdu,
                below_minimum_reject_seen: limit.below_minimum_reject_seen,
            })
            .collect();
        Self {
            paths,
            too_long_reject_drops: client.too_long_reject_drops(),
            below_minimum_reject_drops: client.below_minimum_reject_drops(),
        }
    }

    /// `path`'s status: its row, or the default for a path not listed.
    pub fn status(&self, path: &RoutedPath) -> RoutedPathStatus {
        self.paths
            .iter()
            .find(|status| {
                status.path.router_mac == path.router_mac && status.path.network == path.network
            })
            .cloned()
            .unwrap_or_else(|| RoutedPathStatus {
                path: path.clone(),
                max_npdu: DEFAULT_ROUTED_PATH_MAX_NPDU,
                source: RoutedPathLimitSource::Default,
                configured_max_npdu: None,
                learned_max_npdu: None,
                below_minimum_reject_seen: false,
            })
    }
}

impl<T: TransportPort + 'static> Client<T> {
    /// The path through the router at `router` on this client's link to
    /// `network`. `router` is an address in the target grammar (a B/IPv6
    /// socket address is first asked its VMAC, the one request this can
    /// send); a device instance or a station behind a router is refused, as
    /// is a network outside 1 to 65534.
    pub async fn routed_path_at(&self, router: &str, network: u16) -> Result<RoutedPath, OpsError> {
        if !(1..=65534).contains(&network) {
            return Err(OpsError::parse(
                "network",
                &network.to_string(),
                "a routed path's network is 1 to 65534".into(),
            ));
        }
        let transport = self.shared.client.transport();
        let mac = address_mac(
            transport,
            router,
            "a routed path names its router by its address on this client's link, not a \
             device instance or a station behind a router",
        )
        .await
        .map_err(|reason| OpsError::parse("address", router, reason))?;
        Ok(self.routed_path(&mac, network))
    }

    /// The path through the router whose MAC on this client's link is
    /// `router_mac` to `network`, decoded for display.
    pub fn routed_path(&self, router_mac: &[u8], network: u16) -> RoutedPath {
        RoutedPath::new(
            AddressStyle::of(self.shared.client.transport()),
            router_mac,
            network,
        )
    }

    /// The routed paths holding more than the default, with the client's two
    /// counts of too-long rejects it did not believe. Sends nothing.
    pub fn routed_paths(&self) -> RoutedPaths {
        RoutedPaths::of(&self.shared.client)
    }

    /// Size requests through `path` to at most `max_npdu` octets of NPDU,
    /// replacing any limit set or learned for it. It applies at once and
    /// sends nothing. A limit under 228 is taken, for a router that carries
    /// less than the standard asks; one that could carry no request (under
    /// 66 on B/IP, 61 on MS/TP) is refused as
    /// [`OpsError::Parse`], and a path new to a full table of 256 paths as
    /// [`OpsError::Client`].
    pub fn set_routed_path_limit(&self, path: &RoutedPath, max_npdu: u16) -> Result<(), OpsError> {
        self.shared
            .client
            .configure_routed_path_max_npdu(&path.router_mac, path.network, max_npdu)
            .map_err(|error| refused(path, error))
    }

    /// Put `path` back on the default of 228 octets, dropping the limit set
    /// or learned for it. It applies at once and sends nothing; a path the
    /// client holds nothing for is left alone.
    pub fn clear_routed_path_limit(&self, path: &RoutedPath) -> Result<(), OpsError> {
        self.shared
            .client
            .clear_routed_path_limit(&path.router_mac, path.network)
            .map_err(|error| refused(path, error))
    }
}

/// The low-level client's refusal of a change to `path`: an argument it
/// refused as [`OpsError::Parse`], anything else as itself.
fn refused(path: &RoutedPath, error: Error) -> OpsError {
    match error {
        Error::Encoding(reason) => OpsError::parse("routed path limit", &path.to_string(), reason),
        other => OpsError::from(other),
    }
}

impl<T: TransportPort + 'static> Device<T> {
    /// The routed path this handle's device is reached by: through the
    /// router the low-level client's discovery table holds for its network,
    /// where it answers now ([`address`](Self::address)). `None` for a
    /// device on this client's own network. A remote device whose network
    /// has no router in the table fails as
    /// [`OpsError::UnknownDevice`]. Sends nothing.
    pub async fn routed_path(&self) -> Result<Option<RoutedPath>, OpsError> {
        let client = self.client();
        let router = crate::device::router_for(client.low_level(), &self.address()).await?;
        Ok(router.map(|(router_mac, network)| client.routed_path(&router_mac, network)))
    }
}

#[cfg(test)]
#[path = "routed_paths_tests.rs"]
mod tests;
