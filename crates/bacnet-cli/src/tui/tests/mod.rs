//! TUI tests.
//!
//! - `app_core`: `update` driven by synthetic keys and worker events, with no
//!   terminal.
//! - `bip6_worker`: the worker on a real B/IPv6 client on `::1`: its status
//!   line and a directed Who-Is to an IPv6 address.
//! - `frames`: `TestBackend` frames at 80x24 and 120x40, compared with
//!   insta snapshots in `snapshots/`.
//! - `inprocess`: discovery with real `BACnetServer`s and a client on the
//!   in-memory network of `site`.
//! - `browser`: the device browser against the lab device of
//!   `tests/support/lab.rs` on that network.
//! - `load`: a 1,000 events/s burst, and the render-time budget.
//! - `event_loop`: the `select!` loop on a `TestBackend` with scripted input.
//! - `network`: the Network screen against real B/IP transports and BBMDs
//!   on loopback; `network_frames`: its frames from synthetic events.
//! - `discovery`: the `:find` panel and the Networks tab, from synthetic
//!   events and against a real BBMD and `BACnetRouter`.
//! - `sc_profiles`: profiles created through the screen and reloaded, the
//!   refusal of an invalid SC profile before any dial, the flags-only path,
//!   and the SC connection screen's frames.
//! - `sc_screen`: the SC screen from synthetic reports, with its frames;
//!   `sc_hub`: the SC worker against in-process mTLS hubs; `sc_lint`: the
//!   credential lint on the preview, in the coordinator and on the SC screen.
//! - `mstp_monitor`: the MS/TP monitor and the safe-join wizard end to end,
//!   on the scripted ring of `bacnet-transport`'s `test-support`.
//! - `mstp_screen`: the MS/TP screen and the join confirmation from
//!   synthetic reports and keys, with their frames, and the sampler.
//! - `footer`: the footer and the status bar at 80, 120 and 200 columns:
//!   notices, hint priorities, the failure summary and the `FD` state.
//! - `help_keys`: the screen strip, help against the key map, and the
//!   command bar's syntax hints.
//! - `names`: object and device names (#1896): only the rows on screen
//!   are asked for, in one command a tick, free names cost nothing, and
//!   refusals and timeouts are kept apart; `names_site`: the reads through
//!   the per-device gate and the rate cap, against the lab device.
//! - `watch_rows`: watch rows' names, routed devices and column fit at
//!   80, 120 and 200 columns, and the name read against refusing devices.
//!
//! Snapshot workflow: CI runs plain `cargo nextest run -p bacnet-cli`, where a
//! changed frame fails the test. After an intended UI change, run
//! `cargo insta test --test-runner nextest -p bacnet-cli` and then
//! `cargo insta review` to accept the new `.snap` files.

mod app_core;
mod bip6_worker;
mod browser;
mod build_retirement;
mod capture;
mod capture_frames;
mod capture_live;
mod capture_pcap;
mod capture_views;
mod connections;
#[cfg(unix)]
mod crash;
mod devices_layout;
mod discovery;
mod event_loop;
mod footer;
mod frames;
mod help_keys;
mod inprocess;
#[allow(dead_code)] // The integration tests use the rest.
#[path = "../../../tests/support/lab.rs"]
mod lab;
mod load;
mod mstp_join_audit;
mod mstp_layout;
mod mstp_monitor;
#[cfg(all(target_os = "linux", feature = "serial"))]
mod mstp_monitor_pty;
mod mstp_screen;
mod multicast_profile;
mod names;
mod names_cache;
mod names_filter;
mod names_site;
mod network;
mod network_frames;
/// Reruns a test whose freed ephemeral port another process takes (#1877).
#[path = "../../../../bacnet-endpoint/tests/support/port_retry.rs"]
mod port_retry;
mod profile_ui;
#[cfg(feature = "sc-tls")]
mod sc_hub;
mod sc_layout;
mod sc_lint;
mod sc_profiles;
#[cfg(feature = "sc-tls")]
mod sc_reconnect;
mod sc_screen;
mod site;
mod values;
mod watch;
mod watch_rows;
mod writes;
mod writes_app;
mod writes_arrays;
mod writes_indexed;
mod writes_ledger;

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
use super::message::{Command, DeviceRow, WorkerEvent};
use crate::core::interfaces::Ipv4Interface;
use crate::core::target::AddressStyle;

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
            join: None,
            passive: false,
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
        via: None,
        network: None,
        vendor_id,
        max_apdu: 1476,
        segmentation: Segmentation::NONE,
        last_seen: at(seen),
    }
}

/// A synthetic row for a device a router at `router` passed on: station
/// `station` on network `network`, written as the worker writes it.
pub(super) fn routed_row(
    instance: u32,
    (network, station): (u16, &[u8]),
    router: &str,
    vendor_id: u16,
    seen: u64,
) -> DeviceRow {
    let address = crate::core::target::routed_station(network, station).expect("a station");
    let mut row = row(instance, &address, vendor_id, seen);
    row.network = Some(network);
    row.via = Some(router.into());
    row
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
