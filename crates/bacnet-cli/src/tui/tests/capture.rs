//! The Capture screen's core (#971): synthetic frames through `update`, the
//! filter, pause, the bounded ring, decoding only the selected frame, the
//! export command; and the pump's exact loss count under 5,000 frames a
//! second.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bacnet_encoding::apdu::Apdu;
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::Bytes;
use ratatui::crossterm::event::KeyCode;
use tokio::sync::mpsc;

use super::*;
use crate::tui::app::capture::{CaptureKey, CaptureState, Focus};
use crate::tui::app::watch::Screen;
use crate::tui::capture::worker::{Source, pump, route};
use crate::tui::capture::{Batch, CaptureCommand, CaptureEvent, CaptureFrame, Direction, Framing};
use crate::tui::worker::EventSink;
use bacnet_ops::decode::frame::summarize_frame;
use bacnet_ops::decode::testing;

/// A fixed wall-clock time, `ms` after 2025-10-09 08:53:20 UTC.
pub(super) fn wall(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_760_000_000) + Duration::from_millis(ms)
}

const ME: &str = "10.0.0.1:47808";
pub(super) const PEER: &str = "10.0.0.2:47808";

/// A B/IP frame carrying `apdu`, sent (`out`) to or received from `PEER`.
pub(super) fn bip_frame(ms: u64, out: bool, apdu: &Apdu, broadcast: bool) -> CaptureFrame {
    let expecting = matches!(apdu, Apdu::ConfirmedRequest(_));
    let bytes = testing::bip(&testing::npdu(apdu, expecting), broadcast);
    raw_bip(ms, out, bytes)
}

pub(super) fn raw_bip(ms: u64, out: bool, bytes: Vec<u8>) -> CaptureFrame {
    let (me, peer): (SocketAddr, SocketAddr) = (ME.parse().unwrap(), PEER.parse().unwrap());
    let (source, destination) = if out { (me, peer) } else { (peer, me) };
    let udp = (source, destination);
    let summary = summarize_frame(Framing::Bip, &bytes);
    // As the worker does: a routed NPDU's ends are routed stations.
    let (source, destination, via) =
        route(source.to_string(), destination.to_string(), &summary, out);
    CaptureFrame {
        time: wall(ms),
        framing: Framing::Bip,
        direction: if out { Direction::Out } else { Direction::In },
        udp: Some(udp),
        source,
        destination,
        via,
        summary,
        redacted: false,
        bytes: Bytes::from(bytes),
    }
}

/// `apdu` relayed to us by PEER, a router, from station 0x05 on network 2.
pub(super) fn routed_in(ms: u64, apdu: &Apdu) -> CaptureFrame {
    let npdu = testing::routed_npdu(apdu, false, Some((2, &[0x05])));
    raw_bip(ms, false, testing::bip(&npdu, false))
}

/// A mixed session: two ReadProperty exchanges (one answered with an
/// Error), a ReadPropertyMultiple Error, an I-Am, a refused registration.
pub(super) fn mixed() -> Vec<CaptureFrame> {
    let rpm = ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE;
    let rp = ConfirmedServiceChoice::READ_PROPERTY;
    vec![
        bip_frame(0, true, &testing::rp_request(1), false),
        bip_frame(12, false, &testing::rp_ack(1), false),
        bip_frame(250, true, &testing::rp_request(2), false),
        bip_frame(261, false, &testing::error(2, rp), false),
        bip_frame(500, false, &testing::error(3, rpm), false),
        bip_frame(750, false, &testing::i_am(1234), true),
        raw_bip(1_000, false, testing::bip_nak()),
    ]
}

/// Hand `frames` to the app as one batch, with `lost` lost before it.
pub(super) fn deliver(app: &mut App, frames: Vec<CaptureFrame>, lost: u64) {
    let batch = Box::new(Batch { frames, lost });
    worker(app, WorkerEvent::Capture(CaptureEvent::Frames(batch)));
}

/// A connected app on the Capture screen.
pub(super) fn capturing() -> App {
    let mut app = connected_app();
    press(&mut app, KeyCode::Char('7'));
    app
}

pub(super) fn shown(app: &App) -> Vec<u64> {
    app.capture.visible().collect()
}

pub(super) fn filter(app: &mut App, text: &str) {
    press(app, KeyCode::Char('/'));
    type_text(app, text);
    press(app, KeyCode::Enter);
}

#[test]
fn opening_the_screen_starts_one_capture_per_session() {
    let mut app = new_app(None);
    assert!(
        press(&mut app, KeyCode::Char('7')).is_empty(),
        "nothing up yet"
    );
    assert_eq!(app.screen, Screen::Capture);
    let start = vec![Command::Capture(CaptureCommand::Start)];
    let connected = WorkerEvent::Connected { local: ME.into() };
    assert_eq!(worker(&mut app, connected.clone()), start);
    // Back and forth: the capture is running already.
    press(&mut app, KeyCode::Char('1'));
    assert!(press(&mut app, KeyCode::Char('7')).is_empty());
    // A new session captures from its start.
    assert_eq!(worker(&mut app, connected), start);

    // A session that never opened the screen never subscribes.
    let mut app = connected_app();
    assert!(worker(&mut app, WorkerEvent::Connected { local: ME.into() }).is_empty());
}

#[test]
fn service_read_property_errors_only_keeps_only_its_error_responses() {
    let mut app = capturing();
    deliver(&mut app, mixed(), 0);
    assert_eq!(shown(&app), [1, 2, 3, 4, 5, 6, 7]);
    filter(&mut app, "svc:ReadProperty errors");
    assert_eq!(shown(&app), [4]);
    // The other spellings of the service name agree.
    for text in ["service:read-property errors", "svc:READ_PROPERTY err"] {
        press(&mut app, KeyCode::Esc);
        filter(&mut app, text);
        assert_eq!(shown(&app), [4], "{text}");
    }
    // Frames that arrive later are filtered as they come.
    let late = bip_frame(
        2_000,
        false,
        &testing::error(9, ConfirmedServiceChoice::READ_PROPERTY),
        false,
    );
    deliver(&mut app, vec![late], 0);
    assert_eq!(shown(&app), [4, 8]);
    // Esc clears the filter, and then leaves the screen.
    press(&mut app, KeyCode::Esc);
    assert_eq!(shown(&app).len(), 8);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Main);
}

#[test]
fn every_filter_term_narrows_the_list() {
    let mut app = capturing();
    // Device 1234 is at PEER in the device table.
    worker(&mut app, WorkerEvent::Discovered(row(1234, PEER, 260, 0)));
    deliver(&mut app, mixed(), 0);
    let cases: [(&str, &[u64]); 9] = [
        ("errors", &[4, 5, 7]),
        ("bcast", &[6]),
        ("pdu:confirmed", &[1, 3]),
        ("pdu:ack", &[2]),
        ("pdu:link", &[7]),
        ("dev:1234", &[1, 2, 3, 4, 5, 6, 7]),
        ("svc:rpm", &[]),
        ("svc:readpropertymultiple", &[5]),
        ("#2 out", &[3]),
    ];
    for (text, expected) in cases {
        filter(&mut app, text);
        assert_eq!(shown(&app), expected, "{text}");
        press(&mut app, KeyCode::Esc);
    }
    // A bad number is refused, and the list stays as it was.
    filter(&mut app, "net:x");
    assert_eq!(shown(&app).len(), 7);
    assert!(
        app.flash
            .as_ref()
            .unwrap()
            .text
            .contains("net takes a number")
    );
}

#[test]
fn space_pauses_the_list_while_frames_keep_arriving() {
    let mut app = capturing();
    deliver(&mut app, mixed()[..3].to_vec(), 0);
    assert_eq!(app.capture.selected(), Some(3), "following the newest");
    press(&mut app, KeyCode::Char(' '));
    deliver(&mut app, mixed()[3..].to_vec(), 0);
    assert_eq!(shown(&app), [1, 2, 3]);
    assert_eq!(app.capture.held_back(), 4);
    assert_eq!(app.capture.selected(), Some(3));
    assert!(screen(&mut app, 120, 40).contains("PAUSED +4"));
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(shown(&app).len(), 7);
    assert_eq!(app.capture.selected(), Some(7));

    // Moving the selection pauses too, and the selection stays put.
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.capture.selected(), Some(5));
    deliver(&mut app, mixed(), 0);
    assert_eq!(app.capture.selected(), Some(5));
    assert!(app.capture.paused.is_some());
    press(&mut app, KeyCode::Home);
    assert_eq!(app.capture.selected(), Some(1));
}

#[test]
fn the_ring_keeps_its_newest_frames_and_numbers_go_on() {
    let mut app = capturing();
    app.capture = CaptureState::new(100);
    for round in 0..5u64 {
        let frames =
            (0..50).map(|i| bip_frame(round * 50 + i, true, &testing::rp_request(1), false));
        deliver(&mut app, frames.collect(), 0);
    }
    assert_eq!(app.capture.ring.len(), 100);
    assert_eq!(shown(&app).first(), Some(&151));
    assert_eq!(shown(&app).last(), Some(&250));
    assert!(app.capture.ring.get(150).is_none());
    // A pause outlived by its frames shows what is left of it.
    press(&mut app, KeyCode::Char(' '));
    let more = (0..60).map(|i| bip_frame(i, true, &testing::rp_request(1), false));
    deliver(&mut app, more.collect(), 0);
    assert_eq!(shown(&app).first(), Some(&211));
    assert_eq!(shown(&app).last(), Some(&250));
    // z empties it.
    press(&mut app, KeyCode::Char('z'));
    assert_eq!(app.capture.ring.len(), 0);
    assert!(screen(&mut app, 80, 24).contains("No frames yet."));
}

#[test]
fn only_the_selected_frame_is_decoded() {
    let mut app = capturing();
    deliver(&mut app, mixed(), 0);
    press(&mut app, KeyCode::Up);
    app.settle();
    assert_eq!(app.capture.decodes, 1);
    // A thousand more frames while paused decode nothing.
    let flood = (0..1_000).map(|i| bip_frame(i, true, &testing::rp_request(5), false));
    deliver(&mut app, flood.collect(), 0);
    for _ in 0..10 {
        app.settle();
    }
    assert_eq!(app.capture.decodes, 1);
    assert_eq!(app.capture.detail.as_ref().unwrap().number, 6);
    // Following, at most one decode a drawn frame, however many arrive.
    press(&mut app, KeyCode::Char(' '));
    for _ in 0..3 {
        let batch = (0..100).map(|i| bip_frame(i, true, &testing::rp_request(5), false));
        deliver(&mut app, batch.collect(), 0);
        app.settle();
    }
    assert_eq!(app.capture.decodes, 4);
    // Off the screen nothing is decoded.
    press(&mut app, KeyCode::Char('1'));
    deliver(&mut app, mixed(), 0);
    app.settle();
    assert_eq!(app.capture.decodes, 4);
}

#[test]
fn the_tree_selects_a_field_and_the_hex_view_marks_its_octets() {
    let mut app = capturing();
    deliver(&mut app, mixed(), 0);
    press(&mut app, KeyCode::Home);
    app.settle();
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.capture.focus, Focus::Tree);
    // BVLL (4 fields), NPDU (2), APDU: type, max, invoke ID.
    let mut label = String::new();
    for _ in 0..20 {
        press(&mut app, KeyCode::Down);
        label = app.capture.selected_field().unwrap().label.clone();
        if label.starts_with("Invoke ID") {
            break;
        }
    }
    assert_eq!(label, "Invoke ID: 1");
    assert_eq!(app.capture.selected_field().unwrap().range, 8..9);
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.capture.focus, Focus::Hex);
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("Hex: octet 8 "), "{text}");
    // Left closes the APDU's node, or climbs to it.
    press(&mut app, KeyCode::BackTab);
    press(&mut app, KeyCode::Left);
    assert!(
        app.capture
            .selected_field()
            .unwrap()
            .label
            .starts_with("APDU")
    );
    // Closed, it hides its fields: it is the last row.
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::End);
    assert!(
        app.capture
            .selected_field()
            .unwrap()
            .label
            .starts_with("APDU")
    );
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::End);
    assert!(
        app.capture
            .selected_field()
            .unwrap()
            .label
            .starts_with("Property: present-value")
    );
}

#[test]
fn export_hands_the_kept_frames_to_the_coordinator() {
    let mut app = capturing();
    assert!(press(&mut app, KeyCode::Char('e')).is_empty());
    assert_eq!(app.flash.as_ref().unwrap().text, "Nothing to export.");
    deliver(&mut app, mixed(), 0);
    filter(&mut app, "errors");
    let commands = press(&mut app, KeyCode::Char('E'));
    let [Command::CaptureExport(job)] = commands.as_slice() else {
        panic!("{commands:?}");
    };
    assert!(job.raw_ipv4);
    assert_eq!(job.frames.len(), 3);
    assert_eq!(job.stem, "capture-2025-10-09T08-53-21.000Z");
    // The export shares the ring's frames rather than copying them.
    let first = app.capture.ring.shared(4).unwrap();
    assert!(Arc::ptr_eq(first, &job.frames[0]));
    let reply = |written: &[&str], error: Option<&str>| {
        let exported = crate::tui::capture::pcap::Exported {
            written: written.iter().map(|w| w.to_string()).collect(),
            error: error.map(str::to_string),
        };
        WorkerEvent::Capture(CaptureEvent::Exported(exported))
    };
    worker(&mut app, reply(&["a-ipv4.pcap"], None));
    assert_eq!(app.flash.as_ref().unwrap().text, "Exported a-ipv4.pcap.");
    worker(&mut app, reply(&["a-bip.pcap"], Some("disk full")));
    assert_eq!(
        app.flash.as_ref().unwrap().text,
        "Exported a-bip.pcap, then failed: disk full"
    );
}

/// An ended source takes only itself off the list: a session ending
/// leaves libpcap running, and libpcap's error names it.
#[test]
fn an_ended_source_leaves_the_others_running() {
    let mut app = capturing();
    for (what, session) in [("libpcap on en0, UDP port 47808", false), ("BIP tap", true)] {
        let started = CaptureEvent::Started {
            what: what.into(),
            session,
        };
        worker(&mut app, WorkerEvent::Capture(started));
    }
    let ended = CaptureEvent::Ended {
        what: "libpcap on en0, UDP port 47808".into(),
        why: "en0: permission denied".into(),
    };
    worker(&mut app, WorkerEvent::Capture(ended));
    assert_eq!(app.capture.sources, [("BIP tap".to_string(), true)]);
    let text = &app.flash.as_ref().unwrap().text;
    assert!(
        text.contains("libpcap on en0") && text.contains("permission denied"),
        "{text}"
    );
}

/// Frames a second, and seconds of them.
const RATE: u64 = 5_000;
const SECONDS: u64 = 3;

#[tokio::test(start_paused = true)]
async fn five_thousand_frames_a_second_stay_bounded_and_every_loss_is_counted() {
    // The source's own queue, as a tap's; the producer counts what it
    // drops, as the tap does.
    let (tx, rx) = mpsc::channel(1_024);
    let lost = Arc::new(AtomicU64::new(0));
    let source = Source::Test {
        rx,
        lost: lost.clone(),
    };
    // Batches go out full or every 250 ms, so a few a second; a small
    // event channel fills with them while the UI stalls.
    let (sink, mut events) = EventSink::channel(8);
    let status_bar = sink.dropped_counter();
    let (_commands, mut command_rx) = mpsc::channel::<CaptureCommand>(1);
    let pumping =
        tokio::spawn(
            async move { pump(source, &sink, Some(&mut command_rx), "test".into()).await },
        );
    let template = bip_frame(0, true, &testing::rp_request(1), false);
    let producer = tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_millis(1));
        for _ in 0..RATE * SECONDS / 5 {
            every.tick().await;
            for _ in 0..5 {
                if tx.try_send(template.clone()).is_err() {
                    lost.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    });

    let mut app = capturing();
    app.capture = CaptureState::new(2_000);
    // The UI stalls for two seconds, then drains at 20 frames a second.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut ended = false;
    while !ended {
        for _ in 0..256 {
            match events.try_recv() {
                Ok(WorkerEvent::Capture(CaptureEvent::Ended { .. })) => ended = true,
                Ok(event) => {
                    worker(&mut app, event);
                }
                Err(_) => break,
            }
        }
        assert!(app.capture.ring.len() <= 2_000, "bounded by the ring");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    producer.await.unwrap();
    pumping.await.unwrap();
    let arrived = app.capture.ring.next() - 1;
    assert!(app.capture.lost > 0, "the stall lost frames");
    assert_eq!(arrived + app.capture.lost, RATE * SECONDS, "exact");
    assert_eq!(
        status_bar.load(Ordering::Relaxed),
        0,
        "capture's refusals stay out of the status bar's drop count"
    );
    assert_eq!(app.capture.ring.len(), 2_000);
    let text = screen(&mut app, 80, 24);
    assert!(
        text.contains(&format!("dropped {}", app.capture.lost)),
        "{text}"
    );
}

#[test]
fn drawing_a_full_ring_at_120x40_fits_the_frame_budget() {
    let mut app = capturing();
    let frames = (0..crate::tui::capture::DEFAULT_RING as u64)
        .map(|i| bip_frame(i, i % 2 == 0, &testing::rp_request((i % 256) as u8), false));
    deliver(&mut app, frames.collect(), 0);
    filter(&mut app, "out");
    app.settle();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    const FRAMES: u32 = 50;
    let start = std::time::Instant::now();
    for _ in 0..FRAMES {
        app.settle();
        terminal.draw(|f| crate::tui::view::draw(f, &app)).unwrap();
    }
    let per_frame = start.elapsed() / FRAMES;
    println!("capture at 120x40 with a full ring: {per_frame:?} per frame");
    let budget = if cfg!(debug_assertions) {
        Duration::from_millis(250)
    } else {
        Duration::from_millis(5)
    };
    assert!(
        per_frame < budget,
        "{per_frame:?} per frame, budget {budget:?}"
    );
}

#[test]
fn the_keys_map_on_the_capture_screen() {
    let mut app = capturing();
    let action = |app: &App, code| map_key(app, KeyEvent::new(code, KeyModifiers::NONE));
    assert_eq!(
        action(&app, KeyCode::Char(' ')),
        Some(Action::Capture(CaptureKey::Pause))
    );
    assert_eq!(action(&app, KeyCode::Char('/')), Some(Action::StartFilter));
    assert_eq!(action(&app, KeyCode::Left), None, "the list has no Left");
    press(&mut app, KeyCode::Tab);
    assert_eq!(
        action(&app, KeyCode::Left),
        Some(Action::Capture(CaptureKey::Collapse))
    );
    for screen in ['1', '3', '4', '5', '6'] {
        let mut app = connected_app();
        press(&mut app, KeyCode::Char(screen));
        assert_eq!(
            action(&app, KeyCode::Char('7')),
            Some(Action::ShowScreen(Screen::Capture)),
            "from {screen}"
        );
    }
}
