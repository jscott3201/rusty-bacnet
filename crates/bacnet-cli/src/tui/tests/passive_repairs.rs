//! Regressions for operator shutdown and retained passive observations.
use super::*;
use crate::tui::app::{Link, Overlay, connection::Connection, watch::Screen};
use crate::tui::message::Failure;
use crate::tui::network::discovery::{DiscoveryCommand, DiscoveryEvent, FoundRow};
use crate::tui::profile::{
    Overrides, Profile,
    service::{Reply, Request},
};
use crate::tui::worker::{EventSink, coordinator};
use bacnet_types::{enums::ObjectType, primitives::ObjectIdentifier};

fn ready() -> App {
    let mut app = new_app(None);
    app.safety.passive = true;
    worker(
        &mut app,
        WorkerEvent::PassiveListening {
            local: "127.0.0.1:47808".into(),
            foreign_ignored: false,
        },
    );
    app
}
fn find(app: &mut App) {
    press(app, KeyCode::Char(':'));
    type_text(app, "find Supply");
    assert!(press(app, KeyCode::Enter).is_empty());
}
fn observed(app: &mut App) {
    worker(
        app,
        WorkerEvent::Discovery(DiscoveryEvent::IHave(FoundRow {
            device: 12,
            object: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
            name: "Supply".into(),
            address: "127.0.0.1:47809".into(),
            via: None,
            network: None,
        })),
    );
    worker(
        app,
        WorkerEvent::Discovery(DiscoveryEvent::Router {
            router: "127.0.0.1:47809".into(),
            networks: vec![7],
        }),
    );
}
fn fail(app: &mut App) {
    worker(
        app,
        WorkerEvent::ConnectFailed {
            error: Failure::text("injected receive failure"),
        },
    );
    app.flash = None;
}
fn failed_search(populated: bool) {
    let mut app = ready();
    if populated {
        observed(&mut app);
    }
    find(&mut app);
    assert!(screen(&mut app, 200, 40).contains("Listening for"));
    fail(&mut app);
    let frame = screen(&mut app, 200, 40);
    assert!(
        frame.contains("Stopped")
            && frame.contains("no live input")
            && !frame.contains("Listening for"),
        "{frame}"
    );
    assert_eq!(
        app.who_has.as_ref().unwrap().found.len(),
        usize::from(populated)
    );
    assert_eq!(app.overheard.0.len(), usize::from(populated));
    assert_eq!(app.network.routers.router_count(), usize::from(populated));
    if populated {
        assert!(frame.contains("127.0.0.1:47809"));
    }
    worker(&mut app, WorkerEvent::SessionStarted { generation: 8 });
    assert!(
        app.overheard.0.is_empty()
            && app.who_has.is_none()
            && app.network.routers.is_empty()
            && app.names.len() == 0
    );
}
#[test]
fn passive_repair_failed_populated_search_is_retained_and_stopped() {
    failed_search(true);
}
#[test]
fn passive_repair_failed_empty_search_is_stopped() {
    failed_search(false);
}
#[test]
fn passive_repair_refused_transmit_cannot_mark_local_search_not_sent() {
    let mut app = ready();
    find(&mut app);
    worker(
        &mut app,
        WorkerEvent::Discovery(DiscoveryEvent::Refused {
            command: DiscoveryCommand::WhoHas {
                name: "Supply".into(),
            },
            reason: "passive".into(),
        }),
    );
    assert!(app.who_has.as_ref().unwrap().failed.is_none());
    assert!(!screen(&mut app, 200, 40).contains("Not sent"));
}
#[test]
fn passive_repair_failed_networks_keep_rows_and_show_no_live_input() {
    for populated in [false, true] {
        let mut app = ready();
        if populated {
            observed(&mut app);
        }
        fail(&mut app);
        app.screen = Screen::Network;
        app.network.view = crate::tui::app::network::View::Networks;
        let frame = screen(&mut app, 200, 40);
        assert!(
            frame.contains("stopped") && frame.contains("no live input"),
            "{frame}"
        );
        assert!(
            !frame.contains("ask routers") && !frame.contains("asks the routers"),
            "{frame}"
        );
        assert_eq!(app.network.routers.router_count(), usize::from(populated));
        if populated {
            assert!(frame.contains("127.0.0.1:47809"));
        }
    }
}
#[test]
fn passive_repair_guidance_matches_listening_and_failed_sessions() {
    let mut app = ready();
    for stopped in [false, true] {
        if stopped {
            fail(&mut app);
        }
        assert!(
            crate::tui::help::screens(&app)
                .iter()
                .any(|tab| tab.label == "Network")
        );
        let help = crate::tui::help::sections(&app);
        let description = &help
            .iter()
            .flat_map(|section| &section.rows)
            .find(|(keys, _)| keys == "find NAME")
            .unwrap()
            .1;
        assert!(
            description.contains("observed") && description.contains("local"),
            "{description}"
        );
        app.screen = Screen::Network;
        app.network.view = crate::tui::app::network::View::Networks;
        let frame = screen(&mut app, 200, 40);
        assert!(!frame.contains("ask routers"), "{frame}");
        app.screen = Screen::Main;
    }
    let active = connected_app();
    let help = crate::tui::help::sections(&active);
    assert!(
        help.iter()
            .flat_map(|section| &section.rows)
            .any(|(keys, what)| keys == "find NAME" && what.contains("Who-Has"))
    );
}
#[tokio::test]
async fn passive_repair_operator_x_releases_socket_and_reopens_profiles() {
    use tokio::sync::mpsc;
    let (sink, mut events) = EventSink::channel(8);
    let (commands, rx) = mpsc::channel(8);
    let root = tempfile::tempdir().unwrap();
    let store = crate::tui::profile::store::Store::new(root.path().into());
    let mut replacement = Profile::default();
    replacement.bip_mut().interface = std::net::Ipv4Addr::LOCALHOST;
    replacement.bip_mut().port = 0;
    store.save("replacement", &replacement, true).unwrap();
    let profiles = crate::tui::profile::service::Service::new(Ok(store), Overrides::default());
    let task = coordinator::spawn(
        Some(super::network::plan(None)),
        Some(profiles),
        sink,
        rx,
        super::passive_discovery::options(),
    );
    let mut app = new_app(None);
    app.safety.passive = true;
    app.connection = Some(Connection::new(Overrides::default()));
    for _ in 0..3 {
        worker(&mut app, events.recv().await.unwrap());
    }
    let address: std::net::SocketAddrV4 = app.passive_listener.as_ref().unwrap().parse().unwrap();
    observed(&mut app);
    find(&mut app);
    let out = press(&mut app, KeyCode::Char('x'));
    assert_eq!(out, [Command::Disconnect]);
    assert_eq!(app.link, Link::Disconnecting);
    for command in out {
        commands.send(command).await.unwrap();
    }
    loop {
        let event = events.recv().await.unwrap();
        let done = matches!(event, WorkerEvent::Disconnected { .. });
        worker(&mut app, event);
        if done {
            break;
        }
    }
    assert_eq!(app.link, Link::Down);
    assert!(
        app.passive_listener.is_none()
            && app.overheard.0.is_empty()
            && app.who_has.is_none()
            && app.network.routers.is_empty()
            && app.names.len() == 0
    );
    let rebound =
        std::net::UdpSocket::bind(address).expect("one-shot rebind after operator disconnect");
    drop(rebound);
    press(&mut app, KeyCode::Char('c'));
    assert_eq!(app.overlay, Overlay::Connection);
    worker(
        &mut app,
        WorkerEvent::Profile(Ok(Reply::Loaded {
            name: "replacement".into(),
            profile: replacement,
            checks: Default::default(),
        })),
    );
    let replacement = press(&mut app, KeyCode::Char('a'));
    assert!(
        matches!(replacement.as_slice(), [Command::Profile(Request::Connect { name, .. })] if name == "replacement")
    );
    assert_eq!(app.link, Link::Connecting);
    for command in replacement {
        commands.send(command).await.unwrap();
    }
    while !app.passive_listening() {
        let event = events.recv().await.unwrap();
        assert!(
            !matches!(
                event,
                WorkerEvent::Profile(Err(_)) | WorkerEvent::Rejected { .. }
            ),
            "{event:?}"
        );
        worker(&mut app, event);
    }
    assert_eq!(app.generation, 2);
    let replacement: std::net::SocketAddrV4 =
        app.passive_listener.as_ref().unwrap().parse().unwrap();
    drop(commands);
    drop(events);
    task.await.unwrap();
    assert!(
        std::net::UdpSocket::bind(replacement).is_ok(),
        "replacement retired with coordinator"
    );
}
