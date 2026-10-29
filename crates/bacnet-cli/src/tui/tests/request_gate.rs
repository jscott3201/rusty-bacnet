//! One gate per device across the TUI's workers (#1791): browsing a device
//! the watch list also follows, by COV with renewals or by polling, never
//! has two of our confirmed requests outstanding to it at once.
//!
//! The device answers slowly, so every request stays outstanding for a
//! while and the workers' requests would overlap without the shared gate.

use std::time::Duration;

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use ratatui::crossterm::event::KeyCode;

use super::site::{ClientLimits, Quirks, Server, Site};
use super::watch::{command, options, settle, sources};
use super::*;
use crate::tui::point::BrowseRead;
use crate::tui::watched::Source;

const LAB: u32 = 100;
const LAB_MAC: [u8; 6] = [10, 0, 0, 100, 0xBA, 0xC0];
const SUBSCRIBE_COV: u8 = 5;
const SUBSCRIBE_COV_PROPERTY: u8 = 28;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// The lab device answering after `latency`, with `quirks`.
fn slow_lab(quirks: Quirks) -> Server {
    Server {
        quirks: Quirks {
            latency: Duration::from_millis(200),
            ..quirks
        },
        ..Server::lab(100, LAB)
    }
}

/// Browse the lab device for `span`: open it, then read an object every
/// 300 ms, as someone paging through its objects would.
async fn browse_for(site: &mut Site, span: Duration) {
    let objects = [
        oid(ObjectType::ANALOG_INPUT, 1),
        oid(ObjectType::ANALOG_INPUT, 2),
        oid(ObjectType::ANALOG_OUTPUT, 1),
        oid(ObjectType::ANALOG_VALUE, 1),
    ];
    site.send(vec![Command::Browse {
        req: 5_000,
        device: LAB,
        read: BrowseRead::Device,
    }])
    .await;
    let step = Duration::from_millis(300);
    let steps = u32::try_from(span.as_millis() / step.as_millis()).unwrap();
    for i in 0..steps {
        let object = objects[i as usize % objects.len()];
        site.send(vec![Command::Browse {
            req: 5_001 + u64::from(i),
            device: LAB,
            read: BrowseRead::Object(object),
        }])
        .await;
        site.run_for(step).await;
    }
}

/// COV rows renewed every 12 s while the device is browsed: the renewals
/// take the same gate as the browser's reads.
#[tokio::test(start_paused = true)]
async fn browsing_a_device_whose_rows_renew_keeps_one_request_outstanding() {
    let mut site = Site::start_with_options(
        vec![slow_lab(Quirks::default())],
        ClientLimits::default(),
        {
            // A 20 s lifetime, renewed 8 s early.
            options(20, 10, 0)
        },
    )
    .await;
    site.who_is("", 1).await;
    command(&mut site, "watch 100 ai:1").await;
    command(&mut site, "watch 100 ai:2 status-flags").await;
    settle(&mut site).await;
    assert_eq!(
        sources(&site),
        [Source::Cov { lifetime: 20 }, Source::Cov { lifetime: 20 }]
    );
    browse_for(&mut site, Duration::from_secs(40)).await;
    let services = site.hub.services_to(&LAB_MAC);
    let subscriptions = services
        .iter()
        .filter(|s| [SUBSCRIBE_COV, SUBSCRIBE_COV_PROPERTY].contains(s))
        .count();
    assert!(subscriptions >= 6, "renewals ran: {services:?}");
    assert_eq!(site.hub.most_outstanding(&LAB_MAC), 1);
    site.stop().await;
}

/// Polled rows read every second while the device is browsed.
#[tokio::test(start_paused = true)]
async fn browsing_a_polled_device_keeps_one_request_outstanding() {
    let quirks = Quirks {
        refuse_cov: true,
        ..Quirks::default()
    };
    let mut site = Site::start_with_options(
        vec![slow_lab(quirks)],
        ClientLimits::default(),
        options(300, 1, 0),
    )
    .await;
    site.who_is("", 1).await;
    command(&mut site, "watch 100 ai:1").await;
    command(&mut site, "watch 100 ai:2").await;
    settle(&mut site).await;
    assert!(
        sources(&site)
            .iter()
            .all(|s| matches!(s, Source::Polled(_))),
        "{:?}",
        sources(&site)
    );
    browse_for(&mut site, Duration::from_secs(20)).await;
    assert_eq!(site.hub.most_outstanding(&LAB_MAC), 1);
    site.stop().await;
}

/// Writes while a COV row renews every 10 s: each write takes the device's
/// turn, so it never overlaps a renewal.
#[tokio::test(start_paused = true)]
async fn writing_to_a_device_whose_rows_renew_keeps_one_request_outstanding() {
    use super::browser::{open_device, open_object};
    use super::writes::{arm, finished, write, writes_sent};
    use crate::tui::profile::Safety;
    use crate::tui::worker::watch::WatchOptions;

    let dir = tempfile::tempdir().unwrap();
    let mut session = super::writes::options(dir.path(), Safety::default(), false);
    session.watch = WatchOptions {
        lifetime: 20,
        poll_every: Duration::from_secs(10),
        workspace: None,
        request_budget: Duration::from_secs(8),
    };
    let mut site = Site::start_with_options(
        vec![slow_lab(Quirks::default())],
        ClientLimits::default(),
        session,
    )
    .await;
    site.pump_until(|app| app.safety.policy.is_some()).await;
    open_device(&mut site).await;
    open_object(&mut site, "analog-output").await;
    command(&mut site, "watch 100 ai:1").await;
    settle(&mut site).await;
    assert_eq!(sources(&site), [Source::Cov { lifetime: 20 }]);
    arm(&mut site).await;
    for i in 0..24 {
        write(&mut site, &(40 + i).to_string(), "8").await;
        site.press(KeyCode::Char('y')).await;
        finished(&mut site).await;
        site.press(KeyCode::Esc).await;
        site.run_for(Duration::from_millis(900)).await;
    }
    assert_eq!(writes_sent(&site), 24);
    let renewals = site
        .hub
        .services_to(&LAB_MAC)
        .iter()
        .filter(|s| **s == SUBSCRIBE_COV)
        .count();
    assert!(renewals >= 3, "{renewals} subscription requests");
    assert_eq!(site.hub.most_outstanding(&LAB_MAC), 1);
    site.stop().await;
}
