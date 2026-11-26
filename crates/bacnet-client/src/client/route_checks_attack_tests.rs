//! The path checks under a forger's moves (#2823, review): who picks the
//! station asked, whether checks can hold the purge back or crowd a real
//! path out, answers the layer turned away, a node that may not initiate,
//! and the cases the checks leave to a forger, pinned as they are.

use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest};
use bacnet_types::MacAddr;
use bacnet_types::enums::{ObjectType, Segmentation, UnconfirmedServiceChoice};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::Bytes;
use tokio::time::{Duration, Instant, sleep, timeout};

use super::super::DeviceEventKind;
use std::sync::Arc;

use bacnet_network::layer::NetworkLayer;
use bacnet_transport::test_transport::TestTransport;
use tokio::sync::mpsc;

use super::RouteChecks;
use super::tests::{
    INSTANCE, NETWORK, OTHER, ROUTER, STATION, Script, Sent, Site, WAIT, i_am, i_am_on, npdu,
    proprietary_message, received,
};
use crate::discovery::{DiscoveredDevice, STRIKE_GAP, now};

fn router_at(site: &Site, network: u16, station: &[u8]) -> Vec<u8> {
    let table = site.client.device_table.lock();
    table.get(network, station).unwrap().mac_address.to_vec()
}

fn probe(via: &[u8], network: u16, station: &[u8]) -> Sent {
    Sent::Probe {
        via: via.to_vec(),
        network,
        station: station.to_vec(),
    }
}

/// Item 1: the forger names a row behind the router whose device is off
/// but not yet aged out. The check asks the row the client heard from last
/// through that router, which answers, so the path stays and no router
/// query goes out.
#[tokio::test(start_paused = true)]
async fn a_forger_naming_a_quiet_row_does_not_fail_the_probe() {
    const QUIET: [u8; 1] = [4];
    let site = Site::empty(Script {
        device_answers: true,
        quiet: vec![QUIET.to_vec()],
        routers: vec![OTHER.to_vec()],
        ..Script::default()
    })
    .await;
    site.hear(&ROUTER, i_am(&QUIET, 8)).await;
    site.hear(&ROUTER, i_am(&STATION, INSTANCE)).await;
    site.hear(&OTHER, i_am(&QUIET, 8)).await;
    sleep(3 * WAIT).await;
    assert_eq!(site.sent(), [probe(&ROUTER, NETWORK, &STATION)]);
    assert_eq!(router_at(&site, NETWORK, &QUIET), ROUTER);
    site.stop().await;
}

/// Item 2: 30 silent paths keep the checks busy for six minutes, and the
/// purge still runs on its five-minute beat: a row 700 seconds silent is
/// taken at 300 seconds.
#[tokio::test(start_paused = true)]
async fn checks_do_not_stall_the_purge() {
    const STALE: u16 = 999;
    let site = Site::empty(Script::default()).await;
    let started = Instant::now();
    let mut events = site.client.device_events();
    site.client.device_table.lock().upsert(DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        mac_address: MacAddr::from_slice(&ROUTER),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::SEGMENTED_BOTH,
        max_segments_accepted: None,
        vendor_id: 0,
        last_seen: now() - Duration::from_secs(700),
        last_announced: None,
        source_network: Some(STALE),
        source_address: Some(MacAddr::from_slice(&STATION)),
    });
    for network in 1..=30 {
        site.hear(&ROUTER, i_am_on(network, &STATION, u32::from(network)))
            .await;
    }
    for network in 1..=30 {
        site.hear(&OTHER, i_am_on(network, &STATION, u32::from(network)))
            .await;
    }
    let lost = timeout(Duration::from_secs(400), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.kind == DeviceEventKind::Lost && event.device.source_network == Some(STALE) {
                return started.elapsed();
            }
        }
    })
    .await
    .expect("the purge ran");
    assert!(
        lost >= Duration::from_secs(300) && lost < Duration::from_secs(301),
        "{lost:?}"
    );
    // About one check in twelve seconds since the start: still running.
    assert!((20..30).contains(&site.probes()), "{}", site.probes());
    site.stop().await;
}

/// Item 2: a forger fills the store with paths of its own, 256 rows it
/// made through its own station, each marked from a second station, all
/// silent (twelve seconds a check). A real router change on a network of
/// five long-known devices still gets its check within a minute: the
/// path outranks the forger's, keeps its place in the full store, and is
/// checked first after the batch running then.
#[tokio::test(start_paused = true)]
async fn a_path_with_more_rows_is_checked_first_and_kept_when_full() {
    const REAL: u16 = 1000;
    const FORGER: [u8; 1] = [0x66];
    const SECOND: [u8; 1] = [0x67];
    const NEW: [u8; 1] = [8];
    let site = Site::empty(Script::default()).await;
    for station in 1..=5u8 {
        let i_am = i_am_on(REAL, &[station], 1000 + u32::from(station));
        site.hear(&ROUTER, i_am).await;
    }
    for network in 1..=256u16 {
        site.hear(&FORGER, i_am_on(network, &[1], u32::from(network)))
            .await;
    }
    for network in 1..=256u16 {
        site.hear(&SECOND, i_am_on(network, &[1], u32::from(network)))
            .await;
    }
    site.hear(&NEW, i_am_on(REAL, &[1], 1001)).await;
    sleep(Duration::from_secs(60)).await;
    let real = |sent: &Sent| matches!(sent, Sent::Probe { network: REAL, .. });
    assert_eq!(site.count(real), 1, "{:?}", site.sent());
    site.stop().await;
}

/// Item 3: a network control the layer's queue turned away during the
/// router query's wait may have been the old router's answer, so the query
/// decides nothing then. The client's own dispatch drains its queue between
/// receives, so the drop is made here on a layer of the test's, whose
/// control queue nobody drains and whose counts the check reads.
#[tokio::test(start_paused = true)]
async fn lost_router_answers_keep_the_path() {
    let site = Site::new(Script {
        routers: vec![OTHER.to_vec()],
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
    let checks = RouteChecks::new(
        Arc::clone(&site.client.network),
        site.client.network_report_tx.clone(),
        Some(drops),
        WAIT,
    );
    let old = MacAddr::from_slice(&ROUTER);
    // Nothing lost: the lone answer decides.
    let whole = checks.routers_to(NETWORK, &old).await.unwrap();
    assert_eq!(
        whole.expect("nothing lost").others,
        [MacAddr::from_slice(&OTHER)]
    );
    // A control turned away during the wait: the question stays open.
    let overfill = async {
        sleep(Duration::from_secs(1)).await;
        for _ in 0..300 {
            lab_tx
                .send(received(&[0x77], proprietary_message()))
                .await
                .unwrap();
        }
        let _ = decode.try_recv();
    };
    let (open, ()) = tokio::join!(checks.routers_to(NETWORK, &old), overfill);
    assert!(open.unwrap().is_none());
    site.stop().await;
}

/// A seated client can't read its node's control queues, so it takes a
/// router query's answers as possibly lost: even a lone answer decides
/// nothing, and its checks never strike.
#[tokio::test(start_paused = true)]
async fn a_seated_client_takes_router_answers_as_possibly_lost() {
    let site = Site::new(Script {
        routers: vec![OTHER.to_vec()],
        ..Script::default()
    })
    .await;
    let seated = RouteChecks::new(
        Arc::clone(&site.client.network),
        site.client.network_report_tx.clone(),
        None,
        WAIT,
    );
    let routers = seated
        .routers_to(NETWORK, &MacAddr::from_slice(&ROUTER))
        .await;
    assert!(routers.unwrap().is_none());
    site.stop().await;
}

/// A decoy: the row the forger names is quiet, but answers the transaction
/// matching accepted (or, on B/IP, a forwarded frame, #2829) have kept its
/// age newest. The probe goes by the last I-Am through the router first,
/// so it asks the device that announced itself last, which answers.
#[tokio::test(start_paused = true)]
async fn a_decoy_kept_young_by_answers_is_not_the_probe() {
    const DECOY: [u8; 1] = [4];
    let site = Site::empty(Script {
        device_answers: true,
        quiet: vec![DECOY.to_vec()],
        routers: vec![OTHER.to_vec()],
        ..Script::default()
    })
    .await;
    site.hear(&ROUTER, i_am(&DECOY, 8)).await;
    site.hear(&ROUTER, i_am(&STATION, INSTANCE)).await;
    sleep(Duration::from_secs(1)).await;
    site.client.device_table.lock().answered(NETWORK, &DECOY);
    site.hear(&OTHER, i_am(&DECOY, 8)).await;
    sleep(3 * WAIT).await;
    assert_eq!(site.sent(), [probe(&ROUTER, NETWORK, &STATION)]);
    site.stop().await;
}

/// Item 6: a node that may not initiate (DeviceCommunicationControl) sends
/// no Who-Is, so it checks nothing and moves nothing.
#[tokio::test(start_paused = true)]
async fn a_restricted_node_sends_no_check() {
    let site = Site::new(Script {
        routers: vec![OTHER.to_vec()],
        ..Script::default()
    })
    .await;
    site.client.device_table.asking.restrict(true);
    site.hear(&OTHER, i_am(&STATION, INSTANCE)).await;
    sleep(3 * WAIT).await;
    assert_eq!((site.probes(), site.router_queries()), (0, 0));
    assert_eq!(site.router(), ROUTER);
    site.stop().await;
}

/// Item 6, a residual (#2824): a forger whose I-Am made the row (first
/// sight) is the path's router, so the check asks it, it answers, and the
/// real router's I-Am changes nothing.
#[tokio::test(start_paused = true)]
async fn a_first_sight_forger_keeps_the_path_by_answering_the_probe() {
    let site = Site::empty(Script {
        forger_answers: true,
        ..Script::default()
    })
    .await;
    site.hear(&OTHER, i_am(&STATION, INSTANCE)).await;
    site.hear(&ROUTER, i_am(&STATION, INSTANCE)).await;
    sleep(3 * WAIT).await;
    assert_eq!(site.sent(), [probe(&OTHER, NETWORK, &STATION)]);
    assert_eq!(site.router(), OTHER);
    site.stop().await;
}

/// Item 6, a residual: on a site whose router never answers router
/// queries, with the device quiet, one check the forger answers alone
/// moves nothing; a second, a minute later, that it answers alone too
/// while the old router and its devices stay silent through both, takes
/// the path, as a router that really took over would.
#[tokio::test(start_paused = true)]
async fn a_forger_answering_alone_takes_a_quiet_path_without_router_answers() {
    let site = Site::new(Script {
        routers: vec![OTHER.to_vec()],
        ..Script::default()
    })
    .await;
    site.hear(&OTHER, i_am(&STATION, INSTANCE)).await;
    sleep(2 * WAIT + Duration::from_millis(1)).await;
    assert_eq!(site.router(), ROUTER, "one answered check moved the path");
    sleep(STRIKE_GAP + 2 * WAIT).await;
    assert_eq!(site.router(), OTHER);
    site.stop().await;
}

/// Item 5: a frame the row's own router carries from the device, a COV
/// notification here, keeps the row young; one through another station
/// doesn't. A device heard only that way is not aged out.
#[tokio::test(start_paused = true)]
async fn frames_through_the_rows_router_keep_it_young() {
    let site = Site::new(Script::default()).await;
    let notification = || {
        npdu(
            Some((NETWORK, &STATION)),
            Apdu::UnconfirmedRequest(UnconfirmedRequest {
                service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
                service_request: Bytes::new(),
            }),
        )
    };
    let seen = || {
        site.client
            .device_table
            .lock()
            .get(NETWORK, &STATION)
            .unwrap()
            .last_seen
    };
    let first = seen();
    sleep(Duration::from_secs(10)).await;
    site.hear(&OTHER, notification()).await;
    assert_eq!(seen(), first);
    site.hear(&ROUTER, notification()).await;
    assert!(seen() > first);
    for _ in 0..20 {
        sleep(Duration::from_secs(60)).await;
        site.hear(&ROUTER, notification()).await;
    }
    assert!(site.client.get_device_at(NETWORK, &STATION).await.is_some());
    site.stop().await;
}
