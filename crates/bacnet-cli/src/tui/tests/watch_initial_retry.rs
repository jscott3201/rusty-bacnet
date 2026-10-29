//! Initial failures recover through the existing subscription backoff.

use super::*;

fn cov_requests(site: &Site, service: u8) -> usize {
    site.hub
        .services_to(&LAB_MAC)
        .iter()
        .filter(|&&s| s == service)
        .count()
}

#[tokio::test(start_paused = true)]
async fn initial_read_timeout_recovers_to_live_cov_and_cancels_on_quit() {
    let server = Server {
        quirks: Quirks {
            swallow_first: 4,
            ..Quirks::default()
        },
        ..Server::lab(100, LAB)
    };
    let mut site = lab_site(server, options(300, 10, 0)).await;
    command(&mut site, "watch 100 ai:2").await;
    settle(&mut site).await;
    assert!(matches!(&sources(&site)[0], Source::Polled(reason) if reason.contains("no answer")));
    assert_eq!(cov_requests(&site, SUBSCRIBE_COV), 0);
    site.run_for(Duration::from_secs(29)).await;
    assert_eq!(cov_requests(&site, SUBSCRIBE_COV), 0, "no early retry");
    site.run_for(Duration::from_secs(2)).await;
    assert_eq!(sources(&site), [Source::Cov { lifetime: 300 }]);
    assert_eq!(active(&site), 1);
    let polls = site
        .hub
        .services_to(&LAB_MAC)
        .iter()
        .filter(|&&s| s == 14)
        .count();
    site.server(0)
        .set_present_value_local(&ai(2), PropertyValue::Real(61.25))
        .await
        .unwrap();
    site.run_for(Duration::from_millis(500)).await;
    assert_eq!(
        site.app.watch.rows()[0].value.as_ref().unwrap().as_deref(),
        Ok("61.25 °F")
    );
    assert_eq!(
        site.hub
            .services_to(&LAB_MAC)
            .iter()
            .filter(|&&s| s == 14)
            .count(),
        polls,
        "the new value arrived by COV, without a poll"
    );
    site.quit().await;
    assert_eq!(active(&site), 0);
    let requests = cov_requests(&site, SUBSCRIBE_COV);
    tokio::time::advance(Duration::from_secs(310)).await;
    assert_eq!(
        cov_requests(&site, SUBSCRIBE_COV),
        requests,
        "no renewal after quit"
    );
    site.stop().await;
}

async fn initial_subscribe_timeout_recovers(property: &str, service: u8, remove: bool) {
    let server = Server {
        quirks: Quirks {
            swallow_first_cov: 4,
            ..Quirks::default()
        },
        ..Server::lab(100, LAB)
    };
    let mut site = lab_site(server, options(300, 10, 0)).await;
    command(&mut site, &format!("watch 100 ai:2{property}")).await;
    settle(&mut site).await;
    assert!(matches!(&sources(&site)[0], Source::Polled(reason) if reason.contains("COV refused")));
    site.pump_until(|app| app.watch.rows()[0].value.is_some())
        .await;
    assert!(
        site.app.watch.rows()[0].value.as_ref().unwrap().is_ok(),
        "first read retained"
    );
    assert_eq!(cov_requests(&site, service), 4);
    assert_eq!(
        active(&site),
        0,
        "request loss did not create a subscription"
    );
    site.run_for(Duration::from_secs(29)).await;
    assert_eq!(cov_requests(&site, service), 4, "no early retry");
    site.run_for(Duration::from_secs(2)).await;
    assert_eq!(sources(&site), [Source::Cov { lifetime: 300 }]);
    assert_eq!(cov_requests(&site, service), 5);
    assert_eq!(active(&site), 1);
    if remove {
        site.press(KeyCode::Char('3')).await;
        site.press(KeyCode::Char('x')).await;
        assert!(site.app.watch.rows().is_empty());
        site.run_for(Duration::from_secs(1)).await;
        assert_eq!(active(&site), 0, "removing the recovered row cancelled it");
    }
    site.quit().await;
    assert_eq!(active(&site), 0);
    assert_eq!(
        cov_requests(&site, service),
        6,
        "same service cancels the recovered subscription"
    );
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn initial_object_subscription_timeout_recovers_and_cancels() {
    initial_subscribe_timeout_recovers("", SUBSCRIBE_COV, false).await;
}

#[tokio::test(start_paused = true)]
async fn initial_property_subscription_timeout_recovers_and_cancels() {
    initial_subscribe_timeout_recovers(" status-flags", SUBSCRIBE_COV_PROPERTY, false).await;
}

#[tokio::test(start_paused = true)]
async fn permanent_cov_refusal_keeps_polling_without_resubscribing() {
    let mut site = lab_site(no_cov(100, LAB, 1, Quirks::default()), options(300, 10, 0)).await;
    command(&mut site, "watch 100 av:1").await;
    settle(&mut site).await;
    assert!(matches!(&sources(&site)[0], Source::Polled(_)));
    assert_eq!(cov_requests(&site, SUBSCRIBE_COV), 1);
    let reads = site
        .hub
        .services_to(&LAB_MAC)
        .iter()
        .filter(|&&s| s == 14)
        .count();
    site.run_for(Duration::from_secs(330)).await;
    assert_eq!(cov_requests(&site, SUBSCRIBE_COV), 1);
    assert!(
        site.hub
            .services_to(&LAB_MAC)
            .iter()
            .filter(|&&s| s == 14)
            .count()
            > reads + 20
    );
    site.quit().await;
    assert_eq!(
        cov_requests(&site, SUBSCRIBE_COV),
        1,
        "no ambiguous subscription to cancel"
    );
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn removing_recovered_property_subscription_cancels_it() {
    initial_subscribe_timeout_recovers(" status-flags", SUBSCRIBE_COV_PROPERTY, true).await;
}

/// Once a permanent refusal excludes retries, every later read is a poll:
/// the original polling ceiling remains independently observable.
#[tokio::test(start_paused = true)]
async fn silent_device_after_permanent_refusal_keeps_original_poll_backoff_bound() {
    let mut site = lab_site(no_cov(100, LAB, 1, Quirks::default()), options(300, 1, 0)).await;
    command(&mut site, "watch 100 av:1").await;
    settle(&mut site).await;
    assert!(matches!(&sources(&site)[0], Source::Polled(reason) if reason.contains("COV refused")));
    assert_eq!(cov_requests(&site, SUBSCRIBE_COV), 1);
    site.hub.silence(&LAB_MAC);
    let from = tokio::time::Instant::now();
    site.run_for(Duration::from_secs(300)).await;
    let to = tokio::time::Instant::now();
    assert_eq!(
        cov_requests(&site, SUBSCRIBE_COV),
        1,
        "no subscription retry reads"
    );
    let polls = requests_in(&site, 100, Some(14), from, to) / 4;
    assert!((4..=10).contains(&polls), "{polls} polls in 300 s");
    site.stop().await;
}

/// A row whose device isn't in the device table fails at once, sending
/// nothing, as it did before the list ran on the hub (#2115). Once the
/// device is discovered, watching the point again subscribes: the failed
/// row left nothing behind in the hub.
#[tokio::test(start_paused = true)]
async fn a_row_for_a_device_not_yet_discovered_fails_without_a_request() {
    let mut site = Site::start_with_options(
        vec![Server::lab(100, LAB)],
        ClientLimits::default(),
        options(300, 10, 0),
    )
    .await;
    command(&mut site, "watch 100 ai:1").await;
    site.pump_until(|app| app.watch.rows().is_empty()).await;
    let flash = site
        .app
        .flash
        .as_ref()
        .map(|f| f.text.clone())
        .unwrap_or_default();
    assert_eq!(
        flash,
        "Can't watch analog-input 1: device 100 is not in the device table"
    );
    assert!(site.hub.services_to(&LAB_MAC).is_empty(), "nothing sent");
    site.who_is("", 1).await;
    command(&mut site, "watch 100 ai:1").await;
    settle(&mut site).await;
    assert_eq!(sources(&site), [Source::Cov { lifetime: 300 }]);
    assert_eq!(active(&site), 1);
    site.stop().await;
}
