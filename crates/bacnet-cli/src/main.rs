//! BACnet command-line tool.
//!
//! `bacnet tui`, or plain `bacnet` on a terminal, opens the full-screen
//! terminal UI (the default `tui` feature; see [`entry`]). Every other
//! subcommand is a one-shot command for scripts, with `--json` output.
#![allow(clippy::print_stdout, clippy::print_stderr)] // a command-line tool prints its results
#![forbid(unsafe_code)]

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use std::{io::IsTerminal, net::Ipv4Addr};

use crate::destination::Destination;
use bacnet_client::client::BACnetClient;
use bacnet_ops::parse;
use bacnet_transport::port::TransportPort;
use clap::{CommandFactory, FromArgMatches};

mod args;
mod bip_commands;
mod commands;
mod core;
// The one-shot capture (`pcap`) and the TUI's Capture screen each use a
// part of it.
#[cfg_attr(not(all(feature = "pcap", feature = "tui")), allow(dead_code))]
mod decode;
mod destination;
mod entry;
mod foreign;
mod heap;
#[cfg(feature = "mcp")]
mod mcp;
mod mstp;
mod mstp_thread;
mod multicast;
mod output;
#[cfg(feature = "tui")]
mod paths;
mod routed_paths;
mod sc_check;
mod timestamp;
mod transport;
#[cfg(feature = "tui")]
mod tui;

use crate::core::guard;
use crate::core::range::parse_discover_range;
use args::{Cli, Command};
use bip_commands::execute_bip_command;
use heap::boxed;
use output::OutputFormat;

fn resolve_format(cli: &Cli) -> OutputFormat {
    if cli.json {
        return OutputFormat::Json;
    }
    match cli.format.as_deref() {
        Some("json") => OutputFormat::Json,
        Some("table") => OutputFormat::Table,
        _ => {
            if std::io::stdout().is_terminal() {
                OutputFormat::Table
            } else {
                OutputFormat::Json
            }
        }
    }
}

/// A boxed future, as the one-shot dispatch's helpers return them (see
/// [`heap`]).
type Boxed<'a, T> = Pin<Box<dyn Future<Output = Result<T, Box<dyn std::error::Error>>> + 'a>>;

/// Resolve a target string to a MAC on this network, for commands without a
/// routed client call (see [`destination::resolve_mac`]). Boxed, as each
/// command is (see [`heap`]).
fn resolve_target_mac<'a, T: TransportPort + 'static>(
    client: &'a BACnetClient<T>,
    target_str: &'a str,
) -> Boxed<'a, Vec<u8>> {
    boxed(move || async move { Ok(destination::resolve_mac(client, target_str).await?) })
}

/// How long a routed target waits for its device to answer a Who-Is sent
/// to its network, as long as `discover` waits by default.
const ROUTED_DISCOVERY_WAIT: Duration = Duration::from_secs(3);

/// Undocumented: milliseconds to wait instead, so tests of a device that
/// never answers don't wait the full three seconds.
const ROUTED_WAIT_VAR: &str = "BACNET_CLI_ROUTED_WAIT_MS";

/// How long a routed target waits for its device: [`ROUTED_DISCOVERY_WAIT`],
/// unless the environment sets another wait for a test.
fn routed_wait() -> Duration {
    std::env::var(ROUTED_WAIT_VAR)
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map_or(ROUTED_DISCOVERY_WAIT, Duration::from_millis)
}

/// Resolve a device command's target string: besides an address or a
/// discovered instance, it takes `dnet:instance` and `dnet:0xMAC` for a
/// device behind a router. Boxed, as each command is (see [`heap`]).
fn resolve_destination<'a, T: TransportPort + 'static>(
    client: &'a BACnetClient<T>,
    target_str: &'a str,
) -> Boxed<'a, Destination> {
    boxed(move || async move { Ok(destination::resolve(client, target_str, routed_wait()).await?) })
}

/// Execute a one-shot CLI command.
async fn execute_command<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    cli: &Cli,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    // Plain `bacnet` became `tui` while parsing (see `entry`).
    let cmd = cli.command.as_ref().expect("a command after parsing");
    // `--format csv` is the inventory's alone; `--json` outranks it, as it
    // does `--format table`.
    let csv = !cli.json && cli.format.as_deref() == Some("csv");
    match cmd {
        Command::Tui { .. }
        | Command::Mcp { .. }
        | Command::ScCheck { .. }
        | Command::MstpPorts => {
            unreachable!()
        }
        Command::Discover {
            range,
            wait,
            target,
            bbmd,
            dnet,
            global,
        } => {
            if bbmd.is_some() {
                return Err(
                    "--bbmd requires BACnet/IP transport (do not use --sc or --ipv6)".into(),
                );
            }
            let range = parse_discover_range(range.as_deref())?;
            let scope =
                boxed(|| commands::discover::scope(client, target.as_deref(), *dnet, *global))
                    .await?;
            boxed(|| commands::discover::discover(client, &scope, range, *wait, format)).await?;
        }
        Command::Find {
            name,
            name_flag,
            wait,
            target,
            dnet,
            global,
        } => {
            // clap takes exactly one of the two.
            let name = name
                .as_ref()
                .or(name_flag.as_ref())
                .ok_or("find needs a NAME")?;
            let scope =
                boxed(|| commands::discover::scope(client, target.as_deref(), *dnet, *global))
                    .await?;
            boxed(|| commands::discover::find_by_name(client, name, &scope, *wait, format)).await?;
        }
        Command::Read {
            target,
            object,
            property,
        } => {
            let to = resolve_destination(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            let (prop, index) = parse::parse_property(property)?;
            boxed(|| {
                commands::read::read_property_cmd(
                    client,
                    &to,
                    object_type,
                    instance,
                    prop,
                    index,
                    format,
                )
            })
            .await?;
        }
        Command::Export { format } => {
            boxed(|| commands::export::export_cmd(client, format)).await?
        }
        Command::Inventory {
            target,
            types,
            values,
            settings,
            csv_exact,
        } => {
            let args = commands::inventory::InventoryArgs {
                target,
                types,
                values: *values,
                settings: *settings,
                csv_exact: *csv_exact,
                layout: commands::inventory::Layout::of(csv, format),
            };
            boxed(|| commands::inventory::inventory_cmd(client, args)).await?;
        }
        Command::Readm { target, specs } => {
            let to = resolve_destination(client, target).await?;
            boxed(|| commands::read::read_multiple_cmd(client, &to, specs, format)).await?;
        }
        Command::Write {
            target,
            object,
            property,
            value,
            priority,
        } => {
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            let (prop, index) = parse::parse_property(property)?;
            let (val, inline_priority) =
                parse::parse_value_for_with_priority(object_type, prop, index, value)?;
            // `72.5@8 --priority 10` names two: refused, never one picked.
            let pri = bacnet_ops::write::one_priority(inline_priority, *priority)?;
            let to = resolve_destination(client, target).await?;
            boxed(|| {
                commands::write::write_property_cmd(
                    client,
                    &to,
                    commands::write::WritePropertyArgs {
                        object_type,
                        instance,
                        property: prop,
                        index,
                        value: val,
                        priority: pri.map(bacnet_ops::Priority::get),
                    },
                    format,
                )
            })
            .await?;
        }
        Command::Writem { target, specs } => {
            let to = resolve_destination(client, target).await?;
            let specs = commands::write::parse_write_multiple(specs)?;
            boxed(|| commands::write::write_property_multiple_cmd(client, &to, specs, format))
                .await?;
        }
        Command::Subscribe {
            target,
            object,
            lifetime,
            confirmed,
        } => {
            let to = resolve_destination(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            let subscription = commands::subscribe::Subscription {
                object_type,
                instance,
                lifetime: *lifetime,
                confirmed: *confirmed,
            };
            boxed(|| {
                commands::subscribe::subscribe_cmd(
                    client,
                    &to,
                    subscription,
                    format,
                    commands::subscribe::stop_signal(),
                )
            })
            .await?;
        }
        Command::Control {
            target,
            action,
            duration,
            password,
        } => {
            let to = resolve_destination(client, target).await?;
            boxed(|| {
                commands::device::control_cmd(
                    client,
                    &to,
                    action,
                    *duration,
                    password.as_deref(),
                    format,
                )
            })
            .await?;
        }
        Command::Reinit {
            target,
            state,
            password,
        } => {
            let to = resolve_destination(client, target).await?;
            boxed(|| commands::device::reinit_cmd(client, &to, state, password.as_deref(), format))
                .await?;
        }
        Command::Alarms { target, max } => {
            let to = resolve_destination(client, target).await?;
            boxed(|| commands::alarms::alarms_cmd(client, &to, *max, format)).await?;
        }
        Command::AlarmSummary { target } => {
            let to = resolve_destination(client, target).await?;
            boxed(|| commands::alarms::alarm_summary_cmd(client, &to, format)).await?;
        }
        Command::EnrollmentSummary { target, filters } => {
            let to = resolve_destination(client, target).await?;
            boxed(|| commands::alarms::enrollment_summary_cmd(client, &to, filters, format))
                .await?;
        }
        Command::FileRead {
            target,
            file_instance,
            access,
            start,
            count,
            output,
        } => {
            let to = resolve_destination(client, target).await?;
            boxed(|| {
                commands::file::file_read_cmd(
                    client,
                    &to,
                    *file_instance,
                    commands::file::FileReadOptions {
                        access: *access,
                        start_position: *start,
                        count: *count,
                        output_path: output.as_deref(),
                    },
                    format,
                )
            })
            .await?;
        }
        Command::FileWrite {
            target,
            file_instance,
            start,
            input,
        } => {
            let to = resolve_destination(client, target).await?;
            boxed(|| {
                commands::file::file_write_cmd(client, &to, *file_instance, *start, input, format)
            })
            .await?;
        }
        Command::Bdt { .. }
        | Command::Fdt { .. }
        | Command::Register { .. }
        | Command::Unregister { .. } => {
            return Err("BBMD management commands (bdt, fdt, register, unregister) are only supported on BACnet/IP transport".into());
        }
        Command::WhoisRouter { dnet, wait } => {
            boxed(|| commands::router::whois_router_cmd(client, *dnet, *wait, format)).await?;
        }
        Command::RoutingTable { target, wait } => {
            let mac = resolve_target_mac(client, target).await?;
            boxed(|| commands::router::routing_table_cmd(client, &mac, *wait, format)).await?;
        }
        Command::AckAlarm {
            target,
            object,
            state,
            source,
            timestamp,
            ack_time,
        } => {
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            let to = resolve_destination(client, target).await?;
            boxed(|| {
                commands::device::acknowledge_alarm_cmd(
                    client,
                    &to,
                    commands::device::AcknowledgeAlarmArgs {
                        object_type,
                        instance,
                        event_state: bacnet_types::enums::EventState::from_raw(*state),
                        source,
                        timestamp: timestamp.clone(),
                        time_of_acknowledgment: ack_time.clone(),
                    },
                    format,
                )
            })
            .await?;
        }
        Command::ReadRange {
            target,
            object,
            property,
            range,
        } => {
            let to = resolve_destination(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            let (prop, index) = parse::parse_property(property)?;
            boxed(|| {
                commands::read_range::read_range_cmd(
                    client,
                    &to,
                    object_type,
                    instance,
                    prop,
                    index,
                    range,
                    format,
                )
            })
            .await?;
        }
        Command::CreateObject { target, object } => {
            let to = resolve_destination(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            boxed(|| {
                commands::device::create_object_cmd(client, &to, object_type, instance, format)
            })
            .await?;
        }
        Command::DeleteObject { target, object } => {
            let to = resolve_destination(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            boxed(|| {
                commands::device::delete_object_cmd(client, &to, object_type, instance, format)
            })
            .await?;
        }
        Command::Capture { .. } => {
            return Err("capture command should be handled before client setup".into());
        }
        Command::TimeSync { target, utc } => {
            let to = resolve_destination(client, target).await?;
            boxed(|| commands::device::time_sync_cmd(client, &to, *utc, format)).await?;
        }
    }
    Ok(())
}

/// Run the one-shot command and stop the client.
async fn run<T: TransportPort + 'static>(
    mut client: BACnetClient<T>,
    cli: &Cli,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let command = boxed(|| execute_command(&client, cli, format));
    boxed(|| routed_paths::around(&client, cli, format, command)).await?;
    client.stop().await?;
    Ok(())
}

type CliResult = Result<(), Box<dyn std::error::Error>>;

// The CLI's futures are polled on the main thread, whose stack is 1 MiB on
// Windows (8 MiB on Linux and macOS), and a debug build overflowed it on its
// first SC command (#950). So `cli_main`'s state lives on the heap. While it
// runs, the main thread's stack holds the box pointer, block_on's frames, and
// the poll frames of the futures and whatever they call. This builds the
// runtime `#[tokio::main]` would (multi-thread, every driver), without the
// attribute's temporary of the whole future in main's own frame.
//
// Clap's derived parser needed more than all of that: in a debug build,
// `Cli::parse()` took about 860 KiB of the main thread's stack on macOS, most
// of it one 620 KiB frame (`Command::augment_subcommands`, which builds every
// subcommand's arguments inline). So the command line is parsed on a thread of
// its own (#953). A read or a discover then needs about 240 KiB of the main
// thread's stack in a debug build, where it needed 860 KiB before.
//
// A fatal error is printed here, with Display (`report`), and exits 1 as an
// `Err` from `main` would; std's own report would use Debug (#2325).
fn main() -> std::process::ExitCode {
    let cli = parse_on_large_stack(|| {
        let mut matches = Cli::command().get_matches();
        // Capture provenance before FromArgMatches removes argument values.
        sc_check::validate(&matches).unwrap_or_else(|error| error.exit());
        let explicit_args = args::capture_sources(&matches).unwrap_or_else(|error| error.exit());
        let command_name = matches.subcommand_name().map(str::to_owned);
        let mut cli = Cli::from_arg_matches_mut(&mut matches).unwrap_or_else(|error| error.exit());
        cli.explicit_args = explicit_args;
        cli.command_name = command_name;
        // Plain `bacnet` opens the TUI, or exits 2 with the usage (#974).
        if cli.command.is_none() {
            let allowed = entry::tui_allowed_here(cli.no_tui);
            entry::resolve(&mut cli, allowed).unwrap_or_else(|error| error.exit());
        }
        cli
    });
    let result = dispatch(cli, &mut guard::Stdin, |cli| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed building the Runtime")
            .block_on(boxed_cli_main(cli))
    });
    result.unwrap_or_else(|error| {
        let _ = report(&*error, &mut std::io::stderr().lock());
        std::process::ExitCode::FAILURE
    })
}

/// Write a fatal error as `Error: ` and its Display text, then each source
/// whose text the line doesn't already hold, after `: `. Most errors here
/// put their source in their own message, and that isn't repeated. A
/// BACnet/SC connection failure ends with its error code by name (#2438).
fn report(
    error: &(dyn std::error::Error + 'static),
    out: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    let mut line = format!("Error: {error}");
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !line.contains(&text) {
            line.push_str(": ");
            line.push_str(&text);
        }
        source = cause.source();
    }
    if let Some(code) = sc_error_code(error) {
        line.push_str(&format!(" (BACnet/SC error code {code})"));
    }
    writeln!(out, "{line}")
}

/// The error code Annex AB.7.5 gives a failed BACnet/SC connection, when
/// `error` or one of its sources is one and a code applies.
fn sc_error_code(
    error: &(dyn std::error::Error + 'static),
) -> Option<bacnet_types::enums::ErrorCode> {
    use bacnet_transport::sc::{ScConnectError, ScDisconnectCause};
    let mut next = Some(error);
    while let Some(error) = next {
        if let Some(bacnet) = error.downcast_ref::<bacnet_types::error::Error>()
            && ScConnectError::from_error(bacnet).is_some()
        {
            return ScDisconnectCause::from_error(bacnet).error_code();
        }
        next = error.source();
    }
    None
}

/// The network path owns runtime/tracing/transport initialization. Keeping it
/// behind this boundary makes offline dispatch testable without any of them,
/// and the write guard runs here, before any of them (#972).
fn dispatch(
    cli: Cli,
    confirm: &mut dyn guard::Confirm,
    network: impl FnOnce(Cli) -> CliResult,
) -> Result<std::process::ExitCode, Box<dyn std::error::Error>> {
    if let Some(Command::ScCheck { at_unix }) = cli.command {
        let status = sc_check::execute(
            &cli,
            at_unix,
            resolve_format(&cli),
            &mut std::io::stdout().lock(),
            &mut std::io::stderr().lock(),
        );
        return Ok(std::process::ExitCode::from(status));
    }
    if let Some(Command::MstpPorts) = cli.command {
        let status = mstp::ports_cmd(
            mstp::list_ports(),
            resolve_format(&cli),
            &mut std::io::stdout().lock(),
            &mut std::io::stderr().lock(),
        );
        return Ok(std::process::ExitCode::from(status));
    }
    if let Err(refusal) = guard_one_shot(&cli, confirm) {
        // Errors go to stderr and leave stdout empty, in either format.
        match resolve_format(&cli) {
            OutputFormat::Json => eprintln!("{}", refusal.to_json()),
            OutputFormat::Table => output::print_error(&refusal.message),
        }
        return Ok(std::process::ExitCode::from(guard::EXIT_REFUSED));
    }
    network(cli).map(|()| std::process::ExitCode::SUCCESS)
}

/// The write guard for a one-shot command. `tui` is a read (plain `bacnet`
/// has become it while parsing): the TUI guards its own writes.
fn guard_one_shot(cli: &Cli, confirm: &mut dyn guard::Confirm) -> Result<(), guard::Refusal> {
    let Some(command) = &cli.command else {
        return Ok(());
    };
    let target = match command {
        Command::Control { target, .. } | Command::Reinit { target, .. } => Some(target.as_str()),
        _ => None,
    };
    // A name that wasn't captured classifies as nothing, so it is refused.
    let name = cli.command_name.as_deref().unwrap_or_default();
    let class = bacnet_tool::class::of(name).map(|class| match command {
        Command::Subscribe { lifetime, .. } => bacnet_tool::class::of_subscribe(*lifetime),
        _ => class,
    });
    guard::check(name, class, target, cli.policy(), confirm)
}

/// Stack for the thread that parses the command line; see `main`.
const PARSE_STACK_BYTES: usize = 8 << 20;

/// Run a clap parse on a thread with a stack of [`PARSE_STACK_BYTES`].
///
/// A parse that fails with `Cli::parse` prints its error or the help text and
/// exits the process from that thread, as it would on the main thread.
fn parse_on_large_stack<T: Send + 'static>(parse: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .name("bacnet-args".into())
        .stack_size(PARSE_STACK_BYTES)
        .spawn(parse)
        .expect("spawn the command-line parser thread")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// Create `cli_main`'s future and move it to the heap. The full-size temporary
/// that creating it takes is in this frame, which returns before polling
/// starts. The command line goes in boxed too, so the future holds a pointer
/// to it, not its copies of the whole `Cli`.
#[inline(never)]
fn boxed_cli_main(cli: Cli) -> std::pin::Pin<Box<impl std::future::Future<Output = CliResult>>> {
    Box::pin(cli_main(Box::new(cli)))
}

async fn cli_main(cli: Box<Cli>) -> CliResult {
    if matches!(cli.command, Some(Command::Tui { .. })) {
        // The TUI sets up its own tracing: nothing may write to the terminal
        // while it is in raw mode. Boxed so its state stays out of this future
        // and its poll frame (see `heap`), as each session below is.
        return boxed(|| run_tui(*cli)).await;
    }
    if matches!(cli.command, Some(Command::Mcp { .. })) {
        // Stdout is the MCP stream: it sets up its own tracing, on stderr.
        return boxed(|| run_mcp(*cli)).await;
    }
    output::setup_tracing(cli.verbose);
    let format = resolve_format(&cli);

    // A one-shot binds --interface, or every interface without it.
    let interface = cli.interface.unwrap_or(Ipv4Addr::UNSPECIFIED);
    let args = transport::TransportArgs::from_cli(&cli, interface, cli.broadcast)?;

    // Handle capture command separately — no BACnet client needed
    if let Some(Command::Capture {
        ref read,
        ref save,
        quiet,
        decode,
        ref device,
        ref filter,
        count,
        snaplen,
    }) = cli.command
    {
        #[cfg(feature = "pcap")]
        {
            let opts = commands::capture::CaptureOpts {
                read: read.clone(),
                save: save.clone(),
                quiet,
                decode,
                device: device.clone(),
                interface_ip: interface,
                port: cli.port,
                filter: filter.clone(),
                count,
                snaplen,
                format,
            };
            return commands::capture::run_capture(opts);
        }
        #[cfg(not(feature = "pcap"))]
        {
            let _ = (read, save, quiet, decode, device, filter, count, snaplen);
            eprintln!(
                "Error: Packet capture requires the 'pcap' feature. Rebuild with:\n  cargo install bacnet-cli --features pcap"
            );
            std::process::exit(1);
        }
    }

    if args.mstp.is_some() {
        return boxed(|| mstp::run(&cli, &args, format)).await;
    }
    if args.sc {
        #[cfg(feature = "sc-tls")]
        {
            let client = transport::build_sc_client(&args).await?;
            boxed(|| run(client, &cli, format)).await?;
        }
        #[cfg(not(feature = "sc-tls"))]
        {
            eprintln!(
                "Error: BACnet/SC requires the 'sc-tls' feature. Rebuild with: cargo install bacnet-cli --features sc-tls"
            );
            std::process::exit(1);
        }
    } else if args.ipv6 {
        let mut client = transport::build_bip6_client(&args).await?;
        // Broadcasts reach the remote network once the BBMD has taken the
        // registration (#2365, #2388).
        if let Some(foreign) = &args.bip6_foreign_device {
            match foreign::await_bip6_registration(&client, &foreign.bbmd).await {
                Ok(bbmd) => tracing::info!("BBMD {bbmd} accepted the foreign-device registration"),
                Err(e) => {
                    return Err(match client.stop().await {
                        Ok(()) => e,
                        Err(stop) => format!("{e}; stopping the client also failed: {stop}"),
                    }
                    .into());
                }
            }
        }
        boxed(|| run(client, &cli, format)).await?;
    } else {
        let mut client = transport::build_bip_client(&args).await?;
        let cmd = cli.command.as_ref().expect("a command after parsing");
        // Broadcasts reach the remote network once the BBMD has taken the
        // registration (#938).
        if let Some(bbmd) = args.foreign_device.as_ref().map(|config| &config.bbmd) {
            match foreign::await_registration(&client, bbmd).await {
                Ok(bbmd) => tracing::info!("BBMD {bbmd} accepted the foreign-device registration"),
                // The registration's failure is the error to report.
                Err(e) => {
                    return Err(match client.stop().await {
                        Ok(()) => e,
                        Err(stop) => format!("{e}; stopping the client also failed: {stop}"),
                    }
                    .into());
                }
            }
        }
        let command = boxed(|| async {
            if !boxed(|| execute_bip_command(&client, cmd, cli.ttl, format)).await? {
                boxed(|| execute_command(&client, &cli, format)).await?;
            }
            Ok(())
        });
        boxed(|| routed_paths::around(&client, &cli, format, command)).await?;
        client.stop().await?;
    }

    Ok(())
}

/// `bacnet tui`, or the rebuild advice when the `tui` feature is off.
async fn run_tui(cli: Cli) -> CliResult {
    #[cfg(feature = "tui")]
    {
        tui::run(cli).await
    }
    #[cfg(not(feature = "tui"))]
    {
        let _ = cli;
        eprintln!(
            "Error: The terminal UI requires the 'tui' feature. Rebuild with:\n  cargo install bacnet-cli --features tui"
        );
        std::process::exit(1);
    }
}

/// `bacnet mcp`, or the rebuild advice when the `mcp` feature is off.
async fn run_mcp(cli: Cli) -> CliResult {
    #[cfg(feature = "mcp")]
    {
        mcp::run(cli).await
    }
    #[cfg(not(feature = "mcp"))]
    {
        let _ = cli;
        eprintln!(
            "Error: The MCP server requires the 'mcp' feature. Rebuild with:\n  cargo install bacnet-cli --features mcp"
        );
        std::process::exit(1);
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

#[cfg(test)]
mod future_size_tests;
