//! SC reconnect and failover from the session's transport settings (#1831),
//! against real mutual-TLS hubs on loopback: a session with a reconnect
//! policy and only a primary hub comes back after that hub restarts, and a
//! flags-only failover hub (`--sc-failover-url`) is used when the primary
//! is down. Both go through the CLI's own SC client builder.

use bacnet_transport::sc::{ScActiveHub, ScConnectionState, ScReconnectConfig};
use rcgen::ExtendedKeyUsagePurpose;
use tokio::sync::mpsc;

use super::sc_hub::{
    args, client_leaf, hub, is_on, rebind, until, Site, FAILOVER_VMAC, PRIMARY_VMAC, WAIT,
};
use super::sc_screen::sc_app;
use super::*;
use crate::transport::TransportArgs;
use crate::tui::worker::session::{ConnectPlan, SessionOptions, TransportKind};
use crate::tui::worker::{coordinator, EventSink};

/// A short policy, so the test waits tenths of a second, not minutes.
fn quick() -> ScReconnectConfig {
    ScReconnectConfig {
        initial_delay_ms: 100,
        max_delay_ms: 200,
        max_retries: 100,
    }
}

/// A flags-only SC session on `args`, its events and its command channel.
fn start(
    args: TransportArgs,
) -> (
    mpsc::Receiver<WorkerEvent>,
    mpsc::Sender<Command>,
    tokio::task::JoinHandle<()>,
) {
    let plan = ConnectPlan {
        kind: TransportKind::Sc,
        args,
        await_interface: false,
    };
    let (sink, events) = EventSink::channel(256);
    let (commands, rx) = mpsc::channel(16);
    let task = coordinator::spawn(Some(plan), None, sink, rx, SessionOptions::default());
    (events, commands, task)
}

async fn stop(
    events: mpsc::Receiver<WorkerEvent>,
    commands: mpsc::Sender<Command>,
    task: tokio::task::JoinHandle<()>,
) {
    drop((commands, events));
    tokio::time::timeout(WAIT, task)
        .await
        .expect("coordinator quit")
        .unwrap();
}

fn connects_to_primary(app: &App) -> usize {
    app.sc
        .transitions
        .iter()
        .filter(|t| t.state == ScConnectionState::Connected && t.hub == Some(ScActiveHub::Primary))
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_primary_only_session_reconnects_after_its_hub_restarts() {
    let site = Site::new("tui reconnect CA");
    let cert = site.leaf(
        "primary hub",
        ExtendedKeyUsagePurpose::ServerAuth,
        None,
        None,
    );
    let mut primary = hub(&site, &cert, "127.0.0.1:0", PRIMARY_VMAC)
        .await
        .unwrap();
    let addr = primary.local_addr().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut args = args(
        dir.path(),
        &site,
        &client_leaf(&site),
        &format!("wss://{addr}"),
    );
    // What a profile with only a primary hub now resolves to.
    args.sc_reconnect = Some(quick());
    args.sc_connect_timeout_ms = Some(5_000);
    let mut app = sc_app();
    let (mut events, commands, task) = start(args);
    until(&mut app, &mut events, "primary", |a| {
        is_on(a, ScActiveHub::Primary)
    })
    .await;
    primary.stop().await;
    let mut primary = rebind(&site, &cert, addr, PRIMARY_VMAC).await;
    until(&mut app, &mut events, "the reconnect", |a| {
        connects_to_primary(a) >= 2 && is_on(a, ScActiveHub::Primary)
    })
    .await;
    stop(events, commands, task).await;
    primary.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_flags_only_failover_hub_is_used_when_the_primary_is_down() {
    let site = Site::new("tui flags failover CA");
    let cert = site.leaf(
        "failover hub",
        ExtendedKeyUsagePurpose::ServerAuth,
        None,
        None,
    );
    let mut failover = hub(&site, &cert, "127.0.0.1:0", FAILOVER_VMAC)
        .await
        .unwrap();
    // A primary that refuses the connection: a port bound, then let go.
    let down = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let primary = format!("wss://{}", down.local_addr().unwrap());
    drop(down);
    let dir = tempfile::tempdir().unwrap();
    let mut args = args(dir.path(), &site, &client_leaf(&site), &primary);
    let failover_url = format!("wss://{}", failover.local_addr().unwrap());
    let flags = crate::tui::profile::tests::cli(&[
        "bacnet",
        "tui",
        "--no-profile",
        "--sc",
        "--sc-url",
        &primary,
        "--sc-failover-url",
        &failover_url,
    ]);
    let parsed = TransportArgs::from_cli(&flags, args.interface, args.broadcast).unwrap();
    // The flag's own settings, on the files and identity `args` wrote.
    (args.sc_failover_url, args.sc_reconnect) = (parsed.sc_failover_url, parsed.sc_reconnect);
    assert!(
        args.sc_reconnect.is_some(),
        "a failover hub brings reconnect"
    );
    let mut app = sc_app();
    let (mut events, commands, task) = start(args);
    until(&mut app, &mut events, "failover", |a| {
        is_on(a, ScActiveHub::Failover)
    })
    .await;
    stop(events, commands, task).await;
    failover.stop().await;
}
