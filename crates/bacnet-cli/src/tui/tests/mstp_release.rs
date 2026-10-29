//! Actual monitor/session/coordinator ownership admission through Lines.
use super::site::bounded;
use crate::tui::message::{Command, WorkerEvent};
use crate::tui::monitor::lines::test_line::TestLine;
use crate::tui::monitor::{Lines, MonitorPlan, MstpOptions};
use crate::tui::worker::session::{ConnectPlan, SessionOptions, TransportKind};
use crate::tui::worker::{EventSink, coordinator};
use bacnet_transport::mstp::monitor::MonitorRead;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, DuplexStream, ReadBuf};
use tokio::sync::{mpsc, oneshot};

struct Deferred {
    stream: DuplexStream,
    entered: Option<oneshot::Sender<()>>,
    release: oneshot::Receiver<bool>,
    held: Arc<AtomicBool>,
    closed: Option<bool>,
}
impl AsyncRead for Deferred {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}
impl MonitorRead for Deferred {
    fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
        }
        let success = match self.closed {
            Some(success) => success,
            None => {
                let success = std::task::ready!(Pin::new(&mut self.release).poll(cx)).unwrap();
                self.closed = Some(success);
                success
            }
        };
        if success {
            self.held.store(false, Ordering::SeqCst);
            Poll::Ready(Ok(()))
        } else {
            Poll::Ready(Err(io::Error::other("release acknowledgment failed")))
        }
    }
}
struct Run {
    line: Arc<TestLine>,
    peer: Option<DuplexStream>,
    entered: oneshot::Receiver<()>,
    release: oneshot::Sender<bool>,
    commands: mpsc::Sender<Command>,
    events: mpsc::Receiver<WorkerEvent>,
    task: tokio::task::JoinHandle<()>,
}
fn start(baud: u32, reader: bool) -> Run {
    let (peer, stream) = tokio::io::duplex(64);
    let (entered, entering) = oneshot::channel();
    let (release, releasing) = oneshot::channel();
    let held = Arc::new(AtomicBool::new(false));
    let input = reader.then(|| {
        Box::new(Deferred {
            stream,
            entered: Some(entered),
            release: releasing,
            held: held.clone(),
            closed: None,
        }) as crate::tui::monitor::lines::MonitorReader
    });
    let line = TestLine::with_reader(input, held, None);
    let options = SessionOptions {
        mstp: MstpOptions {
            scan: Default::default(),
            monitor: Some(MonitorPlan::new("fixture".into(), baud, 1)),
            lines: Lines::Test(line.clone()),
            thread: Default::default(),
        },
        ..Default::default()
    };
    let plan = ConnectPlan {
        kind: TransportKind::MstpMonitor,
        args: crate::tui::profile::Profile::default()
            .transport_args()
            .unwrap(),
        await_interface: false,
    };
    let (sink, events) = EventSink::channel(64);
    let (commands, rx) = mpsc::channel(16);
    let task = coordinator::spawn(Some(plan), None, sink, rx, options);
    Run {
        line,
        peer: Some(peer),
        entered: entering,
        release,
        commands,
        events,
        task,
    }
}
fn join() -> Command {
    Command::JoinTrunk {
        settings: crate::mstp::MstpArgs {
            port: "fixture".into(),
            baud: 38_400,
            mac: 3,
            max_master: 10,
            max_info_frames: 1,
        },
    }
}
fn inner(event: &WorkerEvent) -> &WorkerEvent {
    match event {
        WorkerEvent::Session { event, .. } => event,
        event => event,
    }
}
async fn until(
    events: &mut mpsc::Receiver<WorkerEvent>,
    predicate: impl Fn(&WorkerEvent) -> bool,
) -> WorkerEvent {
    bounded(async {
        loop {
            let event = events.recv().await.expect("coordinator ended");
            if predicate(&event) {
                return event;
            }
        }
    })
    .await
}
async fn ready(run: &mut Run) {
    until(&mut run.events, |event| {
        matches!(inner(event), WorkerEvent::Passive)
    })
    .await;
}

async fn refused_release(action: u8) {
    let mut run = start(38_400, true);
    ready(&mut run).await;
    match action {
        0 => run.commands.send(join()).await.unwrap(),
        1 => run.commands.send(Command::Disconnect).await.unwrap(),
        _ => drop(run.peer.take()),
    }
    bounded(&mut run.entered).await.unwrap();
    assert!(run.line.held());
    assert_eq!(run.line.opens.load(Ordering::SeqCst), 0, "close still held");
    if action != 2 {
        run.commands.send(join()).await.unwrap();
        run.commands.send(Command::Reconnect).await.unwrap();
    }
    run.release.send(false).unwrap();
    if action == 2 {
        // The actual session ended naturally before the replacement request.
        until(&mut run.events, |event| {
            matches!(
                inner(event),
                WorkerEvent::Monitor(crate::tui::monitor::MonitorReport::Ended(_))
            )
        })
        .await;
        let _ = run.commands.send(join()).await;
        let _ = run.commands.send(Command::Reconnect).await;
    }
    drop(run.commands);
    bounded(run.task).await.unwrap();
    let mut fatal = None;
    while let Some(event) = run.events.recv().await {
        match event {
            WorkerEvent::Fatal { error } => {
                let mut app = super::new_app(None);
                super::worker(
                    &mut app,
                    WorkerEvent::Fatal {
                        error: error.clone(),
                    },
                );
                assert!(app.quit);
                assert_eq!(app.exit_error.as_deref(), Some(error.as_str()));
                fatal = Some(error);
            }
            WorkerEvent::Disconnected { .. } => panic!("failed release reported Disconnected"),
            _ => {}
        }
    }
    assert_eq!(
        run.line.opens.load(Ordering::SeqCst),
        0,
        "failed release admitted a participant factory open"
    );
    assert!(run.line.held(), "fixture retains the unacknowledged owner");
    assert!(
        fatal.is_some_and(|error| error.contains("release acknowledgment failed")),
        "coordinator must report release failure outside the retired session"
    );
}
#[tokio::test(start_paused = true)]
async fn failed_release_refuses_confirmed_and_queued_joins() {
    refused_release(0).await;
}
#[tokio::test(start_paused = true)]
async fn failed_release_refuses_disconnect_and_later_join() {
    refused_release(1).await;
}
#[tokio::test(start_paused = true)]
async fn failed_natural_completion_refuses_later_join() {
    refused_release(2).await;
}

#[tokio::test(start_paused = true)]
async fn successful_release_admits_join_only_after_acknowledgment() {
    let mut run = start(38_400, true);
    ready(&mut run).await;
    run.commands.send(join()).await.unwrap();
    bounded(&mut run.entered).await.unwrap();
    assert!(run.line.held());
    assert_eq!(run.line.opens.load(Ordering::SeqCst), 0);
    run.release.send(true).unwrap();
    until(&mut run.events, |event| {
        matches!(inner(event), WorkerEvent::ConnectFailed { .. })
    })
    .await;
    assert!(!run.line.held());
    assert_eq!(run.line.opens.load(Ordering::SeqCst), 1);
    drop(run.commands);
    bounded(run.task).await.unwrap();
}

async fn rejected_startup(success: bool) {
    let mut run = start(0, true);
    bounded(&mut run.entered).await.unwrap();
    assert!(run.line.held());
    assert_eq!(run.line.opens.load(Ordering::SeqCst), 0);
    run.release.send(success).unwrap();
    until(&mut run.events, |event| {
        matches!(inner(event), WorkerEvent::ConnectFailed { .. })
    })
    .await;
    let _ = run.commands.send(join()).await;
    let _ = run.commands.send(Command::Reconnect).await;
    if success {
        until(&mut run.events, |event| {
            matches!(inner(event), WorkerEvent::ConnectFailed { .. })
        })
        .await;
        assert!(!run.line.held());
        assert_eq!(run.line.opens.load(Ordering::SeqCst), 1);
    }
    drop(run.commands);
    bounded(run.task).await.unwrap();
    let mut fatal = false;
    while let Some(event) = run.events.recv().await {
        if let WorkerEvent::Fatal { error } = event {
            assert!(error.contains("release acknowledgment failed"));
            fatal = true;
        }
    }
    assert_eq!(fatal, !success);
    if !success {
        assert!(run.line.held());
        assert_eq!(run.line.opens.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test(start_paused = true)]
async fn rejected_startup_with_successful_cleanup_permits_join() {
    rejected_startup(true).await;
}
#[tokio::test(start_paused = true)]
async fn rejected_startup_with_failed_cleanup_refuses_join() {
    rejected_startup(false).await;
}
#[tokio::test(start_paused = true)]
async fn no_owner_open_failure_permits_join() {
    let mut run = start(38_400, false);
    until(&mut run.events, |event| {
        matches!(inner(event), WorkerEvent::ConnectFailed { .. })
    })
    .await;
    assert!(!run.line.held());
    run.commands.send(join()).await.unwrap();
    until(&mut run.events, |event| {
        matches!(inner(event), WorkerEvent::ConnectFailed { .. })
    })
    .await;
    assert_eq!(run.line.opens.load(Ordering::SeqCst), 1);
    drop(run.commands);
    bounded(run.task).await.unwrap();
    while let Some(event) = run.events.recv().await {
        assert!(!matches!(event, WorkerEvent::Fatal { .. }));
    }
}
