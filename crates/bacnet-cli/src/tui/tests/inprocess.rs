//! Discovery on the in-memory site ([`super::site`]).

use insta::assert_snapshot;

use super::site::Site;
use super::*;

const LAB: [(u8, u32); 5] = [(100, 100), (101, 101), (102, 102), (103, 103), (104, 104)];

#[tokio::test(start_paused = true)]
async fn range_who_is_lists_exactly_the_three_devices_in_range() {
    let mut site = Site::start(&LAB).await;
    site.who_is("101-103", 3).await;
    assert_eq!(visible(&mut site.app), [101, 102, 103]);
    assert_eq!(site.app.op.as_ref().unwrap().replies, 3);
    assert_snapshot!("lab_range_120x40", screen(&mut site.app, 120, 40));
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn unbounded_who_is_lists_all_five_devices() {
    let mut site = Site::start(&LAB).await;
    site.who_is("", 5).await;
    assert_eq!(visible(&mut site.app), [100, 101, 102, 103, 104]);
    let row = site.app.devices.get(104).unwrap();
    assert_eq!(row.address, "10.0.0.104:47808");
    assert_eq!(row.network, None);
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn two_servers_with_one_instance_raise_a_banner_naming_both() {
    let mut site = Site::start(&[(20, 200), (21, 200), (30, 300)]).await;
    site.who_is("", 2).await;
    site.pump_until(|app| !app.duplicates.is_empty()).await;
    assert_eq!(
        site.app.duplicates.lines(),
        ["instance 200 claimed by 10.0.0.20:47808 and 10.0.0.21:47808"]
    );
    let text = screen(&mut site.app, 120, 40);
    assert!(
        text.contains("DUPLICATE instance 200 claimed by 10.0.0.20:47808 and 10.0.0.21:47808"),
        "{text}"
    );
    assert_eq!(
        visible(&mut site.app),
        [200, 300],
        "the client keeps one row"
    );
    site.stop().await;
}
