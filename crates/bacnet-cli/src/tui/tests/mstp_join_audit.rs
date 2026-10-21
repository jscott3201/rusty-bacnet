//! A confirmed MS/TP trunk join on a profile session goes into the
//! profile's audit log (#1862): a declined one records nothing, a passive
//! session records none, and a join that can't be recorded isn't made.

use std::path::Path;

use ratatui::crossterm::event::KeyCode;
use serde_json::Value;
use tokio::sync::mpsc;

use super::connections::event;
use super::*;
use crate::tui::app::connection::Connection;
use crate::tui::app::{Link, Overlay};
use crate::tui::profile::service::{Reply, Request, Service};
use crate::tui::profile::store::Store;
use crate::tui::profile::{Overrides, Profile};
use crate::tui::safety::audit::MSTP_JOIN;
use crate::tui::worker::session::SessionOptions;
use crate::tui::worker::{coordinator, EventSink};

/// A port no host has, so the session's open fails and nothing is sent.
const PORT: &str = "/nonexistent/ttyBACnet-1862";

/// Profile `trunk`, already authorized.
fn trunk() -> Profile {
    let mut p = Profile::mstp_default();
    let m = p.mstp_mut();
    m.port = PORT.into();
    m.mac = 3;
    m.baud = 76_800;
    p.authorized_at_unix = Some(1);
    p.authorized_safety = Some(p.safety.clone());
    p
}

fn records(root: &Path) -> Vec<Value> {
    let path = root.join("workspaces/trunk/audit.jsonl");
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

struct Run {
    events: mpsc::Receiver<WorkerEvent>,
    commands: mpsc::Sender<Command>,
    task: tokio::task::JoinHandle<()>,
}

impl Run {
    /// A coordinator over a store under `root` holding `trunk`.
    fn start(root: &Path, options: SessionOptions) -> Self {
        let store = Store::new(root.into());
        store.save("trunk", &trunk(), true).unwrap();
        let service = Service::new(Ok(store), Overrides::default());
        let (sink, events) = EventSink::channel(16);
        let (commands, rx) = mpsc::channel(16);
        let task = coordinator::spawn(None, Some(service), sink, rx, options);
        Self {
            events,
            commands,
            task,
        }
    }

    /// Send `commands` and return the coordinator's first answer.
    async fn send(&mut self, commands: Vec<Command>) -> WorkerEvent {
        for command in commands {
            self.commands.send(command).await.unwrap();
        }
        event(&mut self.events).await
    }

    /// Quit, and return every event left, in order.
    async fn quit(self) -> Vec<WorkerEvent> {
        let Self {
            mut events,
            commands,
            task,
        } = self;
        drop(commands);
        task.await.unwrap();
        let mut left = Vec::new();
        while let Ok(event) = events.try_recv() {
            left.push(event);
        }
        left
    }
}

/// Connect `trunk`, as the app does once its join is confirmed.
fn connect() -> Command {
    Command::Profile(Request::Connect {
        name: "trunk".into(),
        profile: trunk(),
        attest: true,
        accepted: Vec::new(),
    })
}

#[tokio::test]
async fn a_confirmed_profile_join_is_recorded_once() {
    let root = tempfile::tempdir().unwrap();
    let mut run = Run::start(root.path(), SessionOptions::default());
    let reply = run.send(vec![connect()]).await;
    assert!(matches!(reply, WorkerEvent::Profile(Ok(_))), "{reply:?}");
    run.quit().await;
    let records = records(root.path());
    let [record] = records.as_slice() else {
        panic!("{records:?}");
    };
    assert_eq!(record["event"], MSTP_JOIN);
    assert_eq!(record["profile"], "trunk");
    assert_eq!(record["transport"], "MSTP");
    assert_eq!(record["port"], PORT);
    assert_eq!(record["station"], 3);
    assert_eq!(record["baud"], 76_800);
    // No thread settings were given (#501).
    assert!(record["realtime_priority"].is_null(), "{record}");
    assert_eq!(record["cpu_affinity"], serde_json::json!([]));
    assert!(record["time"].as_str().is_some_and(|t| t.ends_with('Z')));
}

/// Through the app: Esc in the join dialog sends nothing, so nothing is
/// recorded; `y` afterwards records the one join.
#[tokio::test]
async fn a_declined_join_records_nothing() {
    let root = tempfile::tempdir().unwrap();
    let mut run = Run::start(root.path(), SessionOptions::default());
    let mut app = new_app(None);
    app.link = Link::Down;
    app.overlay = Overlay::Connection;
    app.connection = Some(Connection::new(Overrides::default()));
    let loaded = Reply::Loaded {
        name: "trunk".into(),
        profile: trunk(),
        checks: Default::default(),
    };
    worker(&mut app, WorkerEvent::Profile(Ok(loaded)));
    assert!(press(&mut app, KeyCode::Char('c')).is_empty(), "held");
    assert!(matches!(app.overlay, Overlay::Join(_)));
    assert!(press(&mut app, KeyCode::Esc).is_empty(), "declined");
    assert!(records(root.path()).is_empty());

    press(&mut app, KeyCode::Char('c'));
    let confirmed = press(&mut app, KeyCode::Char('y'));
    let reply = run.send(confirmed).await;
    assert!(matches!(reply, WorkerEvent::Profile(Ok(_))), "{reply:?}");
    run.quit().await;
    assert_eq!(records(root.path()).len(), 1);
}

#[tokio::test]
async fn a_passive_profile_session_records_no_join() {
    let root = tempfile::tempdir().unwrap();
    let mut options = SessionOptions::default();
    options.safety.passive = true;
    let mut run = Run::start(root.path(), options);
    let reply = run.send(vec![connect()]).await;
    assert!(matches!(reply, WorkerEvent::Profile(Ok(_))), "{reply:?}");
    run.quit().await;
    assert!(records(root.path()).is_empty());
}

/// An audit log that can't be opened (a directory in its place) refuses the
/// join: no session starts, so the port is never opened.
#[tokio::test]
async fn a_join_that_cannot_be_recorded_is_not_made() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("workspaces/trunk/audit.jsonl")).unwrap();
    let mut run = Run::start(root.path(), SessionOptions::default());
    let reply = run.send(vec![connect()]).await;
    let WorkerEvent::Profile(Err(error)) = &reply else {
        panic!("{reply:?}");
    };
    assert!(error.contains("the join was not made"), "{error}");
    let left = run.quit().await;
    assert!(
        !left
            .iter()
            .any(|e| matches!(e, WorkerEvent::SessionStarted { .. })),
        "{left:?}"
    );
}
