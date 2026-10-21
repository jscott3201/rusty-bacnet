//! The MS/TP monitor screen and the safe-join wizard (#970), driven end to
//! end on paused time: the scripted ring of `bacnet-transport`'s monitor
//! tests plays into the monitor session through the coordinator, and the
//! app takes its reports as the UI loop would. The test keeps the line's
//! other end, so it can check that nothing was ever written to it.

use std::sync::Arc;

use bacnet_server::server::BACnetServer;
use bacnet_transport::mstp::codec::FrameType;
use bacnet_transport::mstp::monitor::script::{Damage, LineScript, ScriptedRing};
use bacnet_transport::mstp::{LoopbackSerial, MstpConfig, MstpTransport};
use insta::assert_snapshot;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::site::bounded;
use super::*;
use crate::tui::app::monitor::{MonitorTab, Wizard};
use crate::tui::app::watch::Screen;
use crate::tui::app::{Link, Overlay};
use crate::tui::monitor::lines::test_line::TestLine;
use crate::tui::monitor::{analysis, Lines, MonitorPlan, MstpOptions};
use crate::tui::worker::session::{ConnectPlan, SessionOptions, TransportKind};
use crate::tui::worker::{coordinator, EventSink, EVENT_CHANNEL_CAPACITY};

const BAUD: u32 = 38_400;
const PORT: &str = "/dev/ttyUSB7";
/// A broadcast NPDU, as the #958 tests use.
const ANNOUNCE: [u8; 3] = [0x01, 0x00, 0x30];
const REQUEST: [u8; 3] = [0x01, 0x04, 0x10];
const REPLY: [u8; 3] = [0x01, 0x00, 0x20];
const HEADER_ERRORS: usize = 3;
const DATA_ERRORS: usize = 5;

/// An I-Am from device `instance`, as a broadcast NPDU.
fn i_am(instance: u32) -> Vec<u8> {
    use bacnet_services::who_is::IAmRequest;
    use bacnet_types::enums::{ObjectType, Segmentation};
    use bacnet_types::primitives::ObjectIdentifier;
    let request = IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        max_apdu_length: 480,
        segmentation_supported: Segmentation::NONE,
        vendor_id: 260,
    };
    let mut apdu = bytes::BytesMut::new();
    request.encode(&mut apdu);
    let mut npdu = vec![0x01, 0x00, 0x10, 0x00];
    npdu.extend_from_slice(&apdu);
    npdu
}

/// The issue's ring: masters 1 and 2, station 2 at Max_Master 3, station
/// 5 answering station 1 but never given the token; station 1 announces
/// itself once, station 2's first broadcasts are damaged; one lost token;
/// and so on to 9.5 s.
pub(super) fn acceptance_line() -> Vec<(Duration, Vec<u8>)> {
    let mut ring = ScriptedRing::new(BAUD).master(1, 127).master(2, 3);
    let (mut announced, mut sent) = (false, 0);
    let mut traffic = |line: &mut LineScript, holder: u8| {
        if holder == 1 {
            if !announced {
                line.frame(
                    FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                    255,
                    1,
                    &i_am(1001),
                );
                announced = true;
            }
            line.frame(FrameType::BACNET_DATA_EXPECTING_REPLY, 5, 1, &REQUEST)
                .frame(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 1, 5, &REPLY);
            return;
        }
        let damage = match sent {
            n if n < HEADER_ERRORS => Some(Damage::HeaderCrc),
            n if n < HEADER_ERRORS + DATA_ERRORS => Some(Damage::DataCrc),
            _ => None,
        };
        sent += 1;
        let kind = FrameType::BACNET_DATA_NOT_EXPECTING_REPLY;
        match damage {
            Some(damage) => line.damaged(kind, 255, 2, &ANNOUNCE, damage),
            None => line.frame(kind, 255, 2, &ANNOUNCE),
        };
    };
    ring.rotate(40, &mut traffic);
    ring.lose_token(Duration::from_millis(650));
    until(&mut ring, Duration::from_millis(9_500), &mut traffic);
    ring.line.chunks().to_vec()
}

/// Rotate `ring` until its line reaches `end`.
fn until(ring: &mut ScriptedRing, end: Duration, traffic: &mut impl FnMut(&mut LineScript, u8)) {
    while ring.line.chunks().last().is_none_or(|(at, _)| *at < end) {
        ring.rotate(10, &mut *traffic);
    }
}

/// A monitor session through the coordinator, with the app beside it.
pub(super) struct Run {
    pub(super) app: App,
    events: mpsc::Receiver<WorkerEvent>,
    commands: Option<mpsc::Sender<Command>>,
    coordinator: Option<JoinHandle<()>>,
    /// The scripted peer's view of the line, what the monitor wrote, and
    /// the task that plays the script.
    peer: Option<(ReadHalf<DuplexStream>, JoinHandle<()>)>,
}

impl Run {
    /// Play `chunks` into a monitor on [`PORT`]; `participant` is the port's
    /// other life, for a join.
    pub(super) async fn start(
        chunks: Vec<(Duration, Vec<u8>)>,
        participant: Option<LoopbackSerial>,
        passive: bool,
    ) -> Self {
        let (line, monitor_end) = tokio::io::duplex(1 << 16);
        let (peer, mut writer) = tokio::io::split(line);
        let player = tokio::spawn(async move {
            let start = tokio::time::Instant::now();
            for (at, octets) in chunks {
                tokio::time::sleep_until(start + at).await;
                // The monitor may have gone (a join): the rest is unheard.
                if writer.write_all(&octets).await.is_err() {
                    return;
                }
            }
        });
        let lines = Lines::Test(TestLine::new(monitor_end, participant));
        let mut run = Self::on(lines, PORT, passive);
        run.peer = Some((peer, player));
        run
    }

    /// A monitor on `port`, opened through `lines`.
    pub(super) fn on(lines: Lines, port: &str, passive: bool) -> Self {
        let plan = MonitorPlan::new(port.into(), BAUD, 1);
        let mut options = SessionOptions {
            mstp: MstpOptions {
                monitor: Some(plan.clone()),
                lines,
                thread: Default::default(),
            },
            ..SessionOptions::default()
        };
        options.safety.passive = passive;
        let connect = ConnectPlan {
            kind: TransportKind::MstpMonitor,
            args: crate::tui::profile::Profile::default()
                .transport_args()
                .unwrap(),
            await_interface: false,
        };
        let (sink, events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
        let (commands, command_rx) = mpsc::channel(16);
        let coordinator = coordinator::spawn(Some(connect), None, sink, command_rx, options);
        let mut app = App::new(
            AppConfig {
                transport: "MSTP",
                style: AddressStyle::Mstp,
                color: false,
                picker: None,
                join: None,
                passive,
                now: t0(),
            },
            LogRing::new(100),
        );
        app.start_monitor(&plan, !passive);
        Self {
            app,
            events,
            commands: Some(commands),
            coordinator: Some(coordinator),
            peer: None,
        }
    }

    async fn send(&self, commands: Vec<Command>) {
        for command in commands {
            self.commands.as_ref().unwrap().send(command).await.unwrap();
        }
    }

    pub(super) async fn press(&mut self, code: KeyCode) {
        let commands = press(&mut self.app, code);
        self.send(commands).await;
    }

    /// Apply worker events until `done` holds.
    async fn pump_until(&mut self, done: impl Fn(&App) -> bool) {
        bounded(async {
            while !done(&self.app) {
                let event = self.events.recv().await.expect("coordinator stopped");
                let commands = worker(&mut self.app, event);
                self.send(commands).await;
            }
        })
        .await;
    }

    /// Until the monitor has listened for `span`.
    pub(super) async fn listen_for(&mut self, span: Duration) {
        self.pump_until(|app| {
            app.monitor
                .as_deref()
                .and_then(|m| m.latest.as_deref())
                .is_some_and(|sample| sample.elapsed >= span)
        })
        .await;
    }

    fn sample(&self) -> &crate::tui::monitor::MonitorSample {
        self.app
            .monitor
            .as_deref()
            .unwrap()
            .shown
            .as_deref()
            .unwrap()
    }

    /// Quit as the UI does, then read what reached the scripted peer: the
    /// monitor's end is closed by then, so this ends at once.
    pub(super) async fn quit(mut self) -> Vec<u8> {
        self.commands = None;
        bounded(self.coordinator.take().unwrap()).await.unwrap();
        let mut written = Vec::new();
        if let Some((mut peer, player)) = self.peer.take() {
            player.abort();
            bounded(peer.read_to_end(&mut written)).await.unwrap();
        }
        written
    }
}

/// The issue's acceptance run: the grid shows stations 1, 2 and 5; the
/// Max_Master warning on station 2 names station 5; the CRC counters hold
/// the injected counts; one lost token is logged; and in 10 s nothing is
/// written to the line.
#[tokio::test(start_paused = true)]
async fn the_monitor_reports_the_scripted_ring_and_writes_nothing() {
    let mut run = Run::start(acceptance_line(), None, false).await;
    run.listen_for(Duration::ZERO).await;
    run.listen_for(Duration::from_secs(10)).await;
    assert_eq!(run.app.screen, Screen::Mstp);
    assert_eq!(run.app.level(), crate::tui::safety::Level::Passive);

    let sample = run.sample().clone();
    let rows = analysis::stations(&sample);
    let grid: Vec<(u8, &str, Option<u32>)> = rows
        .iter()
        .map(|row| (row.mac, row.role.label(), row.device))
        .collect();
    assert_eq!(
        grid,
        [
            (1, "master", Some(1001)),
            (2, "master", None),
            (5, "slave", None)
        ]
    );
    let findings = analysis::findings(&sample, &Default::default());
    let warning = findings
        .iter()
        .find(|f| f.text.starts_with("mstp:2:"))
        .expect("a Max_Master warning on station 2");
    assert_eq!(warning.severity, analysis::Severity::Warning);
    assert!(warning.text.contains("mstp:5"), "{}", warning.text);
    let errors = run.app.monitor.as_deref().unwrap().errors().unwrap();
    assert_eq!(errors[0], ("Header CRC errors", HEADER_ERRORS as u64));
    assert_eq!(errors[1], ("Data CRC errors", DATA_ERRORS as u64));
    assert_eq!(errors[5], ("Lost tokens", 1));
    assert_eq!(sample.lost.len(), 1);
    assert_eq!(sample.lost[0].regenerated_by, Some(1));

    let text = screen(&mut run.app, 120, 40);
    for want in [
        "Masters",
        "  1 master   1001",
        "  5 slave",
        "listen only",
        "PASSIVE",
    ] {
        assert!(text.contains(want), "{want}:\n{text}");
    }
    assert_snapshot!("monitor_stations_120x40", text);
    assert_snapshot!("monitor_stations_80x24", screen(&mut run.app, 80, 24));
    run.press(KeyCode::Tab).await;
    assert_eq!(run.app.monitor.as_deref().unwrap().tab, MonitorTab::Ring);
    let text = screen(&mut run.app, 120, 40);
    assert!(
        text.contains("Successors  mstp:1 -> mstp:2 -> mstp:1"),
        "{text}"
    );
    assert_snapshot!("monitor_ring_120x40", text);
    assert_snapshot!("monitor_ring_80x24", screen(&mut run.app, 80, 24));
    run.press(KeyCode::Tab).await;
    assert_snapshot!("monitor_findings_80x24", screen(&mut run.app, 80, 24));
    run.press(KeyCode::Tab).await;
    let text = screen(&mut run.app, 80, 24);
    assert_eq!(
        text.matches("Token lost at").count(),
        1,
        "one event:\n{text}"
    );
    assert_snapshot!("monitor_errors_80x24", text);

    assert_eq!(run.quit().await, b"", "the monitor wrote to the line");
}

/// `z` counts errors from now and Space holds the screen, as on the
/// participant panel; `:mstp hold` moves the threshold.
#[tokio::test(start_paused = true)]
async fn errors_clear_and_pause_and_thresholds_move() {
    let mut run = Run::start(acceptance_line(), None, false).await;
    run.listen_for(Duration::from_secs(2)).await;
    run.press(KeyCode::Char('z')).await;
    let errors = |run: &Run| run.app.monitor.as_deref().unwrap().errors().unwrap();
    assert_eq!(errors(&run)[0].1, 0, "cleared");
    run.press(KeyCode::Char(' ')).await;
    let held = run.sample().elapsed;
    run.listen_for(Duration::from_secs(4)).await;
    assert_eq!(run.sample().elapsed, held, "paused");
    run.press(KeyCode::Char(' ')).await;
    assert!(run.sample().elapsed >= Duration::from_secs(4));

    let holds = |run: &Run| {
        analysis::findings(
            run.sample(),
            &run.app.monitor.as_deref().unwrap().thresholds,
        )
        .iter()
        .filter(|f| f.text.contains("held the token"))
        .count()
    };
    assert_eq!(holds(&run), 0);
    run.press(KeyCode::Char(':')).await;
    type_text(&mut run.app, "mstp hold 5");
    run.press(KeyCode::Enter).await;
    assert_eq!(holds(&run), 2, "both masters hold longer than 5 ms");
    assert_eq!(run.quit().await, b"");
}

/// A ring of masters 1 and 2 at Max_Master 10, on paused time.
fn join_line() -> Vec<(Duration, Vec<u8>)> {
    let mut ring = ScriptedRing::new(BAUD).master(1, 10).master(2, 10);
    until(&mut ring, Duration::from_secs(20), &mut |_, _| {});
    ring.line.chunks().to_vec()
}

/// [`join_line`], with station 3 answering from 12 s and station 4 from
/// 16 s: after the wizard has proposed 3, then 4.
fn crowding_line() -> Vec<(Duration, Vec<u8>)> {
    let mut ring = ScriptedRing::new(BAUD).master(1, 10).master(2, 10);
    until(
        &mut ring,
        Duration::from_secs(22),
        &mut |line: &mut LineScript, _| {
            let now = line.chunks().last().map_or(Duration::ZERO, |(at, _)| *at);
            for (station, from) in [(3, 12), (4, 16)] {
                if now >= Duration::from_secs(from) {
                    line.frame(
                        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                        255,
                        station,
                        &ANNOUNCE,
                    );
                }
            }
        },
    );
    ring.line.chunks().to_vec()
}

/// A proposal is checked against the newest reading on Enter and again on
/// `y`: a station that turns up at the proposed MAC after the listen
/// window is never offered, and never joined as.
#[tokio::test(start_paused = true)]
async fn a_mac_taken_after_the_proposal_is_never_joined() {
    let mut run = Run::start(crowding_line(), None, false).await;
    run.listen_for(Duration::ZERO).await;
    assert_eq!(propose(&mut run).await.mac, 3);
    run.listen_for(Duration::from_secs(14)).await;
    assert!(press(&mut run.app, KeyCode::Enter).is_empty());
    assert_eq!(run.app.overlay, Overlay::None, "mstp:3 is taken now");
    let flash = run.app.flash.as_ref().unwrap().text.clone();
    assert!(flash.contains("mstp:3 is no longer free"), "{flash}");
    let monitor = run.app.monitor.as_deref().unwrap();
    assert_eq!(monitor.join_settings().unwrap().mac, 4, "proposed again");

    // Enter offers 4; station 4 turns up while the dialog is open; `y`
    // sends nothing.
    press(&mut run.app, KeyCode::Enter);
    assert!(matches!(run.app.overlay, Overlay::Join(_)));
    run.listen_for(Duration::from_secs(18)).await;
    assert!(press(&mut run.app, KeyCode::Char('y')).is_empty());
    assert_eq!(run.app.overlay, Overlay::None);
    let monitor = run.app.monitor.as_deref().unwrap();
    assert_eq!(monitor.joining, None);
    assert_eq!(monitor.join_settings().unwrap().mac, 5);
    assert_eq!(run.app.link, Link::Passive);
    assert_eq!(run.quit().await, b"");
}

/// A join the coordinator refuses, or the command queue doesn't admit,
/// leaves the session monitoring, passive, with the reason shown.
#[tokio::test(start_paused = true)]
async fn a_refused_join_keeps_the_monitor() {
    // A passive coordinator refuses every join; let the screen ask anyway.
    let mut run = Run::start(join_line(), None, true).await;
    run.app.monitor.as_deref_mut().unwrap().join_allowed = true;
    run.listen_for(Duration::ZERO).await;
    propose(&mut run).await;
    run.press(KeyCode::Enter).await;
    let join = press(&mut run.app, KeyCode::Char('y'));
    assert_eq!(join.len(), 1);
    run.send(join).await;
    run.pump_until(|app| app.monitor.as_deref().is_some_and(|m| m.joining.is_none()))
        .await;
    let flash = run.app.flash.as_ref().unwrap().text.clone();
    assert!(flash.starts_with("Not joined: passive"), "{flash}");
    assert_eq!(run.app.link, Link::Passive);
    assert_eq!(run.app.level(), crate::tui::safety::Level::Passive);
    let before = run.sample().elapsed;
    run.listen_for(before + Duration::from_secs(2)).await;

    // Not admitted to the command queue: the same.
    run.press(KeyCode::Enter).await;
    let mut join = press(&mut run.app, KeyCode::Char('y'));
    let command = join.pop().unwrap();
    run.app
        .command_rejected(command, Link::Passive, "Command not admitted: full".into());
    assert!(run.app.monitor.as_deref().unwrap().joining.is_none());
    assert_eq!(run.app.link, Link::Passive);
    assert!(run.app.join_again.is_none());
    assert_eq!(run.quit().await, b"");
}

/// The device the joined station finds.
const PEER: u32 = 2002;

/// The safe join: listen, propose a free MAC within Max_Master, confirm in
/// #969's dialog (T2), then the monitor closes the port and the participant
/// opens it; a Who-Is from the new station finds the peer's device.
#[tokio::test(start_paused = true)]
async fn the_wizard_proposes_a_free_mac_and_joins_on_y() {
    let (peer_line, joiner_line) = LoopbackSerial::pair();
    let mut server = peer_server(peer_line).await;
    let mut run = Run::start(join_line(), Some(joiner_line), false).await;
    run.listen_for(Duration::ZERO).await;
    let settings = propose(&mut run).await;
    assert!(
        ![1, 2].contains(&settings.mac) && settings.mac <= 10,
        "{settings:?}"
    );
    assert_eq!((settings.mac, settings.max_master), (3, 10));
    assert_eq!(settings.port, PORT);
    assert_snapshot!("monitor_proposal_80x24", screen(&mut run.app, 80, 24));
    // The test line refuses the participant while the monitor's reader is
    // alive, so joining proves the monitor closed the port first.
    join_and_discover(&mut run, &settings, true).await;
    assert_eq!(run.quit().await, b"", "the monitor wrote to the line");
    bounded(server.stop()).await.unwrap();
}

/// The device the joined station finds: station 2 at Max_Master 10.
pub(super) async fn peer_server<S: bacnet_transport::mstp::SerialPort>(
    serial: S,
) -> BACnetServer<MstpTransport<S>> {
    let identity = bacnet_objects::DeviceIdentity::new(PEER, 260)
        .and_then(|identity| identity.with_max_apdu(480))
        .unwrap();
    let mut database = bacnet_objects::database::ObjectDatabase::new();
    let device = bacnet_objects::device::DeviceObject::new(bacnet_objects::device::DeviceConfig {
        identity: identity.clone(),
        ..Default::default()
    })
    .unwrap();
    database.add(Box::new(device)).unwrap();
    let config = MstpConfig {
        this_station: 2,
        max_master: 10,
        ..MstpConfig::default()
    };
    BACnetServer::generic_builder()
        .device(identity)
        .database(database)
        .transport(MstpTransport::new(serial, config))
        .build()
        .await
        .unwrap()
}

/// `J`, then wait for the proposal; its settings.
pub(super) async fn propose(run: &mut Run) -> crate::mstp::MstpArgs {
    run.press(KeyCode::Char('J')).await;
    run.pump_until(|app| {
        !matches!(
            app.monitor.as_deref().and_then(|m| m.wizard.as_ref()),
            Some(Wizard::Listening { .. }) | None
        )
    })
    .await;
    let monitor = run.app.monitor.as_deref().unwrap();
    match monitor.join_settings() {
        Some(settings) => settings,
        None => panic!("no proposal: {:?}", monitor.wizard),
    }
}

/// Enter opens #969's confirmation, where nothing but `y` joins; then a
/// Who-Is from the joined station finds [`PEER`] at `mstp:2`.
pub(super) async fn join_and_discover(
    run: &mut Run,
    settings: &crate::mstp::MstpArgs,
    snapshot: bool,
) {
    run.press(KeyCode::Enter).await;
    let Overlay::Join(dialog) = &run.app.overlay else {
        panic!("expected the join confirmation, got {:?}", run.app.overlay);
    };
    assert_eq!(&dialog.settings, settings);
    if snapshot {
        assert_snapshot!("monitor_join_80x24", screen(&mut run.app, 80, 24));
    }
    assert!(press(&mut run.app, KeyCode::Enter).is_empty());
    let join = press(&mut run.app, KeyCode::Char('y'));
    assert_eq!(
        join,
        [Command::JoinTrunk {
            settings: settings.clone()
        }]
    );
    // Still the monitor, and passive, until the new session starts.
    let monitor = run.app.monitor.as_deref().unwrap();
    assert_eq!(monitor.joining.as_ref(), Some(settings));
    assert_eq!(run.app.level(), crate::tui::safety::Level::Passive);
    run.send(join).await;
    run.pump_until(|app| matches!(app.link, Link::Up { .. } | Link::Failed(_)))
        .await;
    assert!(run.app.monitor.is_none());
    assert_eq!(run.app.level(), crate::tui::safety::Level::ReadOnly);
    assert!(
        matches!(run.app.link, Link::Up { .. }),
        "{:?}",
        run.app.link
    );
    assert_eq!(run.app.mstp.station.as_ref(), Some(settings));

    run.press(KeyCode::Char('1')).await;
    run.press(KeyCode::Char('d')).await;
    let mut commands = press(&mut run.app, KeyCode::Enter);
    if commands.is_empty() {
        commands = press(&mut run.app, KeyCode::Enter);
    }
    run.send(commands).await;
    run.pump_until(|app| app.devices.get(PEER).is_some()).await;
    assert_eq!(run.app.devices.get(PEER).unwrap().address, "mstp:2");
}

/// With `--passive` the wizard still proposes, but neither the screen nor
/// the coordinator joins.
#[tokio::test(start_paused = true)]
async fn a_passive_monitor_proposes_but_never_joins() {
    let mut run = Run::start(join_line(), None, true).await;
    run.listen_for(Duration::ZERO).await;
    run.press(KeyCode::Char(':')).await;
    type_text(&mut run.app, "mstp listen 3");
    run.press(KeyCode::Enter).await;
    run.press(KeyCode::Char('J')).await;
    run.pump_until(|app| {
        matches!(
            app.monitor.as_deref().and_then(|m| m.wizard.as_ref()),
            Some(Wizard::Proposed { .. })
        )
    })
    .await;
    assert!(press(&mut run.app, KeyCode::Enter).is_empty());
    assert_eq!(
        run.app.overlay,
        Overlay::None,
        "no confirmation in a passive session"
    );
    let settings = run.app.monitor.as_deref().unwrap().join_settings().unwrap();
    run.send(vec![Command::JoinTrunk { settings }]).await;
    run.pump_until(|app| {
        app.flash
            .as_ref()
            .is_some_and(|f| f.text == "passive: this session never joins the trunk")
    })
    .await;
    // The monitor goes on: later samples still come from it.
    let before = run.sample().elapsed;
    run.listen_for(before + Duration::from_secs(2)).await;
    assert_eq!(run.app.link, Link::Passive, "still monitoring");
    assert_eq!(run.quit().await, b"");
}

/// Statistics from a line played into a monitor, for the analysis tests.
async fn stats_of(script: &LineScript) -> crate::tui::monitor::MonitorSample {
    let mut run = Run::start(script.chunks().to_vec(), None, false).await;
    let end = script.chunks().last().map_or(Duration::ZERO, |(at, _)| *at);
    run.listen_for(end + Duration::from_secs(1)).await;
    let sample = run.sample().clone();
    assert_eq!(run.quit().await, b"");
    sample
}

/// No master on the line: nothing to propose. A wide gap and a Max_Master
/// far above the highest station are notices; a lower bound is never
/// called too low.
#[tokio::test(start_paused = true)]
async fn the_analysis_is_honest_about_what_the_line_shows() {
    let mut quiet = LineScript::new(BAUD);
    quiet.frame(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        255,
        9,
        &ANNOUNCE,
    );
    let sample = stats_of(&quiet).await;
    assert!(analysis::propose(&sample)
        .unwrap_err()
        .contains("No master"));

    // A sole master at 4 polls round to itself: once its polls wrap at 6,
    // everything but 4 up to 6 is free and polled.
    let mut sole = ScriptedRing::new(BAUD).master(4, 6);
    until(&mut sole, Duration::from_millis(500), &mut |_, _| {});
    let sample = stats_of(&sole.line).await;
    let proposal = analysis::propose(&sample).unwrap();
    assert_eq!(proposal.candidates, [0, 1, 2, 3, 5, 6]);
    assert_eq!(proposal.candidates[proposal.preferred], 5);

    // Masters 1 and 20: station 1 polls 2..19, station 20 wraps at 127.
    let mut ring = ScriptedRing::new(BAUD).master(1, 127).master(20, 127);
    until(&mut ring, Duration::from_secs(3), &mut |_, _| {});
    let sample = stats_of(&ring.line).await;
    let texts: Vec<String> = analysis::findings(&sample, &Default::default())
        .into_iter()
        .map(|f| f.text)
        .collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("polls the 18 empty addresses")),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("mstp:20: Max_Master is at least")),
        "{texts:?}"
    );
    assert!(!texts.iter().any(|t| t.contains("below")), "{texts:?}");
    // Station 1's polls stop at 19, so 19 is all its Max_Master is known
    // to reach: the proposal stays in its gap, which it polls.
    let proposal = analysis::propose(&sample).unwrap();
    assert_eq!(proposal.candidates, (2..=19).collect::<Vec<u8>>());
    assert_eq!(proposal.candidates[proposal.preferred], 2);

    // Early on station 1 has polled only 2 to 8, but passing the token to
    // 20 proves its Max_Master reaches 20: 9 to 19 aren't excluded.
    let mut ring = ScriptedRing::new(BAUD).master(1, 127).master(20, 127);
    ring.rotate(14, |_, _| {});
    let sample = stats_of(&ring.line).await;
    let one = sample.stats.station(1).unwrap().max_master;
    assert_eq!(
        one,
        Some(bacnet_transport::mstp::monitor::MaxMaster::AtLeast(8))
    );
    let proposal = analysis::propose(&sample).unwrap();
    assert_eq!(proposal.candidates, (2..=19).collect::<Vec<u8>>());

    // Masters 1, 3 and 5: only 0 and 2 are within every bound the line
    // proves (station 1 reaches 3), and the gap above the lowest master is
    // preferred to 0.
    let mut ring = ScriptedRing::new(BAUD)
        .master(1, 10)
        .master(3, 10)
        .master(5, 10);
    until(&mut ring, Duration::from_secs(3), &mut |_, _| {});
    let sample = stats_of(&ring.line).await;
    let proposal = analysis::propose(&sample).unwrap();
    assert_eq!(proposal.candidates, [0, 2]);
    assert_eq!(proposal.candidates[proposal.preferred], 2);
}

/// The monitor session has closed the port by the time it returns, with
/// no task left to drop it later: the coordinator starts the participant
/// right after.
#[tokio::test(start_paused = true)]
async fn the_monitor_session_closes_the_port_before_it_returns() {
    let (_peer, monitor_end) = tokio::io::duplex(64);
    let line = TestLine::new(monitor_end, None);
    let (sink, mut events) = EventSink::channel(16);
    let (commands, rx) = mpsc::channel(4);
    commands.send(Command::Disconnect).await.unwrap();
    let plan = MonitorPlan::new(PORT.into(), BAUD, 1);
    let lines = Lines::Test(Arc::clone(&line));
    bounded(crate::tui::monitor::worker::serve(plan, &lines, &sink, rx)).await;
    assert!(!line.held(), "the port is still open");
    assert_eq!(events.recv().await, Some(WorkerEvent::Passive));
}

/// The test line refuses a second owner, as an exclusive open of a real
/// port does: the guarantee the join test leans on.
#[tokio::test]
async fn the_test_line_has_one_owner_at_a_time() {
    let (_peer, monitor_end) = tokio::io::duplex(64);
    let (_, joiner) = LoopbackSerial::pair();
    let line = TestLine::new(monitor_end, Some(joiner));
    let lines = Lines::Test(Arc::clone(&line));
    let reader = lines.open_monitor(PORT, BAUD).unwrap();
    let mut args = crate::tui::profile::Profile::default()
        .transport_args()
        .unwrap();
    args.mstp = Some(crate::mstp::MstpArgs {
        port: PORT.into(),
        baud: BAUD,
        mac: 3,
        max_master: 10,
        max_info_frames: 1,
    });
    let plain = bacnet_transport::mstp::MstpThreadConfig::new();
    // Thread settings the platform refuses fail the join (#501).
    #[cfg(not(target_os = "linux"))]
    {
        let realtime = plain.clone().with_realtime_priority(50);
        let refused = lines.participant(&args, &realtime).await.err();
        let refused = refused.expect("refused").to_string();
        assert!(refused.contains("Linux only"), "{refused}");
    }
    let busy = lines.participant(&args, &plain).await.err().expect("busy");
    assert!(busy.to_string().contains("still holds the port"), "{busy}");
    drop(reader);
    let (mut client, _) = lines.participant(&args, &plain).await.unwrap();
    client.stop().await.unwrap();
}
