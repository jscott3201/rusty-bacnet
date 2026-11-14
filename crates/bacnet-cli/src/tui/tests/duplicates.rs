//! The device table and the duplicate banner follow the client's table,
//! one device per address (#2560): an address that announces another
//! instance takes the old one's row with it, a contest that ends shows the
//! claimant left, and a missed notice is mended by the snapshot.

use tokio::sync::broadcast::error::RecvError;

use super::*;
use crate::tui::worker::{DeviceFeed, EventSink};

/// The client's events when the device at an address announces another
/// instance: the old one lost, then the new one discovered.
fn reannounce(app: &mut App, old: DeviceRow, new: DeviceRow) {
    worker(app, WorkerEvent::Lost(old));
    worker(app, WorkerEvent::Discovered(new));
}

#[test]
fn an_address_that_changes_instance_leaves_no_stale_row() {
    let mut app = connected_app();
    let seven = row(7, "10.0.0.20:47808", 1, 0);
    worker(&mut app, WorkerEvent::Discovered(seven.clone()));
    reannounce(&mut app, seven, row(8, "10.0.0.20:47808", 1, 1));
    assert!(app.devices.get(8).is_some());
    assert!(app.devices.get(7).is_none(), "instance 7 still shown");
}

/// The claimant the table shows announces another instance: its row goes,
/// and the contest's end shows the claimant left.
#[test]
fn a_contest_ended_by_the_shown_claimant_reannouncing() {
    let mut app = connected_app();
    let (a, b) = ("10.0.0.21:47808", "10.0.0.22:47808");
    worker(&mut app, WorkerEvent::Discovered(row(7, b, 1, 0)));
    worker(&mut app, WorkerEvent::Discovered(row(7, a, 1, 0)));
    worker(
        &mut app,
        WorkerEvent::Collision {
            instance: 7,
            claimants: vec![row(7, a, 1, 0), row(7, b, 1, 0)],
        },
    );
    assert_eq!(app.devices.get(7).map(|r| r.address.as_str()), Some(b));
    reannounce(&mut app, row(7, b, 1, 0), row(8, b, 1, 1));
    worker(
        &mut app,
        WorkerEvent::Collision {
            instance: 7,
            claimants: vec![row(7, a, 1, 0)],
        },
    );
    assert_eq!(app.devices.get(7).map(|r| r.address.as_str()), Some(a));
    assert_eq!(app.devices.get(8).map(|r| r.address.as_str()), Some(b));
    assert!(app.duplicates.banner().is_empty());
}

/// A snapshot lists every claimant, so it rebuilds the banner: a contest
/// whose end was missed goes, and one whose start was missed comes.
#[test]
fn a_snapshot_rebuilds_the_duplicates() {
    let mut app = connected_app();
    let (a, b) = ("10.0.0.21:47808", "10.0.0.22:47808");
    worker(
        &mut app,
        WorkerEvent::Collision {
            instance: 7,
            claimants: vec![row(7, a, 1, 0), row(7, b, 1, 0)],
        },
    );
    assert_eq!(app.duplicates.lines().len(), 1);
    worker(
        &mut app,
        WorkerEvent::Snapshot(vec![row(7, a, 1, 0), row(9, a, 1, 0), row(9, b, 1, 0)]),
    );
    assert_eq!(
        app.duplicates.lines(),
        [format!("instance 9 claimed by {a} and {b}")]
    );
    worker(&mut app, WorkerEvent::Snapshot(vec![row(9, b, 1, 0)]));
    assert!(app.duplicates.is_empty());
    assert!(app.duplicates.banner().is_empty());
}

/// Missed collision notices may have held a contest's end, so they ask for
/// the snapshot that rebuilds the banner, as missed device events do.
#[test]
fn missed_collision_notices_ask_for_a_snapshot() {
    let (sink, _events) = EventSink::channel(8);
    let mut feed = DeviceFeed::new(AddressStyle::Bip);
    assert!(!feed.needs_resync);
    assert!(
        feed.on_collision(&sink, Err(RecvError::Lagged(3)))
            .is_none()
    );
    assert!(feed.needs_resync);
    assert!(feed.collisions_open);
}
