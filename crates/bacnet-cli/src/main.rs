//! BACnet command-line tool.
//!
//! `bacnet tui`, or plain `bacnet` on a terminal, opens the full-screen
//! terminal UI (the default `tui` feature; see [`entry`]). Every other
//! subcommand is a one-shot command for scripts, with `--json` output.
#![allow(clippy::print_stdout, clippy::print_stderr)] // a command-line tool prints its results
#![forbid(unsafe_code)]

use std::{io::IsTerminal, net::Ipv4Addr};

use bacnet_client::client::BACnetClient;
use bacnet_transport::{bip::BipTransport, port::TransportPort};
use clap::{CommandFactory, FromArgMatches};

mod args;
mod commands;
mod core;
// The one-shot capture (`pcap`) and the TUI's Capture screen each use a
// part of it.
#[cfg_attr(not(all(feature = "pcap", feature = "tui")), allow(dead_code))]
mod decode;
mod entry;
mod foreign;
mod mstp;
mod mstp_thread;
mod multicast;
mod output;
mod parse;
#[cfg(feature = "tui")]
mod paths;
mod sc_check;
mod timestamp;
mod transport;
#[cfg(feature = "tui")]
mod tui;

use crate::core::destination::{self, Destination};
use crate::core::guard;
use crate::core::range::parse_discover_range;
use crate::core::target;
use args::{Cli, Command};
use output::OutputFormat;

fn setup_tracing(verbosity: u8, to_stderr: bool) {
    use tracing_subscriber::EnvFilter;
    let filter = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(false);
    if to_stderr {
        // SC close/handshake diagnostics, and a foreign device's refused or
        // lapsed registration, must not corrupt a JSON result.
        subscriber.with_writer(std::io::stderr).init();
    } else {
        subscriber.init();
    }
}

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

/// Resolve a target string to a MAC on this network, for commands without a
/// routed client call (see [`destination::resolve_mac`]).
async fn resolve_target_mac<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    target_str: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    Ok(destination::resolve_mac(client, target_str).await?)
}

/// Resolve a target string for a command with a routed client call: read,
/// readm, write and subscribe also take `dnet:instance` and `dnet:0xMAC`.
async fn resolve_destination<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    target_str: &str,
) -> Result<Destination, Box<dyn std::error::Error>> {
    Ok(destination::resolve(client, target_str, destination::routed_wait()).await?)
}

/// Execute a one-shot CLI command.
async fn execute_command<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    cmd: &Command,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        Command::Tui { .. } | Command::ScCheck { .. } | Command::MstpPorts => {
            unreachable!()
        }
        Command::Discover {
            range,
            wait,
            target,
            bbmd,
            dnet,
            ..
        } => {
            if bbmd.is_some() {
                return Err(
                    "--bbmd requires BACnet/IP transport (do not use --sc or --ipv6)".into(),
                );
            }
            let range = parse_discover_range(range.as_deref())?;
            if let Some(target_str) = target {
                let mac = target::address_mac(
                    client.transport(),
                    target_str,
                    target::TARGET_NOT_AN_ADDRESS,
                )
                .await?;
                commands::discover::discover_directed(client, &mac, range, *wait, format).await?;
            } else if let Some(network) = dnet {
                commands::discover::discover_network(client, *network, range, *wait, format)
                    .await?;
            } else {
                commands::discover::discover(client, range, *wait, format).await?;
            }
        }
        Command::Find {
            name,
            name_flag,
            wait,
        } => {
            // clap takes exactly one of the two.
            let name = name
                .as_ref()
                .or(name_flag.as_ref())
                .ok_or("find needs a NAME")?;
            commands::discover::find_by_name(client, name, *wait, format).await?;
        }
        Command::Read {
            target,
            object,
            property,
        } => {
            let to = resolve_destination(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            let (prop, index) = parse::parse_property(property)?;
            commands::read::read_property_cmd(
                client,
                &to,
                object_type,
                instance,
                prop,
                index,
                format,
            )
            .await?;
        }
        Command::Readm { target, specs } => {
            let to = resolve_destination(client, target).await?;
            commands::read::read_multiple_cmd(client, &to, specs, format).await?;
        }
        Command::Write {
            target,
            object,
            property,
            value,
            priority,
        } => {
            let to = resolve_destination(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            let (prop, index) = parse::parse_property(property)?;
            let (val, inline_priority) = parse::parse_value_with_priority(value)?;
            let pri = priority.or(inline_priority);
            commands::write::write_property_cmd(
                client,
                &to,
                commands::write::WritePropertyArgs {
                    object_type,
                    instance,
                    property: prop,
                    index,
                    value: val,
                    priority: pri,
                },
                format,
            )
            .await?;
        }
        Command::Writem { target, specs } => {
            let to = resolve_destination(client, target).await?;
            let specs = commands::write::parse_write_multiple(specs)?;
            commands::write::write_property_multiple_cmd(client, &to, specs, format).await?;
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
            commands::subscribe::subscribe_cmd(
                client,
                &to,
                subscription,
                format,
                commands::subscribe::stop_signal(),
            )
            .await?;
        }
        Command::Control {
            target,
            action,
            duration,
            password,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::device::control_cmd(
                client,
                &mac,
                action,
                *duration,
                password.as_deref(),
                format,
            )
            .await?;
        }
        Command::Reinit {
            target,
            state,
            password,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::device::reinit_cmd(client, &mac, state, password.as_deref(), format).await?;
        }
        Command::Alarms { target } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::device::alarms_cmd(client, &mac, format).await?;
        }
        Command::FileRead {
            target,
            file_instance,
            access,
            start,
            count,
            output,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::file::file_read_cmd(
                client,
                &mac,
                *file_instance,
                commands::file::FileReadOptions {
                    access: *access,
                    start_position: *start,
                    count: *count,
                    output_path: output.as_deref(),
                },
                format,
            )
            .await?;
        }
        Command::FileWrite {
            target,
            file_instance,
            start,
            input,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::file::file_write_cmd(client, &mac, *file_instance, *start, input, format)
                .await?;
        }
        Command::Bdt { .. }
        | Command::Fdt { .. }
        | Command::Register { .. }
        | Command::Unregister { .. } => {
            return Err("BBMD management commands (bdt, fdt, register, unregister) are only supported on BACnet/IP transport".into());
        }
        Command::WhoisRouter { dnet, wait } => {
            commands::router::whois_router_cmd(client, *dnet, *wait, format).await?;
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
            let mac = resolve_target_mac(client, target).await?;
            commands::device::acknowledge_alarm_cmd(
                client,
                &mac,
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
            .await?;
        }
        Command::ReadRange {
            target,
            object,
            property,
            range,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            let (prop, index) = parse::parse_property(property)?;
            commands::read_range::read_range_cmd(
                client,
                &mac,
                object_type,
                instance,
                prop,
                index,
                range,
                format,
            )
            .await?;
        }
        Command::CreateObject { target, object } => {
            let mac = resolve_target_mac(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            commands::device::create_object_cmd(client, &mac, object_type, instance, format)
                .await?;
        }
        Command::DeleteObject { target, object } => {
            let mac = resolve_target_mac(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            commands::device::delete_object_cmd(client, &mac, object_type, instance, format)
                .await?;
        }
        Command::Capture { .. } => {
            return Err("capture command should be handled before client setup".into());
        }
        Command::TimeSync { target, utc } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::device::time_sync_cmd(client, &mac, *utc, format).await?;
        }
    }
    Ok(())
}

/// Try to execute a BIP-specific BBMD management command.
/// Returns `Ok(true)` if handled, `Ok(false)` if not a BIP-specific command.
async fn execute_bip_command(
    client: &BACnetClient<BipTransport>,
    cmd: &Command,
    ttl: u16,
    format: OutputFormat,
) -> Result<bool, Box<dyn std::error::Error>> {
    match cmd {
        Command::Discover {
            range,
            wait,
            target,
            bbmd: Some(bbmd_addr),
            dnet,
        } => {
            let bbmd_mac = target::address_mac(
                client.transport(),
                bbmd_addr,
                "--bbmd requires an IP address, not a device instance",
            )
            .await?;
            client.register_foreign_device_bvlc(&bbmd_mac, ttl).await?;
            eprintln!("Registered as foreign device with BBMD {bbmd_addr}");
            // Brief pause to allow registration to propagate.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let range = parse_discover_range(range.as_deref())?;
            if let Some(target_str) = target {
                let mac = target::address_mac(
                    client.transport(),
                    target_str,
                    target::TARGET_NOT_AN_ADDRESS,
                )
                .await?;
                commands::discover::discover_directed(client, &mac, range, *wait, format).await?;
            } else if let Some(network) = dnet {
                commands::discover::discover_network(client, *network, range, *wait, format)
                    .await?;
            } else {
                commands::discover::discover(client, range, *wait, format).await?;
            }
        }
        Command::Bdt { target } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::router::bdt_cmd(client, &mac, format).await?;
        }
        Command::Fdt { target } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::router::fdt_cmd(client, &mac, format).await?;
        }
        Command::Register { target } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::router::register_cmd(client, &mac, ttl, format).await?;
        }
        Command::Unregister { target } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::router::unregister_cmd(client, &mac, format).await?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Run the one-shot command and stop the client.
async fn run<T: TransportPort + 'static>(
    mut client: BACnetClient<T>,
    cli: &Cli,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    // Plain `bacnet` became `tui` while parsing (see `entry`).
    let cmd = cli.command.as_ref().expect("a command after parsing");
    execute_command(&client, cmd, format).await?;
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
fn main() -> Result<std::process::ExitCode, Box<dyn std::error::Error>> {
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
    dispatch(cli, &mut guard::Stdin, |cli| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed building the Runtime")
            .block_on(boxed_cli_main(cli))
    })
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
    let class = core::class::of(name).map(|class| match command {
        Command::Subscribe { lifetime, .. } => core::class::of_subscribe(*lifetime),
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
/// starts.
#[inline(never)]
fn boxed_cli_main(cli: Cli) -> std::pin::Pin<Box<impl std::future::Future<Output = CliResult>>> {
    Box::pin(cli_main(cli))
}

async fn cli_main(cli: Cli) -> CliResult {
    if matches!(cli.command, Some(Command::Tui { .. })) {
        // The TUI sets up its own tracing: nothing may write to the terminal
        // while it is in raw mode. Boxed so its state stays out of this future.
        return Box::pin(run_tui(cli)).await;
    }
    setup_tracing(
        cli.verbose,
        cli.sc || cli.foreign_bbmd.is_some() || cli.mstp.is_some(),
    );
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
            eprintln!("Error: Packet capture requires the 'pcap' feature. Rebuild with:\n  cargo install bacnet-cli --features pcap");
            std::process::exit(1);
        }
    }

    if args.mstp.is_some() {
        return mstp::run(&cli, &args, format).await;
    }
    if args.sc {
        #[cfg(feature = "sc-tls")]
        {
            let client = transport::build_sc_client(&args).await?;
            run(client, &cli, format).await?;
        }
        #[cfg(not(feature = "sc-tls"))]
        {
            eprintln!("Error: BACnet/SC requires the 'sc-tls' feature. Rebuild with: cargo install bacnet-cli --features sc-tls");
            std::process::exit(1);
        }
    } else if args.ipv6 {
        let client = transport::build_bip6_client(&args).await?;
        run(client, &cli, format).await?;
    } else {
        let mut client = transport::build_bip_client(&args).await?;
        let cmd = cli.command.as_ref().expect("a command after parsing");
        // Broadcasts reach the remote network once the BBMD has taken the
        // registration (#938).
        if let Some(bbmd) = cli.foreign_bbmd {
            if let Err(e) = foreign::await_registration(&client, bbmd).await {
                // The registration's failure is the error to report.
                return Err(match client.stop().await {
                    Ok(()) => e,
                    Err(stop) => format!("{e}; stopping the client also failed: {stop}"),
                }
                .into());
            }
        }
        if !execute_bip_command(&client, cmd, cli.ttl, format).await? {
            execute_command(&client, cmd, format).await?;
        }
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
        eprintln!("Error: The terminal UI requires the 'tui' feature. Rebuild with:\n  cargo install bacnet-cli --features tui");
        std::process::exit(1);
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
