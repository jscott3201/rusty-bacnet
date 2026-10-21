//! The MS/TP monitor's layouts (#1893) at 80x24, 120x40 and 200x50: a
//! summary that wraps between its items instead of being cut, the station
//! table fitted to its content and ending with `+N more` where it runs out
//! of rows, and the map beside the table on a wide terminal.

use bacnet_transport::mstp::codec::FrameType;
use bacnet_transport::mstp::monitor::script::{LineScript, ScriptedRing};
use insta::assert_snapshot;

use super::mstp_monitor::Run;
use super::*;
use crate::tui::monitor::analysis;

const SIZES: [(u16, u16); 3] = [(80, 24), (120, 40), (200, 50)];
const BAUD: u32 = 38_400;
const REQUEST: [u8; 3] = [0x01, 0x04, 0x10];
const REPLY: [u8; 3] = [0x01, 0x00, 0x20];

/// Twelve masters, 1 to 12, each asking its own slave (101 to 112) once a
/// token hold, for 10 s: 24 stations, more than an 80x24 table holds.
fn busy_line() -> Vec<(Duration, Vec<u8>)> {
    let mut ring = ScriptedRing::new(BAUD);
    for master in 1..=12 {
        ring = ring.master(master, 12);
    }
    let mut traffic = |line: &mut LineScript, holder: u8| {
        let slave = 100 + holder;
        line.frame(
            FrameType::BACNET_DATA_EXPECTING_REPLY,
            slave,
            holder,
            &REQUEST,
        )
        .frame(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            holder,
            slave,
            &REPLY,
        );
    };
    while ring
        .line
        .chunks()
        .last()
        .is_none_or(|(at, _)| *at < Duration::from_secs(10))
    {
        ring.rotate(12, &mut traffic);
    }
    ring.line.chunks().to_vec()
}

/// The station view at each size: stacked and cut with `+N more` at 80x24,
/// stacked and whole at 120x40, side by side at 200x50.
#[tokio::test(start_paused = true)]
async fn the_station_view_fits_each_size() {
    let mut run = Run::start(busy_line(), None, false).await;
    run.listen_for(Duration::from_secs(10)).await;
    let stations = analysis::stations(
        run.app
            .monitor
            .as_deref()
            .unwrap()
            .shown
            .as_deref()
            .unwrap(),
    );
    assert_eq!(stations.len(), 24);
    for (width, height) in SIZES {
        let text = screen(&mut run.app, width, height);
        assert_snapshot!(format!("monitor_busy_{width}x{height}"), text);
        let cut = text.contains(" more");
        assert_eq!(cut, width == 80, "{width}x{height}:\n{text}");
    }
    let small = screen(&mut run.app, 80, 24);
    assert!(small.contains("+18 more"), "{small}");
    let wide = screen(&mut run.app, 200, 50);
    // The map's first row and the table's header share a line.
    assert!(
        wide.lines()
            .any(|line| line.contains("Masters   0-63") && line.contains("MAC Role")),
        "{wide}"
    );
    assert_eq!(run.quit().await, b"");
}

/// A long port name and a pause don't cut the summary at 80 columns: it
/// goes on to a second line between its items.
#[tokio::test(start_paused = true)]
async fn the_summary_wraps_at_80_columns() {
    let mut run = Run::start(busy_line(), None, false).await;
    run.listen_for(Duration::from_secs(10)).await;
    run.press(KeyCode::Char(' ')).await;
    let monitor = run.app.monitor.as_deref_mut().unwrap();
    assert!(monitor.paused);
    monitor.port = "/dev/cu.usbserial-B0037OSC".into();
    let text = screen(&mut run.app, 80, 24);
    for want in [
        "/dev/cu.usbserial-B0037OSC  38400 baud  up 10 s  24 stations",
        "timestamps ±16 ms  PAUSED",
    ] {
        assert!(text.contains(want), "{want}:\n{text}");
    }
    assert_snapshot!("monitor_summary_wrapped_80x24", text);
    for (width, height) in SIZES {
        let text = screen(&mut run.app, width, height);
        assert!(text.contains("PAUSED"), "{width}x{height}:\n{text}");
    }
    assert_eq!(run.quit().await, b"");
}
