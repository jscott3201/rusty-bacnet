//! The SC screen fitted to its content (#1890): a fresh session's first
//! connect on the timeline, the timeline taking the spare height, the
//! identity panel readable at 80 columns, the doctor keeping the whole
//! error on screen, and the credentials beside the chain from 160 columns.
//! Frames at 80x24, 120x40 and 200x50.

use insta::assert_snapshot;

use super::sc_lint::credentials;
use super::sc_screen::{connected, hub_chain, identity, refused, sc, sc_app, PRIMARY_VMAC};
use super::*;
use crate::tui::app::sc::View;
use crate::tui::app::watch::Screen;
use crate::tui::sc::credentials::Level;
use crate::tui::sc::{LinkEvent, ScReport};
use bacnet_transport::sc::ScActiveHub;

/// A session just connected to the primary hub, its first connect on the
/// timeline, on the SC screen.
fn fresh() -> App {
    let mut app = sc_app();
    worker(
        &mut app,
        WorkerEvent::Connected {
            local: "22:10:20:30:40:50".into(),
        },
    );
    sc(&mut app, ScReport::Dialing(identity()));
    let finding = (
        Level::Warning,
        "Operational certificate [0]: It expires within thirty days. Plan renewal.".to_owned(),
    );
    let checked = Ok(credentials(vec![finding]));
    sc(&mut app, ScReport::Credentials(Box::new(checked)));
    tick(&mut app, at(1));
    sc(
        &mut app,
        ScReport::Status(connected(ScActiveHub::Primary, PRIMARY_VMAC)),
    );
    sc(&mut app, ScReport::Chain(hub_chain()));
    sc(
        &mut app,
        ScReport::Event(LinkEvent::Connected {
            hub: ScActiveHub::Primary,
            hub_vmac: PRIMARY_VMAC,
        }),
    );
    tick(&mut app, at(4));
    app.screen = Screen::Sc;
    app
}

/// The row of the frame's first line containing `text`.
fn row_of(text: &str, frame: &str) -> usize {
    frame
        .lines()
        .position(|line| line.contains(text))
        .unwrap_or_else(|| panic!("no {text:?} in\n{frame}"))
}

/// Whether a pane in the frame ends with a `+N more` line.
fn more_marker(frame: &str) -> bool {
    frame.lines().any(|line| {
        let text = line.trim_matches(|c| c == '"' || c == '│' || c == ' ');
        text.strip_prefix('+')
            .and_then(|rest| rest.strip_suffix(" more"))
            .is_some_and(|n| n.parse::<usize>().is_ok())
    })
}

const CONNECTED: &str = "connected to the primary hub (hub VMAC 10:10:10:10:10:10)";

#[test]
fn the_first_connect_is_on_the_timeline_in_every_view() {
    let mut app = fresh();
    for view in [View::Status, View::Doctor, View::Credentials] {
        app.sc.view = view;
        for (width, height) in [(80, 24), (120, 40), (200, 50)] {
            let text = screen(&mut app, width, height);
            assert!(
                text.contains(CONNECTED),
                "{view:?} {width}x{height}\n{text}"
            );
        }
    }
}

#[test]
fn the_timeline_gets_the_spare_height() {
    let mut app = fresh();
    let text = screen(&mut app, 120, 40);
    let events = row_of("┌ Events", &text);
    let middle = row_of("[Hub certificates]", &text);
    // Status bar, panels and the chain above; the rest, bar the footer, is
    // the timeline's.
    assert!(events - middle <= 12, "{text}");
    assert!(39 - events >= 20, "{text}");
}

#[test]
fn the_identity_panel_reads_whole_at_80_columns() {
    let mut app = fresh();
    let text = screen(&mut app, 80, 24);
    // Each panel's text, read down its column without the spaces: the
    // UUID wraps between its groups and the heartbeat under itself, and
    // nothing of either is cut.
    let column = |side: usize| -> String {
        text.lines()
            .filter_map(|line| line.split("││").nth(side))
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace() && !"│\"".contains(*c))
            .collect()
    };
    assert!(
        column(1).contains("DeviceUUID6f1c2a9e-4407-4b3d-9a51-0e7c8821d40b"),
        "{text}"
    );
    assert!(
        column(0).contains("Heartbeatheard6sago,roundtrip12ms"),
        "{text}"
    );
    // The row the hub's URL no longer shares leaves the chain room for the
    // issuer's details.
    assert!(text.contains("expires 2041-01-01T00:00:00Z"), "{text}");
    assert!(!more_marker(&text), "{text}");
    assert_snapshot!("sc_connected_80x24", text);
}

#[test]
fn a_cut_timeline_counts_the_events_above() {
    let mut app = fresh();
    for _ in 0..5 {
        sc(&mut app, ScReport::Event(LinkEvent::FailoverEngaged));
    }
    let text = screen(&mut app, 80, 24);
    let shown = text
        .lines()
        .filter(|line| line.contains("primary hub unreachable"))
        .count();
    // Six events, the newest shown, the title counting the rest.
    let title = text.lines().nth(row_of("┌ Events", &text)).unwrap();
    assert!(title.contains(&format!("+{} earlier", 6 - shown)), "{text}");
    assert!(
        screen(&mut app, 120, 40).contains("┌ Events ─"),
        "nothing cut at 120x40"
    );
}

#[test]
fn sc_connected_frames() {
    let mut app = fresh();
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("signed with ecdsa-with-SHA256"), "{text}");
    assert_snapshot!("sc_connected_120x40", text);
    let text = screen(&mut app, 200, 50);
    // Side by side: the chain and the credentials on one row.
    let row = row_of("[Hub certificates]", &text);
    assert!(
        text.lines().nth(row).unwrap().contains("My credentials"),
        "{text}"
    );
    assert_snapshot!("sc_connected_200x50", text);
    app.sc.view = View::Doctor;
    assert_snapshot!("sc_connected_doctor_80x24", screen(&mut app, 80, 24));
    app.sc.view = View::Credentials;
    let text = screen(&mut app, 80, 24);
    // The verdicts come first, so the short pane keeps the lint's finding
    // and the checks, and says what it cut.
    assert!(text.contains("It expires within thirty days"), "{text}");
    assert!(text.contains("[warn] Issuer"), "{text}");
    assert!(more_marker(&text), "{text}");
    assert_snapshot!("sc_connected_credentials_80x24", text);
    let text = screen(&mut app, 200, 50);
    assert!(text.contains("[My credentials]"), "{text}");
    assert!(text.contains("EKU client-auth"), "{text}");
    assert!(!more_marker(&text), "{text}");
    assert_snapshot!("sc_connected_credentials_200x50", text);
}

#[test]
fn the_doctor_keeps_the_whole_error_on_screen_at_80x24() {
    let mut app = refused();
    // Read credentials, so the wide frame is the same in every build.
    let checked = Ok(credentials(Vec::new()));
    sc(&mut app, ScReport::Credentials(Box::new(checked)));
    let text = screen(&mut app, 80, 24);
    let squeezed: String = text
        .lines()
        .map(|line| line.trim_matches(|c| c == '"' || c == '│' || c == ' '))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        squeezed.contains("invalid peer certificate: certificate expired"),
        "{text}"
    );
    assert!(squeezed.contains("signed with ecdsa-with-SHA256"), "{text}");
    // The phases it passed fold into one line to make the room.
    assert!(text.contains("[ ok ] URL, DNS, TCP"), "{text}");
    assert_snapshot!("sc_doctor_80x24", text);
    assert_snapshot!("sc_doctor_200x50", screen(&mut app, 200, 50));
}
