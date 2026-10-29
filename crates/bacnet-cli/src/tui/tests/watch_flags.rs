//! On the hub (#2115): a property row's badges and logged flags follow
//! the flags its notifications carry, and a first read is logged with the
//! mode its start led to.

use super::*;

/// The CSV lines of the one log in `dir`, header first.
fn log_lines(dir: &std::path::Path) -> Vec<Vec<String>> {
    let files: Vec<_> = std::fs::read_dir(dir).unwrap().collect();
    assert_eq!(files.len(), 1);
    let text = std::fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap();
    text.lines()
        .map(|line| line.split(',').map(str::to_string).collect())
        .collect()
}

fn logging(dir: &tempfile::TempDir) -> SessionOptions {
    let mut options = options(300, 10, 0);
    options.watch.workspace = Some(dir.path().to_owned());
    options
}

/// A row watching another property than Present_Value is subscribed by
/// SubscribeCOVProperty; the Status_Flags its notifications carry update
/// the row's badges and the log's status_flags column.
#[tokio::test(start_paused = true)]
async fn a_property_row_follows_the_flags_it_is_notified() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab_site(Server::lab(100, LAB), logging(&dir)).await;
    command(&mut site, "csv on").await;
    command(&mut site, "watch 100 ai:2 out-of-service").await;
    settle(&mut site).await;
    assert_eq!(sources(&site), [Source::Cov { lifetime: 300 }]);
    let badges = |site: &Site| site.app.watch.rows()[0].flags.map(|f| f.badges());
    assert!(
        !badges(&site).unwrap_or_default().contains(&"[OOS]"),
        "{:?}",
        badges(&site)
    );
    site.server(0)
        .write_local(
            &ai(2),
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
            LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    site.run_for(Duration::from_millis(500)).await;
    assert!(
        badges(&site).unwrap().contains(&"[OOS]"),
        "{:?}",
        badges(&site)
    );
    command(&mut site, "csv off").await;
    site.run_for(Duration::from_secs(1)).await;
    let lines = log_lines(dir.path());
    let last = lines.last().unwrap();
    assert_eq!(
        (last[4].as_str(), last[6].as_str()),
        ("true", "cov"),
        "{last:?}"
    );
    assert!(last[5].contains("[OOS]"), "{last:?}");
    site.stop().await;
}

/// The first read of a point the device won't subscribe is logged as
/// polled, the mode its start led to.
#[tokio::test(start_paused = true)]
async fn a_first_read_is_logged_with_the_mode_it_led_to() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab_site(no_cov(100, LAB, 1, Quirks::default()), logging(&dir)).await;
    command(&mut site, "csv on").await;
    command(&mut site, "watch 100 av:1").await;
    settle(&mut site).await;
    assert!(
        matches!(&sources(&site)[0], Source::Polled(_)),
        "{:?}",
        sources(&site)
    );
    command(&mut site, "csv off").await;
    site.run_for(Duration::from_secs(1)).await;
    let lines = log_lines(dir.path());
    assert!(lines.len() >= 2, "{lines:?}");
    assert_eq!(lines[1][6], "polled", "{:?}", lines[1]);
    site.stop().await;
}
