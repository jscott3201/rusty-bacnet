//! Site profiles end to end: created through the screen and reloaded after a
//! restart, refused before any dial when validation fails, the flags-only
//! path, and the connection screen's frames for BACnet/SC.

use super::connections::{connected, disconnected, event};
use super::*;
use crate::tui::app::connection::{Connection, Screen};
use crate::tui::app::{Link, Overlay};
use crate::tui::profile::checks::Checks;
use crate::tui::profile::sc::{DeviceUuid, Vmac};
use crate::tui::profile::sc_tests::{pem_files, sc_profile};
use crate::tui::profile::service::{Reply, Request, Service};
use crate::tui::profile::store::Store;
use crate::tui::profile::tests::cli;
use crate::tui::profile::{startup, Overrides, Profile};
use crate::tui::worker::session::{ConnectPlan, TransportKind};
use crate::tui::worker::{coordinator, EventSink};
use insta::assert_snapshot;
use std::path::Path;
use tokio::sync::mpsc;

fn profile_app() -> App {
    let mut app = new_app(None);
    app.link = Link::Down;
    app.overlay = Overlay::Connection;
    app.connection = Some(Connection::new(Overrides::default()));
    app
}

fn service(root: &Path) -> Service {
    Service::new(Ok(Store::new(root.to_owned())), Overrides::default())
}

/// Fill the focused field, then move to the next one.
fn field(app: &mut App, text: &str) {
    type_text(app, text);
    press(app, KeyCode::Tab);
}

/// Drive the editor for a new profile, then hand its save to `service`.
fn create_through_the_screen(app: &mut App, service: &Service, sc: Option<&Path>) -> Profile {
    if let Some(files) = sc {
        let (ca, cert, key, _) = pem_files(files);
        press(app, KeyCode::Char('s'));
        field(app, "west-hub");
        field(app, "wss://127.0.0.1:47808/hub");
        field(app, "wss://127.0.0.2:47808/hub");
        for path in [ca, cert, key] {
            field(app, path.to_str().unwrap());
        }
    } else {
        // The editor starts from the defaults; change the port only.
        press(app, KeyCode::Char('n'));
        field(app, "west-bip");
        press(app, KeyCode::Tab);
        for _ in 0..5 {
            press(app, KeyCode::Backspace);
        }
        type_text(app, "47809");
    }
    let commands = press(app, KeyCode::Enter);
    let [Command::Profile(Request::Save {
        name,
        profile,
        create: true,
        previous: None,
    })] = commands.as_slice()
    else {
        panic!("expected one save, got {commands:?}");
    };
    let reply = service.save(name, profile.clone(), true, None);
    worker(app, WorkerEvent::Profile(reply));
    let connection = app.connection.as_ref().unwrap();
    assert_eq!(connection.screen, Screen::Preview, "{:?}", connection.error);
    connection.saved.clone().unwrap()
}

#[test]
fn a_profile_created_through_the_screen_reloads_identically_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let mut app = profile_app();
    let bip = create_through_the_screen(&mut app, &service(root.path()), None);
    let mut app = profile_app();
    let sc = create_through_the_screen(&mut app, &service(root.path()), Some(files.path()));
    let sc_settings = sc.sc.as_ref().unwrap();
    assert_eq!(sc_settings.hub_url, "wss://127.0.0.1:47808/hub");
    assert!(sc_settings.device_uuid.is_some() && sc_settings.vmac.is_some());
    for (name, created) in [("west-bip", &bip), ("west-hub", &sc)] {
        // A new process: new service and store over the same directory.
        let restarted = startup::select(&service(root.path()), Some(name));
        assert_eq!(restarted.screen, Screen::Preview);
        assert_eq!(restarted.saved.as_ref(), Some(created));
        assert!(restarted.checks.errors.is_empty(), "{:?}", restarted.checks);
    }
    assert_eq!(bip.bip().port, 47809);
    // With two profiles, startup shows the picker.
    let picker = startup::select(&service(root.path()), None);
    assert_eq!(picker.screen, Screen::List);
    assert_eq!(picker.names, ["west-bip", "west-hub"]);
}

#[test]
fn connecting_an_sc_profile_switches_the_status_bar_to_sc() {
    let files = tempfile::tempdir().unwrap();
    let mut p = sc_profile(files.path(), "wss://127.0.0.1:47808");
    p.sc.as_mut().unwrap().assign_identity().unwrap();
    p.authorized_at_unix = Some(1_794_000_000);
    p.authorized_safety = Some(p.safety.clone());
    let mut app = profile_app();
    worker(
        &mut app,
        WorkerEvent::Profile(Ok(Reply::Loaded {
            name: "hub".into(),
            profile: p,
            checks: Checks::default(),
        })),
    );
    assert_eq!(app.transport, "BIP");
    let commands = press(&mut app, KeyCode::Char('c'));
    assert!(matches!(
        commands.as_slice(),
        [Command::Profile(Request::Connect { .. })]
    ));
    assert_eq!((app.transport, app.style), ("SC", AddressStyle::Hex));
    // Once up, the connection screen offers disconnect.
    app.link = Link::Up {
        local: "VMAC".into(),
    };
    app.overlay = Overlay::Connection;
    assert!(screen(&mut app, 120, 40).contains("x disconnect"));
    assert_eq!(press(&mut app, KeyCode::Char('x')), [Command::Disconnect]);
    assert_eq!(app.link, Link::Disconnecting);
    assert_eq!(app.overlay, Overlay::Connection, "x stays on the screen");
    worker(&mut app, WorkerEvent::Disconnected { generation: 0 });
    assert_eq!(app.link, Link::Down);
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("c connect") && !text.contains("x disconnect"));
}

/// Editing a saved SC profile never changes its identity: the fields are
/// read-only in the editor, and a save keeps what is stored whatever the
/// request carries (Annex AB.1.5.3).
#[test]
fn editing_an_sc_profile_keeps_its_identity() {
    use crate::tui::app::connection::UUID_FIELD;
    let root = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let service = service(root.path());
    let p = sc_profile(files.path(), "wss://127.0.0.1:47808");
    let Ok(Reply::Loaded { profile: saved, .. }) = service.save("hub", p, true, None) else {
        panic!("save");
    };
    let identity = saved.sc.as_ref().map(|s| (s.vmac, s.device_uuid));
    let mut app = profile_app();
    worker(&mut app, WorkerEvent::Profile(service.load("hub")));
    press(&mut app, KeyCode::Char('e'));
    while app.connection.as_ref().unwrap().focus != UUID_FIELD {
        press(&mut app, KeyCode::Tab);
    }
    let before = app.connection.as_ref().unwrap().fields[UUID_FIELD].clone();
    for _ in 0..40 {
        press(&mut app, KeyCode::Backspace);
    }
    type_text(&mut app, "0");
    let connection = app.connection.as_ref().unwrap();
    assert_eq!(connection.fields[UUID_FIELD], before, "read-only");
    assert!(connection.error.as_ref().unwrap().contains("profile file"));
    assert!(screen(&mut app, 120, 40).contains("(read-only)"));
    // Change something else, then save through the service.
    press(&mut app, KeyCode::Tab);
    type_text(&mut app, "0");
    let commands = press(&mut app, KeyCode::Enter);
    let [Command::Profile(Request::Save {
        name,
        profile,
        previous,
        ..
    })] = commands.as_slice()
    else {
        panic!("expected a save, got {commands:?}");
    };
    assert_eq!(
        profile.sc.as_ref().map(|s| (s.vmac, s.device_uuid)),
        identity
    );
    let Ok(Reply::Loaded {
        profile: edited, ..
    }) = service.save(name, profile.clone(), false, previous.clone())
    else {
        panic!("save after edit");
    };
    assert_eq!(edited.sc.as_ref().unwrap().timeout_ms, 60000);
    assert_eq!(
        edited.sc.as_ref().map(|s| (s.vmac, s.device_uuid)),
        identity
    );
    // A request with the identity blanked out still keeps the stored one.
    let mut blanked = edited.clone();
    let sc = blanked.sc.as_mut().unwrap();
    (sc.vmac, sc.device_uuid) = (None, None);
    let Ok(Reply::Loaded { profile: kept, .. }) = service.save("hub", blanked, false, Some(edited))
    else {
        panic!("save with blanks");
    };
    assert_eq!(kept.sc.as_ref().map(|s| (s.vmac, s.device_uuid)), identity);
    let on_disk = Store::new(root.path().into()).load("hub").unwrap();
    assert_eq!(
        on_disk.sc.as_ref().map(|s| (s.vmac, s.device_uuid)),
        identity
    );
}

/// The interface picker's Connect never starts a profile session, even
/// after one ran: only the reviewed preview connects a profile.
#[tokio::test]
async fn a_profile_session_refuses_the_interface_pickers_connect() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    let mut p = Profile::default();
    p.bip_mut().interface = Ipv4Addr::LOCALHOST;
    p.bip_mut().port = 0;
    p.bip_mut().broadcast = Ipv4Addr::new(127, 0, 0, 255);
    store.save("local", &p, true).unwrap();
    let (sink, mut events) = EventSink::channel(32);
    let (commands, rx) = mpsc::channel(16);
    let task = coordinator::spawn(
        None,
        Some(service(root.path())),
        sink,
        rx,
        Default::default(),
    );
    let connect = Command::Connect {
        interface: Ipv4Addr::LOCALHOST,
        broadcast: Ipv4Addr::new(127, 0, 0, 255),
    };
    commands.send(connect.clone()).await.unwrap();
    assert!(matches!(
        event(&mut events).await,
        WorkerEvent::Rejected { .. }
    ));
    // Connect the profile, disconnect, and try the picker's Connect again.
    commands
        .send(Command::Profile(Request::Connect {
            name: "local".into(),
            profile: p,
            attest: true,
            accepted: Vec::new(),
        }))
        .await
        .unwrap();
    connected(&mut events, 1).await;
    commands.send(Command::Disconnect).await.unwrap();
    disconnected(&mut events, 1).await;
    commands.send(connect).await.unwrap();
    match event(&mut events).await {
        WorkerEvent::Rejected { error } => assert!(error.contains("review the profile"), "{error}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(events.try_recv().is_err(), "no session was started");
    drop(events);
    quit(commands, task).await;
}

async fn quit(commands: mpsc::Sender<Command>, task: tokio::task::JoinHandle<()>) {
    drop(commands);
    tokio::time::timeout(Duration::from_secs(30), task)
        .await
        .expect("coordinator quit")
        .unwrap();
}

#[tokio::test]
async fn a_missing_ca_fails_validation_by_path_and_nothing_dials_the_hub() {
    let hub = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    hub.set_nonblocking(true).unwrap();
    let url = format!("wss://127.0.0.1:{}", hub.local_addr().unwrap().port());
    let root = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    let missing = files.path().join("missing-ca.pem");
    let mut p = sc_profile(files.path(), &url);
    p.sc.as_mut().unwrap().ca = missing.clone();
    store.save("site", &p, true).unwrap();
    let missing = missing.display().to_string();
    // The preview names it before any connect is asked for.
    let Ok(Reply::Loaded { checks, .. }) = service(root.path()).load("site") else {
        panic!("preview");
    };
    assert!(checks.errors[0].contains(&missing), "{checks:?}");

    let (sink, mut events) = EventSink::channel(16);
    let (commands, rx) = mpsc::channel(16);
    let task = coordinator::spawn(
        None,
        Some(service(root.path())),
        sink,
        rx,
        Default::default(),
    );
    commands
        .send(Command::Profile(Request::Connect {
            name: "site".into(),
            profile: p.clone(),
            attest: true,
            accepted: Vec::new(),
        }))
        .await
        .unwrap();
    match event(&mut events).await {
        WorkerEvent::Profile(Err(error)) => assert!(error.contains(&missing), "{error}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(events.try_recv().is_err(), "no session was started");
    assert_eq!(store.load("site").unwrap(), p, "nothing was recorded");
    quit(commands, task).await;
    // Every session is joined: a dial would have left a connection queued.
    assert_eq!(
        hub.accept().map(|_| ()).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );

    // The control: with the CA in place, the same profile does dial.
    #[cfg(feature = "sc-tls")]
    {
        let (ca, ..) = pem_files(files.path());
        let mut fixed = p.clone();
        fixed.sc.as_mut().unwrap().ca = ca;
        store.save("site", &fixed, false).unwrap();
        let hub = tokio::net::TcpListener::from_std(hub).unwrap();
        let (sink, mut events) = EventSink::channel(16);
        let (commands, rx) = mpsc::channel(16);
        let task = coordinator::spawn(
            None,
            Some(service(root.path())),
            sink,
            rx,
            Default::default(),
        );
        commands
            .send(Command::Profile(Request::Connect {
                name: "site".into(),
                profile: fixed,
                attest: true,
                accepted: Vec::new(),
            }))
            .await
            .unwrap();
        assert!(matches!(
            event(&mut events).await,
            WorkerEvent::Profile(Ok(_))
        ));
        let (socket, _) = tokio::time::timeout(Duration::from_secs(30), hub.accept())
            .await
            .expect("the valid profile dials the hub")
            .unwrap();
        // End the TLS handshake so the attempt fails and can be joined.
        drop(socket);
        quit(commands, task).await;
    }
}

/// The profile's failover hub reaches the client builder: with the primary
/// down, the connection attempt dials the failover hub instead.
#[cfg(feature = "sc-tls")]
#[tokio::test]
async fn an_sc_profile_dials_its_failover_hub_when_the_primary_is_down() {
    let primary = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let down = primary.local_addr().unwrap().port();
    drop(primary);
    let failover = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let mut p = sc_profile(files.path(), &format!("wss://127.0.0.1:{down}"));
    p.sc.as_mut().unwrap().failover_hub_url = Some(format!(
        "wss://127.0.0.1:{}",
        failover.local_addr().unwrap().port()
    ));
    Store::new(root.path().into())
        .save("site", &p, true)
        .unwrap();
    let (sink, mut events) = EventSink::channel(16);
    let (commands, rx) = mpsc::channel(16);
    let task = coordinator::spawn(
        None,
        Some(service(root.path())),
        sink,
        rx,
        Default::default(),
    );
    commands
        .send(Command::Profile(Request::Connect {
            name: "site".into(),
            profile: p,
            attest: true,
            accepted: Vec::new(),
        }))
        .await
        .unwrap();
    assert!(matches!(
        event(&mut events).await,
        WorkerEvent::Profile(Ok(_))
    ));
    let (socket, _) = tokio::time::timeout(Duration::from_secs(30), failover.accept())
        .await
        .expect("the failover hub is dialled")
        .unwrap();
    drop((socket, failover));
    drop(events);
    quit(commands, task).await;
}

#[tokio::test]
async fn a_flags_only_bip_run_connects_without_touching_the_config_dir() {
    let root = tempfile::tempdir().unwrap();
    let cli = cli(&[
        "bacnet",
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--broadcast",
        "127.0.0.255",
        "tui",
    ]);
    let mut discovered = false;
    let opened = startup::open(&cli, || {
        discovered = true;
        Ok(Store::new(root.path().into()))
    })
    .unwrap();
    assert!(opened.is_none() && !discovered);
    let args = crate::transport::TransportArgs::from_cli(
        &cli,
        Ipv4Addr::LOCALHOST,
        Ipv4Addr::new(127, 0, 0, 255),
    )
    .unwrap();
    let plan = ConnectPlan {
        kind: TransportKind::Bip,
        args,
        await_interface: false,
    };
    let (sink, mut events) = EventSink::channel(32);
    let (commands, rx) = mpsc::channel(16);
    let task = coordinator::spawn(Some(plan), None, sink, rx, Default::default());
    connected(&mut events, 1).await;
    drop(events);
    quit(commands, task).await;
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

/// A fixed SC profile for frames: no real files, a stable identity.
fn framed_sc_profile() -> Profile {
    let mut p = Profile::sc_default();
    let sc = p.sc.as_mut().unwrap();
    sc.hub_url = "wss://hub.site.example:443/bacnet".into();
    sc.failover_hub_url = Some("wss://hub2.site.example:443/bacnet".into());
    sc.ca = "/etc/bacnet/site/ca.pem".into();
    sc.cert = "/etc/bacnet/site/node.pem".into();
    sc.key = "/etc/bacnet/site/node.key".into();
    sc.vmac = Some(Vmac([0x22, 0x10, 0x20, 0x30, 0x40, 0x50]));
    sc.device_uuid = Some(DeviceUuid([
        0x6f, 0x1c, 0x2a, 0x9e, 0x44, 0x07, 0x4b, 0x3d, 0x9a, 0x51, 0x0e, 0x7c, 0x88, 0x21, 0xd4,
        0x0b,
    ]));
    p
}

#[test]
fn sc_connection_screen_frames_at_120x40() {
    let mut app = profile_app();
    press(&mut app, KeyCode::Char('s'));
    field(&mut app, "hub-west");
    type_text(&mut app, "wss://hub.site.example:443/bacnet");
    assert_snapshot!("sc_profile_editor_120x40", screen(&mut app, 120, 40));

    let checks = Checks {
        errors: vec!["CA file /etc/bacnet/site/ca.pem does not exist".into()],
        warnings: vec![
            "Key file /etc/bacnet/site/node.key is readable by group or others (mode 644); chmod 600 it.".into(),
        ],
    };
    let mut app = profile_app();
    worker(
        &mut app,
        WorkerEvent::Profile(Ok(Reply::Loaded {
            name: "hub-west".into(),
            profile: framed_sc_profile(),
            checks,
        })),
    );
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("/etc/bacnet/site/ca.pem does not exist"));
    assert!(text.contains("a attest"));
    assert_snapshot!("sc_profile_preview_120x40", text);
    // The same preview still fits the minimum terminal.
    let small = screen(&mut app, 80, 24);
    assert!(
        small.contains("a attest") && small.contains("ca.pem"),
        "{small}"
    );

    let mut p = framed_sc_profile();
    p.authorized_at_unix = Some(1_794_000_000);
    p.authorized_safety = Some(p.safety.clone());
    let mut app = profile_app();
    worker(
        &mut app,
        WorkerEvent::Profile(Ok(Reply::Loaded {
            name: "hub-west".into(),
            profile: p,
            checks: Checks::default(),
        })),
    );
    assert_snapshot!("sc_profile_ready_120x40", screen(&mut app, 120, 40));
}

/// The SC editor fits the minimum terminal with its error showing: a
/// failover hub with reconnect turned off is refused (#1831).
#[test]
fn sc_editor_at_80x24_shows_the_failover_needs_reconnect_error() {
    let mut app = profile_app();
    press(&mut app, KeyCode::Char('s'));
    field(&mut app, "hub-west");
    field(&mut app, "wss://hub.site.example:443/bacnet");
    field(&mut app, "wss://hub2.site.example:443/bacnet");
    // CA, cert, key, VMAC, UUID, timeout, pacing, connect ms: to Reconnect.
    for _ in 0..8 {
        press(&mut app, KeyCode::Tab);
    }
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Backspace);
    type_text(&mut app, "off");
    press(&mut app, KeyCode::Enter);
    let error = app.connection.as_ref().unwrap().error.clone().unwrap();
    assert!(error.contains("needs reconnect on"), "{error}");
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("needs reconnect on"), "{text}");
    assert!(text.contains("Reconnect     off"), "{text}");
    assert_snapshot!("sc_profile_editor_error_80x24", text);
}
