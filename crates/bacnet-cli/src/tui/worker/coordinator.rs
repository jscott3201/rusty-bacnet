//! One retained owner for connection attempts, commands and complete retirement.

use super::session::{ConnectPlan, SessionOptions, TransportKind};
use super::write::{SafetyOptions, keep_offline};
use super::{COMMAND_CHANNEL_CAPACITY, EventSink, session};
use crate::tui::capture::worker::TAP;
use crate::tui::capture::{CaptureEvent, ExportJob, pcap};
use crate::tui::message::{Command, OpId, WorkerEvent};
use crate::tui::network::NetworkEvent;
use crate::tui::network::discovery::DiscoveryEvent;
use crate::tui::point::{BrowseResult, ReqId};
use crate::tui::profile::service::{Reply, Request, Service};
use crate::tui::safety::{Level, SafetyCommand, SafetyEvent};
use crate::tui::watched::{Source, WatchCommand, WatchEvent};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

struct Active {
    scan: crate::tui::monitor::scan::ScanGate,
    cancel: watch::Sender<bool>,
    commands: mpsc::Sender<Command>,
    task: JoinHandle<Result<(), String>>,
}

/// The coordinator lives until the UI drops its command channel. It never
/// detaches an attempt or starts a replacement before the old one is joined.
/// Every session runs with `options`; a profile's adds its workspace.
pub(crate) fn spawn(
    plan: Option<ConnectPlan>,
    profiles: Option<Service>,
    sink: EventSink,
    commands: mpsc::Receiver<Command>,
    options: SessionOptions,
) -> JoinHandle<()> {
    tokio::spawn(Box::pin(run(plan, profiles, sink, commands, options)))
}

async fn start(
    plan: &ConnectPlan,
    generation: u64,
    sink: &EventSink,
    options: &SessionOptions,
) -> Option<Active> {
    let (cancel, cancelled) = watch::channel(false);
    let (commands, command_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
    if !sink
        .deliver(WorkerEvent::SessionStarted { generation })
        .await
    {
        return None;
    }
    let scoped = sink.for_session(generation, cancelled);
    let mut options = options.clone();
    options.mstp.scan = Default::default();
    let scan = options.mstp.scan.clone();
    let task = session::spawn(plan.clone(), scoped, command_rx, options);
    Some(Active {
        scan,
        cancel,
        commands,
        task,
    })
}

async fn retire(active: &mut Option<Active>) -> Result<(), String> {
    if let Some(Active {
        cancel,
        commands,
        task,
        ..
    }) = active.take()
    {
        cancel.send_replace(true);
        drop(commands);
        task.await
            .map_err(|error| format!("network session panicked: {error}"))??;
    }
    Ok(())
}

async fn completed(
    active: &mut Option<Active>,
) -> Result<Result<(), String>, tokio::task::JoinError> {
    match active {
        Some(active) => (&mut active.task).await,
        None => std::future::pending().await,
    }
}

async fn run(
    mut plan: Option<ConnectPlan>,
    profiles: Option<Service>,
    sink: EventSink,
    mut commands: mpsc::Receiver<Command>,
    mut options: SessionOptions,
) {
    let mut generation = 0;
    let mut active = None;
    let mut exports: Vec<JoinHandle<()>> = Vec::new();
    if let Some(plan) = &plan
        && !plan.await_interface
    {
        generation += 1;
        active = start(plan, generation, &sink, &options).await;
    }
    loop {
        tokio::select! {
            done = completed(&mut active), if active.is_some() => {
                active = None;
                let result = done.map_err(|error| format!("network worker failed: {error}")).and_then(|result| result);
                if let Err(error) = result {
                    sink.deliver(WorkerEvent::Fatal { error }).await;
                    break;
                }
                // A failed attempt already reported its error. Keep the
                // coordinator alive so the user can edit and retry.
            }
            command = commands.recv() => {
                let Some(command) = command else { break; };
                if active.as_ref().is_some_and(|active| active.task.is_finished())
                    && let Err(error) = retire(&mut active).await {
                        sink.deliver(WorkerEvent::Fatal { error }).await;
                        break;
                    }
                match command {
                    Command::Disconnect => {
                        if let Err(error) = retire(&mut active).await {
                            sink.deliver(WorkerEvent::Fatal { error }).await;
                            break;
                        }
                        sink.deliver(WorkerEvent::Disconnected { generation }).await;
                    }
                    Command::WhoIs { op, .. } | Command::Cancel { op } => {
                        forward_operation(active.as_ref(), command, op, generation, &sink).await;
                    }
                    Command::Browse { req, .. } | Command::Names { req, .. } => {
                        forward_browse(active.as_ref(), command, req, generation, &sink).await;
                    }
                    Command::Watch(watch) => {
                        forward_watch(active.as_ref(), watch, generation, &sink).await;
                    }
                    Command::Safety(request) => {
                        forward_safety(active.as_ref(), request, generation, &sink, &options.safety).await;
                    }
                    Command::Network(request) => {
                        let refused = |reason| NetworkEvent::Refused { command: request, reason };
                        let refused = move |reason| WorkerEvent::Network(refused(reason));
                        forward_or_refuse(active.as_ref(), command, generation, &sink, refused).await;
                    }
                    Command::Discovery(ref request) => {
                        let command_copy = request.clone();
                        let refused = |reason| DiscoveryEvent::Refused { command: command_copy, reason };
                        let refused = move |reason| WorkerEvent::Discovery(refused(reason));
                        forward_or_refuse(active.as_ref(), command, generation, &sink, refused).await;
                    }
                    Command::MonitorScan(request) => {
                        forward_scan(active.as_ref(), plan.as_ref(), request, generation, &sink).await;
                    }
                    Command::Capture(_) => {
                        let refused = |why| WorkerEvent::Capture(CaptureEvent::Ended { what: TAP.into(), why });
                        forward_or_refuse(active.as_ref(), command, generation, &sink, refused).await;
                    }
                    Command::CaptureExport(job) => {
                        // A profile's workspace, or the working directory.
                        let dir = options.watch.workspace.clone().unwrap_or_else(|| ".".into());
                        exports.retain(|export| !export.is_finished());
                        exports.push(tokio::spawn(export(*job, dir, sink.clone())));
                    }
                    Command::StopBrowse { .. } => {
                        // Best effort: a stale browse result is dropped by the UI anyway.
                        if let Some(active) = &active
                            && let Err(error) = active.commands.try_send(command) {
                                tracing::debug!("browse stop not admitted: {error}");
                            }
                    }
                    Command::Profile(request) => {
                        let Some(profiles) = &profiles else {
                            sink.deliver(WorkerEvent::Profile(Err("profiles are disabled in this flags-only session".into()))).await;
                            continue;
                        };
                        // Saving/loading another configuration while active
                        // would make the displayed selection differ from the
                        // live connection. Retire explicitly first.
                        if active.is_some() {
                            sink.deliver(WorkerEvent::Profile(Err("disconnect before changing or connecting a profile".into()))).await;
                            continue;
                        }
                        let result = match request {
                            Request::List => profiles.catalog(),
                            Request::Load(name) => profiles.load(&name),
                            Request::Save { name, profile, create, previous } => profiles.save(&name, profile, create, previous),
                            Request::Connect { name, profile, attest, accepted } => {
                                // Validation, file checks and authorization all run
                                // here, before a session (and so any socket) exists.
                                match profiles.prepare(&name, &profile, attest, &accepted).and_then(|prepared| Ok((prepared.resolved.transport_args()?, prepared))) {
                                    Ok((args, prepared)) => {
                                        // Publish the persisted snapshot used by this attempt,
                                        // without reloading a possibly newer external edit.
                                        let kind = prepared.stored.transport.kind();
                                        if kind != TransportKind::Mstp && options.mstp.thread != Default::default() {
                                            crate::mstp_thread::warn_unused(kind.label());
                                        }
                                        let workspace = profiles.workspace(&name);
                                        // The app sends an MS/TP profile's Connect only once
                                        // its join is confirmed: record that join first, and
                                        // change nothing when it can't be.
                                        if let Err(error) = record_join(workspace.as_deref(), &name, &options, &args).await {
                                            sink.deliver(WorkerEvent::Profile(Err(error))).await;
                                            continue;
                                        }
                                        options.watch.workspace = workspace;
                                        options.safety.workspace = options.watch.workspace.clone();
                                        options.safety.profile = Some(name.clone());
                                        options.safety.policy = prepared.stored.safety.clone();
                                        options.safety.transport = kind.label();
                                        let checks = crate::tui::profile::checks::inspect(&prepared.resolved);
                                        sink.deliver(WorkerEvent::Profile(Ok(Reply::Loaded { name, profile: prepared.stored, checks }))).await;
                                        let next = ConnectPlan { kind, args, await_interface: false };
                                        generation += 1;
                                        active = start(&next, generation, &sink, &options).await;
                                        plan = Some(next);
                                        continue;
                                    }
                                    Err(error) => Err(error),
                                }
                            }
                        };
                        sink.deliver(WorkerEvent::Profile(result)).await;
                    }
                    Command::Connect { interface, broadcast } => {
                        if active.is_some() { reject(&sink, "disconnect before reconnecting").await; continue; }
                        // Like Reconnect: a profile session connects only through
                        // its reviewed preview, never through the interface picker.
                        if profiles.is_some() { reject(&sink, "review the profile before connecting").await; continue; }
                        // MS/TP joins only through its confirmation (JoinTrunk), every time.
                        if plan.as_ref().is_some_and(|plan| plan.args.mstp.is_some()) { reject(&sink, JOIN_FIRST).await; continue; }
                        if let Some(plan) = &mut plan {
                            plan.args.interface = interface; plan.args.broadcast = broadcast;
                            plan.await_interface = false;
                            generation += 1;
                            active = start(plan, generation, &sink, &options).await;
                        } else { reject(&sink, "select a profile before connecting").await; }
                    }
                    Command::JoinTrunk { settings } => {
                        if active.as_ref().is_some_and(|active| active.scan.busy()) { reject(&sink, "finish or cancel the baud scan before joining").await; continue; }
                        // A profile joins through its reviewed preview and its own confirmation.
                        if profiles.is_some() { reject(&sink, "review the profile before joining").await; continue; }
                        if let Err(error) = settings.validate() { reject(&sink, &error).await; continue; }
                        let Some(next) = plan.as_mut() else { reject(&sink, NO_TRUNK).await; continue; };
                        match next.kind {
                            TransportKind::MstpMonitor if options.safety.passive => { reject(&sink, "passive: this session never joins the trunk").await; continue; }
                            TransportKind::MstpMonitor => {
                                // One owner of the port at a time: the monitor's
                                // session stops it and closes the port before
                                // `retire` returns, and only then does the
                                // participant open it.
                                if let Err(error) = retire(&mut active).await {
                                    sink.deliver(WorkerEvent::Fatal { error }).await;
                                    break;
                                }
                                next.kind = TransportKind::Mstp;
                            }
                            TransportKind::Mstp if active.is_some() => { reject(&sink, "disconnect before joining again").await; continue; }
                            TransportKind::Mstp => {}
                            _ => { reject(&sink, NO_TRUNK).await; continue; }
                        }
                        next.args.mstp = Some(settings);
                        next.await_interface = false;
                        generation += 1;
                        active = start(next, generation, &sink, &options).await;
                    }
                    Command::Reconnect => {
                        if profiles.is_some() { reject(&sink, "review the profile before reconnecting").await; continue; }
                        if active.is_some() { reject(&sink, "disconnect before reconnecting").await; continue; }
                        // MS/TP joins only through its confirmation (JoinTrunk), every time.
                        if plan.as_ref().is_some_and(|plan| plan.args.mstp.is_some()) { reject(&sink, JOIN_FIRST).await; continue; }
                        if let Some(plan) = &plan {
                            if plan.await_interface { reject(&sink, "choose an interface, or confirm joining the trunk, first").await; continue; }
                            generation += 1;
                            active = start(plan, generation, &sink, &options).await;
                        }
                    }
                }
            }
        }
    }
    if let Err(error) = retire(&mut active).await {
        tracing::error!("{error}");
    }
    // An export under way finishes its file before the TUI exits.
    for export in exports {
        if let Err(error) = export.await {
            tracing::error!("capture export failed: {error}");
        }
    }
}

/// Write a capture export on a blocking thread and report the files.
async fn export(job: ExportJob, dir: PathBuf, sink: EventSink) {
    let exported = tokio::task::spawn_blocking(move || pcap::export(&job, &dir))
        .await
        .unwrap_or_else(|error| pcap::Exported {
            written: Vec::new(),
            error: Some(format!("the export failed: {error}")),
        });
    if !exported.written.is_empty() {
        tracing::info!("capture exported to {}", exported.written.join(", "));
    }
    if let Some(error) = &exported.error {
        tracing::warn!("capture export: {error}");
    }
    sink.deliver(WorkerEvent::Capture(CaptureEvent::Exported(exported)))
        .await;
}

/// Record a profile session's MS/TP join in its workspace's audit log
/// (#1862). Nothing for another transport, or a passive session, which
/// never joins. A join that can't be recorded is not made.
async fn record_join(
    workspace: Option<&std::path::Path>,
    profile: &str,
    options: &SessionOptions,
    args: &crate::transport::TransportArgs,
) -> Result<(), String> {
    let (Some(settings), Some(dir)) = (&args.mstp, workspace) else {
        return Ok(());
    };
    if options.safety.passive {
        return Ok(());
    }
    crate::tui::safety::audit::record_join(dir, profile, settings, &options.mstp.thread)
        .await
        .map_err(|error| format!("the join was not made: {error}"))
}

/// Why a JoinTrunk is refused outside an MS/TP session.
const NO_TRUNK: &str = "this session has no MS/TP trunk to join";

/// Why an MS/TP plan refuses Connect and Reconnect.
const JOIN_FIRST: &str = "confirm joining the trunk first: an MS/TP session joins only on y";

async fn reject(sink: &EventSink, error: &str) {
    sink.deliver(WorkerEvent::Rejected {
        error: error.into(),
    })
    .await;
}

/// Forward across the second bounded queue, retaining operation/session identity.
async fn forward_operation(
    active: Option<&Active>,
    command: Command,
    op: OpId,
    generation: u64,
    sink: &EventSink,
) {
    let cancel = matches!(command, Command::Cancel { .. });
    let error = match active {
        Some(active) => active
            .commands
            .try_send(command)
            .err()
            .map(|e| format!("session command not admitted: {e}")),
        None => Some("no active connection".into()),
    };
    if let Some(error) = error {
        sink.deliver(WorkerEvent::Session {
            generation,
            event: Box::new(WorkerEvent::OpRejected { op, cancel, error }),
        })
        .await;
    }
}

/// Forward a browse read; one that can't be admitted fails at once, under
/// its request id, so the browser doesn't wait for it.
async fn forward_browse(
    active: Option<&Active>,
    command: Command,
    req: ReqId,
    generation: u64,
    sink: &EventSink,
) {
    let error = match active {
        Some(active) => active
            .commands
            .try_send(command)
            .err()
            .map(|e| format!("session command not admitted: {e}")),
        None => Some("no active connection".into()),
    };
    if let Some(error) = error {
        sink.deliver(WorkerEvent::Session {
            generation,
            event: Box::new(WorkerEvent::Browse {
                req,
                result: BrowseResult::Failed(error),
            }),
        })
        .await;
    }
}

/// How long a watch command waits for room in the session's queue.
pub(crate) const WATCH_SEND_WAIT: Duration = Duration::from_secs(5);

/// Forward a watch command, waiting for room: a lost remove would leave a
/// renewed subscription on the device with no row to show it, and a lost
/// add a row that never starts. The wait is bounded so a stuck session
/// can't hold up a disconnect; a command still refused then is reported.
/// Hand `command` to the session, or answer it with `refused(reason)`, so
/// the screen that asked stops waiting.
async fn forward_or_refuse(
    active: Option<&Active>,
    command: Command,
    generation: u64,
    sink: &EventSink,
    refused: impl FnOnce(String) -> WorkerEvent,
) {
    let reason = match active {
        Some(active) => active
            .commands
            .try_send(command)
            .err()
            .map(|e| format!("session command not admitted: {e}")),
        None => Some("no active connection".into()),
    };
    if let Some(reason) = reason {
        let event = Box::new(refused(reason));
        sink.deliver(WorkerEvent::Session { generation, event })
            .await;
    }
}

async fn forward_watch(
    active: Option<&Active>,
    command: WatchCommand,
    generation: u64,
    sink: &EventSink,
) {
    let sent = match active {
        Some(active) => matches!(
            tokio::time::timeout(
                WATCH_SEND_WAIT,
                active.commands.send(Command::Watch(command.clone()))
            )
            .await,
            Ok(Ok(()))
        ),
        None => false,
    };
    if sent {
        return;
    }
    let event = match command {
        WatchCommand::Add { row, .. } => WatchEvent::Source {
            row,
            source: Source::Failed("the session did not take the request".into()),
        },
        WatchCommand::Remove { row } => {
            tracing::warn!(
                "watch row {row}: the session did not take the remove; its subscription ends with the session"
            );
            return;
        }
        WatchCommand::Csv(_) => WatchEvent::Notice("The session did not take the request.".into()),
    };
    sink.deliver(WorkerEvent::Session {
        generation,
        event: Box::new(WorkerEvent::Watch(event)),
    })
    .await;
}

/// Forward a write session request with the same bounded wait as a watch
/// command: a lost `Keep` or `Disarm` would leave the audit log short. One
/// still refused is answered, so the UI doesn't wait for it. A `Keep` no
/// session takes (disconnected) is done here, from the profile workspace.
async fn forward_safety(
    active: Option<&Active>,
    request: SafetyCommand,
    generation: u64,
    sink: &EventSink,
    options: &SafetyOptions,
) {
    let sent = match active {
        Some(active) => matches!(
            tokio::time::timeout(
                WATCH_SEND_WAIT,
                active.commands.send(Command::Safety(request.clone()))
            )
            .await,
            Ok(Ok(()))
        ),
        None => false,
    };
    if sent {
        return;
    }
    if request == SafetyCommand::Keep {
        let level = if options.passive {
            Level::Passive
        } else {
            Level::ReadOnly
        };
        return keep_offline(options, level).await;
    }
    let req = match request {
        SafetyCommand::Prepare { req, .. }
        | SafetyCommand::Execute { req, .. }
        | SafetyCommand::Manage { req, .. }
        | SafetyCommand::Revert { req } => Some(req),
        _ => None,
    };
    tracing::warn!("write session request not taken: {request:?}");
    let reason = "no connected session took the request".into();
    sink.deliver(WorkerEvent::Session {
        generation,
        event: Box::new(WorkerEvent::Safety(SafetyEvent::Refused { req, reason })),
    })
    .await;
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;

/// Reserve at the coordinator boundary before another queued join can retire it.
async fn forward_scan(
    active: Option<&Active>,
    plan: Option<&ConnectPlan>,
    request: crate::tui::monitor::scan::ScanCommand,
    generation: u64,
    sink: &EventSink,
) {
    use crate::tui::monitor::{
        scan::{ScanCommand, ScanReport},
        scan_worker::valid_window,
    };
    let id = request.id();
    let cancel = matches!(request, ScanCommand::Cancel { .. });
    let reason = match active.filter(|_| plan.is_some_and(|p| p.kind == TransportKind::MstpMonitor))
    {
        None => Some("a scan needs a live receive-only MS/TP monitor".to_string()),
        Some(active) => {
            // Repeated delivery of the same Start is idempotent.
            if !cancel && active.scan.owns(id) {
                return;
            }
            let admitted = match request {
                ScanCommand::Start { id, window } => {
                    valid_window(window) && active.scan.reserve(id)
                }
                ScanCommand::Cancel { id } => active.scan.owns(id),
            };
            if !admitted {
                Some("another scan owns the monitor, or the window is invalid".into())
            } else if let Err(error) = active.commands.try_send(Command::MonitorScan(request)) {
                if !cancel {
                    active.scan.release(id);
                }
                Some(format!("scan command was not admitted: {error}"))
            } else {
                None
            }
        }
    };
    if let Some(reason) = reason {
        sink.deliver(WorkerEvent::Session {
            generation,
            event: Box::new(WorkerEvent::MonitorScan(ScanReport::Rejected {
                id,
                cancel,
                reason,
            })),
        })
        .await;
    }
}
