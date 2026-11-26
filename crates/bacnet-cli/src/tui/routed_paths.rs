//! Routed path size limits in the TUI (#2667): `:path-limit` sets or
//! clears one for the session, named by its router or by a device reached
//! through it (#2726), and the device browser says why a device behind a
//! router may not answer.
//!
//! The client sizes each request through a router to a limit per path, the
//! router and the destination network: 228 NPDU octets unless set by hand or
//! lowered by a router's too-long reject. It does not believe such a reject
//! for a request of 228 octets or fewer, the least every router must carry
//! (Table 6-1), so a router that really carries less shows as timeouts until
//! its path's limit is set. A limit is a setting of this session's client: it
//! sends nothing to any device, so no write level guards it, as `:csv` and
//! `:mstp` are not guarded. It applies at once, also to a request waiting out
//! the hold a timeout left on the path (#2683).

use bacnet_client::client::BACnetClient;
use bacnet_ops::target::{AddressStyle, Target, parse_target_on};
use bacnet_ops::{DeviceAddress, RoutedPath};
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;

use crate::routed_paths::parse_path_arg;

/// The command's forms, for its refusals.
pub(crate) const USAGE: &str = "usage: path-limit DEVICE SIZE|clear | \
     path-limit ROUTER/NETWORK=SIZE | path-limit ROUTER/NETWORK clear";

/// `:path-limit`: set or clear the size limit of one routed path for the
/// session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathLimit {
    /// The path, as the command named it.
    pub(crate) path: PathTo,
    /// The limit in NPDU octets; `None` puts the default back.
    pub(crate) max_npdu: Option<u16>,
}

/// How `:path-limit` named its path (#2726). A router form always has a
/// `/` between the router and the network, and no device form has one, so
/// the first word says which it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PathTo {
    /// `ROUTER/NETWORK`, read against the link by the worker.
    Router {
        /// The router's address on this network, as typed.
        router: String,
        /// The destination network, 1 to 65534.
        network: u16,
    },
    /// A device of the device table: the path the client reaches it by,
    /// which the worker looks up as `bacnet_ops`' `Device::routed_path`
    /// does.
    Device {
        /// The device's instance, for what the worker says.
        instance: u32,
        /// Where the device answers, as its row of the table has it.
        at: DeviceAddress,
    },
}

impl PathLimit {
    /// The command bar's words after `path-limit` for the router forms:
    /// `ROUTER/NETWORK=SIZE`, or `ROUTER/NETWORK clear`.
    pub(crate) fn parse(words: &[&str]) -> Result<Self, String> {
        let (router, network, max_npdu) = match words {
            [path, "clear"] => {
                let (router, network) = parse_path_arg(path)?;
                (router, network, None)
            }
            [limit] => {
                let limit = crate::routed_paths::parse_limit_arg(limit)?;
                (limit.router, limit.network, Some(limit.max_npdu))
            }
            [path, size] if !path.contains('=') && size.parse::<u16>().is_ok() => {
                return Err(format!(
                    "'{path} {size}': a path takes its size after '=': \
                     path-limit {path}={size}"
                ));
            }
            _ => return Err(USAGE.into()),
        };
        Ok(Self {
            path: PathTo::Router { router, network },
            max_npdu,
        })
    }

    /// `SIZE` or `clear` after a device: the limit, or `None` for the
    /// default.
    pub(crate) fn size(word: &str) -> Result<Option<u16>, String> {
        match word {
            "clear" => Ok(None),
            size => size.parse::<u16>().map(Some).map_err(|_| {
                format!("'{size}' is neither clear nor a number of NPDU octets from 0 to 65535")
            }),
        }
    }

    /// Set or clear the limit on `client`, and say what was done or why
    /// not. A router is an address as whois-router shows it; a B/IPv6
    /// router is named by its VMAC (`bip6:0x...`), so nothing is sent. A
    /// device's path is found in the client's discovery table, which sends
    /// nothing either.
    pub(crate) async fn apply<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
    ) -> String {
        let style = AddressStyle::of(client.transport());
        let changed = match &self.path {
            PathTo::Router { router, network } => router_path(client, style, router, *network)
                .and_then(|path| self.set(client, &path, "")),
            PathTo::Device { instance, at } => {
                match bacnet_ops::raw::routed_path_to(client, at).await {
                    Ok(Some(path)) => {
                        self.set(client, &path, &format!(" (device {instance}'s path)"))
                    }
                    Ok(None) => Err(format!(
                        "device {instance} at {} is on this network, not behind a router, \
                         so no routed path's limit applies to it",
                        at.styled(style)
                    )),
                    Err(error) => Err(error.to_string()),
                }
            }
        };
        changed.unwrap_or_else(|why| match &self.path {
            PathTo::Router { router, network } => format!("path-limit {router}/{network}: {why}"),
            PathTo::Device { instance, .. } => format!("path-limit device {instance}: {why}"),
        })
    }

    /// Set or clear the limit of `path` on `client`, and say what became
    /// of the path's requests; `whose` follows the path in the text.
    fn set<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        path: &RoutedPath,
        whose: &str,
    ) -> Result<String, String> {
        // The client's refusals (#1833) show their text alone.
        let refused = |error: Error| error.to_string();
        let (router_mac, network) = (path.router_mac(), path.network());
        let done = match self.max_npdu {
            Some(max_npdu) => {
                client
                    .configure_routed_path_max_npdu(router_mac, network, max_npdu)
                    .map_err(refused)?;
                format!("now go in at most {max_npdu} NPDU octets")
            }
            None => {
                client
                    .clear_routed_path_limit(router_mac, network)
                    .map_err(refused)?;
                format!(
                    "are back on the default of {} NPDU octets",
                    bacnet_ops::client::DEFAULT_ROUTED_PATH_MAX_NPDU
                )
            }
        };
        Ok(format!(
            "Requests to network {network} through {}{whose} {done}",
            path.router()
        ))
    }
}

/// The path through the router typed as `router` to `network`. The router
/// must be an address on this link: a device or a station behind a router
/// is refused, and a B/IPv6 router is named by its VMAC.
fn router_path<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    style: AddressStyle,
    router: &str,
    network: u16,
) -> Result<RoutedPath, String> {
    let mac = match parse_target_on(client.transport(), router)? {
        Target::Mac(mac) => mac,
        Target::Bip6(_) => {
            return Err(
                "name a B/IPv6 router by its VMAC (bip6:0x...), as the routers \
                        list shows it"
                    .into(),
            );
        }
        _ => {
            return Err(
                "give the router's own address on this network, not a device or a \
                        station behind a router; a device's path is path-limit DEVICE SIZE"
                    .into(),
            );
        }
    };
    RoutedPath::new(style, &mac, network).map_err(|e| e.to_string())
}

/// Why a read of `device`, the device at `address`, may have timed out,
/// when it is behind a router and a too-long reject for a request of 228
/// octets or fewer was dropped on its path: the router may carry less than
/// every router must. `None` for any other failure or device. Sends
/// nothing.
pub(crate) async fn hint<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    device: u32,
    address: &DeviceAddress,
    error: &Error,
) -> Option<String> {
    if !bacnet_ops::raw::timed_out(error) {
        return None;
    }
    let path = bacnet_ops::raw::routed_path_to(client, address)
        .await
        .ok()??;
    bacnet_ops::raw::routed_paths(client)
        .status(&path)
        .below_minimum_reject_seen
        .then(|| hint_text(&format!("{}:{device}", path.network()), &path))
}

/// The hint for `path`, the path to the device the command bar names as
/// `device` (`DNET:instance`): the command that sets the limit by that
/// device first, then by the path itself.
pub(crate) fn hint_text(device: &str, path: &RoutedPath) -> String {
    format!(
        "The router at {} refused a request of 228 NPDU octets or fewer to network {} as too \
         long. No router that follows the standard does, so the client did not believe it, and \
         the request timed out. If that router really carries less, set the path's limit: \
         :path-limit {device} SIZE (or :path-limit {path}=SIZE), with SIZE what it carries (at \
         least the 50-octet minimum APDU and its header).",
        path.router(),
        path.network()
    )
}

#[cfg(test)]
#[path = "routed_paths_tests.rs"]
mod tests;
