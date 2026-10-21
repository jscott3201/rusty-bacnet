//! The watch list against the lab device on the in-memory site: COV rows
//! that follow AI:1, renewal before the lifetime ends, cancels on remove
//! and on quit, and polling within the rate cap when a device has no COV.
//! All on paused time, so a 30 s run costs nothing.

use std::time::Duration;

use bacnet_server::LocalCommandSource;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;

use super::browser::open_device;
use super::site::{ClientLimits, Quirks, Server, Site};
use super::*;
use crate::tui::app::watch::Screen;
use crate::tui::watched::Source;
use crate::tui::worker::session::SessionOptions;
use crate::tui::worker::watch::WatchOptions;

const LAB: u32 = 100;
const LAB_MAC: [u8; 6] = [10, 0, 0, 100, 0xBA, 0xC0];
const SUBSCRIBE_COV: u8 = 5;
const SUBSCRIBE_COV_PROPERTY: u8 = 28;

fn ai(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

fn options(lifetime: u32, poll_secs: u64, max_rate: u32) -> SessionOptions {
    SessionOptions {
        max_rate,
        watch: WatchOptions {
            lifetime,
            poll_every: Duration::from_secs(poll_secs),
            workspace: None,
            // The site's client: a 2 s timeout, sent four times.
            request_budget: Duration::from_secs(8),
        },
        ..SessionOptions::default()
    }
}

/// The lab site, discovered, with the session run with `options`.
async fn lab_site(server: Server, options: SessionOptions) -> Site {
    let mut site = Site::start_with_options(vec![server], ClientLimits::default(), options).await;
    site.who_is("", 1).await;
    site
}

/// Type `line` on the command bar and run it.
async fn command(site: &mut Site, line: &str) {
    site.press(KeyCode::Char(':')).await;
    type_text(&mut site.app, line);
    site.press(KeyCode::Enter).await;
}

/// Run until every row has left `Starting`.
async fn settle(site: &mut Site) {
    site.pump_until(|app| {
        app.watch
            .rows()
            .iter()
            .all(|row| !matches!(row.source, Source::Starting))
    })
    .await;
}

fn sources(site: &Site) -> Vec<Source> {
    site.app
        .watch
        .rows()
        .iter()
        .map(|row| row.source.clone())
        .collect()
}

fn active(site: &Site) -> u64 {
    site.server(0).cov_counters().subscriptions_active
}

/// Draw the watch screen at the site's current time.
fn watch_screen(site: &mut Site, width: u16, height: u16) -> String {
    tick(&mut site.app, tokio::time::Instant::now().into_std());
    site.app.screen = Screen::Watch;
    screen(&mut site.app, width, height)
}

/// `a` in the browser watches AI:1 by COV; as the lab changes it every
/// 100 ms, the row's value and sparkline follow.
#[tokio::test(start_paused = true)]
async fn the_ai1_row_follows_its_value_by_cov() {
    let mut site = lab_site(Server::lab(100, LAB), SessionOptions::default()).await;
    open_device(&mut site).await;
    site.press(KeyCode::Char('/')).await;
    type_text(&mut site.app, "analog-input 1");
    site.press(KeyCode::Enter).await;
    site.press(KeyCode::Esc).await;
    site.press(KeyCode::Char('a')).await;
    settle(&mut site).await;
    assert_eq!(sources(&site), [Source::Cov { lifetime: 300 }]);
    let row = &site.app.watch.rows()[0];
    assert_eq!(row.point.object, ai(1));
    assert_eq!(row.units.as_deref(), Some("degrees-fahrenheit"));

    for step in 1..=20u8 {
        site.server(0)
            .write_local(
                &ai(1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(70.0 + f32::from(step % 8)),
                None,
                LocalCommandSource::ServerDevice,
            )
            .await
            .unwrap();
        site.run_for(Duration::from_millis(100)).await;
    }
    site.run_for(Duration::from_millis(500)).await;
    let row = &site.app.watch.rows()[0];
    assert_eq!(row.value.as_ref().unwrap().as_deref(), Ok("74 °F"));
    assert!(row.history.len() >= 20, "{:?}", row.history);
    assert_eq!(row.flags.unwrap().badges(), ["[ALM]", "[FLT]", "[OOS]"]);
    assert_eq!(site.app.watch.stats.subscriptions, 1);
    for (w, h) in [(120, 40), (80, 24)] {
        let text = watch_screen(&mut site, w, h);
        assert!(text.contains("COV 1"), "{text}");
        assert!(text.contains("▁▂▃▄▅▆▇█"), "{text}");
        assert_snapshot!(format!("watch_cov_{w}x{h}"), text);
    }
    site.stop().await;
}

/// The subscription is renewed before its lifetime runs out, so it is
/// still on the device after twice the lifetime.
#[tokio::test(start_paused = true)]
async fn the_renewal_goes_before_the_lifetime_ends() {
    let mut site = lab_site(Server::lab(100, LAB), options(60, 10, 0)).await;
    command(&mut site, "watch 100 ai:2").await;
    settle(&mut site).await;
    assert_eq!(sources(&site), [Source::Cov { lifetime: 60 }]);
    site.run_for(Duration::from_secs(130)).await;
    let subscribes: Vec<_> = site
        .hub
        .requests_to(&LAB_MAC)
        .into_iter()
        .filter(|(service, _)| *service == SUBSCRIBE_COV)
        .map(|(_, at)| at)
        .collect();
    assert!(subscribes.len() >= 3, "{subscribes:?}");
    for pair in subscribes.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            gap < Duration::from_secs(60) && gap >= Duration::from_secs(50),
            "renewed after {gap:?}"
        );
    }
    assert_eq!(active(&site), 1, "the subscription never lapsed");
    site.stop().await;
}

/// Quitting cancels both kinds of subscription and the device's COV table
/// is empty afterwards.
#[tokio::test(start_paused = true)]
async fn quitting_leaves_the_device_cov_table_empty() {
    let mut site = lab_site(Server::lab(100, LAB), SessionOptions::default()).await;
    command(&mut site, "watch 100 ai:1").await;
    command(&mut site, "watch 100 ai:2 status-flags").await;
    settle(&mut site).await;
    assert_eq!(
        sources(&site),
        [Source::Cov { lifetime: 300 }, Source::Cov { lifetime: 300 }]
    );
    site.run_for(Duration::from_secs(1)).await;
    assert_eq!(active(&site), 2);
    assert_eq!(site.app.watch.stats.subscriptions, 2);
    let text = screen(&mut site.app, 120, 40);
    assert!(text.contains("COV 2"), "{text}");
    site.quit().await;
    assert_eq!(active(&site), 0, "the cancels reached the device");
    let services = site.hub.services_to(&LAB_MAC);
    let count = |service| services.iter().filter(|s| **s == service).count();
    // A subscription and a cancel of each kind.
    assert_eq!(
        (count(SUBSCRIBE_COV), count(SUBSCRIBE_COV_PROPERTY)),
        (2, 2)
    );
    // No renewal task outlived the session.
    tokio::time::sleep(Duration::from_secs(700)).await;
    assert_eq!(site.hub.services_to(&LAB_MAC), services);
    site.stop().await;
}

/// Removing a row cancels that row's subscription and only that one.
#[tokio::test(start_paused = true)]
async fn removing_a_row_cancels_its_subscription() {
    let mut site = lab_site(Server::lab(100, LAB), SessionOptions::default()).await;
    command(&mut site, "watch 100 ai:1").await;
    command(&mut site, "watch 100 ai:2").await;
    settle(&mut site).await;
    site.run_for(Duration::from_secs(1)).await;
    assert_eq!(active(&site), 2);
    site.press(KeyCode::Char('3')).await;
    site.press(KeyCode::Char('x')).await;
    assert_eq!(site.app.watch.rows().len(), 1);
    assert_eq!(site.app.watch.rows()[0].point.object, ai(2));
    site.run_for(Duration::from_secs(1)).await;
    assert_eq!(active(&site), 1);
    assert_eq!(site.app.watch.stats.subscriptions, 1);
    site.quit().await;
    assert_eq!(active(&site), 0);
    site.stop().await;
}

/// A device without COV, with these quirks and `points` analog values.
fn no_cov(octet: u8, instance: u32, points: u32, quirks: Quirks) -> Server {
    Server {
        database: Some(super::lab::large_database(instance, points)),
        quirks: Quirks {
            refuse_cov: true,
            ..quirks
        },
        ..Server::bare(octet, instance)
    }
}

/// Requests the hub saw to the device at 10.0.0.`octet` in `[from, to)`,
/// of `service` when given.
fn requests_in(
    site: &Site,
    octet: u8,
    service: Option<u8>,
    from: tokio::time::Instant,
    to: tokio::time::Instant,
) -> usize {
    site.hub
        .requests_to(&[10, 0, 0, octet, 0xBA, 0xC0])
        .into_iter()
        .filter(|(s, at)| service.is_none_or(|w| w == *s) && *at >= from && *at < to)
        .count()
}

/// Four devices without COV: every row is polled by RPM, and over 30 s the
/// requests stay within the cap of 2 a second, which binds: each device on
/// its own would be polled about once a second.
#[tokio::test(start_paused = true)]
async fn devices_without_cov_are_polled_within_the_rate_cap() {
    const DEVICES: [u8; 4] = [101, 102, 103, 104];
    const ROWS: u32 = 3;
    const CAP: u32 = 2;
    let servers = DEVICES
        .iter()
        .map(|&octet| no_cov(octet, u32::from(octet), ROWS, Quirks::default()))
        .collect();
    let mut site =
        Site::start_with_options(servers, ClientLimits::default(), options(300, 1, CAP)).await;
    site.who_is("", DEVICES.len()).await;
    for octet in DEVICES {
        for n in 1..=ROWS {
            command(&mut site, &format!("watch {octet} av:{n}")).await;
        }
    }
    settle(&mut site).await;
    assert!(
        sources(&site)
            .iter()
            .all(|s| matches!(s, Source::Polled(reason) if reason.contains("COV refused"))),
        "{:?}",
        sources(&site)
    );
    // Let the start-up reads drain, then measure 30 s.
    site.run_for(Duration::from_secs(10)).await;
    let from = tokio::time::Instant::now();
    site.run_for(Duration::from_secs(30)).await;
    let to = tokio::time::Instant::now();
    let requests: usize = DEVICES
        .iter()
        .map(|&octet| requests_in(&site, octet, None, from, to))
        .sum();
    let cap = (CAP * 30) as usize;
    assert!(requests <= cap, "{requests} requests in 30 s");
    assert!(requests >= cap - 4, "{requests}: the cap binds");
    assert!(site.app.watch.stats.recent_requests <= (CAP * 10) as usize);
    for octet in DEVICES {
        let mac = [10, 0, 0, octet, 0xBA, 0xC0];
        assert_eq!(site.hub.most_outstanding(&mac), 1, "device {octet}");
    }
    assert_eq!(site.app.watch.stats.subscriptions, 0);
    let row = &site.app.watch.rows()[0];
    assert_eq!(row.value.as_ref().unwrap().as_deref(), Ok("0"));
    let text = watch_screen(&mut site, 120, 40);
    assert!(text.contains("polled"), "{text}");
    assert!(text.contains("COV 0"), "{text}");
    assert_snapshot!("watch_polled_120x40", text);
    site.stop().await;
}

/// A device that stops answering is polled again after a wait that
/// doubles, not every interval: over 300 s at a 1 s interval it gets a
/// handful of polls rather than one every nine seconds.
#[tokio::test(start_paused = true)]
async fn a_silent_device_is_polled_with_a_growing_wait() {
    const RPM: u8 = 14;
    let silent = Quirks {
        silent: true,
        ..Quirks::default()
    };
    let mut site = lab_site(no_cov(100, LAB, 1, silent), options(300, 1, 0)).await;
    command(&mut site, "watch 100 av:1").await;
    settle(&mut site).await;
    let sources_now = sources(&site);
    let [Source::Polled(reason)] = sources_now.as_slice() else {
        panic!("{:?}", sources(&site));
    };
    assert!(reason.contains("no answer"), "{reason}");
    let from = tokio::time::Instant::now();
    site.run_for(Duration::from_secs(300)).await;
    let to = tokio::time::Instant::now();
    // Each poll is sent four times (three retries) before it times out.
    let polls = requests_in(&site, 100, Some(RPM), from, to) / 4;
    assert!((4..=10).contains(&polls), "{polls} polls in 300 s");
    let row = &site.app.watch.rows()[0];
    assert!(row.value.as_ref().unwrap().is_err(), "{:?}", row.value);
    site.stop().await;
}

/// A device that fails a whole RPM for one unknown object: that batch is
/// read one property at a time, so its healthy rows keep their values.
#[tokio::test(start_paused = true)]
async fn one_bad_object_leaves_its_batch_healthy() {
    let failing = Quirks {
        fail_rpm: true,
        ..Quirks::default()
    };
    let mut site = lab_site(no_cov(100, LAB, 2, failing), options(300, 1, 0)).await;
    for object in ["av:1", "av:99", "av:2"] {
        command(&mut site, &format!("watch 100 {object}")).await;
    }
    settle(&mut site).await;
    site.run_for(Duration::from_secs(5)).await;
    let values: Vec<_> = site
        .app
        .watch
        .rows()
        .iter()
        .map(|row| row.value.clone().unwrap().is_ok())
        .collect();
    assert_eq!(values, [true, false, true]);
    site.stop().await;
}

/// A renewal that times out, retries and all, turns the row to polling,
/// and a later subscription brings it back to COV.
#[tokio::test(start_paused = true)]
async fn a_failed_renewal_subscribes_again() {
    let server = Server {
        quirks: Quirks {
            lose_cov_after_first: 4,
            ..Quirks::default()
        },
        ..Server::lab(100, LAB)
    };
    let mut site = lab_site(server, options(60, 10, 0)).await;
    command(&mut site, "watch 100 ai:2").await;
    settle(&mut site).await;
    assert_eq!(sources(&site), [Source::Cov { lifetime: 60 }]);
    // Renewed 8 s early, at 52 s; four tries time out by 60 s.
    site.run_for(Duration::from_secs(70)).await;
    let sources_now = sources(&site);
    let [Source::Polled(reason)] = sources_now.as_slice() else {
        panic!("{:?}", sources(&site));
    };
    assert!(reason.contains("renewal failed"), "{reason}");
    // Polled meanwhile, then subscribed again 30 s after the failure.
    site.run_for(Duration::from_secs(40)).await;
    assert_eq!(sources(&site), [Source::Cov { lifetime: 60 }]);
    assert_eq!(active(&site), 1);
    site.quit().await;
    assert_eq!(active(&site), 0);
    site.stop().await;
}

/// `csv on` logs every sample to a file in the profile workspace, and
/// `csv off` closes it; without a workspace it says why it can't.
#[tokio::test(start_paused = true)]
async fn csv_logging_writes_samples_to_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let mut with_workspace = options(300, 10, 0);
    with_workspace.watch.workspace = Some(dir.path().to_owned());
    let mut site = lab_site(Server::lab(100, LAB), with_workspace).await;
    command(&mut site, "csv on").await;
    command(&mut site, "watch 100 ai:2").await;
    settle(&mut site).await;
    for value in [52.0, 53.5] {
        site.server(0)
            .set_present_value_local(&ai(2), PropertyValue::Real(value))
            .await
            .unwrap();
        site.run_for(Duration::from_millis(500)).await;
    }
    assert!(site.app.watch.stats.csv.is_some());
    command(&mut site, "csv off").await;
    site.run_for(Duration::from_secs(1)).await;
    assert_eq!(site.app.watch.stats.csv, None);
    let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(files.len(), 1);
    let text = std::fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], crate::tui::worker::watch::csv::HEADER);
    let values: Vec<&str> = lines[1..]
        .iter()
        .map(|line| {
            let fields: Vec<&str> = line.split(',').collect();
            assert_eq!(fields[1..4], ["100", "analog-input 2", "present-value"]);
            fields[4]
        })
        .collect();
    assert!(values.ends_with(&["52", "53.5"]), "{values:?}");
    site.stop().await;

    let mut site = lab_site(Server::lab(100, LAB), SessionOptions::default()).await;
    command(&mut site, "csv on").await;
    site.run_for(Duration::from_secs(1)).await;
    let flash = site.app.flash.as_ref().unwrap();
    assert!(flash.text.contains("profile workspace"), "{}", flash.text);
    site.stop().await;
}
