//! `bacnet mcp serve --stdio` and `bacnet mcp authorize` (#2066): the MCP
//! server of the `bacnet-mcp` crate, on a client built from a profile or
//! the transport flags; read-only unless the profile's `[mcp]` table
//! offers writes (#2068), which go through the profile's workspace. And
//! `bacnet mcp connect` (#2185): stdio bridged to the socket of a TUI that
//! has the profile open.
//!
//! Stdout belongs to the JSON-RPC stream while the server runs, so this
//! module denies the print lints the rest of the CLI allows, and tracing
//! goes to stderr. `authorize` asks on stderr too.
#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::collections::BTreeSet;
use std::io::Write as _;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bacnet_client::client::BACnetClient;
use bacnet_mcp::policy::{ArmMode, Caps, PlanGrant, Rates, Workspace, WriteMode, WritePolicy};
use bacnet_mcp::{McpServer, Policy};
use bacnet_ops::engine::Engine;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::port::TransportPort;

use crate::args::{Cli, Command, McpAction};
use crate::core::guard::Confirm;
use crate::transport::TransportArgs;
use crate::tui::profile::mcp::McpChoice;
use crate::tui::profile::mcp::device_range;
use crate::tui::profile::overrides::has_transport_flags;
use crate::tui::profile::store::Store;
use crate::tui::profile::{McpSection, Profile};

/// Logs on stderr, never stdout: `-v` raises the level, or `RUST_LOG`
/// sets it, and the read log (`bacnet_mcp::reads`, a line per call and a
/// summary at shutdown) is always on at info, whatever either says.
/// Colour only on a terminal.
fn init_tracing(verbosity: u8) {
    use std::io::IsTerminal as _;
    use tracing_subscriber::EnvFilter;
    let level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = match std::env::var("RUST_LOG") {
        Ok(text) if !text.is_empty() => EnvFilter::new(text),
        _ => EnvFilter::new(level),
    };
    let reads = "bacnet_mcp::reads=info".parse().expect("a fixed directive");
    tracing_subscriber::fmt()
        .with_env_filter(filter.add_directive(reads))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
}

/// `bacnet mcp ...`.
pub(crate) async fn run(cli: Cli) -> crate::CliResult {
    init_tracing(cli.verbose);
    match &cli.command {
        Some(Command::Mcp {
            action: McpAction::Serve {
                profile, passive, ..
            },
        }) => serve(&cli, profile.as_deref(), *passive).await,
        Some(Command::Mcp {
            action: McpAction::Connect { profile },
        }) => connect(&Store::discover()?, profile.as_deref()).await,
        Some(Command::Mcp {
            action: McpAction::Authorize { profile },
        }) => {
            let store = Store::discover()?;
            authorize(&store, profile, &mut crate::core::guard::Stdin)
        }
        _ => unreachable!("dispatched on the mcp command"),
    }
}

/// The policy a profile's `[mcp]` table sets (the defaults for each key
/// left out), checked.
pub(crate) fn policy(section: &McpSection, passive: bool) -> Result<Policy, String> {
    let rates = Rates::default();
    let caps = Caps::default();
    let devices = section
        .devices
        .as_ref()
        .map(|list| list.iter().map(|text| device_range(text)).collect())
        .transpose()?;
    let policy = Policy {
        passive,
        networks: section
            .networks
            .as_ref()
            .map(|list| list.iter().copied().collect()),
        devices,
        slow_networks: section
            .slow_networks
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        fast_networks: section
            .fast_networks
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        broad_discovery: section.broad_discovery,
        rates: Rates {
            global: section.max_rate.unwrap_or(rates.global),
            network: section.network_rate.unwrap_or(rates.network),
            slow: section.slow_rate.unwrap_or(rates.slow),
            fast: section.fast_rate.unwrap_or(rates.fast),
            principal: section.principal_rate.unwrap_or(rates.principal),
            burst: section.principal_burst.unwrap_or(rates.burst),
        },
        caps: Caps {
            values: section.max_values.unwrap_or(caps.values),
            devices: section.page_devices.unwrap_or(caps.devices),
            objects: section.page_objects.unwrap_or(caps.objects),
            text: section.max_text.unwrap_or(caps.text),
            budget: section
                .call_budget_s
                .map_or(caps.budget, Duration::from_secs),
            ..caps
        },
        // Writes need the profile's workspace: `from_profile` adds them.
        writes: WritePolicy::default(),
    };
    policy.validate().map_err(|e| format!("[mcp] {e}"))?;
    Ok(policy)
}

/// The `[mcp.plan]` grant, which runs from the authorization for its
/// hours; its write budget is counted for that authorization (#2159).
fn plan_grant(section: &McpSection) -> Result<Option<PlanGrant>, String> {
    let Some(plan) = &section.plan else {
        return Ok(None);
    };
    let (devices, properties) = plan.parsed()?;
    // From the authorization, but never from a time still to come: a
    // hand-edited future time can't lengthen the plan.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let authorized = section.authorized_at_unix.unwrap_or(0);
    let start = authorized.min(now);
    Ok(Some(PlanGrant {
        devices,
        properties,
        priorities: (plan.priority_min, plan.priority_max),
        max_writes: plan.max_writes,
        authorized_unix: authorized,
        expires_unix: start.saturating_add(plan.hours.saturating_mul(3_600)),
        reason: plan.reason.clone(),
    }))
}

/// What the `[mcp]` table lets the server change, recording to
/// `workspace` (#2068).
pub(crate) fn write_policy(
    section: &McpSection,
    workspace: Workspace,
) -> Result<WritePolicy, String> {
    let choice = |c: McpChoice| c == McpChoice::Elicit;
    let plan = plan_grant(section)?;
    Ok(WritePolicy {
        mode: if choice(section.mcp_writes) {
            WriteMode::Elicit
        } else {
            WriteMode::Never
        },
        arm: if choice(section.mcp_arm) {
            ArmMode::Elicit
        } else {
            ArmMode::Never
        },
        plan,
        workspace: Some(workspace),
    })
}

/// `url` without what may carry a credential: user information, the query
/// and the fragment.
fn without_secrets(url: &str) -> String {
    let url = url.split(['?', '#']).next().unwrap_or_default();
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            let path = if path.is_empty() {
                String::new()
            } else {
                format!("/{path}")
            };
            format!("{scheme}://{host}{path}")
        }
        None => url.to_string(),
    }
}

/// The local end of `args`' link, for audit records.
pub(crate) fn local(args: &TransportArgs) -> (&'static str, String) {
    if let Some(mstp) = &args.mstp {
        ("MSTP", mstp.port.clone())
    } else if args.sc {
        (
            "SC",
            without_secrets(args.sc_url.as_deref().unwrap_or_default()),
        )
    } else if args.ipv6 {
        ("BIP6", "ipv6".into())
    } else {
        ("BIP", format!("{}:{}", args.interface, args.port))
    }
}

/// The one principal of a stdio server, and of the TUI's attached
/// sessions: the OS user running it.
pub(crate) fn principal() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "local-user".into())
}

/// Why a profile can't serve MCP yet.
fn unauthorized(name: &str) -> String {
    format!(
        "profile {name} is not authorized for MCP (or its [mcp] table now reaches further than \
         was authorized); run `bacnet mcp authorize --profile {name}` in a terminal first"
    )
}

/// Why live traffic needs a profile.
const NEEDS_PROFILE: &str = "a live MCP server needs an authorized profile: give --profile NAME \
     (authorized with `bacnet mcp authorize --profile NAME`); without one only --passive, which \
     sends nothing, is served";

/// Why transport flags can't go with a profile.
const FLAGS_WITH_PROFILE: &str = "transport flags can't change an authorized profile's transport; \
     edit the profile and authorize it again, or leave the flags out";

/// The profile's transport and policy, checked: it must be authorized for
/// MCP, for this table and these transport settings, and the command line
/// may not change them.
fn from_profile(cli: &Cli, name: &str) -> Result<(TransportArgs, Policy), String> {
    if has_transport_flags(cli) {
        return Err(FLAGS_WITH_PROFILE.into());
    }
    let store = Store::discover()?;
    let profile = store.load(name)?;
    let section = profile.mcp.as_deref().cloned().unwrap_or_default();
    if !section.authorized() {
        return Err(unauthorized(name));
    }
    let args = profile.transport_args()?;
    let mut policy = policy(&section, false)?;
    if section.mcp_writes == McpChoice::Elicit {
        let (transport, local) = local(&args);
        let workspace = Workspace {
            dir: store.workspace(name)?,
            profile: Some(name.to_string()),
            transport,
            local,
        };
        policy.writes = write_policy(&section, workspace)?;
        policy.validate().map_err(|e| format!("[mcp] {e}"))?;
        if section.mcp_arm == McpChoice::Never && section.plan.is_none() {
            tracing::warn!(
                "[mcp] mcp_writes = \"elicit\" with mcp_arm = \"never\" and no [mcp.plan]: the \
                 write tools are listed, but every write will be refused (not-armed)"
            );
        }
    }
    Ok((args, policy))
}

async fn serve(cli: &Cli, profile: Option<&str>, passive: bool) -> crate::CliResult {
    if passive {
        // Nothing is sent, so no consent is needed: a profile only sets the
        // policy, and the transport flags are left unused.
        let section = match profile {
            Some(name) => Store::discover()?
                .load(name)?
                .mcp
                .as_deref()
                .cloned()
                .unwrap_or_default(),
            None => McpSection::default(),
        };
        tracing::info!("passive MCP server: no client is built, nothing is sent");
        let server = McpServer::<BipTransport>::new(None, policy(&section, true)?, principal())?;
        return Ok(server.serve_stdio_until(shutdown_signal()).await?);
    }
    let Some(name) = profile else {
        return Err(NEEDS_PROFILE.into());
    };
    let (args, policy) = from_profile(cli, name)?;
    if args.mstp.is_some() {
        return serve_mstp(cli, &args, policy).await;
    }
    if args.sc {
        #[cfg(feature = "sc-tls")]
        return serve_on(crate::transport::build_sc_client(&args).await?, policy).await;
        #[cfg(not(feature = "sc-tls"))]
        return Err("BACnet/SC requires the 'sc-tls' feature".into());
    }
    if args.ipv6 {
        return serve_on(crate::transport::build_bip6_client(&args).await?, policy).await;
    }
    let client = crate::transport::build_bip_client(&args).await?;
    // Broadcasts reach the remote network once the BBMD has taken the
    // registration, as for a one-shot command.
    if let Some(config) = &args.foreign_device {
        let bbmd = std::net::SocketAddrV4::new(config.bbmd_ip, config.bbmd_port);
        crate::foreign::await_registration(&client, bbmd).await?;
    }
    serve_on(client, policy).await
}

#[cfg(feature = "serial")]
async fn serve_mstp(cli: &Cli, args: &TransportArgs, policy: Policy) -> crate::CliResult {
    let thread = crate::mstp_thread::from_cli(cli);
    let (client, _diagnostics) = crate::mstp::build_client(args, &thread).await?;
    serve_on(client, policy).await
}

#[cfg(not(feature = "serial"))]
async fn serve_mstp(_cli: &Cli, _args: &TransportArgs, _policy: Policy) -> crate::CliResult {
    Err("MS/TP requires the 'serial' feature".into())
}

/// Resolves on SIGTERM or SIGINT (Ctrl-C on Windows): the server then
/// completes its listen streams and cancels its COV subscriptions before
/// it exits. A signal that can't be watched never resolves.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
        ) {
            (Ok(mut term), Ok(mut int)) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = int.recv() => {}
                }
            }
            _ => std::future::pending().await,
        }
    }
    #[cfg(not(unix))]
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
    tracing::info!("shutting down: completing listen streams and cancelling COV subscriptions");
}

/// Serve MCP over `client` until the agent host closes stdin or a signal
/// stops the server, then stop the client.
async fn serve_on<T: TransportPort + 'static>(
    client: BACnetClient<T>,
    policy: Policy,
) -> crate::CliResult {
    let client = Arc::new(client);
    let engine = Engine::new(Arc::clone(&client), Arc::new(policy.request_limits()));
    let server = McpServer::new(Some(Arc::new(engine)), policy, principal())?;
    let served = server.serve_stdio_until(shutdown_signal()).await;
    // The server has let go of the engine, and with it the client.
    if let Ok(mut client) = Arc::try_unwrap(client) {
        client.stop().await?;
    }
    Ok(served?)
}

/// Why `bacnet mcp connect` can't run here yet.
#[cfg(not(unix))]
const NO_ATTACH: &str = "bacnet mcp connect (attaching to the TUI) isn't available on Windows yet; \
     run `bacnet mcp serve --stdio --profile NAME` instead";

/// `bacnet mcp connect`: stdin and stdout passed to the socket of the TUI
/// that has `profile` open (or the only TUI listening), byte for byte.
async fn connect(store: &Store, profile: Option<&str>) -> crate::CliResult {
    #[cfg(unix)]
    {
        let path = attached_socket(store, profile)?;
        let bridged = bacnet_mcp::attach::bridge_stdio(&path).await;
        // A read of stdin may still be blocked on a runtime thread, which
        // the runtime would wait for: exit now instead.
        let code = match bridged {
            Ok(()) => 0,
            Err(error) => {
                let _ = writeln!(std::io::stderr(), "Error: {error}");
                1
            }
        };
        std::process::exit(code);
    }
    #[cfg(not(unix))]
    {
        let _ = (store, profile);
        Err(NO_ATTACH.into())
    }
}

/// The socket `bacnet mcp connect` attaches to: `profile`'s, or without
/// one the only profile with a socket in its workspace.
#[cfg(unix)]
fn attached_socket(store: &Store, profile: Option<&str>) -> Result<std::path::PathBuf, String> {
    use bacnet_mcp::attach::socket_path;
    if let Some(name) = profile {
        return Ok(socket_path(&store.workspace_path(name)?));
    }
    let open: Vec<String> = store
        .list()?
        .into_iter()
        .filter(|name| {
            store.workspace_path(name).is_ok_and(|dir| {
                use std::os::unix::fs::FileTypeExt as _;
                std::fs::symlink_metadata(socket_path(&dir))
                    .is_ok_and(|meta| meta.file_type().is_socket())
            })
        })
        .collect();
    match open.as_slice() {
        [] => Err(
            "no TUI is listening for MCP: open a profile in `bacnet tui`, or name one \
                   with --profile NAME"
                .into(),
        ),
        [name] => Ok(socket_path(&store.workspace_path(name)?)),
        names => Err(format!(
            "TUIs are listening on several profiles ({}); pick one with --profile NAME",
            names.join(", ")
        )),
    }
}

/// What an authorization shows: the reach the `[mcp]` table gives.
fn summary(name: &str, section: &McpSection) -> String {
    let list = |items: Option<String>| items.unwrap_or_else(|| "all".into());
    let networks = list(section.networks.as_ref().map(|n| format!("{n:?}")));
    let devices = list(section.devices.as_ref().map(|d| d.join(", ")));
    let writes = match (section.mcp_writes, &section.plan) {
        (McpChoice::Never, _) => "no writes".to_string(),
        (McpChoice::Elicit, None) if section.mcp_arm == McpChoice::Never => {
            "write tools listed, but with mcp_arm = \"never\" and no [mcp.plan] every write \
             will be refused (not-armed)"
                .to_string()
        }
        (McpChoice::Elicit, None) => {
            "writes at priority 9-16 (or to properties that aren't commandable) once the user \
             confirms them in the agent host"
                .to_string()
        }
        (McpChoice::Elicit, Some(plan)) => {
            let safety: Vec<String> = [1u8, 2, 5]
                .into_iter()
                .filter(|p| (plan.priority_min..=plan.priority_max).contains(p))
                .map(|p| p.to_string())
                .collect();
            let warning = if safety.is_empty() {
                String::new()
            } else {
                format!(
                    "; CAUTION: priority {} is for life safety or critical equipment",
                    safety.join(", ")
                )
            };
            format!(
                "writes the user confirms in the agent host, and WITHOUT ANY PROMPT up to {} \
                 writes of {} on devices {} at priority {}-{} for the next {} hour(s){warning}",
                plan.max_writes,
                plan.properties.join(", "),
                plan.devices.join(", "),
                plan.priority_min,
                plan.priority_max,
                plan.hours
            )
        }
    };
    let attached = if section.tui_listener == Some(false) {
        ""
    } else {
        ", and read-only through the TUI's local socket while the profile is open there"
    };
    format!(
        "Authorize AI agents to use profile {name} through `bacnet mcp serve`{attached}.\n\
         They can discover devices and read them ({writes}), on networks: {networks}; \
         devices: {devices}; broad discovery: {}.\n\
         Device text and site data will reach the agent and its host.\n\
         Type the profile name to authorize: ",
        if section.broad_discovery {
            "allowed"
        } else {
            "refused"
        }
    )
}

/// `bacnet mcp authorize`: show what the profile's `[mcp]` table lets an
/// agent reach, and record consent when the person types the profile's
/// name. Needs a terminal: an agent host can't answer it.
pub(crate) fn authorize(store: &Store, name: &str, confirm: &mut dyn Confirm) -> crate::CliResult {
    let mut profile: Profile = store.load(name)?;
    let section = profile.mcp.as_deref().cloned().unwrap_or_default();
    policy(&section, false)?;
    if !confirm.interactive() {
        return Err(
            "bacnet mcp authorize needs a terminal: run it yourself, not from an agent".into(),
        );
    }
    let answer = confirm.ask(&summary(name, &section)).unwrap_or_default();
    if answer.trim() != name {
        return Err("not authorized: the answer didn't match the profile name".into());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("cannot timestamp authorization: {e}"))?
        .as_secs();
    let link = profile.link();
    let section = profile.mcp.get_or_insert_default();
    section.authorize(now, link);
    // A fresh write count for the plan, before the authorization is
    // saved: a count left by an earlier authorization never matches this
    // one, so if saving the profile fails the plan's writes stay refused.
    if section.mcp_writes == McpChoice::Elicit
        && let Some(plan) = plan_grant(section)?
    {
        plan.start_budget(&store.workspace(name)?)?;
    }
    store.save(name, &profile, false)?;
    // stderr, as the question was: an explicit write, not a print.
    let _ = writeln!(std::io::stderr(), "Profile {name} is authorized for MCP.");
    Ok(())
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
