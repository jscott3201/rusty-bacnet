//! `bacnet tui`: the full-screen terminal UI.
//!
//! Layout of this module tree (see `docs/design/tui.md`):
//!
//! - [`app`]: the TEA core. `App`, `Action` and a pure `update` that returns
//!   `Command`s. No I/O, no transport generics.
//! - [`view`]: rendering, pure functions of `&App`.
//! - [`keymap`]: keys to actions.
//! - [`event_loop`]: the `select!` loop over input, frames, worker events and
//!   signals.
//! - [`worker`]: the only code generic over `TransportPort`. It owns the
//!   client and talks to the UI over bounded channels ([`message`]).
//! - [`terminal`]: the TTY guard, raw mode, the panic hook and signals.
//! - [`log_layer`]: tracing into an in-memory ring for the log pane.
//!
//! - [`point`]: the device browser's data, shared by the worker and the app.
//! - [`watched`]: the watch list's messages.
//! - [`safety`]: write sessions: levels, tiers, the ledger and the audit log.
//! - [`network`]: the Network screen's messages and its BDT consistency
//!   matrix (B/IP foreign-device status, BDT and FDT).
//! - [`sc`]: the SC screen's messages and the connection doctor (#967).
//! - [`mstp`]: the MS/TP screen's messages and diagnostics sampling (#969).
//! - [`monitor`]: the MS/TP passive monitor, its analysis and the safe-join
//!   hand-off (#970).
//! - [`capture`]: live capture for the Capture screen, its sources, ring,
//!   filter and pcap export (#971).
//!
//! The TUI is read-only by default: discovery, the device browser's reads,
//! the watch list's reads and finite COV subscriptions, and finite
//! foreign-device registrations. Writes need a write session armed with
//! `W` on a profile that allows it; `--passive` transmits nothing at all.

mod app;
mod capture;
mod event_loop;
mod help;
mod keymap;
mod log_layer;
mod message;
// Without `serial` no port opens; the MS/TP screen still draws.
#[cfg_attr(not(feature = "serial"), allow(dead_code))]
mod monitor;
// Without `serial` no session joins a trunk; the MS/TP screen still draws.
#[cfg_attr(not(feature = "serial"), allow(dead_code))]
mod mstp;
mod network;
mod point;
pub(crate) mod profile;
mod row_error;
mod safety;
// Without `sc-tls` nothing reports to the SC screen; it still draws.
#[cfg_attr(not(feature = "sc-tls"), allow(dead_code))]
mod sc;
mod terminal;
mod view;
mod watched;
mod worker;

#[cfg(test)]
mod tests;

use std::io::Write as _;
use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use ratatui::crossterm::event::EventStream;
use tokio::sync::mpsc;

pub(crate) use self::terminal::unusable_terminal;

use self::app::picker::Picker;
use self::app::{App, AppConfig, Link, Overlay};
use self::event_loop::LoopIo;
use self::log_layer::{LOG_CAPACITY, LogRing, RingLayer};
use self::terminal::Signals;
use self::worker::coordinator;
use self::worker::session::{ConnectPlan, SessionOptions, TransportKind};
use self::worker::watch::WatchOptions;
use self::worker::write::SafetyOptions;
use self::worker::{COMMAND_CHANNEL_CAPACITY, EVENT_CHANNEL_CAPACITY, EventSink};
use crate::args::{Cli, Command};
use crate::core::interfaces::list_ipv4_interfaces;
use crate::transport::TransportArgs;

/// When to report that shutdown is taking longer than expected.
/// The join remains owned and is always completed.
const STOP_TIMEOUT: Duration = Duration::from_secs(3);

/// Run `bacnet tui`. Returns only on a clean quit; errors are printed after
/// the terminal is restored and the process exits non-zero.
pub(crate) async fn run(cli: Cli) -> crate::CliResult {
    if let Err(hint) = terminal::check_terminal() {
        eprintln!("{hint}");
        std::process::exit(1);
    }
    let monitor = monitor::plan_from(&cli);
    let kind = if monitor.is_some() {
        TransportKind::MstpMonitor
    } else if cli.mstp.is_some() {
        TransportKind::Mstp
    } else if cli.sc {
        TransportKind::Sc
    } else if cli.ipv6 {
        TransportKind::Bip6
    } else {
        TransportKind::Bip
    };
    let (capture_ring, capture_device) = match &cli.command {
        Some(Command::Tui {
            capture_ring,
            capture_device,
            ..
        }) => (*capture_ring as usize, capture_device.clone()),
        _ => (capture::DEFAULT_RING, None),
    };
    if capture_device.is_some() && !cfg!(feature = "pcap") {
        eprintln!(
            "Error: --capture-device requires the 'pcap' feature. Rebuild with: cargo install bacnet-cli --features pcap"
        );
        std::process::exit(1);
    }
    let (fps, log_file, mut options) = match &cli.command {
        Some(Command::Tui {
            fps,
            log_file,
            max_rate,
            cov_lifetime,
            poll_interval,
            passive,
            ..
        }) => (
            *fps,
            log_file.clone(),
            SessionOptions {
                max_rate: *max_rate,
                watch: WatchOptions {
                    lifetime: *cov_lifetime,
                    poll_every: Duration::from_secs(*poll_interval),
                    workspace: None,
                    request_budget: worker::watch::DEFAULT_REQUEST_BUDGET,
                },
                safety: SafetyOptions {
                    passive: *passive,
                    ..SafetyOptions::default()
                },
                mstp: Default::default(),
            },
        ),
        _ => (
            20,
            None,
            SessionOptions {
                max_rate: worker::browse::DEFAULT_MAX_RATE,
                watch: WatchOptions::default(),
                safety: SafetyOptions::default(),
                mstp: Default::default(),
            },
        ),
    };
    options.safety.transport = kind.label();
    if kind == TransportKind::Sc && !cfg!(feature = "sc-tls") {
        eprintln!(
            "Error: BACnet/SC requires the 'sc-tls' feature. Rebuild with: cargo install bacnet-cli --features sc-tls"
        );
        std::process::exit(1);
    }
    options.mstp.monitor.clone_from(&monitor);
    options.mstp.thread = crate::mstp_thread::from_cli(&cli);
    let serial = matches!(kind, TransportKind::Mstp | TransportKind::MstpMonitor);
    if serial && !cfg!(feature = "serial") {
        eprintln!("Error: {}", crate::mstp::NO_SERIAL);
        std::process::exit(1);
    }

    let log = LogRing::new(LOG_CAPACITY);
    if let Err(error) = init_tracing(cli.verbose, log_file.as_deref(), &log) {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }

    let (plan, profiles, connection, picker, join) = if let Some((service, connection)) =
        profile::startup::open(&cli, profile::store::Store::discover)
            .map_err(std::io::Error::other)?
    {
        (None, Some(service), Some(connection), None, None)
    } else {
        // A serial monitor needs no IP interface selection.
        let (interface, broadcast, picker) = if monitor.is_some() {
            let interface = cli.interface.unwrap_or(Ipv4Addr::UNSPECIFIED);
            (interface, cli.broadcast, None)
        } else {
            choose_interface(&cli, kind, &log)
        };
        let args =
            TransportArgs::from_cli(&cli, interface, broadcast).map_err(std::io::Error::other)?;
        // Joining a trunk waits for its confirmation (T2), which can also
        // pick another port; a passive session joins nothing.
        let join = args
            .mstp
            .clone()
            .filter(|_| !options.safety.passive)
            .map(|settings| {
                app::mstp::JoinDialog::for_flags(
                    settings,
                    crate::mstp::list_ports().unwrap_or_default(),
                )
            });
        let plan = ConnectPlan {
            kind,
            args,
            await_interface: picker.is_some() || join.is_some(),
        };
        (Some(plan), None, None, picker, join)
    };
    let color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    let mut app = Box::new(App::new(
        AppConfig {
            transport: kind.label(),
            style: kind.style(),
            color,
            picker,
            join,
            passive: options.safety.passive,
            now: tokio::time::Instant::now().into_std(),
        },
        log,
    ));

    if let Some(connection) = connection {
        app.connection = Some(connection);
        app.link = Link::Down;
        // No transport until a profile connects: the status bar says so
        // rather than naming B/IP before an SC or MS/TP profile (#1894).
        app.transport = "";
        app.overlay = Overlay::Connection;
    }
    let thread = &options.mstp.thread;
    app.mstp_thread = crate::mstp_thread::describe(thread.realtime_priority, &thread.cpu_affinity);
    // A profile session warns when its profile connects, if it isn't MS/TP.
    if app.connection.is_none()
        && crate::mstp_thread::given(&cli)
        && !matches!(kind, TransportKind::Mstp | TransportKind::MstpMonitor)
    {
        crate::mstp_thread::warn_unused(kind.label());
    }
    if let Some(plan) = &monitor {
        app.start_monitor(plan, !options.safety.passive);
    }
    app.capture = app::capture::CaptureState::new(capture_ring);

    let signals = Signals::install()?;
    // Before any network traffic: a terminal that passes the TTY check can
    // still refuse raw mode (mintty and MSYS ptys on Windows).
    let mut terminal = match terminal::enter() {
        Ok(terminal) => terminal,
        Err(error) => {
            eprintln!(
                "{}",
                terminal::unusable_hint(&format!("raw mode failed: {error}"))
            );
            std::process::exit(1);
        }
    };
    let (sink, events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
    let dropped = sink.dropped_counter();
    let (commands, command_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
    // libpcap runs for the whole TUI, beside the sessions; its task ends
    // when the event channel closes, and is joined below.
    #[cfg(feature = "pcap")]
    let wire = capture_device.map(|device| {
        let interface = cli.interface.unwrap_or(Ipv4Addr::UNSPECIFIED);
        capture::worker::spawn_pcap(Some(device), interface, cli.port, sink.clone())
    });
    #[cfg(not(feature = "pcap"))]
    let wire: Option<tokio::task::JoinHandle<()>> = {
        // Refused above: without `pcap` there is no wire capture.
        drop(capture_device);
        None
    };
    let mut worker = coordinator::spawn(plan, profiles, sink, command_rx, options);
    let mut io = LoopIo {
        input: EventStream::new(),
        events,
        commands,
        dropped,
        fps,
        signals,
        alive: terminal::is_active,
    };
    let result = Box::pin(event_loop::run(&mut terminal, &mut app, &mut io)).await;
    // Leave raw mode first: `Terminal`'s drop can report a cursor error on stderr.
    terminal::restore();
    drop(terminal);

    // Closing the command channel tells the worker to stop the client. The
    // signal handlers stay installed for the life of the process, so keep
    // listening while waiting: otherwise Ctrl-C, now SIGINT again, would be
    // swallowed until the timeout.
    let LoopIo {
        input,
        events,
        commands,
        mut signals,
        ..
    } = io;
    drop((input, events, commands));
    let mut signal = None;
    let notice = tokio::time::sleep(STOP_TIMEOUT);
    tokio::pin!(notice);
    let mut warned = false;
    loop {
        tokio::select! {
            joined = &mut worker => {
                if let Err(error) = joined { app.fail(format!("network coordinator failed: {error}"), 1); }
                break;
            }
            _ = &mut notice, if !warned => {
                warned = true;
                tracing::warn!("waiting for the network worker to release its resources");
            }
            received = signals.recv() => { signal.get_or_insert(received); }
        }
    }
    if let Some(wire) = wire
        && let Err(error) = wire.await
    {
        app.fail(format!("packet capture failed: {error}"), 1);
    }
    let (error, code) = match (result, signal) {
        (Err(error), _) => (Some(error.to_string()), 1),
        (Ok(_), Some(signal)) if app.exit_error.is_none() => {
            (Some(format!("terminated by {}", signal.name)), signal.code)
        }
        (Ok(_), _) => (app.exit_error.take(), app.exit_code.unwrap_or(1)),
    };
    if let Some(error) = error {
        // stderr may be gone (SIGHUP), so a failed write is not a panic.
        let _ = writeln!(std::io::stderr(), "Error: {error}");
        std::process::exit(code);
    }
    Ok(())
}

/// The BACnet/IP interface: `-i`, the only one there is, or the picker.
fn choose_interface(
    cli: &Cli,
    kind: TransportKind,
    log: &LogRing,
) -> (Ipv4Addr, Ipv4Addr, Option<Picker>) {
    if let Some(interface) = cli.interface {
        return (interface, cli.broadcast, None);
    }
    if kind != TransportKind::Bip {
        return (Ipv4Addr::UNSPECIFIED, cli.broadcast, None);
    }
    let explicit_broadcast = cli.explicit_args.iter().any(|id| id == "broadcast");
    let mut interfaces = list_ipv4_interfaces();
    if explicit_broadcast {
        for interface in &mut interfaces {
            interface.broadcast = cli.broadcast;
        }
    }
    match interfaces.len() {
        0 => {
            log.info("No network interfaces found; binding to 0.0.0.0.");
            (Ipv4Addr::UNSPECIFIED, cli.broadcast, None)
        }
        1 => {
            let only = interfaces.remove(0);
            log.info(format!(
                "Using interface {} ({}, broadcast {}).",
                only.name, only.ip, only.broadcast
            ));
            (only.ip, only.broadcast, None)
        }
        _ => (
            Ipv4Addr::UNSPECIFIED,
            cli.broadcast,
            Some(Picker::new(interfaces)),
        ),
    }
}

/// Route tracing to the log pane's ring and, with `--log-file`, to a file.
/// Nothing goes to stdout or stderr while the terminal is in raw mode.
fn init_tracing(verbosity: u8, log_file: Option<&Path>, ring: &LogRing) -> Result<(), String> {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let level = match verbosity {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let file_layer = match log_file {
        Some(path) => {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| format!("cannot open --log-file {}: {e}", path.display()))?;
            Some(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_target(false)
                    .with_writer(Mutex::new(file)),
            )
        }
        None => None,
    };
    tracing_subscriber::registry()
        .with(EnvFilter::new(level))
        .with(RingLayer::new(ring.clone()))
        .with(file_layer)
        .try_init()
        .map_err(|e| e.to_string())
}
