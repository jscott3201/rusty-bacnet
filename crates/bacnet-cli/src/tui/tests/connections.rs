//! Real loopback lifecycle tests plus pure UI generation/admission checks.

use super::port_retry::rerun_on_lost_port;
use super::*;
use crate::tui::app::{Link, Overlay};
use crate::tui::message::Failure;
use crate::tui::profile::service::{Request, Service};
use crate::tui::profile::store::Store;
use crate::tui::profile::{Overrides, Profile};
use crate::tui::worker::session::{ConnectPlan, TransportKind};
use crate::tui::worker::{EventSink, coordinator};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;
use tokio::sync::mpsc;

fn profile() -> Profile {
    let mut p = Profile::default();
    p.bip_mut().interface = Ipv4Addr::LOCALHOST;
    p.bip_mut().port = 0;
    p.bip_mut().broadcast = Ipv4Addr::new(127, 0, 0, 255);
    p
}

pub(super) async fn event(events: &mut mpsc::Receiver<WorkerEvent>) -> WorkerEvent {
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("worker event timeout")
        .expect("coordinator stopped")
}

pub(super) async fn connected(events: &mut mpsc::Receiver<WorkerEvent>, generation: u64) -> String {
    match outcome(events, generation).await {
        Ok(local) => local,
        Err(error) => panic!("connection failed: {error}"),
    }
}

/// How connection attempt `generation` ends: the local address, or why it
/// failed.
async fn outcome(
    events: &mut mpsc::Receiver<WorkerEvent>,
    generation: u64,
) -> Result<String, Failure> {
    loop {
        match event(events).await {
            WorkerEvent::Session {
                generation: actual,
                event,
            } if actual == generation => match *event {
                WorkerEvent::Connected { local } => return Ok(local),
                WorkerEvent::ConnectFailed { error } => return Err(error),
                _ => {}
            },
            _ => {}
        }
    }
}

/// [`connected`] on a port the test picked and freed, which another process
/// may take before the session binds it (#1877). When the session's own bind
/// found another socket on the port, the failed attempt comes back for
/// [`rerun_on_lost_port`], which reruns the test on a fresh port; any other
/// failure panics (`lost_port`, #2776).
async fn connected_on(
    events: &mut mpsc::Receiver<WorkerEvent>,
    generation: u64,
) -> std::io::Result<String> {
    outcome(events, generation)
        .await
        .map_err(|error| super::lost_port(&error))
}

pub(super) async fn disconnected(events: &mut mpsc::Receiver<WorkerEvent>, generation: u64) {
    loop {
        if event(events).await == (WorkerEvent::Disconnected { generation }) {
            return;
        }
    }
}

/// The session binds a port the test freed, frees it on each disconnect and
/// binds it again, and the closing probe binds it once more. Another process
/// can take it in any of those gaps, so the test then reruns on a fresh port
/// (#1877).
#[tokio::test]
async fn coordinator_disconnects_reconnects_same_port_and_joins_on_quit() {
    rerun_on_lost_port(async || {
        let mut p = profile();
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        p.bip_mut().port = port;
        drop(socket);
        let args = p.transport_args().unwrap();
        let plan = ConnectPlan {
            kind: TransportKind::Bip,
            args,
            await_interface: false,
        };
        let (sink, mut events) = EventSink::channel(32);
        let (commands, rx) = mpsc::channel(16);
        let task = coordinator::spawn(Some(plan), None, sink, rx, Default::default());
        let cycled = async {
            let first = connected_on(&mut events, 1).await?;
            for generation in 1..=3 {
                commands.send(Command::Disconnect).await.unwrap();
                disconnected(&mut events, generation).await;
                commands.send(Command::Reconnect).await.unwrap();
                assert_eq!(connected_on(&mut events, generation + 1).await?, first);
            }
            std::io::Result::Ok(())
        }
        .await;
        // Join the coordinator on a lost port too, so a rerun starts clean.
        drop((commands, events));
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        cycled?;
        // The joined coordinator released the port.
        std::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).map(drop)
    })
    .await;
}

#[tokio::test]
async fn failed_attempt_keeps_coordinator_available_for_corrected_interface() {
    let mut p = profile();
    p.bip_mut().interface = Ipv4Addr::new(203, 0, 113, 222);
    let plan = ConnectPlan {
        kind: TransportKind::Bip,
        args: p.transport_args().unwrap(),
        await_interface: false,
    };
    let (sink, mut events) = EventSink::channel(32);
    let (commands, rx) = mpsc::channel(16);
    let task = coordinator::spawn(Some(plan), None, sink, rx, Default::default());
    loop {
        if let WorkerEvent::Session {
            generation: 1,
            event,
        } = event(&mut events).await
            && matches!(*event, WorkerEvent::ConnectFailed { .. })
        {
            break;
        }
    }
    commands
        .send(Command::Connect {
            interface: Ipv4Addr::LOCALHOST,
            broadcast: p.bip().broadcast,
        })
        .await
        .unwrap();
    connected(&mut events, 2).await;
    drop((commands, events));
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
}

/// The foreign-device tests' session sends from a port the test picked, so
/// the BBMD socket can tell its datagrams from a late one meant for another
/// test's process (#2718). Another process can take the port before the
/// session binds it, so each runs under [`rerun_on_lost_port`].
fn foreign_plan(bbmd: &tokio::net::UdpSocket) -> (ConnectPlan, SocketAddr) {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let session = socket.local_addr().unwrap();
    drop(socket);
    let mut p = profile();
    p.bip_mut().port = session.port();
    p.bip_mut().bbmd = Some(match bbmd.local_addr().unwrap() {
        std::net::SocketAddr::V4(a) => a.into(),
        _ => unreachable!(),
    });
    p.bip_mut().ttl = 2;
    p.bip_mut().renewal_secs = Some(1);
    let plan = ConnectPlan {
        kind: TransportKind::Bip,
        args: p.transport_args().unwrap(),
        await_interface: false,
    };
    (plan, session)
}

/// The session's first registration on `bbmd`, from `session`'s address
/// alone. The session waits for that registration's answer before it
/// connects, so a connect outcome first is a failed start: a port lost to
/// another socket comes back as the session's own bind error, as in
/// [`connected_on`].
async fn first_registration(
    bbmd: &tokio::net::UdpSocket,
    session: SocketAddr,
    events: &mut mpsc::Receiver<WorkerEvent>,
) -> std::io::Result<Vec<u8>> {
    let first = async {
        tokio::select! {
            datagram = super::sender::next_from(bbmd, session) => Ok(datagram),
            started = connected_on(events, 1) => {
                panic!("connected as {} before registering", started?)
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(2), first)
        .await
        .expect("the session's first registration")
}

#[tokio::test]
async fn disconnect_cancels_pending_foreign_registration_and_stops_renewals() {
    rerun_on_lost_port(async || {
        let bbmd = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let (plan, session) = foreign_plan(&bbmd);
        let (sink, mut events) = EventSink::channel(32);
        let (commands, rx) = mpsc::channel(16);
        let task = coordinator::spawn(Some(plan), None, sink, rx, Default::default());
        let registered = first_registration(&bbmd, session, &mut events).await;
        if let Ok(registration) = &registered {
            assert_eq!(registration, &[0x81, 0x05, 0x00, 0x06, 0x00, 0x02]);
            commands.send(Command::Disconnect).await.unwrap();
            tokio::time::timeout(Duration::from_millis(500), disconnected(&mut events, 1))
                .await
                .expect("disconnect must not wait for registration timeout");
            assert!(
                tokio::time::timeout(
                    Duration::from_millis(1200),
                    super::sender::next_from(&bbmd, session)
                )
                .await
                .is_err(),
                "retired connection renewed from {session}"
            );
        }
        drop((commands, events));
        task.await.unwrap();
        registered.map(drop)
    })
    .await;
}

#[tokio::test]
async fn a_saturated_event_channel_cannot_block_retirement_or_quit() {
    let (sink, events) = EventSink::channel(1);
    let (commands, rx) = mpsc::channel(16);
    let plan = ConnectPlan {
        kind: TransportKind::Bip,
        args: profile().transport_args().unwrap(),
        await_interface: false,
    };
    let task = coordinator::spawn(Some(plan), None, sink, rx, Default::default());
    // SessionStarted occupies the only slot; Connected waits for room.
    tokio::time::sleep(Duration::from_millis(30)).await;
    commands.send(Command::Disconnect).await.unwrap();
    drop((events, commands));
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("full control channel deadlocked shutdown")
        .unwrap();
}

#[tokio::test]
async fn unauthorized_or_unsaved_profiles_never_start_a_network_attempt() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    let p = profile();
    store.save("local", &p, true).unwrap();
    let service = Service {
        store: Ok(store.clone()),
        overrides: Overrides::default(),
    };
    let (sink, mut events) = EventSink::channel(16);
    let (commands, rx) = mpsc::channel(16);
    let task = coordinator::spawn(None, Some(service), sink, rx, Default::default());
    commands
        .send(Command::Profile(Request::Connect {
            name: "local".into(),
            profile: p.clone(),
            attest: false,
            accepted: Vec::new(),
        }))
        .await
        .unwrap();
    assert!(matches!(
        event(&mut events).await,
        WorkerEvent::Profile(Err(_))
    ));
    assert!(events.try_recv().is_err());
    let mut changed = p.clone();
    changed.bip_mut().port = 47808;
    commands
        .send(Command::Profile(Request::Connect {
            name: "local".into(),
            profile: changed,
            attest: true,
            accepted: Vec::new(),
        }))
        .await
        .unwrap();
    assert!(matches!(
        event(&mut events).await,
        WorkerEvent::Profile(Err(_))
    ));
    assert!(events.try_recv().is_err());
    assert!(store.load("local").unwrap().authorized_at_unix.is_none());
    commands
        .send(Command::Profile(Request::Connect {
            name: "local".into(),
            profile: p,
            attest: true,
            accepted: Vec::new(),
        }))
        .await
        .unwrap();
    // Before SessionStarted can be emitted, persistence has completed.
    assert!(matches!(
        event(&mut events).await,
        WorkerEvent::Profile(Ok(_))
    ));
    assert!(store.load("local").unwrap().authorized_at_unix.is_some());
    assert!(root.path().join("workspaces/local").is_dir());
    connected(&mut events, 1).await;
    drop((commands, events));
    task.await.unwrap();
}

#[test]
fn stale_sessions_cannot_restore_rows_collisions_or_connection_state() {
    let mut app = connected_app();
    five_devices_for_session(&mut app);
    worker(&mut app, WorkerEvent::SessionStarted { generation: 2 });
    assert!(app.devices.is_empty());
    assert!(app.duplicates.banner().is_empty());
    for event in [
        WorkerEvent::Connected {
            local: "old".into(),
        },
        WorkerEvent::ConnectFailed {
            error: "old".into(),
        },
        WorkerEvent::Snapshot(vec![row(7, "old", 1, 0)]),
        WorkerEvent::Discovered(row(8, "old", 1, 0)),
        WorkerEvent::Discovered(row(9, "old-a", 1, 0)),
        WorkerEvent::Discovered(row(9, "old-b", 1, 0)),
    ] {
        worker(
            &mut app,
            WorkerEvent::Session {
                generation: 1,
                event: Box::new(event),
            },
        );
    }
    assert_eq!(app.link, Link::Connecting);
    assert!(app.devices.is_empty());
    assert!(app.duplicates.banner().is_empty());
    worker(
        &mut app,
        WorkerEvent::Session {
            generation: 2,
            event: Box::new(WorkerEvent::Connected {
                local: "new".into(),
            }),
        },
    );
    assert_eq!(
        app.link,
        Link::Up {
            local: "new".into()
        }
    );
    assert_eq!(press(&mut app, KeyCode::Char('x')), [Command::Disconnect]);
    assert_eq!(app.link, Link::Disconnecting);
    worker(
        &mut app,
        WorkerEvent::Session {
            generation: 2,
            event: Box::new(WorkerEvent::Discovered(row(10, "late", 1, 0))),
        },
    );
    assert!(app.devices.is_empty());
    worker(&mut app, WorkerEvent::Disconnected { generation: 2 });
    assert_eq!(app.link, Link::Down);
}

fn five_devices_for_session(app: &mut App) {
    worker(app, WorkerEvent::Discovered(row(1, "old-a", 1, 0)));
    worker(app, WorkerEvent::Discovered(row(1, "old-b", 1, 0)));
    assert!(!app.duplicates.banner().is_empty());
}

#[test]
fn rejected_disconnect_keeps_the_live_link_and_reports_the_failure() {
    let mut app = connected_app();
    let prior = app.link.clone();
    let command = press(&mut app, KeyCode::Char('x')).pop().unwrap();
    assert_eq!(app.link, Link::Disconnecting);
    app.command_rejected(command, prior.clone(), "queue full".into());
    assert_eq!(app.link, prior);
    assert_eq!(app.flash.unwrap().text, "queue full");
}

#[test]
fn connected_profile_cannot_be_edited_or_switched() {
    use crate::tui::app::connection::{Connection, Key};
    let mut app = connected_app();
    app.connection = Some(Connection::new(Overrides::default()));
    press(&mut app, KeyCode::Char('p'));
    assert_eq!(app.overlay, Overlay::Connection);
    assert!(act(&mut app, Action::Connection(Key::New)).is_empty());
    assert!(!app.connection.as_ref().unwrap().creating);
    assert!(app.flash.unwrap().text.contains("Disconnect"));
}

#[test]
fn a_start_ack_does_not_hide_an_already_requested_disconnect() {
    let mut app = new_app(None);
    assert_eq!(press(&mut app, KeyCode::Char('x')), [Command::Disconnect]);
    worker(&mut app, WorkerEvent::SessionStarted { generation: 1 });
    assert_eq!(app.link, Link::Disconnecting);
    worker(
        &mut app,
        WorkerEvent::Session {
            generation: 1,
            event: Box::new(WorkerEvent::Connected {
                local: "late".into(),
            }),
        },
    );
    assert_eq!(app.link, Link::Disconnecting);
    worker(&mut app, WorkerEvent::Disconnected { generation: 1 });
    assert_eq!(app.link, Link::Down);
}

#[tokio::test]
async fn a_refused_first_registration_still_connects_and_retries_in_background() {
    rerun_on_lost_port(async || {
        let bbmd = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let (plan, session) = foreign_plan(&bbmd);
        let (sink, mut events) = EventSink::channel(32);
        let (commands, rx) = mpsc::channel(16);
        let task = coordinator::spawn(Some(plan), None, sink, rx, Default::default());
        let registered = first_registration(&bbmd, session, &mut events).await;
        if registered.is_ok() {
            bbmd.send_to(&[0x81, 0x00, 0x00, 0x06, 0x00, 0x30], session)
                .await
                .unwrap();
            connected(&mut events, 1).await;
            let retry = tokio::time::timeout(
                Duration::from_secs(2),
                super::sender::next_from(&bbmd, session),
            )
            .await
            .expect("a registration retried in the background");
            assert_eq!(retry, [0x81, 0x05, 0x00, 0x06, 0x00, 0x02]);
        }
        drop((commands, events));
        task.await.unwrap();
        registered.map(drop)
    })
    .await;
}
