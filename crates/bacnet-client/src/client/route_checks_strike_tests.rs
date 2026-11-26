//! Two strikes (decision, maintainer, 2026-11-26): a known path moves only
//! after two checks about a minute apart each find its router silent and
//! the same one other router alone answering the router query. Anything
//! else between or in the second clears the strike and keeps the path.

use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest};
use bacnet_types::enums::UnconfirmedServiceChoice;
use bytes::Bytes;
use tokio::time::{Duration, sleep};

use std::sync::Arc;

use bacnet_network::layer::NetworkLayer;
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::MacAddr;
use tokio::sync::mpsc;

use super::tests::{
    INSTANCE, NETWORK, ROUTER, STATION, Script, Sent, Site, WAIT, i_am, i_am_on, npdu,
    proprietary_message, received,
};
use super::{Found, RouteChecks};
use crate::discovery::{Change, Checked, STRIKE_GAP, Strike, now};

/// The router that took over, or claims to have.
const NEW: [u8; 1] = [8];
/// Past the end of one check: two steps of the APDU timeout.
const CHECK: Duration = Duration::from_secs(12);
const TICK: Duration = Duration::from_millis(1);

fn notification() -> Bytes {
    npdu(
        Some((NETWORK, &STATION)),
        Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
            service_request: Bytes::new(),
        }),
    )
}

/// A site whose device is quiet, a claim through [`NEW`], and the first
/// strike taken: the new router alone answered the first router query.
async fn struck() -> Site {
    let site = Site::new(Script {
        routers: vec![NEW.to_vec()],
        ..Script::default()
    })
    .await;
    site.hear(&NEW, i_am(&STATION, INSTANCE)).await;
    sleep(CHECK + TICK).await;
    assert_eq!((site.probes(), site.router_queries()), (1, 1));
    assert_eq!(site.router(), ROUTER, "moved on the first strike");
    site
}

/// One strike, then the old router answers the second router query: the
/// strike clears. The hold paces the path again, and a later check that
/// strikes is a first strike again, which moves nothing.
#[tokio::test(start_paused = true)]
async fn one_strike_then_the_old_router_answers_resets() {
    let site = struck().await;
    site.script.lock().unwrap().routers = vec![ROUTER.to_vec(), NEW.to_vec()];
    sleep(STRIKE_GAP + CHECK).await;
    assert_eq!((site.probes(), site.router_queries()), (2, 2));
    assert_eq!(site.router(), ROUTER);
    site.script.lock().unwrap().routers = vec![NEW.to_vec()];
    site.hear(&NEW, i_am(&STATION, INSTANCE)).await;
    sleep(Duration::from_secs(60) + CHECK).await;
    assert_eq!(site.router_queries(), 3, "{:?}", site.sent());
    assert_eq!(site.router(), ROUTER, "a cleared strike still counted");
    site.stop().await;
}

/// One strike, then the old router carries a notification from the device
/// before the second check: that check finds the old path working, asks no
/// router query and clears the strike.
#[tokio::test(start_paused = true)]
async fn one_strike_then_a_frame_through_the_old_router_resets() {
    let site = struck().await;
    site.hear(&ROUTER, notification()).await;
    sleep(STRIKE_GAP + CHECK).await;
    assert_eq!((site.probes(), site.router_queries()), (2, 1));
    assert_eq!(site.router(), ROUTER);
    site.stop().await;
}

/// One strike, then a control turned away during the second check's
/// router query: that check decides nothing, where a whole window would
/// have moved the rows. As in `lost_router_answers_keep_the_path`, the
/// drop is made on a layer of the test's whose counts the check reads; the
/// path is marked on the table directly, so the client's own loop stays
/// idle.
#[tokio::test(start_paused = true)]
async fn one_strike_then_a_lost_answer_moves_nothing() {
    let site = Site::new(Script {
        routers: vec![NEW.to_vec()],
        ..Script::default()
    })
    .await;
    let (lab_tx, lab_rx) = mpsc::channel(1024);
    let mut lab = NetworkLayer::new(
        TestTransport::builder()
            .local_mac(&[0x50])
            .inbound(lab_rx)
            .build(),
    );
    let (_controls, drops) = lab
        .enable_network_control_receiver_counting_drops()
        .unwrap();
    let mut decode = lab.start().await.unwrap();
    let client = &site.client;
    let checks = RouteChecks::new(
        Arc::clone(&client.network),
        client.network_report_tx.clone(),
        Some(drops),
        WAIT,
    );
    let path = (NETWORK, MacAddr::from_slice(&ROUTER));
    let mut claim = client
        .device_table
        .lock()
        .get(NETWORK, &STATION)
        .unwrap()
        .clone();
    claim.mac_address = MacAddr::from_slice(&NEW);
    let first = Strike {
        to: MacAddr::from_slice(&NEW),
        since: now(),
    };
    {
        let mut table = client.device_table.lock();
        assert_eq!(table.heard(claim).change, Change::Held);
        table.path_checked(&path, Checked::Struck(first), now());
    }
    let notices = (&client.device_tx, &client.device_collision_tx);
    let overfill = async {
        sleep(WAIT + Duration::from_secs(1)).await;
        for _ in 0..300 {
            lab_tx
                .send(received(&[0x77], proprietary_message()))
                .await
                .unwrap();
        }
        let _ = decode.try_recv();
    };
    let (found, ()) = tokio::join!(checks.check(&client.device_table, &path, notices), overfill);
    assert_eq!(found, Found::Kept);
    assert_eq!(site.router(), ROUTER);
    site.stop().await;
}

/// Two strikes, each a different router answering alone, move nothing.
#[tokio::test(start_paused = true)]
async fn two_strikes_with_different_routers_move_nothing() {
    let site = struck().await;
    site.script.lock().unwrap().routers = vec![vec![7]];
    sleep(STRIKE_GAP + CHECK).await;
    assert_eq!((site.probes(), site.router_queries()), (2, 2));
    assert_eq!(site.router(), ROUTER);
    site.stop().await;
}

/// The client stops between the strikes: the second check never runs,
/// nothing more is sent, and the row keeps its router.
#[tokio::test(start_paused = true)]
async fn a_client_stop_between_strikes_moves_nothing() {
    let site = struck().await;
    let before = site.sent.len();
    let Site { client, sent, .. } = site;
    let Ok(mut client) = Arc::try_unwrap(client) else {
        panic!("a task kept the client");
    };
    client.stop().await.unwrap();
    sleep(3 * STRIKE_GAP).await;
    assert_eq!(sent.len(), before);
    let table = client.device_table.lock();
    let row = table.get(NETWORK, &STATION).unwrap();
    assert_eq!(row.mac_address.as_slice(), ROUTER);
}

/// A forger's crowd: 256 rows it made through its own station and keeps
/// young through it, each marked from a second station of its own every
/// minute for an hour. Its own I-Ams answer each check through the path's
/// router, so each check ends early, and the per-path pacing bounds the
/// cost: at most 9 checks a path in the hour (at once, then after 1, 2, 4
/// and 8 minutes, then every 10), all unicast to the forger's own station,
/// with few router queries, the only broadcasts, among them.
#[tokio::test(start_paused = true)]
async fn a_crowd_of_forged_paths_is_paced_per_path() {
    const FORGER: [u8; 1] = [0x66];
    const SECOND: [u8; 1] = [0x67];
    let site = Site::empty(Script::default()).await;
    for network in 1..=256u16 {
        site.hear(&FORGER, i_am_on(network, &[1], u32::from(network)))
            .await;
    }
    for _ in 0..60 {
        for network in 1..=256u16 {
            let i_am = || i_am_on(network, &[1], u32::from(network));
            site.hear(&FORGER, i_am()).await;
            site.hear(&SECOND, i_am()).await;
        }
        sleep(Duration::from_secs(60) - 512 * TICK).await;
    }
    let to_forger = |sent: &Sent| matches!(sent, Sent::Probe { via, .. } if via == &FORGER);
    let probes = site.probes();
    let queries = site.count(|sent| matches!(sent, Sent::RouterQuery(_)));
    assert_eq!(site.count(to_forger), probes);
    assert!(probes <= 256 * 9, "{probes}");
    assert!(queries <= 60, "{queries}");
    site.stop().await;
}
