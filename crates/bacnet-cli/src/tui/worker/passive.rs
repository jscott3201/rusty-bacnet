//! `--passive`: a session that transmits nothing.
//!
//! In production a passive session builds no client at all, so nothing can
//! reach the wire, not even an automatic answer to an incoming request. As
//! a second line, the session loop of a built client also refuses anything
//! that transmits when its options say passive.

use bacnet_transport::bip::PassiveBip;
use std::{io, net::SocketAddrV4};
use tokio::sync::mpsc;

use super::EventSink;
use super::write::{PASSIVE, SafetyOptions, keep_offline, ready_offline};
use crate::tui::capture::CaptureEvent;
use crate::tui::message::{Command, OpOutcome, WorkerEvent};
use crate::tui::network::NetworkEvent;
use crate::tui::network::discovery::DiscoveryEvent;
use crate::tui::point::BrowseResult;
use crate::tui::safety::{Level, SafetyCommand, SafetyEvent};
use crate::tui::watched::{Source, WatchCommand, WatchEvent};

/// Bind one receive-only B/IP listener, list the ledger, and stream decoded
/// observations while refusing transmission requests until the session ends.
pub(crate) async fn serve(
    plan: &super::session::ConnectPlan,
    sink: &EventSink,
    commands: mpsc::Receiver<Command>,
    options: &SafetyOptions,
) {
    let opened = async {
        if plan.kind != super::session::TransportKind::Bip || plan.args.multicast.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "passive discovery supports B/IP broadcast/unicast only; multicast, IPv6, SC and participant MS/TP are unsupported",
            ));
        }
        bacnet_transport::bip::PassiveBip::bind(plan.args.interface, plan.args.port).await
    };
    let listener = tokio::select! {
        biased;
        () = sink.cancelled() => return,
        () = sink.closed() => return,
        result = opened => match result {
            Ok(listener) => listener,
            Err(error) => { failed(sink, error).await; return; }
        },
    };
    listen(
        listener,
        plan.args.foreign_device.is_some(),
        sink,
        commands,
        options,
    )
    .await;
}

/// The receive-only I/O boundary, permitting deterministic receive-error tests.
trait ReceiveOnly {
    fn local_addr(&self) -> io::Result<SocketAddrV4>;
    fn groups(&self) -> io::Result<bacnet_bip_core::node::Groups>;
    async fn recv_from(&self, buffer: &mut [u8]) -> io::Result<(usize, SocketAddrV4)>;
}
impl ReceiveOnly for PassiveBip {
    fn groups(&self) -> io::Result<bacnet_bip_core::node::Groups> {
        self.groups()
    }
    fn local_addr(&self) -> io::Result<SocketAddrV4> {
        self.local_addr()
    }
    async fn recv_from(&self, buffer: &mut [u8]) -> io::Result<(usize, SocketAddrV4)> {
        self.recv_from(buffer).await
    }
}
async fn listen(
    listener: impl ReceiveOnly,
    foreign_ignored: bool,
    sink: &EventSink,
    mut commands: mpsc::Receiver<Command>,
    options: &SafetyOptions,
) {
    let groups = match listener.groups() {
        Ok(groups) => groups,
        Err(error) => {
            failed(sink, error).await;
            return;
        }
    };
    let local = match listener.local_addr() {
        Ok(local) => local.to_string(),
        Err(error) => {
            failed(sink, error).await;
            return;
        }
    };
    tracing::info!(%local, "passive B/IP listening; no BACnet send capability");
    if !sink
        .deliver(WorkerEvent::PassiveListening {
            local,
            foreign_ignored,
        })
        .await
    {
        return;
    }
    ready_offline(sink, options).await;
    let mut buffer = vec![0; 65_535];
    loop {
        tokio::select! {
            biased;
            () = sink.cancelled() => break,
            () = sink.closed() => break,
            command = commands.recv() => match command {
                None | Some(Command::Disconnect) => break,
                // Keeping transmits nothing: flag the entries and record it.
                Some(Command::Safety(SafetyCommand::Keep)) => {
                    keep_offline(options, Level::Passive).await;
                }
                Some(command) => refuse(sink, command).await,
            },
            received = listener.recv_from(&mut buffer) => match received {
                Ok((len, peer)) => if let Some(event) = super::passive_decode::decode(&buffer[..len], peer, &groups) {
                    sink.offer(WorkerEvent::Discovery(event));
                },
                Err(error) => { failed(sink, error).await; break; }
            },
        }
    }
}

async fn failed(sink: &EventSink, error: std::io::Error) {
    sink.deliver(WorkerEvent::ConnectFailed {
        error: crate::tui::message::Failure::text(format!("passive B/IP listener: {error}")),
    })
    .await;
}

/// True for a command that would put something on the wire.
pub(crate) fn transmits(command: &Command) -> bool {
    matches!(
        command,
        Command::WhoIs { .. }
            | Command::Browse { .. }
            | Command::Names { .. }
            | Command::Watch(_)
            | Command::Safety(_)
            | Command::Network(_)
            | Command::Discovery(_)
    )
}

/// Answer a command a passive session won't carry out, so nothing waits
/// for it.
pub(crate) async fn refuse(sink: &EventSink, command: Command) {
    if let Some(event) = refusal(command) {
        sink.deliver(event).await;
    }
}

/// Build a refusal without making cancellation wait for an event queue slot.
pub(crate) fn refusal(command: Command) -> Option<WorkerEvent> {
    let reason = PASSIVE.to_string();
    let event = match command {
        Command::WhoIs { op, .. } => WorkerEvent::OpFinished {
            op,
            outcome: OpOutcome::Failed(reason),
        },
        Command::Browse { req, .. } | Command::Names { req, .. } => WorkerEvent::Browse {
            req,
            result: BrowseResult::Failed(reason),
        },
        Command::Watch(WatchCommand::Add { row, .. }) => WorkerEvent::Watch(WatchEvent::Source {
            row,
            source: Source::Failed(reason),
        }),
        Command::Safety(
            SafetyCommand::Prepare { req, .. }
            | SafetyCommand::Execute { req, .. }
            | SafetyCommand::Manage { req, .. }
            | SafetyCommand::Revert { req },
        ) => WorkerEvent::Safety(SafetyEvent::Refused {
            req: Some(req),
            reason,
        }),
        Command::Network(command) => {
            WorkerEvent::Network(NetworkEvent::Refused { command, reason })
        }
        Command::Discovery(command) => {
            WorkerEvent::Discovery(DiscoveryEvent::Refused { command, reason })
        }
        Command::Safety(SafetyCommand::Arm { .. }) => {
            WorkerEvent::Safety(SafetyEvent::Refused { req: None, reason })
        }
        // Capturing transmits nothing, but without a client there is no
        // transport to tap.
        Command::MonitorScan(request) => {
            WorkerEvent::MonitorScan(crate::tui::monitor::scan::ScanReport::Rejected {
                id: request.id(),
                cancel: matches!(
                    request,
                    crate::tui::monitor::scan::ScanCommand::Cancel { .. }
                ),
                reason: "only a receive-only MS/TP monitor can scan".into(),
            })
        }
        Command::Capture(_) => WorkerEvent::Capture(CaptureEvent::Ended {
            what: crate::tui::capture::worker::TAP.into(),
            why: "a passive session has no transport to tap".into(),
        }),
        _ => return None,
    };
    Some(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Broken {
        calls: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
    }
    impl ReceiveOnly for Broken {
        fn groups(&self) -> io::Result<bacnet_bip_core::node::Groups> {
            Ok(bacnet_bip_core::node::Groups::new(
                "255.255.255.255:12345".parse().unwrap(),
                *self.local_addr()?.ip(),
            ))
        }
        fn local_addr(&self) -> io::Result<SocketAddrV4> {
            Ok("127.0.0.1:12345".parse().unwrap())
        }
        async fn recv_from(&self, _: &mut [u8]) -> io::Result<(usize, SocketAddrV4)> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "injected receive failure",
            ))
        }
    }
    impl Drop for Broken {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
    }
    #[tokio::test]
    async fn passive_receive_failure_reports_once_and_drops_the_owner() {
        let calls = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let listener = Broken {
            calls: calls.clone(),
            drops: drops.clone(),
        };
        let (sink, mut events) = EventSink::channel(4);
        let (_commands, rx) = mpsc::channel(1);
        listen(listener, false, &sink, rx, &SafetyOptions::default()).await;
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert!(matches!(
            events.recv().await,
            Some(WorkerEvent::PassiveListening { .. })
        ));
        assert!(matches!(events.recv().await, Some(WorkerEvent::Safety(_))));
        assert!(
            matches!(events.recv().await, Some(WorkerEvent::ConnectFailed { error }) if error.detail.contains("injected receive failure"))
        );
        assert!(events.try_recv().is_err());
    }
}
