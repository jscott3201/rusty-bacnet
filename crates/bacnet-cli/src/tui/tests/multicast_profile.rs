//! A B/IP profile with a multicast group (#941): the editor's Group and
//! Group TTL fields, and the summary that shows the group in place of the
//! broadcast address.

use super::*;
use crate::tui::app::connection::{Connection, Screen};
use crate::tui::app::{Link, Overlay};
use crate::tui::profile::service::{Reply, Request};
use crate::tui::profile::{Overrides, Profile};
use insta::assert_snapshot;

fn profile_app() -> App {
    let mut app = new_app(None);
    app.link = Link::Down;
    app.overlay = Overlay::Connection;
    app.connection = Some(Connection::new(Overrides::default()));
    app
}

fn member() -> Profile {
    let mut p = Profile::default();
    p.bip_mut().interface = "192.0.2.20".parse().unwrap();
    p.bip_mut().multicast = Some("239.255.66.1".parse().unwrap());
    p.bip_mut().multicast_ttl = 2;
    p
}

fn saved(commands: &[Command]) -> Profile {
    let [Command::Profile(Request::Save { profile, .. })] = commands else {
        panic!("unexpected {commands:?}");
    };
    profile.clone()
}

/// Move the editor's focus to field `index`.
fn focus(app: &mut App, index: usize) {
    while app.connection.as_ref().unwrap().focus != index {
        press(app, KeyCode::Tab);
    }
}

#[test]
fn the_editor_sets_and_clears_a_group() {
    let mut app = profile_app();
    press(&mut app, KeyCode::Char('n'));
    type_text(&mut app, "group");
    focus(&mut app, 1);
    for _ in 0..10 {
        press(&mut app, KeyCode::Backspace);
    }
    type_text(&mut app, "192.0.2.20");
    focus(&mut app, 9);
    type_text(&mut app, "239.255.66.1");
    focus(&mut app, 10);
    press(&mut app, KeyCode::Backspace);
    type_text(&mut app, "3");
    let profile = saved(&press(&mut app, KeyCode::Enter));
    assert_eq!(
        (profile.bip().multicast, profile.bip().multicast_ttl),
        (Some("239.255.66.1".parse().unwrap()), 3)
    );

    // An empty Group clears multicast.
    let mut app = profile_app();
    worker(
        &mut app,
        WorkerEvent::Profile(Ok(Reply::Loaded {
            name: "group".into(),
            profile: member(),
            checks: Default::default(),
        })),
    );
    press(&mut app, KeyCode::Char('e'));
    focus(&mut app, 9);
    for _ in 0..20 {
        press(&mut app, KeyCode::Backspace);
    }
    let profile = saved(&press(&mut app, KeyCode::Enter));
    assert_eq!(profile.bip().multicast, None);
}

#[test]
fn a_zero_group_ttl_is_refused_even_without_a_group() {
    let mut app = profile_app();
    press(&mut app, KeyCode::Char('n'));
    type_text(&mut app, "no-group");
    focus(&mut app, 10);
    press(&mut app, KeyCode::Backspace);
    type_text(&mut app, "0");
    assert!(press(&mut app, KeyCode::Enter).is_empty(), "not saved");
    let error = app.connection.as_ref().unwrap().error.clone().unwrap();
    assert!(error.contains("Group TTL must be 1-255"), "{error}");
}

#[test]
fn a_bad_group_is_refused_in_the_editor() {
    let mut app = profile_app();
    press(&mut app, KeyCode::Char('n'));
    type_text(&mut app, "bad");
    focus(&mut app, 9);
    type_text(&mut app, "10.0.0.1");
    assert!(press(&mut app, KeyCode::Enter).is_empty());
    let error = app.connection.as_ref().unwrap().error.clone().unwrap();
    assert!(
        error.contains("class D") || error.contains("interface"),
        "{error}"
    );
}

#[test]
fn multicast_profile_frames_at_minimum_and_full_size() {
    for (w, h) in [(80, 24), (120, 40)] {
        let mut app = profile_app();
        worker(
            &mut app,
            WorkerEvent::Profile(Ok(Reply::Loaded {
                name: "group".into(),
                profile: member(),
                checks: Default::default(),
            })),
        );
        assert_eq!(app.connection.as_ref().unwrap().screen, Screen::Preview);
        let text = screen(&mut app, w, h);
        assert!(
            text.contains("Group: 239.255.66.1:47808   TTL: 2"),
            "{text}"
        );
        assert!(!text.contains("Broadcast:"), "{text}");
        assert_snapshot!(format!("multicast_preview_{w}x{h}"), text);
        press(&mut app, KeyCode::Char('e'));
        let text = screen(&mut app, w, h);
        assert!(text.contains("239.255.66.1"), "{text}");
        assert_snapshot!(format!("multicast_editor_{w}x{h}"), text);
    }
}
