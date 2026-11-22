//! Routed path size limits in the TUI (#2667): `:path-limit` sets or
//! clears one for the session, and the device browser says why a device
//! behind a router may not answer.
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
use bacnet_ops::target::{Target, parse_target_on};
use bacnet_ops::{DeviceAddress, RoutedPath};
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;

use crate::routed_paths::parse_path_arg;

/// `:path-limit ROUTER/NETWORK=SIZE`, or `ROUTER/NETWORK clear`: the router
/// as typed, read against the link by the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathLimit {
    /// The router's address on this network, as typed.
    pub(crate) router: String,
    /// The destination network, 1 to 65534.
    pub(crate) network: u16,
    /// The limit in NPDU octets; `None` puts the default back.
    pub(crate) max_npdu: Option<u16>,
}

impl PathLimit {
    /// The command bar's words after `path-limit`.
    pub(crate) fn parse(words: &[&str]) -> Result<Self, String> {
        match words {
            [path, "clear"] => {
                let (router, network) = parse_path_arg(path)?;
                Ok(Self {
                    router,
                    network,
                    max_npdu: None,
                })
            }
            [limit] => {
                let limit = crate::routed_paths::parse_limit_arg(limit)?;
                Ok(Self {
                    router: limit.router,
                    network: limit.network,
                    max_npdu: Some(limit.max_npdu),
                })
            }
            _ => Err(
                "usage: path-limit ROUTER/NETWORK=SIZE | path-limit ROUTER/NETWORK clear".into(),
            ),
        }
    }

    /// Set or clear the limit on `client`, and say what was done or why
    /// not. The router is an address as whois-router shows it; a B/IPv6
    /// router is named by its VMAC (`bip6:0x...`), so nothing is sent.
    pub(crate) fn apply<T: TransportPort + 'static>(&self, client: &BACnetClient<T>) -> String {
        match self.change(client) {
            Ok(text) => text,
            Err(why) => format!("path-limit {}/{}: {why}", self.router, self.network),
        }
    }

    fn change<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
    ) -> Result<String, String> {
        let mac = match parse_target_on(client.transport(), &self.router)? {
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
                            station behind a router"
                        .into(),
                );
            }
        };
        let style = bacnet_ops::target::AddressStyle::of(client.transport());
        let path = RoutedPath::new(style, &mac, self.network).map_err(|e| e.to_string())?;
        let refused = |error: Error| match error {
            Error::Encoding(reason) => reason,
            other => other.to_string(),
        };
        match self.max_npdu {
            Some(max_npdu) => {
                client
                    .configure_routed_path_max_npdu(&mac, self.network, max_npdu)
                    .map_err(refused)?;
                Ok(format!(
                    "Requests to network {} through {} now go in at most {max_npdu} NPDU octets",
                    path.network(),
                    path.router()
                ))
            }
            None => {
                client
                    .clear_routed_path_limit(&mac, self.network)
                    .map_err(refused)?;
                Ok(format!(
                    "Requests to network {} through {} are back on the default of {} NPDU octets",
                    path.network(),
                    path.router(),
                    bacnet_ops::client::DEFAULT_ROUTED_PATH_MAX_NPDU
                ))
            }
        }
    }
}

/// Why a read of the device at `address` may have timed out, when it is
/// behind a router and a too-long reject for a request of 228 octets or
/// fewer was dropped on its path: the router may carry less than every
/// router must. `None` for any other failure or device. Sends nothing.
pub(crate) async fn hint<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
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
        .then(|| hint_text(&path))
}

/// The hint for `path`, naming the command that sets its limit.
pub(crate) fn hint_text(path: &RoutedPath) -> String {
    format!(
        "The router at {} refused a request of 228 NPDU octets or fewer to network {} as too \
         long. No router that follows the standard does, so the client did not believe it, and \
         the request timed out. If that router really carries less, set the path's limit: \
         :path-limit {path}=SIZE, with SIZE what it carries (at least the 50-octet minimum APDU \
         and its header).",
        path.router(),
        path.network()
    )
}

#[cfg(test)]
#[path = "routed_paths_tests.rs"]
mod tests;
