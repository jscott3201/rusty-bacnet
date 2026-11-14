//! Where a discovery's Who-Is goes (#2560): a local broadcast unless the
//! request names another scope, on the shared test transport's send log.
//! Behind a router, the routed lab (`routed_tests`) shows what each scope
//! reaches.

use std::time::Duration;

use bacnet_encoding::apdu::Apdu;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_transport::test_transport::{SendLog, SentFrame};
use bacnet_types::enums::UnconfirmedServiceChoice;
use bacnet_types::error::Error;

use super::test_bus::{Bus, N, at, client, mac, server, stop};
use crate::client::{DISCOVER_WAIT, Discover, Scope};

/// The Who-Is frames in `log`, broadcast or not.
fn who_is_frames(log: &SendLog) -> Vec<SentFrame> {
    log.frames()
        .into_iter()
        .filter(|frame| {
            matches!(frame.apdu(), Apdu::UnconfirmedRequest(request)
                if request.service_choice == UnconfirmedServiceChoice::WHO_IS)
        })
        .collect()
}

/// The one Who-Is in `log` (cleared after): where its NPDU sends it, and
/// whether the link broadcast it.
fn the_who_is(log: &SendLog) -> (Option<NpduAddress>, bool) {
    let frames = who_is_frames(log);
    log.clear();
    let [frame] = frames.as_slice() else {
        panic!("one Who-Is, not {frames:?}");
    };
    (frame.decode_npdu().destination, frame.broadcast)
}

/// A discovery that names no scope sends one local broadcast, which no
/// router passes on: no destination network in its NPDU, so neither a
/// remote network nor every network (65535). It still finds the device on
/// this network.
#[tokio::test(start_paused = true)]
async fn a_discovery_with_no_scope_is_a_local_broadcast() {
    assert_eq!(Scope::default(), Scope::Local);
    assert_eq!(Discover::default(), Discover::local());
    assert_eq!(Discover::default().wait, DISCOVER_WAIT);
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let lab = server(bus.port(2), N, "A").await;
    let found = client
        .discover(Discover::default().wait(Duration::from_millis(500)))
        .await
        .unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].address, at(2));
    assert_eq!(the_who_is(&sent), (None, true));
    // Its range goes out as given.
    client
        .discover(Discover::local().range(N..=N).wait(Duration::ZERO))
        .await
        .unwrap();
    let frames = who_is_frames(&sent);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].decode_npdu().destination, None);
    stop(vec![lab]).await;
}

/// A named network and every network are explicit choices, and reach the
/// wire as such: a broadcast to that network's number, or to 65535. A
/// directed one is a unicast to the station, with no destination network.
#[tokio::test(start_paused = true)]
async fn an_explicit_scope_reaches_the_wire_as_named() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let quick = |request: Discover| request.wait(Duration::ZERO);
    let remote = |network| NpduAddress {
        network,
        mac_address: Default::default(),
    };
    client.discover(quick(Discover::network(5))).await.unwrap();
    assert_eq!(the_who_is(&sent), (Some(remote(5)), true));
    client.discover(quick(Discover::global())).await.unwrap();
    assert_eq!(the_who_is(&sent), (Some(remote(0xFFFF)), true));
    client
        .discover(quick(Discover::directed(at(2))))
        .await
        .unwrap();
    let frames = who_is_frames(&sent);
    assert_eq!(frames.len(), 1);
    assert!(!frames[0].broadcast);
    assert_eq!(frames[0].mac.as_slice(), mac(2));
    assert_eq!(frames[0].decode_npdu().destination, None);
}

/// A scope or range that can't be sent is refused before anything goes:
/// network 0 (this one is `Local`) and 65535 (every one is `Global`), a
/// directed address behind a router, and an empty range.
#[tokio::test(start_paused = true)]
async fn a_refused_scope_or_range_sends_nothing() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let routed = super::DeviceAddress::new(2, &[5]).unwrap();
    let (low, high) = (5, 3);
    for request in [
        Discover::network(0),
        Discover::network(0xFFFF),
        Discover::directed(routed),
        Discover::local().range(low..=high),
        Discover::local().range(0..=4_194_304),
    ] {
        let refused = client.discover(request.clone()).await;
        assert!(
            matches!(refused, Err(Error::OutOfRange(_) | Error::Range(_))),
            "{request:?}: {refused:?}"
        );
    }
    assert!(who_is_frames(&sent).is_empty(), "nothing was sent");
}

/// A front end on the low-level client sends through the same scope: the
/// default is a local broadcast there too.
#[tokio::test(start_paused = true)]
async fn the_low_level_send_takes_the_same_default() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    Scope::default()
        .who_is(client.low_level(), None)
        .await
        .unwrap();
    assert_eq!(the_who_is(&sent), (None, true));
    Scope::Global
        .who_is(client.low_level(), None)
        .await
        .unwrap();
    assert_eq!(
        the_who_is(&sent),
        (
            Some(NpduAddress {
                network: 0xFFFF,
                mac_address: Default::default()
            }),
            true
        )
    );
}
