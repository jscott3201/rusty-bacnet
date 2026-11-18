//! Routed path size limits in a one-shot command (#2667): the repeatable
//! `--routed-path-limit ROUTER/NETWORK=MAX_NPDU`, set on the client before
//! the command runs, and a summary of the client's routed paths on stderr
//! after it.
//!
//! The client sizes each request through a router to a limit per path, the
//! router and the destination network: 228 NPDU octets unless set by hand
//! or lowered by a router's too-long reject. It does not believe such a
//! reject for a request of 228 octets or fewer, the least every router must
//! carry (Table 6-1), so a router that really carries less shows as
//! timeouts until the flag sets its path's limit.
//!
//! The summary prints when something the command line did not ask for
//! happened, a learned limit or a dropped reject, and with `-v` whenever
//! a path holds a limit. Stdout stays the command's own output.

use std::future::Future;

use bacnet_client::client::{BACnetClient, RoutedPathLimit};
use bacnet_ops::target::{AddressStyle, address_mac};
use bacnet_transport::port::TransportPort;
use serde::Serialize;

use crate::CliResult;
use crate::args::Cli;
use crate::output::OutputFormat;

/// The flag's shape, for its errors and help.
const SHAPE: &str = "ROUTER/NETWORK=MAX_NPDU, such as 192.168.1.20:47808/5=480";

/// One `--routed-path-limit` as typed: the router's address in the target
/// syntax, read against the link once the client is up, the destination
/// network and the limit in NPDU octets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LimitArg {
    router: String,
    network: u16,
    max_npdu: u16,
}

/// Parse `ROUTER/NETWORK=MAX_NPDU`. The router is everything before the
/// last `/`, so any address form the targets take fits (`ip:port`,
/// `mstp:N`, `bip6:0x...`, `[ipv6]:port`, hex octets). The network is 1 to
/// 65534. The client checks the limit itself once it knows its own link.
pub(crate) fn parse_limit_arg(text: &str) -> Result<LimitArg, String> {
    let (path, max_npdu) = text
        .rsplit_once('=')
        .ok_or_else(|| format!("'{text}' is not {SHAPE}"))?;
    let (router, network) = path
        .rsplit_once('/')
        .ok_or_else(|| format!("'{text}' is not {SHAPE}"))?;
    if router.is_empty() {
        return Err(format!("'{text}' names no router: {SHAPE}"));
    }
    let network = match network.parse::<u16>() {
        Ok(network @ 1..=65534) => network,
        _ => {
            return Err(format!(
                "'{network}' is not a remote network number from 1 to 65534"
            ));
        }
    };
    let max_npdu = max_npdu
        .parse::<u16>()
        .map_err(|_| format!("'{max_npdu}' is not a number of NPDU octets from 0 to 65535"))?;
    Ok(LimitArg {
        router: router.to_owned(),
        network,
        max_npdu,
    })
}

/// The subcommands with no client of their own to set a limit on, or whose
/// sessions take none from the command line yet.
const NO_CLIENT: [&str; 5] = ["tui", "mcp", "capture", "sc-check", "mstp-ports"];

/// Refuse `--routed-path-limit` where nothing would use it, rather than
/// ignore it: plain `bacnet` and the commands in [`NO_CLIENT`].
pub(crate) fn check_matches(matches: &clap::ArgMatches) -> Result<(), clap::Error> {
    if matches
        .get_many::<LimitArg>("routed_path_limit")
        .is_none_or(|mut limits| limits.next().is_none())
    {
        return Ok(());
    }
    let command = matches.subcommand_name();
    if command.is_none_or(|name| NO_CLIENT.contains(&name)) {
        return Err(clap::Error::raw(
            clap::error::ErrorKind::ArgumentConflict,
            format!(
                "--routed-path-limit applies to the one-shot commands that send requests, not to {}\n",
                command.map_or_else(|| "plain bacnet".to_owned(), |name| format!("'{name}'"))
            ),
        ));
    }
    Ok(())
}

/// Set each `--routed-path-limit` on `client`, then run `command`, then
/// print the routed-path summary (see the module docs) whatever the
/// command's outcome, and return that outcome.
pub(crate) async fn around<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    cli: &Cli,
    format: OutputFormat,
    command: impl Future<Output = CliResult>,
) -> CliResult {
    let mut paths: Vec<(Vec<u8>, &LimitArg)> = Vec::with_capacity(cli.routed_path_limit.len());
    for limit in &cli.routed_path_limit {
        let mac = address_mac(
            client.transport(),
            &limit.router,
            "it names a device or a station behind a router; give the router's own address \
             on this network, as whois-router prints it",
        )
        .await
        .map_err(|reason| {
            format!(
                "--routed-path-limit {}: {}",
                limit.router,
                crate::destination::with_link_flag(reason)
            )
        })?;
        if let Some((_, first)) = paths
            .iter()
            .find(|(other, first)| *other == mac && first.network == limit.network)
        {
            return Err(format!(
                "--routed-path-limit names network {} through router {} twice ({}/{} and {}/{}); \
                 give each path once",
                limit.network,
                AddressStyle::of(client.transport()).format(&mac),
                first.router,
                first.network,
                limit.router,
                limit.network
            )
            .into());
        }
        paths.push((mac, limit));
    }
    for (mac, limit) in &paths {
        client
            .configure_routed_path_max_npdu(mac, limit.network, limit.max_npdu)
            .await
            .map_err(|error| format!("--routed-path-limit {}: {error}", limit.router))?;
    }
    let outcome = command.await;
    let summary = Summary::of(client, timed_out(&outcome));
    if summary.shown(cli.verbose) {
        eprintln!("{}", summary.render(format));
    }
    outcome
}

/// Whether the command failed because a request timed out: the client's
/// own timeout or the state machine's, as [`bacnet_ops::raw::timed_out`]
/// reads them, anywhere in the error's chain, or an [`OpsError`] that says
/// so. Its `source()` skips the client error it wraps, so it is asked
/// itself.
///
/// [`OpsError`]: bacnet_ops::OpsError
fn timed_out(outcome: &CliResult) -> bool {
    let Err(error) = outcome else {
        return false;
    };
    let mut next: Option<&(dyn std::error::Error + 'static)> = Some(&**error);
    while let Some(error) = next {
        let client = error
            .downcast_ref::<bacnet_types::error::Error>()
            .is_some_and(bacnet_ops::raw::timed_out);
        let ops = error
            .downcast_ref::<bacnet_ops::OpsError>()
            .is_some_and(bacnet_ops::OpsError::timed_out);
        if client || ops {
            return true;
        }
        next = error.source();
    }
    false
}

/// One path as the summary shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct PathRow {
    /// The router's address on this network, in the link's own form.
    router: String,
    network: u16,
    max_npdu: u16,
    /// `default`, `configured` or `learned`.
    source: &'static str,
    configured_max_npdu: Option<u16>,
    learned_max_npdu: Option<u16>,
    below_minimum_reject_seen: bool,
}

impl PathRow {
    fn of(path: &RoutedPathLimit, style: AddressStyle) -> Self {
        Self {
            router: style.format(&path.router_mac),
            network: path.dnet,
            max_npdu: path.max_npdu(),
            source: path.source().name(),
            configured_max_npdu: path.configured_max_npdu,
            learned_max_npdu: path.learned_max_npdu,
            below_minimum_reject_seen: path.below_minimum_reject_seen,
        }
    }
}

/// The client's routed paths and its two counts of too-long rejects it
/// did not believe, after the command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Summary {
    routed_paths: Vec<PathRow>,
    too_long_reject_drops: u64,
    below_minimum_reject_drops: u64,
    /// What to do about a timeout on a path whose router refused a small
    /// request as too long; `null` otherwise.
    hint: Option<String>,
}

impl Summary {
    fn of<T: TransportPort + 'static>(client: &BACnetClient<T>, timed_out: bool) -> Self {
        let style = AddressStyle::of(client.transport());
        let routed_paths: Vec<PathRow> = client
            .routed_path_limits()
            .iter()
            .map(|path| PathRow::of(path, style))
            .collect();
        let below_minimum_reject_drops = client.below_minimum_reject_drops();
        Self::new(
            routed_paths,
            client.too_long_reject_drops(),
            below_minimum_reject_drops,
            timed_out,
        )
    }

    fn new(
        routed_paths: Vec<PathRow>,
        too_long_reject_drops: u64,
        below_minimum_reject_drops: u64,
        timed_out: bool,
    ) -> Self {
        let hint = (timed_out && below_minimum_reject_drops > 0).then(|| hint(&routed_paths));
        Self {
            routed_paths,
            too_long_reject_drops,
            below_minimum_reject_drops,
            hint,
        }
    }

    /// Whether to print it: when the client learned a limit or dropped a
    /// reject, which the command line did not ask for, and with `-v` when
    /// there is anything to show.
    fn shown(&self, verbose: u8) -> bool {
        let unasked = self.too_long_reject_drops > 0
            || self.below_minimum_reject_drops > 0
            || self
                .routed_paths
                .iter()
                .any(|path| path.learned_max_npdu.is_some());
        unasked || (verbose > 0 && !self.routed_paths.is_empty())
    }

    /// A table and two lines, or one line of JSON.
    fn render(&self, format: OutputFormat) -> String {
        if format == OutputFormat::Json {
            return serde_json::to_string(self).expect("serialize the routed-path summary");
        }
        let dash = |value: Option<u16>| value.map_or_else(|| "-".to_owned(), |v| v.to_string());
        let mut text = String::from("Routed paths:\n");
        if self.routed_paths.is_empty() {
            text.push_str("  none beyond the default of 228 NPDU octets\n");
        } else {
            let mut table = comfy_table::Table::new();
            table.set_header(vec![
                "Router",
                "Network",
                "Max NPDU",
                "Source",
                "Configured",
                "Learned",
                "Small request refused",
            ]);
            for path in &self.routed_paths {
                table.add_row(vec![
                    path.router.clone(),
                    path.network.to_string(),
                    path.max_npdu.to_string(),
                    path.source.to_owned(),
                    dash(path.configured_max_npdu),
                    dash(path.learned_max_npdu),
                    if path.below_minimum_reject_seen {
                        "yes"
                    } else {
                        "no"
                    }
                    .to_owned(),
                ]);
            }
            text.push_str(&format!("{table}\n"));
        }
        text.push_str(&format!(
            "Too-long rejects not believed: {} that could answer an earlier request's larger \
             frame, {} for a request of 228 NPDU octets or fewer.",
            self.too_long_reject_drops, self.below_minimum_reject_drops
        ));
        if let Some(hint) = &self.hint {
            text.push_str(&format!("\nHint: {hint}"));
        }
        text
    }
}

/// The hint for a timeout after a dropped below-minimum reject, naming the
/// flag for each path that drew one.
fn hint(paths: &[PathRow]) -> String {
    let flagged: Vec<String> = paths
        .iter()
        .filter(|path| path.below_minimum_reject_seen)
        .map(|path| format!("--routed-path-limit {}/{}=SIZE", path.router, path.network))
        .collect();
    let set = if flagged.is_empty() {
        "--routed-path-limit ROUTER/NETWORK=SIZE".to_owned()
    } else {
        flagged.join(" or ")
    };
    format!(
        "a router refused a request of 228 NPDU octets or fewer as too long, which no router \
         that follows the standard does, so the client did not believe it and the request \
         timed out. If that router really carries less, set its path's limit by hand: {set}, \
         with SIZE what the router carries (at least the 50-octet minimum APDU and its header)"
    )
}

#[cfg(test)]
#[path = "routed_paths_tests.rs"]
mod tests;
