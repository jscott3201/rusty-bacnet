//! TUI tests.
//!
//! - `app_core`: `update` driven by synthetic keys and worker events, with no
//!   terminal.
//! - `bip6_worker`: the worker on a real B/IPv6 client on `::1`: its status
//!   line and a directed Who-Is to an IPv6 address.
//! - `frames`: `TestBackend` frames at 80x24 and 120x40, compared with
//!   insta snapshots in `snapshots/`.
//! - `inprocess`: real `BACnetServer`s and a client on an in-memory network.
//! - `load`: a 1,000 events/s burst, and the render-time budget.
//! - `event_loop`: the `select!` loop on a `TestBackend` with scripted input.
//!
//! Snapshot workflow: CI runs plain `cargo nextest run -p bacnet-cli`, where a
//! changed frame fails the test. After an intended UI change, run
//! `cargo insta test --test-runner nextest -p bacnet-cli` and then
//! `cargo insta review` to accept the new `.snap` files.

mod app_core;
mod bip6_worker;
mod event_loop;
mod frames;
mod inprocess;
mod load;

use std::net::Ipv4Addr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use bacnet_types::enums::Segmentation;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;

use super::app::picker::Picker;
use super::app::{update, Action, App, AppConfig};
use super::keymap::map_key;
use super::log_layer::LogRing;
use super::message::{AddressStyle, Command, DeviceRow, WorkerEvent};
use crate::core::interfaces::Ipv4Interface;

/// One fixed start time per test process, so ages are exact offsets from it.
pub(super) fn t0() -> Instant {
    static BASE: OnceLock<Instant> = OnceLock::new();
    *BASE.get_or_init(Instant::now)
}

/// `t0() + secs`.
pub(super) fn at(secs: u64) -> Instant {
    t0() + Duration::from_secs(secs)
}

/// A BACnet/IP app, before connecting, without colour.
pub(super) fn new_app(picker: Option<Picker>) -> App {
    App::new(
        AppConfig {
            transport: "BIP",
            style: AddressStyle::Bip,
            color: false,
            picker,
            now: t0(),
        },
        LogRing::new(100),
    )
}

/// An app whose client is up at 10.0.0.1:47808.
pub(super) fn connected_app() -> App {
    let mut app = new_app(None);
    worker(
        &mut app,
        WorkerEvent::Connected {
            local: "10.0.0.1:47808".into(),
        },
    );
    app
}

/// A synthetic device row seen `seen` seconds after `t0()`.
pub(super) fn row(instance: u32, address: &str, vendor_id: u16, seen: u64) -> DeviceRow {
    DeviceRow {
        instance,
        address: address.into(),
        network: None,
        vendor_id,
        max_apdu: 1476,
        segmentation: Segmentation::NONE,
        last_seen: at(seen),
    }
}

/// Apply an action.
pub(super) fn act(app: &mut App, action: Action) -> Vec<Command> {
    update(app, action)
}

/// Apply a worker event.
pub(super) fn worker(app: &mut App, event: WorkerEvent) -> Vec<Command> {
    update(app, Action::Worker(event))
}

/// Press a key through the real keymap.
pub(super) fn press(app: &mut App, code: KeyCode) -> Vec<Command> {
    press_with(app, code, KeyModifiers::NONE)
}

/// Press a key with modifiers through the real keymap.
pub(super) fn press_with(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> Vec<Command> {
    match map_key(app, KeyEvent::new(code, modifiers)) {
        Some(action) => update(app, action),
        None => Vec::new(),
    }
}

/// Type text through the real keymap.
pub(super) fn type_text(app: &mut App, text: &str) {
    for ch in text.chars() {
        press(app, KeyCode::Char(ch));
    }
}

/// Move the app's clock to `now` with no other change.
pub(super) fn tick(app: &mut App, now: Instant) {
    let dropped = app.dropped;
    let log_generation = app.log.generation();
    update(
        app,
        Action::Tick {
            now,
            dropped,
            log_generation,
        },
    );
}

/// Move the app's clock forward by `by`.
pub(super) fn advance(app: &mut App, by: Duration) {
    let now = app.now + by;
    tick(app, now);
}

/// Instances of the visible rows, in order.
pub(super) fn visible(app: &mut App) -> Vec<u32> {
    app.settle();
    app.devices.visible().map(|r| r.instance).collect()
}

/// Draw one frame on a `width` x `height` test terminal.
pub(super) fn render(app: &mut App, width: u16, height: u16) -> Terminal<TestBackend> {
    app.settle();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| super::view::draw(frame, app))
        .unwrap();
    terminal
}

/// The frame as text, one quoted line per row.
pub(super) fn screen(app: &mut App, width: u16, height: u16) -> String {
    render(app, width, height).backend().to_string()
}

/// Two interfaces for the picker.
pub(super) fn interfaces() -> Vec<Ipv4Interface> {
    vec![
        Ipv4Interface {
            name: "en0".into(),
            ip: Ipv4Addr::new(10, 0, 1, 5),
            broadcast: Ipv4Addr::new(10, 0, 1, 255),
        },
        Ipv4Interface {
            name: "en7".into(),
            ip: Ipv4Addr::new(192, 168, 50, 20),
            broadcast: Ipv4Addr::new(192, 168, 50, 255),
        },
    ]
}
