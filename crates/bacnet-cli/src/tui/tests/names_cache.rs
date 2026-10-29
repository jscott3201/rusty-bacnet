//! The session name cache's bounds and keys (#1931): an LRU of at most
//! `MAX_NAMES`, names filed by device address as well as instance, and
//! the dim `?` for a name the device didn't answer for.

use bacnet_types::primitives::ObjectIdentifier;
use insta::assert_snapshot;
use ratatui::style::Modifier;

use super::names::{asks, device, frame_commands, named, names, table};
use super::*;
use crate::tui::app::names::{MAX_NAMES, Name};
use crate::tui::network::discovery::{DiscoveryEvent, FoundRow};
use crate::tui::point::ReadName;

/// An I-Have naming `object` of `device`, from `address`.
fn i_have(app: &mut App, device: u32, object: ObjectIdentifier, name: &str, address: &str) {
    let found = FoundRow {
        device,
        object,
        name: name.into(),
        address: address.into(),
        via: None,
        network: None,
    };
    worker(app, WorkerEvent::Discovery(DiscoveryEvent::IHave(found)));
}

/// A flood of unsolicited I-Haves never holds more than `MAX_NAMES`
/// names. The rows on screen are used every tick, so they stay, and
/// nothing is asked for again.
#[test]
fn an_i_have_flood_stays_at_the_cap() {
    let mut app = table(3);
    assert_eq!(asks(&frame_commands(&mut app, 120, 40)).len(), 3);
    for instance in 1..=3 {
        let name = named(&format!("AHU-{instance}"));
        names(&mut app, instance, vec![(device(instance), name)]);
    }
    let flood = u32::try_from(MAX_NAMES).unwrap() * 3;
    for n in 0..flood {
        let instance = 10_000 + n % 500;
        let object = super::names::ai(n / 500);
        let address = format!("10.1.{}.{}:47808", instance / 256 % 256, instance % 256);
        i_have(&mut app, instance, object, "Zone Temp", &address);
        if n % 1_000 == 0 {
            assert!(frame_commands(&mut app, 120, 40).is_empty(), "at {n}");
        }
    }
    assert_eq!(app.names.len(), MAX_NAMES);
    for instance in 1..=3 {
        let name = app.name_of(instance, device(instance));
        assert_eq!(name, Some(&Name::Known(format!("AHU-{instance}"))));
    }
    assert!(frame_commands(&mut app, 120, 40).is_empty());
    // Asks with nothing left to say are dropped.
    assert_eq!(app.names.asked_devices(), 0);
}

/// Two devices claim instance 7. Each address keeps its own name: the
/// table shows the name of the one the client kept, and the other's
/// once it takes over the row.
#[test]
fn a_duplicate_instance_pair_keeps_each_devices_name() {
    let mut app = connected_app();
    let first = row(7, "10.0.0.7:47808", 260, 0);
    let second = row(7, "10.0.9.7:47808", 260, 1);
    worker(&mut app, WorkerEvent::Discovered(first.clone()));
    let commands = frame_commands(&mut app, 120, 40);
    assert_eq!(asks(&commands), [(7, vec![device(7)])]);
    names(&mut app, 7, vec![(device(7), named("Plant AHU"))]);
    // The other device's I-Have names its own Device object.
    i_have(&mut app, 7, device(7), "Lab AHU", &second.address);
    worker(
        &mut app,
        WorkerEvent::Collision {
            retained: first.clone(),
            incoming: second.clone(),
        },
    );
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("Plant AHU") && !text.contains("Lab AHU"),
        "{text}"
    );
    // The first goes quiet and the client keeps the second.
    worker(&mut app, WorkerEvent::Lost(first.clone()));
    worker(&mut app, WorkerEvent::Discovered(second));
    assert!(
        frame_commands(&mut app, 120, 40).is_empty(),
        "known already"
    );
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("Lab AHU") && !text.contains("Plant AHU"),
        "{text}"
    );
    assert_eq!(
        app.names.get(7, &first.address, device(7)),
        Some(&Name::Known("Plant AHU".into()))
    );
}

/// A name the device didn't answer for shows a dim `?`, apart from a row
/// nobody asked about (blank) and one being read (`…`).
#[test]
fn an_unread_name_shows_a_dim_marker() {
    let mut app = table(3);
    frame_commands(&mut app, 120, 40);
    names(&mut app, 1, vec![(device(1), named("AHU-1"))]);
    names(&mut app, 2, vec![(device(2), ReadName::Unreachable)]);
    assert_snapshot!("names_unread_120x40", screen(&mut app, 120, 40));
    assert_snapshot!("names_unread_80x24", screen(&mut app, 80, 24));
    let terminal = render(&mut app, 120, 40);
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    let line = |y| -> String { (0..area.width).map(|x| buffer[(x, y)].symbol()).collect() };
    let y = (0..area.height)
        .find(|&y| line(y).contains("10.0.0.2:47808"))
        .expect("device 2's row");
    let x = line(y).find(" ? ").expect("the marker") + 1;
    let x = u16::try_from(line(y)[..x].chars().count()).unwrap();
    assert!(buffer[(x, y)].modifier.contains(Modifier::DIM));
    assert!(!buffer[(x - 3, y)].modifier.contains(Modifier::DIM));
}
