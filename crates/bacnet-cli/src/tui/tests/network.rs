//! The Network screen against real B/IP transports on loopback (#966):
//! BBMDs, a foreign-device registration and its renewals, a refusing BBMD,
//! and a Who-Is that reaches the BBMD's subnet through it.
//!
//! The UI side runs as the event loop runs it: worker events go through
//! `update`, and the commands it returns go back to the worker. The
//! registration test runs on paused time, so its minute of renewals takes
//! no wall-clock time; tokio only moves the clock while every task waits.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use bacnet_objects::DeviceIdentity;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bbmd::{BdtEntry, ForeignDevicePolicy};
use bacnet_transport::bip::{BbmdOptions, BipMode, BipTransport};
use bacnet_transport::port::TransportPort;
use ratatui::crossterm::event::KeyCode;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::*;
use crate::tui::app::Link;
use crate::tui::app::network::View;
use crate::tui::app::watch::Screen;
use crate::tui::message::{WhoIsScope, WhoIsSpec};
use crate::tui::network::address_of;
use crate::tui::profile::Profile;
use crate::tui::worker::session::{ConnectPlan, TransportKind};
use crate::tui::worker::{EVENT_CHANNEL_CAPACITY, EventSink, coordinator};
use bacnet_ops::bbmd::{Cell, Finding};

/// Long enough for any step here, short of a hang.
const STEP: Duration = Duration::from_secs(10);

/// The longest one wait for a state may take, of tokio time: more than a
/// renewal interval, which the registration test waits out on paused time.
const PUMP_LIMIT: Duration = Duration::from_secs(45);

/// A BBMD on loopback whose own device is `instance`; it takes foreign
/// devices when `accept` is set.
async fn bbmd_server(instance: u32, accept: bool) -> BACnetServer<BipTransport> {
    let identity = DeviceIdentity::new(instance, 0).unwrap();
    let mut db = ObjectDatabase::new();
    let device = DeviceObject::new(DeviceConfig {
        identity: identity.clone().with_name(format!("bbmd-{instance}")),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();
    BACnetServer::bip_builder()
        .device(identity)
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(db)
        .mode(BipMode::Bbmd(BbmdOptions {
            foreign_device_policy: accept.then(ForeignDevicePolicy::default),
            ..BbmdOptions::default()
        }))
        .build()
        .await
        .unwrap()
}

fn server_address(server: &BACnetServer<BipTransport>) -> SocketAddrV4 {
    address_of(server.local_mac()).unwrap()
}

/// A bare BBMD transport on loopback, with only its own BDT entry.
async fn bbmd_transport() -> BipTransport {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    transport.enable_bbmd(Vec::new());
    let _rx = transport.start().await.unwrap();
    transport
}

fn entry(address: SocketAddrV4) -> BdtEntry {
    BdtEntry {
        ip: address.ip().octets(),
        port: address.port(),
        broadcast_mask: [255; 4],
    }
}

/// Give `transport` exactly these BDT entries (it adds its own when missing).
fn set_bdt(transport: &BipTransport, peers: &[SocketAddrV4]) {
    let state = transport.bbmd_state().unwrap();
    let mut state = state.lock().unwrap();
    state
        .set_bdt(peers.iter().copied().map(entry).collect())
        .unwrap();
}

/// A flags-only B/IP connection on loopback, registering with `bbmd` when
/// given, for a 60 s time-to-live.
pub(super) fn plan(bbmd: Option<SocketAddrV4>) -> ConnectPlan {
    let mut profile = Profile::default();
    profile.bip_mut().interface = Ipv4Addr::LOCALHOST;
    profile.bip_mut().port = 0;
    profile.bip_mut().broadcast = Ipv4Addr::LOCALHOST;
    profile.bip_mut().bbmd = bbmd;
    profile.bip_mut().ttl = 60;
    ConnectPlan {
        kind: TransportKind::Bip,
        args: profile.transport_args().unwrap(),
        await_interface: false,
    }
}

/// The app and the worker, wired as the event loop wires them.
pub(super) struct Net {
    pub(super) app: App,
    events: mpsc::Receiver<WorkerEvent>,
    commands: mpsc::Sender<Command>,
    task: JoinHandle<()>,
}

impl Net {
    pub(super) async fn start(plan: ConnectPlan) -> Self {
        let (sink, events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
        let (commands, rx) = mpsc::channel(16);
        let task = coordinator::spawn(Some(plan), None, sink, rx, Default::default());
        let mut net = Self {
            app: new_app(None),
            events,
            commands,
            task,
        };
        net.pump_until("connected", |app| matches!(app.link, Link::Up { .. }))
            .await;
        net
    }

    fn now() -> std::time::Instant {
        tokio::time::Instant::now().into_std()
    }

    /// Hand the worker what `update` asked for.
    pub(super) async fn send(&mut self, commands: Vec<Command>) {
        for command in commands {
            self.commands.send(command).await.unwrap();
        }
    }

    /// Apply worker events until `done` holds.
    pub(super) async fn pump_until(&mut self, what: &str, done: impl Fn(&App) -> bool) {
        // A bound on the whole wait too, since status polls keep arriving.
        let deadline = tokio::time::Instant::now() + PUMP_LIMIT;
        loop {
            assert!(
                tokio::time::Instant::now() < deadline,
                "gave up waiting for {what}"
            );
            tick(&mut self.app, Self::now());
            if done(&self.app) {
                return;
            }
            let event = tokio::time::timeout(STEP, self.events.recv())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
                .expect("the worker is running");
            let commands = worker(&mut self.app, event);
            self.send(commands).await;
        }
    }

    /// Press a key and send what it asks for.
    pub(super) async fn press(&mut self, code: KeyCode) {
        let commands = press(&mut self.app, code);
        self.send(commands).await;
    }

    /// Run a `:` command line.
    pub(super) async fn command_line(&mut self, text: &str) {
        self.press(KeyCode::Char(':')).await;
        type_text(&mut self.app, text);
        self.press(KeyCode::Enter).await;
    }

    pub(super) async fn stop(self) {
        drop((self.commands, self.events));
        tokio::time::timeout(STEP, self.task)
            .await
            .expect("the coordinator stopped")
            .unwrap();
    }
}

fn local(app: &App) -> SocketAddrV4 {
    let Link::Up { local } = &app.link else {
        panic!("not connected");
    };
    local.parse().unwrap()
}

/// Survey from `a` and return the matrix the app built.
async fn survey(a: SocketAddrV4) -> bacnet_ops::bbmd::Matrix {
    let mut net = Net::start(plan(None)).await;
    net.press(KeyCode::Char('4')).await;
    assert_eq!(net.app.screen, Screen::Network);
    net.command_line(&format!("bbmd {a}")).await;
    net.pump_until("the tables", |app| app.network.bdt.is_some())
        .await;
    net.press(KeyCode::Char('m')).await;
    assert_eq!(net.app.network.view, View::Matrix);
    net.pump_until("the survey", |app| app.network.matrix.is_some())
        .await;
    let matrix = net.app.network.matrix.clone().unwrap().0;
    net.stop().await;
    matrix
}

#[tokio::test]
async fn symmetric_bdts_read_as_consistent() {
    let (mut a, mut b) = (bbmd_transport().await, bbmd_transport().await);
    let (a_at, b_at) = (
        address_of(a.local_mac()).unwrap(),
        address_of(b.local_mac()).unwrap(),
    );
    set_bdt(&a, &[a_at, b_at]);
    set_bdt(&b, &[a_at, b_at]);
    let matrix = survey(a_at).await;
    assert_eq!(matrix.bbmds, [a_at, b_at]);
    assert!(matrix.consistent(), "{:?}", matrix.findings);
    let unicast = Cell::Listed {
        mask: Ipv4Addr::BROADCAST,
        mixed: false,
    };
    assert!(matrix.cells.iter().flatten().all(|cell| *cell == unicast));
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}

#[tokio::test]
async fn a_bdt_missing_its_peer_is_flagged_one_way() {
    let (mut a, mut b) = (bbmd_transport().await, bbmd_transport().await);
    let (a_at, b_at) = (
        address_of(a.local_mac()).unwrap(),
        address_of(b.local_mac()).unwrap(),
    );
    set_bdt(&a, &[a_at, b_at]);
    set_bdt(&b, &[b_at]);
    let matrix = survey(a_at).await;
    // A to B present; B to A missing.
    assert!(matches!(matrix.cells[0][1], Cell::Listed { .. }));
    assert_eq!(matrix.cells[1][0], Cell::Missing);
    assert_eq!(
        matrix.findings,
        [Finding::Asymmetric {
            from: a_at,
            to: b_at
        }]
    );
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}

/// A peer that answers nothing is named as unreachable, and one that is no
/// BBMD as refusing; neither stalls the survey.
#[tokio::test(start_paused = true)]
async fn silent_and_refusing_peers_are_named() {
    let mut a = bbmd_transport().await;
    let a_at = address_of(a.local_mac()).unwrap();
    let silent = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let silent_at = match silent.local_addr().unwrap() {
        std::net::SocketAddr::V4(at) => at,
        _ => unreachable!(),
    };
    let mut plain = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let _rx = plain.start().await.unwrap();
    let plain_at = address_of(plain.local_mac()).unwrap();
    // Read directly, a node that is no BBMD refuses both tables.
    let mut net = Net::start(plan(None)).await;
    net.command_line(&format!("bbmd {plain_at}")).await;
    net.pump_until("the refusals", |app| app.network.fdt.is_some())
        .await;
    let state = &net.app.network;
    let refused = |result: Result<(), &String>| {
        result.is_err_and(|error| error.starts_with("refused: BVLC-Result"))
    };
    let bdt = state.bdt.as_ref().unwrap().result.as_ref().map(|_| ());
    let fdt = state.fdt.as_ref().unwrap().result.as_ref().map(|_| ());
    assert!(refused(bdt), "{bdt:?}");
    assert!(refused(fdt), "{fdt:?}");
    net.stop().await;
    set_bdt(&a, &[a_at, silent_at, plain_at]);
    let matrix = survey(a_at).await;
    assert!(
        matrix
            .findings
            .iter()
            .any(|f| matches!(f, Finding::Unreachable { bbmd, .. } if *bbmd == silent_at))
    );
    assert!(
        matrix
            .findings
            .iter()
            .any(|f| matches!(f, Finding::Nak { bbmd, .. } if *bbmd == plain_at))
    );
    a.stop().await.unwrap();
    plain.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn registration_counts_down_renews_in_time_and_finds_our_fdt_entry() {
    let mut server = bbmd_server(3301, true).await;
    let bbmd = server_address(&server);
    let mut net = Net::start(plan(Some(bbmd))).await;
    net.press(KeyCode::Char('4')).await;
    let foreign = net.app.network.foreign.unwrap();
    assert_eq!((foreign.bbmd, foreign.ttl), (bbmd, 60));
    assert_eq!(foreign.renewal, Duration::from_secs(30));

    // Accepted: the FDT is read once, and our entry is the one marked.
    net.pump_until("our FDT entry", |app| app.network.own.is_some())
        .await;
    let own = net.app.network.own.unwrap();
    let fdt = net
        .app
        .network
        .fdt
        .as_ref()
        .unwrap()
        .result
        .clone()
        .unwrap();
    assert_eq!(fdt[own].address, local(&net.app));
    assert_eq!(fdt[own].ttl, 60);
    net.press(KeyCode::Tab).await;
    assert_eq!(net.app.network.view, View::Fdt);
    let text = screen(&mut net.app, 120, 40);
    assert!(text.contains("this client"), "{text}");
    assert!(text.contains("As the BBMD sees us"), "{text}");

    // The countdown runs down with the clock.
    let first = net.app.network.next_attempt(net.app.now).unwrap();
    let mark = net.app.now;
    net.pump_until("five seconds", |app| {
        app.now.duration_since(mark) >= Duration::from_secs(5)
    })
    .await;
    let later = net.app.network.next_attempt(net.app.now).unwrap();
    assert!(
        later + Duration::from_secs(4) <= first,
        "{first:?} then {later:?}"
    );

    // Over three minutes the client re-registers every renewal interval,
    // well before the 60 s time-to-live runs out: the lease never lapses.
    let start = net.app.now;
    let mut accepted_at = Vec::new();
    while net.app.now.duration_since(start) < Duration::from_secs(180) {
        let before = net.app.network.registration.unwrap().0.accepted;
        net.pump_until("a status poll", |app| {
            app.network.registration.unwrap().0.accepted != before
                || app.now.duration_since(start) >= Duration::from_secs(180)
        })
        .await;
        let (snapshot, _) = net.app.network.registration.unwrap();
        assert!(snapshot.registered, "the lease lapsed: {snapshot:?}");
        assert_eq!(snapshot.lapses, 0);
        if snapshot.accepted != before {
            accepted_at.push(net.app.now);
        }
    }
    assert!(
        accepted_at.len() >= 5,
        "renewed {} times",
        accepted_at.len()
    );
    for pair in accepted_at.windows(2) {
        assert!(pair[1].duration_since(pair[0]) < Duration::from_secs(60));
    }
    assert_eq!(
        server
            .transport()
            .unwrap()
            .fdt_counters()
            .unwrap()
            .registrations_accepted,
        net.app.network.registration.unwrap().0.accepted
    );
    net.stop().await;
    server.stop().await.unwrap();
}

#[tokio::test]
async fn a_refusing_bbmd_raises_the_refusal_banner() {
    let mut server = bbmd_server(3302, false).await;
    let bbmd = server_address(&server);
    let mut net = Net::start(plan(Some(bbmd))).await;
    net.pump_until("the refusal", |app| {
        app.network.registration_banner().is_some()
    })
    .await;
    let banner = net.app.network.registration_banner().unwrap();
    assert!(banner.contains(&format!("BBMD {bbmd} refused")), "{banner}");
    assert!(banner.contains("register-foreign-device-nak"), "{banner}");
    // On every screen, and in the panel.
    let text = screen(&mut net.app, 120, 40);
    assert!(text.contains("REFUSED BBMD"), "{text}");
    net.press(KeyCode::Char('4')).await;
    let text = screen(&mut net.app, 120, 40);
    assert!(text.contains("Status REFUSED"), "{text}");
    net.stop().await;
    server.stop().await.unwrap();
}

/// The client's Who-Is goes to the BBMD as Distribute-Broadcast-To-Network
/// (Annex J.5); the BBMD's own device on its subnet answers, and the I-Am
/// comes back forwarded.
#[tokio::test]
async fn a_who_is_through_the_bbmd_reaches_its_subnet() {
    let mut server = bbmd_server(3303, true).await;
    let bbmd = server_address(&server);
    let mut net = Net::start(plan(Some(bbmd))).await;
    net.pump_until("registered", |app| {
        app.network
            .registration
            .is_some_and(|(snapshot, _)| snapshot.registered)
    })
    .await;
    let spec = WhoIsSpec {
        scope: WhoIsScope::Global,
        range: None,
        listen: Duration::from_secs(2),
    };
    net.send(vec![Command::WhoIs { op: 1, spec }]).await;
    net.pump_until("the I-Am", |app| app.devices.get(3303).is_some())
        .await;
    assert_eq!(net.app.devices.get(3303).unwrap().address, bbmd.to_string());
    net.stop().await;
    server.stop().await.unwrap();
}
