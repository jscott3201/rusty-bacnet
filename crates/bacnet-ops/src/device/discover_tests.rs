//! Where a discovery's Who-Is goes (#2560): a local broadcast unless the
//! request names another scope, on the shared test transport's send log.
//! Behind a router, the routed lab (`routed_tests`) shows what each scope
//! reaches.

use std::time::Duration;

use bacnet_encoding::apdu::Apdu;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_transport::test_transport::{SendLog, SentFrame};
use bacnet_types::enums::UnconfirmedServiceChoice;

use super::test_bus::{Bus, N, at, client, mac, server, stop};
use crate::OpsError;
use crate::client::{DISCOVER_WAIT, Discover, Scope};

/// The Who-Is frames in `log`, broadcast or not.
pub(super) fn who_is_frames(log: &SendLog) -> Vec<SentFrame> {
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
pub(super) fn the_who_is(log: &SendLog) -> (Option<NpduAddress>, bool) {
    let frames = who_is_frames(log);
    log.clear();
    let [frame] = frames.as_slice() else {
        panic!("one Who-Is, not {frames:?}");
    };
    (frame.decode_npdu().destination, frame.broadcast)
}

/// A remote broadcast's destination: `network`, no MAC.
pub(super) fn remote(network: u16) -> Option<NpduAddress> {
    Some(NpduAddress {
        network,
        mac_address: Default::default(),
    })
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
    client.discover(quick(Discover::network(5))).await.unwrap();
    assert_eq!(the_who_is(&sent), (remote(5), true));
    client.discover(quick(Discover::global())).await.unwrap();
    assert_eq!(the_who_is(&sent), (remote(0xFFFF), true));
    client
        .discover(quick(Discover::directed(at(2).into())))
        .await
        .unwrap();
    let frames = who_is_frames(&sent);
    assert_eq!(frames.len(), 1);
    assert!(!frames[0].broadcast);
    assert_eq!(frames[0].mac.as_slice(), mac(2));
    assert_eq!(frames[0].decode_npdu().destination, None);
}

/// A scope or range that can't be sent is refused before anything goes,
/// as text to fix, with the scope to use instead: network 0 (this one is
/// local), 65535 (every one is global); an empty range or one past the highest
/// instance.
#[tokio::test(start_paused = true)]
async fn a_refused_scope_or_range_sends_nothing() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let (low, high) = (5, 3);
    for (request, what, says) in [
        (Discover::network(0), "network", "Discover::local()"),
        (Discover::network(0xFFFF), "network", "Discover::global()"),
        (Discover::local().range(low..=high), "range", "5"),
        (Discover::local().range(0..=4_194_304), "range", "4194303"),
    ] {
        let refused = client.discover(request.clone()).await.unwrap_err();
        let OpsError::Parse { what: given, .. } = &refused else {
            panic!("{request:?}: {refused:?}");
        };
        assert_eq!(*given, what, "{request:?}");
        assert!(refused.to_string().contains(says), "{request:?}: {refused}");
    }
    assert!(who_is_frames(&sent).is_empty(), "nothing was sent");
}

/// A front end on the low-level client sends through the same scope
/// (`raw::who_is`, no rate cap): the default is a local broadcast there
/// too.
#[tokio::test(start_paused = true)]
async fn the_low_level_send_takes_the_same_default() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    crate::raw::who_is(client.low_level(), &Scope::default(), None)
        .await
        .unwrap();
    assert_eq!(the_who_is(&sent), (None, true));
    crate::raw::who_is(client.low_level(), &Scope::Global, None)
        .await
        .unwrap();
    assert_eq!(the_who_is(&sent), (remote(0xFFFF), true));
    let refused = crate::raw::who_is(client.low_level(), &Scope::Network(0), None).await;
    assert!(
        matches!(refused, Err(OpsError::Parse { .. })),
        "{refused:?}"
    );
    assert!(who_is_frames(&sent).is_empty());
}

/// Only the scope asked answers a discovery: an I-Am heard during the wait
/// from somewhere the Who-Is didn't go (a local device announcing itself,
/// or answering another caller) is in the table but not in the result. A
/// discovery of a remote network lists no local device, a directed one
/// only its station, and a local or global one the local device.
#[tokio::test(start_paused = true)]
async fn an_i_am_from_outside_the_scope_is_no_answer() {
    let bus = Bus::new();
    let (client, _sent) = client(&bus).await;
    let wait = Duration::from_secs(1);
    let heard = |found: Vec<super::Discovered>| -> Vec<u32> {
        found.iter().map(|found| found.instance).collect()
    };
    for (request, expected) in [
        (Discover::network(7), vec![]),
        (Discover::directed(at(2).into()), vec![1002]),
        (Discover::directed(at(3).into()), vec![1003]),
        (Discover::local(), vec![1002, 1003]),
        (Discover::global(), vec![1002, 1003]),
    ] {
        let (found, ()) = tokio::join!(
            client.discover(request.clone().wait(wait)),
            announce_twice(&bus)
        );
        assert_eq!(heard(found.unwrap()), expected, "{request:?}");
    }
    // Both stay in the table, whichever discovery heard them.
    assert_eq!(client.devices().await.len(), 2);

    async fn announce_twice(bus: &std::sync::Arc<Bus>) {
        tokio::time::sleep(Duration::from_millis(100)).await;
        bus.i_am(2, 1002);
        bus.i_am(3, 1003);
    }
}

/// The network number becomes known after the directed request has gone
/// out. A local answer now identifies the station named by that network.
#[tokio::test(start_paused = true)]
async fn a_directed_scope_localizes_answers_when_the_network_is_learned_during_the_wait() {
    use super::test_bus::{client_on, settle};
    use bacnet_client::client::Station;

    let bus = Bus::new();
    let (client, sent) = client_on(bus.numbered_port(1)).await;
    let target = Station::remote(5, &mac(2));
    let request = Discover::directed(target).wait(Duration::from_secs(2));
    let (found, ()) = tokio::join!(client.discover(request), async {
        sent.wait_for_len(2).await; // What-Is-Network-Number and the Who-Is.
        bus.number_is(254, 5);
        settle().await;
        assert_eq!(client.low_level().network_number(), Some(5));
        bus.i_am(2, 1002);
        bus.i_am(3, 1002);
    });
    let found = found.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].address, at(2));
    assert!(found[0].duplicate);
}

/// The one rule for which answers a scope covers, a Who-Is's and a
/// Who-Has's alike: a network named by this client's own number takes the
/// local stations; a local scope takes no routed station, and takes one
/// reported with the client's own network number, which is local.
#[test]
fn a_scope_covers_stations_by_where_they_are() {
    let local = at(2);
    let routed = super::DeviceAddress::new(9, &[7]).unwrap();
    let own_routed = super::DeviceAddress::new(5, &mac(2)).unwrap();
    for (scope, own, address, covered) in [
        (Scope::Network(5), Some(5), &local, true),
        (Scope::Network(5), None, &local, false),
        (Scope::Network(5), Some(5), &own_routed, true),
        (Scope::Network(9), Some(5), &routed, true),
        (Scope::Network(9), Some(5), &local, false),
        (Scope::Local, Some(5), &routed, false),
        (Scope::Local, None, &routed, false),
        (Scope::Local, Some(5), &own_routed, true),
        (Scope::Local, None, &local, true),
        (
            Scope::Directed(local.clone().into()),
            Some(5),
            &own_routed,
            true,
        ),
        (Scope::Directed(local.clone().into()), None, &routed, false),
        (Scope::Global, None, &routed, true),
    ] {
        assert_eq!(
            scope.covers(address, own),
            covered,
            "{scope:?} {own:?} {address}"
        );
    }
}

/// A local broadcast the operating system refuses ("No route to host", as
/// from a loopback interface with the limited broadcast address) says where
/// it was going, the client's broadcast address, and keeps the system's
/// error with its code and kind beneath it. A link with no such address
/// reports the error as it came.
#[tokio::test(start_paused = true)]
async fn a_refused_local_broadcast_names_its_destination() {
    use std::net::{Ipv4Addr, SocketAddrV4};

    use bacnet_transport::test_transport::TestTransport;
    use bacnet_types::error::Error;

    use super::test_bus::client_on;
    // EHOSTUNREACH on macOS, where CI met it.
    let no_route = || async { Err(Error::Transport(std::io::Error::from_raw_os_error(65))) };
    let bip = TestTransport::builder()
        .local_mac(&mac(1))
        .on_send(move |frame| {
            let refused = frame.broadcast;
            async move { if refused { no_route().await } else { Ok(()) } }
        })
        .on_bip_broadcast_endpoint(|| Some(SocketAddrV4::new(Ipv4Addr::BROADCAST, 47808)))
        .build();
    let (client, _) = client_on(bip).await;
    let quick = Discover::local().wait(Duration::ZERO);
    for refused in [
        client.discover(quick.clone()).await.unwrap_err(),
        client.device(4000).await.map(drop).unwrap_err(),
        client
            .find(crate::Find::name("Zone").wait(Duration::ZERO))
            .await
            .map(drop)
            .unwrap_err(),
    ] {
        let text = refused.to_string();
        assert!(text.contains("255.255.255.255:47808"), "{text}");
        assert!(text.contains("broadcast address"), "{text}");
        let OpsError::Client(Error::Transport(io)) = &refused else {
            panic!("{refused:?}");
        };
        let inner = io.get_ref().and_then(|wrapped| wrapped.source());
        let system = inner.and_then(|source| source.downcast_ref::<std::io::Error>());
        assert_eq!(system.and_then(std::io::Error::raw_os_error), Some(65));
        assert_eq!(Some(io.kind()), system.map(std::io::Error::kind));
    }
    let plain = TestTransport::builder()
        .local_mac(&mac(1))
        .on_send(move |_| no_route())
        .build();
    let (client, _) = client_on(plain).await;
    let refused = client.discover(quick).await.unwrap_err();
    assert!(
        !refused.to_string().contains("broadcast address"),
        "{refused}"
    );
}
