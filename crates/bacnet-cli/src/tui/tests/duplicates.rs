//! The device table and the duplicate banner follow the client's table,
//! one row per address (#2560): two devices sharing an instance are two
//! rows, the banner is read off them, an address that announces another
//! instance takes the old one's row with it, and a snapshot rebuilds both.

use super::*;

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
    assert_eq!(app.devices.len(), 1);
}

/// Two devices sharing an instance are two rows and a banner line; one of
/// them announcing another instance ends the contest, and the instance
/// names the one left.
#[test]
fn a_shared_instance_is_two_rows_until_one_reannounces() {
    let mut app = connected_app();
    let (a, b) = ("10.0.0.21:47808", "10.0.0.22:47808");
    worker(&mut app, WorkerEvent::Discovered(row(7, b, 1, 0)));
    worker(&mut app, WorkerEvent::Discovered(row(7, a, 1, 0)));
    assert_eq!(app.devices.len(), 2);
    assert!(app.devices.get(7).is_none(), "the instance names neither");
    assert_eq!(
        app.duplicates.lines(),
        [format!("instance 7 claimed by {a} and {b}")]
    );
    reannounce(&mut app, row(7, b, 1, 0), row(8, b, 1, 1));
    assert_eq!(app.devices.get(7).map(|r| r.address.as_str()), Some(a));
    assert_eq!(app.devices.get(8).map(|r| r.address.as_str()), Some(b));
    assert!(app.duplicates.banner().is_empty());
}

/// Each row of a shared instance is its own device: opening the second
/// browses at its address, not the first's, and `:dcc` and `writem` there
/// go to it too.
#[test]
fn each_row_of_a_shared_instance_opens_its_own_device() {
    let mut app = connected_app();
    let (a, b) = ("10.0.0.21:47808", "10.0.0.22:47808");
    worker(&mut app, WorkerEvent::Discovered(row(7, a, 1, 0)));
    worker(&mut app, WorkerEvent::Discovered(row(7, b, 1, 0)));
    app.settle();
    press(&mut app, KeyCode::Down);
    let commands = press(&mut app, KeyCode::Enter);
    let [Command::Browse { device, at, .. }] = commands.as_slice() else {
        panic!("{commands:?}");
    };
    assert_eq!((*device, at.to_string().as_str()), (7, b));
    assert_eq!(app.browser.as_ref().unwrap().address, b);
    // Back to the table and into the first: a fresh browser at its address.
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Up);
    let commands = press(&mut app, KeyCode::Enter);
    let at_first = commands.iter().find_map(|command| match command {
        Command::Browse { at, .. } => Some(at.to_string()),
        _ => None,
    });
    assert_eq!(at_first.as_deref(), Some(a), "{commands:?}");
}

/// A snapshot holds every claimant, so it rebuilds the banner: a contest
/// whose end was missed goes, and one whose start was missed comes.
#[test]
fn a_snapshot_rebuilds_the_duplicates() {
    let mut app = connected_app();
    let (a, b) = ("10.0.0.21:47808", "10.0.0.22:47808");
    worker(&mut app, WorkerEvent::Discovered(row(7, a, 1, 0)));
    worker(&mut app, WorkerEvent::Discovered(row(7, b, 1, 0)));
    assert_eq!(app.duplicates.lines().len(), 1);
    worker(
        &mut app,
        WorkerEvent::Snapshot(vec![
            row(7, a, 1, 0),
            row(9, "10.0.0.23:47808", 1, 0),
            row(9, b, 1, 0),
        ]),
    );
    assert_eq!(
        app.duplicates.lines(),
        [format!("instance 9 claimed by {b} and 10.0.0.23:47808")]
    );
    worker(&mut app, WorkerEvent::Snapshot(vec![row(9, b, 1, 0)]));
    assert!(app.duplicates.is_empty());
    assert!(app.duplicates.banner().is_empty());
}

/// A watch goes to the one address claiming its instance (#2560 slice 6):
/// the command carries the row's address for the watch hub, and an
/// instance two rows claim is refused, naming both, with no row added.
#[test]
fn a_watch_goes_to_the_one_address_claiming_its_instance() {
    use crate::tui::watched::WatchCommand;
    let mut app = connected_app();
    let (a, b) = ("10.0.0.21:47808", "10.0.0.22:47808");
    worker(&mut app, WorkerEvent::Discovered(row(7, a, 1, 0)));
    let watch = |app: &mut App, line: &str| {
        press(app, KeyCode::Char(':'));
        type_text(app, line);
        press(app, KeyCode::Enter)
    };
    let commands = watch(&mut app, "watch 7 ai:1");
    let [Command::Watch(WatchCommand::Add { at, .. })] = commands.as_slice() else {
        panic!("{commands:?}");
    };
    assert_eq!(at.as_ref(), Some(&address_of(a)));
    worker(&mut app, WorkerEvent::Discovered(row(7, b, 1, 0)));
    let commands = watch(&mut app, "watch 7 ai:2");
    assert!(commands.is_empty(), "{commands:?}");
    let flash = &app.flash.as_ref().unwrap().text;
    assert!(flash.contains(a) && flash.contains(b), "{flash}");
    assert_eq!(app.watch.rows().len(), 1, "no row for the contested one");
}
