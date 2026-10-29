//! Build the client for the chosen transport, then serve UI commands and
//! forward client notifications until the UI closes the command channel.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_ops::limits::RequestLimits;
use bacnet_ops::target::AddressStyle;
use bacnet_services::who_is::WhoIsRequest;
use bacnet_transport::port::TransportPort;
use bacnet_transport::sc::ScEvent;
use bacnet_types::enums::UnconfirmedServiceChoice;
use bacnet_types::error::Error;
use bytes::BytesMut;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::watch::WatchOptions;
use super::write::SafetyOptions;
use super::{COMMAND_CHANNEL_CAPACITY, DeviceFeed, EventSink, passive};
use crate::transport::{self, TransportArgs};
use crate::tui::message::{
    Command, DirectedTo, OpId, OpOutcome, WhoIsScope, WhoIsSpec, WorkerEvent,
};
use crate::tui::network::Foreign;
use crate::tui::point::BrowseResult;
#[cfg(feature = "sc-tls")]
use crate::tui::sc::{ScFailure, ScIdentity, ScReport, credentials::Credentials};

/// How often the worker checks whether the UI needs a resync.
const RESYNC_EVERY: Duration = Duration::from_secs(1);

/// Which client to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportKind {
    /// BACnet/IP.
    Bip,
    /// BACnet/IPv6.
    Bip6,
    /// BACnet/SC.
    Sc,
    /// MS/TP over a serial port.
    Mstp,
    /// Listening to an MS/TP trunk, transmitting nothing (#970).
    MstpMonitor,
}

impl TransportKind {
    /// Status bar label.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Bip => "BIP",
            Self::Bip6 => "BIP6",
            Self::Sc => "SC",
            Self::Mstp | Self::MstpMonitor => "MSTP",
        }
    }

    /// How this transport's MACs are shown.
    pub(crate) fn style(self) -> AddressStyle {
        match self {
            Self::Bip => AddressStyle::Bip,
            Self::Bip6 => AddressStyle::Bip6,
            Self::Sc => AddressStyle::Hex,
            Self::Mstp | Self::MstpMonitor => AddressStyle::Mstp,
        }
    }
}

/// What the worker needs to connect.
#[derive(Clone)]
pub(crate) struct ConnectPlan {
    /// The transport.
    pub(crate) kind: TransportKind,
    /// The global flags.
    pub(crate) args: TransportArgs,
    /// Wait for a [`Command::Connect`] from the interface picker first.
    pub(crate) await_interface: bool,
}

/// A session's request limits and watch list settings.
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionOptions {
    /// Confirmed requests a second across the browser and the watch list
    /// (0: no cap).
    pub(crate) max_rate: u32,
    /// The watch list's settings.
    pub(crate) watch: WatchOptions,
    /// The write session's settings, `--passive` included.
    pub(crate) safety: SafetyOptions,
    /// The MS/TP monitor's plan and where serial ports are opened.
    pub(crate) mstp: crate::tui::monitor::MstpOptions,
}

/// Capacity of the session's queue to the watch worker: a burst of adds
/// and removes must not be refused, since a lost remove would leave a
/// subscription until the session ends.
const WATCH_CHANNEL_CAPACITY: usize = 128;

/// Start the worker task. Boxed so the transport futures never sit on a
/// small stack. An error means monitor input release is unproven and
/// the coordinator must refuse any replacement.
pub(crate) fn spawn(
    plan: ConnectPlan,
    sink: EventSink,
    commands: mpsc::Receiver<Command>,
    options: SessionOptions,
) -> JoinHandle<Result<(), String>> {
    tokio::spawn(Box::pin(connect(plan, sink, commands, options)))
}

async fn connect(
    plan: ConnectPlan,
    sink: EventSink,
    commands: mpsc::Receiver<Command>,
    mut options: SessionOptions,
) -> Result<(), String> {
    // The monitor transmits nothing, so it runs with `--passive` too.
    if plan.kind == TransportKind::MstpMonitor {
        return match options.mstp.monitor.clone() {
            Some(monitor) => {
                let lines = &options.mstp.lines;
                Box::pin(crate::tui::monitor::worker::serve(
                    monitor,
                    lines,
                    &sink,
                    commands,
                    &options.mstp.scan,
                ))
                .await
            }
            None => {
                failed(&sink, Error::Encoding("no MS/TP port to monitor".into())).await;
                Ok(())
            }
        };
    }
    if options.safety.passive {
        Box::pin(passive::serve(&plan, &sink, commands, &options.safety)).await;
        return Ok(());
    }
    let args = plan.args;
    // The CLI's clients keep bacnet-client's default of three retries.
    options.watch.request_budget = Duration::from_millis(args.timeout_ms) * 4;
    let style = plan.kind.style();
    match plan.kind {
        TransportKind::Bip => {
            if let Some(mut client) =
                finish_build(Box::pin(transport::build_bip_client(&args)), &sink).await
            {
                if let Some(foreign) = &args.foreign_device {
                    let bbmd = std::net::SocketAddrV4::new(foreign.bbmd_ip, foreign.bbmd_port);
                    let outcome = tokio::select! {
                        biased;
                        () = sink.cancelled() => None,
                        result = crate::foreign::await_registration(&client, bbmd) => Some(result),
                    };
                    let Some(outcome) = outcome else {
                        if let Err(error) = client.stop().await {
                            tracing::warn!("client stop: {error}");
                        }
                        return Ok(());
                    };
                    match outcome {
                        Ok(()) => tracing::info!(
                            "BBMD {bbmd} accepted the foreign-device registration; it renews automatically"
                        ),
                        Err(error) => tracing::warn!("{error}; retrying in the background"),
                    }
                    // Accepted or retrying, the BBMD may hold an FDT entry
                    // for us until its TTL runs out: part of the footprint.
                    let renewal = foreign
                        .renewal_interval
                        .unwrap_or(Duration::from_secs(u64::from(foreign.ttl)) / 2);
                    let foreign = Foreign {
                        bbmd,
                        ttl: foreign.ttl,
                        renewal,
                    };
                    sink.deliver(WorkerEvent::ForeignDevice(foreign)).await;
                }
                Box::pin(serve(client, style, sink, commands, options)).await;
            }
        }
        TransportKind::Bip6 => {
            if let Some(client) =
                finish_build(Box::pin(transport::build_bip6_client(&args)), &sink).await
            {
                Box::pin(serve(client, style, sink, commands, options)).await;
            }
        }
        #[cfg(feature = "sc-tls")]
        TransportKind::Sc => {
            if let Some(identity) = sc_identity(&args) {
                sink.deliver(WorkerEvent::Sc(ScReport::Dialing(identity)))
                    .await;
            }
            if let Some(credentials) = sc_credentials(&args) {
                let report = ScReport::Credentials(Box::new(credentials));
                sink.deliver(WorkerEvent::Sc(report)).await;
            }
            // The doctor reads the typed error before it becomes text. The
            // events are subscribed before the first connect, for the
            // timeline's first entry.
            let mut events = None;
            let build = async {
                let built = match transport::sc_client_builder(&args) {
                    Ok(builder) => builder.build_with_events().await,
                    Err(error) => Err(error),
                };
                match built {
                    Ok((client, subscribed)) => {
                        events = Some(subscribed);
                        Ok(client)
                    }
                    Err(error) => {
                        let failure = Box::new(ScFailure::from_error(&error));
                        sink.deliver(WorkerEvent::Sc(ScReport::Failed(failure)))
                            .await;
                        Err(error)
                    }
                }
            };
            if let Some(client) = finish_build(Box::pin(build), &sink).await {
                let serving = serve_with(client, style, sink, commands, options, events);
                Box::pin(serving).await;
            }
        }
        #[cfg(not(feature = "sc-tls"))]
        TransportKind::Sc => {
            failed(
                &sink,
                Error::Encoding("BACnet/SC requires the 'sc-tls' feature".into()),
            )
            .await;
        }
        TransportKind::Mstp => {
            let mut handle = None;
            let lines = options.mstp.lines.clone();
            let thread = options.mstp.thread.clone();
            let build = async {
                let (client, diagnostics) = lines.participant(&args, &thread).await?;
                handle = Some(diagnostics);
                Ok(client)
            };
            let built = finish_build(Box::pin(build), &sink).await;
            // A built client always came with its handle, from settings.
            if let (Some(client), Some(handle), Some(station)) = (built, handle, args.mstp.clone())
            {
                tracing::info!(
                    "joined the MS/TP trunk on {} as station {}",
                    station.port,
                    station.mac
                );
                let thread = crate::mstp_thread::status(handle.execution_status().as_ref());
                let report = crate::tui::mstp::MstpReport::Joined(station, thread);
                sink.deliver(WorkerEvent::Mstp(report)).await;
                let sampler = crate::tui::mstp::sample(handle, sink.clone());
                // The sampler never finishes, so the session always does,
                // and stops its client; the sampler is dropped with it.
                tokio::select! {
                    biased;
                    () = Box::pin(serve(client, style, sink, commands, options)) => {}
                    () = sampler => {}
                }
            }
        }
        // Handled above, before the passive check.
        TransportKind::MstpMonitor => {}
    }
    Ok(())
}

/// What an SC session dials, for the SC screen. `build_sc_client` needs the
/// VMAC and device UUID (a flag or the profile's persisted identity; there
/// is no random-VMAC path), so `None` only when the build refuses anyway.
#[cfg(feature = "sc-tls")]
fn sc_identity(args: &TransportArgs) -> Option<ScIdentity> {
    Some(ScIdentity {
        primary: args.sc_url.clone()?,
        failover: args.sc_failover_url.clone(),
        vmac: args.sc_vmac?,
        uuid: args.sc_device_uuid?,
    })
}

/// This node's credentials for the SC screen (#968), read by the same lint
/// that `build_sc_client` applies next; `None` without a certificate path,
/// which the build refuses anyway.
///
/// For a profile, the coordinator's `prepare` checked the files a moment
/// earlier and the operator confirmed its warnings. This reads them again
/// (as the build does), so a warning that appears in between is shown here
/// and logged by the build but never confirmed. That window is accepted by
/// design: hard failures are refused again by the build, and flags-only
/// sessions have no `prepare` result to reuse.
#[cfg(feature = "sc-tls")]
fn sc_credentials(args: &TransportArgs) -> Option<Result<Credentials, String>> {
    let lint = crate::sc_check::lint::Lint::run(
        args.sc_cert.as_deref()?,
        args.sc_ca.as_deref(),
        args.sc_key.as_deref(),
        None,
    );
    Some(lint.map(|lint| Credentials::from_lint(&lint)))
}

/// Retain a partially completed builder through cancellation: it may already
/// own resources. Finish construction, then stop fully without publishing it.
pub(crate) async fn finish_build<T: TransportPort + 'static>(
    builder: Pin<Box<impl Future<Output = Result<BACnetClient<T>, Error>>>>,
    sink: &EventSink,
) -> Option<BACnetClient<T>> {
    match builder.await {
        Ok(mut client) if sink.is_cancelled() => {
            if let Err(error) = client.stop().await {
                tracing::warn!("client stop: {error}");
            }
            None
        }
        Ok(client) => Some(client),
        Err(error) => {
            failed(sink, error).await;
            None
        }
    }
}

async fn failed(sink: &EventSink, error: Error) {
    tracing::error!("connect failed: {error}");
    sink.deliver(WorkerEvent::ConnectFailed {
        error: crate::tui::message::Failure::of(&error),
    })
    .await;
}

/// A directed Who-Is waiting for the VMAC of the B/IPv6 node it goes to.
///
/// The worker loop polls the lookup beside its other branches, so Cancel,
/// device events and a newer Who-Is don't wait the up to three seconds a
/// Virtual-Address-Resolution can take (U.5). The lookup holds no lock while
/// it waits; dropping it gives up the wait.
struct Resolving<'a> {
    op: OpId,
    spec: WhoIsSpec,
    vmac: Pin<Box<dyn Future<Output = Result<Vec<u8>, String>> + Send + 'a>>,
}

/// The VMAC `resolving` waits for. Never ready while there is none, so the
/// loop's branch for it stays idle.
async fn resolved(resolving: &mut Option<Resolving<'_>>) -> Result<Vec<u8>, String> {
    match resolving {
        Some(lookup) => lookup.vmac.as_mut().await,
        None => std::future::pending().await,
    }
}

/// Serve one connected client until the UI drops its command sender.
///
/// The client is shared with the watch list's renewal tasks while the
/// session runs. The watch worker stops every one of them before it
/// returns, so the client is this function's alone again when it stops it.
pub(crate) async fn serve<T: TransportPort + 'static>(
    client: BACnetClient<T>,
    style: AddressStyle,
    sink: EventSink,
    commands: mpsc::Receiver<Command>,
    options: SessionOptions,
) {
    Box::pin(serve_with(client, style, sink, commands, options, None)).await;
}

/// [`serve`], with the SC hub-connector events when they were subscribed
/// before the client started, so the SC worker sees the first connect.
async fn serve_with<T: TransportPort + 'static>(
    client: BACnetClient<T>,
    style: AddressStyle,
    sink: EventSink,
    mut commands: mpsc::Receiver<Command>,
    options: SessionOptions,
    sc_events: Option<broadcast::Receiver<ScEvent>>,
) {
    let client = Arc::new(client);
    let rate = Arc::new(RequestLimits::new(options.max_rate));
    let mut local = style.format(client.local_mac());
    // A B/IPv6 VMAC alone doesn't say where the client is bound.
    if let Some(bound) =
        bacnet_ops::target::as_bip6(client.transport()).and_then(|t| t.local_address())
    {
        local = format!("{local} ({bound})");
    }
    tracing::info!("connected; local address {local}");
    let mut devices = client.device_events();
    let mut collisions = client.device_collision_events();
    if sink
        .deliver(WorkerEvent::Connected {
            local: local.clone(),
        })
        .await
    {
        let (browse, browse_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let (watch, watch_rx) = mpsc::channel(WATCH_CHANNEL_CAPACITY);
        let (safety, safety_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let (network, network_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let (discovery, discovery_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let (capture, capture_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        // Dropped with the session loop, which ends the SC worker.
        let (sc_stop, sc_stop_rx) = mpsc::channel::<()>(1);
        let quiet = options.safety.passive;
        // The browser and the watch list run beside this loop under
        // `join!`, so their jobs keep being polled while an arm here awaits;
        // the loop ending drops `browse` and `watch`, which ends them.
        let session = async {
            let browse = browse;
            let watch = watch;
            let safety = safety;
            let network = network;
            let discovery = discovery;
            let capture = capture;
            let _sc_stop = sc_stop;
            let mut feed = DeviceFeed::new(style);
            let mut listening: Option<(OpId, Instant)> = None;
            let mut resolving: Option<Resolving<'_>> = None;
            let mut resync = tokio::time::interval(RESYNC_EVERY);
            resync.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                let deadline = listening.map_or_else(far_future, |(_, until)| until);
                tokio::select! {
                    biased;
                    () = sink.cancelled() => break,
                    command = commands.recv() => match command {
                        Some(command) if quiet && passive::transmits(&command) => {
                            passive::refuse(&sink, command).await;
                        }
                        Some(Command::Safety(command)) => {
                            // Polled beside this loop, so waiting can't deadlock.
                            if safety.send(command).await.is_err() {
                                tracing::debug!("the write session has ended");
                            }
                        }
                        Some(Command::WhoIs { op, spec }) => {
                            let superseded = listening.take().map(|(old, _)| old);
                            let superseded = superseded.or(resolving.take().map(|lookup| lookup.op));
                            if let Some(old) = superseded {
                                sink.deliver(WorkerEvent::OpFinished { op: old, outcome: OpOutcome::Cancelled }).await;
                            }
                            if let WhoIsScope::Directed { to: DirectedTo::Bip6(peer), .. } = &spec.scope {
                                let vmac = Box::pin(bacnet_ops::target::bip6_vmac(client.transport(), *peer));
                                resolving = Some(Resolving { op, spec, vmac });
                            } else {
                                listening = who_is(&client, &sink, op, &spec).await;
                            }
                        }
                        Some(Command::Cancel { op }) => {
                            let running = listening.is_some_and(|(id, _)| id == op)
                                || resolving.as_ref().is_some_and(|lookup| lookup.op == op);
                            if running {
                                listening = None;
                                resolving = None;
                                tracing::info!("Who-Is {op} cancelled");
                                sink.deliver(WorkerEvent::OpFinished { op, outcome: OpOutcome::Cancelled }).await;
                            }
                        }
                        Some(command @ (Command::Browse { .. } | Command::Names { .. } | Command::StopBrowse { .. })) => {
                            if let Err(refused) = browse.try_send(command) {
                                let refused = refused.into_inner();
                                if let Command::StopBrowse { device } = refused {
                                    tracing::debug!("browser busy; dropped the stop for device {device}");
                                }
                                if let Command::Browse { req, .. } | Command::Names { req, .. } = refused {
                                    let result = BrowseResult::Failed("the browser is busy; try again".into());
                                    sink.deliver(WorkerEvent::Browse { req, result }).await;
                                }
                            }
                        }
                        Some(command @ Command::Watch(_)) => {
                            // The watch worker is polled beside this loop, so
                            // waiting for room can't deadlock; it only fails
                            // once the worker has ended with the session.
                            if watch.send(command).await.is_err() {
                                tracing::debug!("the watch list has ended");
                            }
                        }
                        Some(Command::Network(command)) => {
                            // Polled beside this loop too; it queues requests.
                            if network.send(command).await.is_err() {
                                tracing::debug!("the network worker has ended");
                            }
                        }
                        Some(Command::Discovery(command)) => {
                            // Polled beside this loop too; each send is one datagram.
                            if discovery.send(command).await.is_err() {
                                tracing::debug!("the discovery worker has ended");
                            }
                        }
                        Some(Command::Capture(command)) => {
                            // Polled beside this loop too; a Start is one subscription.
                            if capture.send(command).await.is_err() {
                                tracing::debug!("the capture has ended");
                            }
                        }
                        Some(Command::MonitorScan(request)) => {
                            sink.deliver(WorkerEvent::MonitorScan(crate::tui::monitor::scan::ScanReport::Rejected { id: request.id(), cancel: matches!(request, crate::tui::monitor::scan::ScanCommand::Cancel { .. }), reason: "only a receive-only MS/TP monitor can scan".into() })).await;
                        }
                        Some(Command::Disconnect) => break,
                        Some(Command::Connect { .. } | Command::JoinTrunk { .. } | Command::Reconnect | Command::Profile(_) | Command::CaptureExport(_)) => {}
                        None => break,
                    },
                    vmac = resolved(&mut resolving), if resolving.is_some() => {
                        if let Some(Resolving { op, spec, .. }) = resolving.take() {
                            listening = match vmac {
                                Ok(mac) => who_is(&client, &sink, op, &spec.with_vmac(mac)).await,
                                Err(error) => {
                                    failed_who_is(&sink, op, error).await;
                                    None
                                }
                            };
                        }
                    }
                    received = devices.recv(), if feed.devices_open => feed.on_device(&sink, received),
                    received = collisions.recv(), if feed.collisions_open => {
                        if let Some(event) = feed.on_collision(&sink, received) {
                            sink.deliver(event).await;
                        }
                    }
                    () = tokio::time::sleep_until(deadline), if listening.is_some() => {
                        if let Some((op, _)) = listening.take() {
                            sink.deliver(WorkerEvent::OpFinished { op, outcome: OpOutcome::Completed }).await;
                        }
                    }
                    _ = resync.tick(), if feed.needs_resync => {
                        let rows = client.discovered_devices().await.iter().map(|d| feed.row(d)).collect();
                        if sink.offer(WorkerEvent::Snapshot(rows)) == super::Offer::Sent {
                            feed.needs_resync = false;
                        }
                    }
                }
            }
            // A write request queued behind the end of the session still
            // reaches the write worker, which records it as not sent.
            while let Ok(command) = commands.try_recv() {
                if let Command::Safety(command) = command {
                    let _ = safety.send(command).await;
                }
            }
        };
        tokio::join!(
            Box::pin(session),
            Box::pin(super::browse::run(&client, &sink, browse_rx, &rate)),
            Box::pin(super::watch::run(
                &client,
                &sink,
                watch_rx,
                &rate,
                &options.watch
            )),
            Box::pin(super::write::run(
                &client,
                &sink,
                safety_rx,
                &rate,
                &options.safety,
                &local
            )),
            Box::pin(super::network::run(&client, &sink, network_rx)),
            Box::pin(super::discovery::run(&client, &sink, discovery_rx, style)),
            Box::pin(super::sc::run(&client, &sink, sc_stop_rx, sc_events)),
            Box::pin(crate::tui::capture::worker::run(
                &*client, &sink, capture_rx, style
            ))
        );
    }
    stop_shared(client).await;
}

/// How long [`stop_shared`] waits quietly for the renewal tasks to let go
/// of the client, and then how long more after warning.
const RELEASE_WAIT: (Duration, Duration) = (Duration::from_secs(1), Duration::from_secs(10));

/// Stop the client once no renewal task holds it.
///
/// The watch worker has told every renewal task to stop before this runs,
/// and a stopping task lets go of the client as it ends. Dropping the client
/// instead of stopping it would leave the transport's sockets to whatever
/// task still held them, so a reconnect could find the port taken: this
/// waits for the tasks, warning after a second. Only a task that never
/// ends would make it give up, with an error, after the longer wait.
async fn stop_shared<T: TransportPort + 'static>(mut client: Arc<BACnetClient<T>>) {
    let start = Instant::now();
    let mut warned = false;
    loop {
        match Arc::try_unwrap(client) {
            Ok(mut owned) => {
                if let Err(error) = owned.stop().await {
                    tracing::warn!("client stop: {error}");
                }
                return;
            }
            Err(shared) => client = shared,
        }
        let waited = start.elapsed();
        if waited >= RELEASE_WAIT.0 + RELEASE_WAIT.1 {
            tracing::error!(
                "a COV renewal task still holds the client after {} s; dropping it unstopped",
                waited.as_secs()
            );
            return;
        }
        if waited >= RELEASE_WAIT.0 && !warned {
            warned = true;
            tracing::warn!("waiting for a COV renewal task to release the client");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn far_future() -> Instant {
    Instant::now() + Duration::from_secs(86_400)
}

/// Send a Who-Is; on success the listen window to wait out.
async fn who_is<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    sink: &EventSink,
    op: OpId,
    spec: &WhoIsSpec,
) -> Option<(OpId, Instant)> {
    let result = tokio::select! {
        biased;
        () = sink.cancelled() => return None,
        result = send_who_is(client, spec) => result,
    };
    match result {
        Ok(()) => {
            tracing::info!("Who-Is {op} sent ({})", spec.describe());
            sink.deliver(WorkerEvent::WhoIsSent { op }).await;
            Some((op, Instant::now() + spec.listen))
        }
        Err(error) => {
            failed_who_is(sink, op, error).await;
            None
        }
    }
}

async fn failed_who_is(sink: &EventSink, op: OpId, error: String) {
    tracing::warn!("Who-Is {op} failed: {error}");
    sink.deliver(WorkerEvent::OpFinished {
        op,
        outcome: OpOutcome::Failed(error),
    })
    .await;
}

/// Send the Who-Is `spec` describes. A directed one to a B/IPv6 address
/// must have had its VMAC found first ([`WhoIsSpec::with_vmac`]).
async fn send_who_is<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    spec: &WhoIsSpec,
) -> Result<(), String> {
    let range = spec.range;
    let sent = match &spec.scope {
        WhoIsScope::Local => {
            let mut buf = BytesMut::new();
            WhoIsRequest { range }.encode(&mut buf);
            client
                .broadcast_unconfirmed(UnconfirmedServiceChoice::WHO_IS, &buf)
                .await
        }
        WhoIsScope::Global => client.who_is(range).await,
        WhoIsScope::Directed {
            to: DirectedTo::Mac(mac),
            ..
        } => client.who_is_directed(mac, range).await,
        WhoIsScope::Directed {
            to: DirectedTo::Bip6(peer),
            ..
        } => return Err(format!("the VMAC at {peer} has not been found")),
        WhoIsScope::Network(dnet) => client.who_is_network(*dnet, range).await,
    };
    sent.map_err(|error| error.to_string())
}
