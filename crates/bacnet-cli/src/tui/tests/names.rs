//! Object and device names (#1896): only the rows on screen are asked
//! for, once, in one command a tick, and names that came free cost
//! nothing. The asks come from synthetic frames and worker events;
//! `names_site` runs the reads against the lab device.

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::tui::app::names::Name;
use crate::tui::app::objects::ListRow;
use crate::tui::app::watch::Screen;
use crate::tui::network::discovery::{DiscoveryEvent, FoundRow};
use crate::tui::point::{BrowseRead, BrowseResult, DeviceCard, NameAsk, ReadName, ReqId};
use crate::tui::watched::WatchEvent;

pub(super) fn ai(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

pub(super) fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

pub(super) fn named(name: &str) -> ReadName {
    ReadName::Named(name.into())
}

/// The name asks among `commands`, as one list.
fn name_asks(commands: &[Command]) -> Vec<NameAsk> {
    commands
        .iter()
        .flat_map(|command| match command {
            Command::Names { asks, .. } => asks.clone(),
            _ => Vec::new(),
        })
        .collect()
}

/// The name asks among `commands`: each device and what it is asked for.
pub(super) fn asks(commands: &[Command]) -> Vec<(u32, Vec<ObjectIdentifier>)> {
    name_asks(commands)
        .into_iter()
        .map(|ask| (ask.device, ask.objects))
        .collect()
}

/// Draw a frame and tick: the commands for what it showed.
pub(super) fn frame_commands(app: &mut App, width: u16, height: u16) -> Vec<Command> {
    render(app, width, height);
    let now = app.now;
    update(
        app,
        Action::Tick {
            now,
            dropped: app.dropped,
            log_generation: app.log.generation(),
        },
    )
}

/// Draw a frame and tick: the asks for what it showed.
fn frame_asks(app: &mut App, width: u16, height: u16) -> Vec<(u32, Vec<ObjectIdentifier>)> {
    asks(&frame_commands(app, width, height))
}

/// The request id of the one command among `commands`.
fn req_of(commands: &[Command]) -> ReqId {
    match commands {
        [Command::Names { req, .. } | Command::Browse { req, .. }] => *req,
        other => panic!("one command: {other:?}"),
    }
}

/// The objects the browser's last frame showed.
fn on_screen(app: &App) -> Vec<ObjectIdentifier> {
    let objects = &app.browser.as_ref().unwrap().objects;
    objects
        .rows()
        .iter()
        .skip(objects.offset.get())
        .take(objects.page.get())
        .filter_map(|row| match row {
            ListRow::Object(object) => Some(*object),
            ListRow::Group { .. } => None,
        })
        .collect()
}

pub(super) fn names(app: &mut App, device: u32, names: Vec<(ObjectIdentifier, ReadName)>) {
    let address = app.device_address(device).to_string();
    let result = BrowseResult::Names {
        device,
        address,
        names,
    };
    worker(app, WorkerEvent::Browse { req: 0, result });
}

/// Deliver device 7's card for request `req`.
fn card(app: &mut App, req: ReqId) {
    let card = DeviceCard {
        name: Some("Plant".into()),
        ..DeviceCard::default()
    };
    let result = BrowseResult::Card(card);
    worker(app, WorkerEvent::Browse { req, result });
}

/// A connected app browsing device 7, whose Object_List holds `count`
/// analog inputs and whose card named it `Plant`.
pub(super) fn browsing(count: u32) -> App {
    let mut app = connected_app();
    worker(
        &mut app,
        WorkerEvent::Discovered(row(7, "10.0.0.7:47808", 260, 0)),
    );
    let req = req_of(&press(&mut app, KeyCode::Enter));
    card(&mut app, req);
    let result = BrowseResult::Objects {
        objects: (0..count).map(ai).collect(),
        indexed: false,
        stopped: None,
    };
    worker(&mut app, WorkerEvent::Browse { req, result });
    app
}

/// A connected app whose device table holds devices 1 to `count`.
pub(super) fn table(count: u32) -> App {
    let mut app = connected_app();
    for instance in 1..=count {
        let address = format!("10.0.0.{instance}:47808");
        worker(
            &mut app,
            WorkerEvent::Discovered(row(instance, &address, 260, 0)),
        );
    }
    app
}

/// Of a 1,000-object list only the rows on screen are asked for, and a
/// frame showing the same rows asks nothing more.
#[test]
fn only_the_rows_on_screen_are_asked_for() {
    let mut app = browsing(1_000);
    let asked = frame_asks(&mut app, 80, 24);
    let shown = on_screen(&app);
    assert!(!shown.is_empty() && shown.len() < 30, "{shown:?}");
    assert_eq!(asked, [(7, shown.clone())]);
    assert!(frame_asks(&mut app, 80, 24).is_empty(), "asked already");
    // Names arriving for some of them change nothing either.
    let half: Vec<_> = shown[..shown.len() / 2]
        .iter()
        .map(|object| {
            (
                *object,
                named(&format!("Point {}", object.instance_number())),
            )
        })
        .collect();
    names(&mut app, 7, half);
    assert!(frame_asks(&mut app, 80, 24).is_empty());
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("analog-input 0    Point 0"), "{text}");
    assert!(text.contains("…"), "a name still being read: {text}");
}

/// Scrolling replaces the ask with the rows now on screen, so rows that
/// scrolled off before their turn are never read; leaving the browser
/// stops the rest.
#[test]
fn scrolling_replaces_the_ask_and_closing_stops_it() {
    let mut app = browsing(1_000);
    let first = frame_asks(&mut app, 80, 24);
    let before = on_screen(&app);
    press(&mut app, KeyCode::PageDown);
    press(&mut app, KeyCode::PageDown);
    let after = frame_asks(&mut app, 80, 24);
    let shown = on_screen(&app);
    assert_ne!(shown, before);
    assert_eq!(after, [(7, shown.clone())], "the new rows only");
    assert!(first[0].1.iter().all(|object| !shown.contains(object)));
    // The device's own name came with its card: the device table asks
    // nothing for it, and leaving the browser withdraws the ask.
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Main);
    assert_eq!(frame_asks(&mut app, 80, 24), [(7, Vec::new())]);
    assert!(frame_asks(&mut app, 80, 24).is_empty());
}

/// Every device's ask goes in one command a tick, however many devices
/// the screen shows, so the worker's 16-command queues never fill.
#[test]
fn one_command_carries_every_device() {
    let mut app = table(40);
    let commands = frame_commands(&mut app, 200, 50);
    assert_eq!(commands.len(), 1, "{commands:?}");
    let asked = asks(&commands);
    assert!(asked.len() > 16, "{}", asked.len());
    assert!(asked.iter().all(|(on, objects)| *objects == [device(*on)]));
}

/// The same rows in another order are not asked for again: sorting by
/// last seen reorders the table on every I-Am.
#[test]
fn reordered_rows_are_not_asked_again() {
    let mut app = table(5);
    for _ in 0..4 {
        press(&mut app, KeyCode::Char('s'));
    }
    assert_eq!(frame_asks(&mut app, 120, 40).len(), 5);
    for (instance, seen) in [(1, 9), (3, 7), (5, 8)] {
        let address = format!("10.0.0.{instance}:47808");
        let row = row(instance, &address, 260, seen);
        worker(&mut app, WorkerEvent::Updated(row));
    }
    assert_eq!(visible(&mut app)[..3], [1, 5, 3]);
    assert!(frame_asks(&mut app, 120, 40).is_empty());
}

/// Names that came free (the card, a watch row's first read, an I-Have)
/// are never asked for.
#[test]
fn names_that_came_free_are_not_asked_for() {
    let mut app = browsing(4);
    worker(
        &mut app,
        WorkerEvent::Discovered(row(8, "10.0.0.8:47808", 260, 0)),
    );
    worker(
        &mut app,
        WorkerEvent::Discovered(row(9, "10.0.0.9:47808", 260, 0)),
    );
    // A watch row's first read names AI:1; an I-Have names AI:2 and,
    // unasked, device 8.
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "watch 7 ai:1");
    press(&mut app, KeyCode::Enter);
    let row = app.watch.rows()[0].id;
    let details = WatchEvent::Details {
        row,
        name: Some("Supply Temp".into()),
        units: None,
    };
    worker(&mut app, WorkerEvent::Watch(details));
    for (on, object, name) in [(7, ai(2), "Return Temp"), (8, device(8), "AHU-8")] {
        let found = FoundRow {
            device: on,
            object,
            name: name.into(),
            address: format!("10.0.0.{on}:47808"),
            via: None,
            network: None,
        };
        worker(
            &mut app,
            WorkerEvent::Discovery(DiscoveryEvent::IHave(found)),
        );
    }
    app.screen = Screen::Browse;
    assert_eq!(frame_asks(&mut app, 120, 40), [(7, vec![ai(0), ai(3)])]);
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("Supply Temp") && text.contains("Return Temp"),
        "{text}"
    );
    // On the device table only device 9 is unnamed; 7 came with its card.
    app.screen = Screen::Main;
    assert_eq!(
        frame_asks(&mut app, 120, 40),
        [(7, Vec::new()), (9, vec![device(9)])]
    );
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("Plant") && text.contains("AHU-8"), "{text}");
}

/// A name the device refused shows `-` and isn't asked for again, even
/// after scrolling away and back, until the device is read afresh: `r`,
/// once the worker took it (its card arrives), not when it was refused.
#[test]
fn a_name_the_device_refused_is_not_asked_again() {
    let mut app = browsing(3);
    assert_eq!(
        frame_asks(&mut app, 80, 24),
        [(7, vec![ai(0), ai(1), ai(2)])]
    );
    names(
        &mut app,
        7,
        vec![(ai(0), ReadName::Refused), (ai(1), named("Fan"))],
    );
    assert_eq!(app.name_of(7, ai(0)), Some(&Name::Unavailable));
    // A later failure doesn't lose a name.
    names(&mut app, 7, vec![(ai(1), ReadName::Refused)]);
    names(&mut app, 7, vec![(ai(1), ReadName::Unreachable)]);
    assert_eq!(app.name_of(7, ai(1)), Some(&Name::Known("Fan".into())));
    assert!(frame_asks(&mut app, 80, 24).is_empty(), "AI:2 is still out");
    names(&mut app, 7, vec![(ai(2), ReadName::Refused)]);
    press(&mut app, KeyCode::End);
    press(&mut app, KeyCode::Home);
    assert!(frame_asks(&mut app, 80, 24).is_empty());
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("analog-input 0  -"), "{text}");
    // `r` that the worker never took changes nothing.
    let refresh = press(&mut app, KeyCode::Char('r'));
    assert!(matches!(
        refresh.as_slice(),
        [Command::Browse {
            read: BrowseRead::Device,
            ..
        }]
    ));
    let link = app.link.clone();
    app.command_rejected(refresh[0].clone(), link, "full".into());
    assert!(frame_asks(&mut app, 80, 24).is_empty());
    // Taken, its card arrives, and the refused names are asked again.
    let req = req_of(&press(&mut app, KeyCode::Char('r')));
    assert!(
        frame_asks(&mut app, 80, 24).is_empty(),
        "not before the card"
    );
    card(&mut app, req);
    assert_eq!(frame_asks(&mut app, 80, 24), [(7, vec![ai(0), ai(2)])]);
}

/// A timeout leaves the name blank, not refused, and the device's next
/// I-Am asks for it again, telling the worker to read it.
#[test]
fn a_timeout_is_asked_again_after_the_next_i_am() {
    let mut app = table(2);
    assert_eq!(frame_asks(&mut app, 120, 40).len(), 2);
    names(&mut app, 2, vec![(device(2), ReadName::Unreachable)]);
    names(&mut app, 1, vec![(device(1), named("AHU-1"))]);
    assert_eq!(app.name_of(2, device(2)), Some(&Name::Unreachable));
    assert!(frame_asks(&mut app, 120, 40).is_empty());
    let text = screen(&mut app, 120, 40);
    assert!(!text.contains('…') && !text.contains(" - "), "{text}");
    // Device 1's I-Am changes nothing; device 2's asks again.
    worker(
        &mut app,
        WorkerEvent::Updated(row(1, "10.0.0.1:47808", 260, 5)),
    );
    assert!(frame_asks(&mut app, 120, 40).is_empty());
    worker(
        &mut app,
        WorkerEvent::Updated(row(2, "10.0.0.2:47808", 260, 5)),
    );
    let asked = name_asks(&frame_commands(&mut app, 120, 40));
    let again = NameAsk {
        device: 2,
        address: "10.0.0.2:47808".into(),
        max_apdu: Some(1476),
        objects: vec![device(2)],
        retry: true,
    };
    assert_eq!(asked, [again]);
    names(&mut app, 2, vec![(device(2), named("AHU-2"))]);
    assert!(screen(&mut app, 120, 40).contains("AHU-2"));
}

/// An ask the worker never took (a full queue, no session) is asked again
/// on the next frame, without a notice; so is a withdrawal, and a retry
/// keeps its flag.
#[test]
fn an_ask_that_was_not_taken_is_asked_again() {
    let mut app = browsing(2);
    let commands = frame_commands(&mut app, 80, 24);
    req_of(&commands);
    let link = app.link.clone();
    app.command_rejected(commands[0].clone(), link.clone(), "full".into());
    assert!(app.flash.is_none(), "no notice for a background read");
    let again = frame_commands(&mut app, 80, 24);
    assert_eq!(asks(&again), [(7, vec![ai(0), ai(1)])]);
    // The coordinator's refusal comes back as a failed result.
    let req = req_of(&again);
    let result = BrowseResult::Failed("no active connection".into());
    worker(&mut app, WorkerEvent::Browse { req, result });
    assert_eq!(frame_asks(&mut app, 80, 24), [(7, vec![ai(0), ai(1)])]);
    // A refused withdrawal is sent again too.
    press(&mut app, KeyCode::Esc);
    let withdraw = frame_commands(&mut app, 80, 24);
    assert_eq!(asks(&withdraw), [(7, Vec::new())]);
    app.command_rejected(withdraw[0].clone(), link, "full".into());
    assert_eq!(frame_asks(&mut app, 80, 24), [(7, Vec::new())]);
    assert!(frame_asks(&mut app, 80, 24).is_empty());
}

/// The device table: a Name column at 80 columns that a long name fits
/// with `…`, all of it from the wide width; and none, with nothing asked,
/// when the other columns leave too little room.
#[test]
fn the_device_table_fits_its_name_column() {
    let mut app = connected_app();
    worker(
        &mut app,
        WorkerEvent::Discovered(row(1001, "10.0.0.2:47808", 260, 0)),
    );
    let long = "Air handler 1 supply fan and economizer, level 3 mechanical room";
    names(&mut app, 1001, vec![(device(1001), named(long))]);
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("Air handler 1 supply"), "{text}");
    assert!(!text.contains(long) && text.contains('…'), "{text}");
    let text = screen(&mut app, 200, 50);
    assert!(text.contains(long), "{text}");
    // A routed device's long address and router squeeze the name out.
    let mut squeezed = row(4_194_302, "65534:0x7F000001C335BAC0DEADBEEF", 65_535, 0);
    squeezed.network = Some(65_534);
    squeezed.via = Some("192.168.100.254:47808".into());
    worker(&mut app, WorkerEvent::Discovered(squeezed));
    let asked = frame_asks(&mut app, 80, 24);
    assert!(asked.is_empty(), "{asked:?}");
    assert!(!app.devices.named.get());
    let text = screen(&mut app, 80, 24);
    assert!(!text.contains(" Name "), "{text}");
    assert_eq!(
        frame_asks(&mut app, 200, 50),
        [(4_194_302, vec![device(4_194_302)])]
    );
}

/// The object list names its objects at 80 and at 200 columns; a name
/// with control characters shows them as spaces.
#[test]
fn the_object_list_shows_names_at_every_width() {
    let mut app = browsing(3);
    names(
        &mut app,
        7,
        vec![
            (ai(0), named("Zone\u{1b}[2J Temp")),
            (ai(1), named(&"x".repeat(120))),
        ],
    );
    for (w, h) in [(80, 24), (200, 50)] {
        let text = screen(&mut app, w, h);
        assert!(
            text.contains("analog-input 0  Zone [2J Temp"),
            "{w}: {text}"
        );
        assert!(text.contains("xxxx…"), "{w}: {text}");
        assert!(text.contains("analog-input 2  …"), "{w}: {text}");
    }
}

/// A passive session asks for no names.
#[test]
fn a_passive_session_asks_nothing() {
    let mut app = new_app(None);
    app.safety.passive = true;
    worker(&mut app, WorkerEvent::Passive);
    worker(
        &mut app,
        WorkerEvent::Discovered(row(1001, "10.0.0.2:47808", 260, 0)),
    );
    assert!(frame_asks(&mut app, 120, 40).is_empty());
}
