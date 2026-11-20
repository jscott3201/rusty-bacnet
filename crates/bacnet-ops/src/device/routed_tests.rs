//! Routed stations behind an in-process router (#2560): a station named by
//! its network and MAC is reached there directly, even while its instance
//! is claimed on another network too; handles following an instance on one
//! network stay there; and an address on the client's own network is the
//! local station.

use std::sync::Arc;
use std::time::Duration;

use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_server::server::BACnetServer;
use bacnet_transport::test_transport::{SendLog, TestTransport};

use super::test_bus::{Bus, N, client_on, mac, name_of, server, settle, stop};
use super::*;
use crate::client::{Discover, Find, ResolveScope, Scope};

/// The client at station 1 of network 1, a router at station 254 of
/// networks 1, 2 and 3, and on network 2 station 5 and on network 3
/// station 6, both claiming N.
struct Routed {
    near: Arc<Bus>,
    three: Arc<Bus>,
    router: BACnetRouter,
    servers: Vec<BACnetServer<TestTransport>>,
    client: Client<TestTransport>,
    sent: SendLog,
}

impl Routed {
    /// The lab, its client's port made by `port` on the near network.
    async fn new(port: fn(&Arc<Bus>) -> TestTransport) -> Self {
        let (near, two, three) = (Bus::new(), Bus::new(), Bus::new());
        let ports = [(&near, 1), (&two, 2), (&three, 3)]
            .into_iter()
            .map(|(bus, network_number)| RouterPort {
                transport: bus.port(254),
                network_number,
            })
            .collect();
        let StartedRouter { router, .. } = BACnetRouter::start(ports, RouterOptions::new())
            .await
            .unwrap();
        let servers = vec![
            server(two.port(5), N, "on two").await,
            server(three.port(6), N, "on three").await,
        ];
        let (client, sent) = client_on(port(&near)).await;
        Self {
            near,
            three,
            router,
            servers,
            client,
            sent,
        }
    }

    /// Discover every network.
    async fn discover(&self) -> Vec<Discovered> {
        self.client
            .discover(Discover::global().wait(Duration::from_millis(500)))
            .await
            .unwrap()
    }

    async fn stop(mut self) {
        stop(std::mem::take(&mut self.servers)).await;
        self.router.stop().await.unwrap();
    }
}

/// Station 5 on network 2 and station 6 on network 3 both claim N; the
/// instance alone is refused, `2:0x..` reaches station 5 through the
/// router, and `3:N` follows the one claim on network 3.
#[tokio::test(start_paused = true)]
async fn a_routed_station_is_reached_by_its_address() {
    let lab = Routed::new(|near| near.port(1)).await;
    let client = &lab.client;
    assert_eq!(lab.discover().await.len(), 2);
    assert!(matches!(
        client.device(N).await,
        Err(OpsError::Ambiguous { instance: N, .. })
    ));
    let station = client.device("2:0x0A000005BAC0").await.unwrap();
    assert_eq!(station.address(), DeviceAddress::new(2, &mac(5)).unwrap());
    assert!(!station.follows());
    assert_eq!(name_of(&station).await.unwrap(), "on two");
    let following = client.device("3:1001").await.unwrap();
    assert!(following.follows());
    assert_eq!(name_of(&following).await.unwrap(), "on three");
    lab.stop().await;
}

/// A discovery that names no scope stays on the client's network, which
/// the router doesn't pass a local broadcast from: neither routed device
/// answers it. Network 2 named reaches station 5 alone, and every network
/// reaches both.
#[tokio::test(start_paused = true)]
async fn the_default_discovery_stays_on_this_network() {
    let lab = Routed::new(|near| near.port(1)).await;
    let quick = |request: Discover| request.wait(Duration::from_millis(500));
    let near = lab.client.discover(quick(Discover::default())).await;
    assert!(near.unwrap().is_empty(), "a routed device answered");
    let two = lab.client.discover(quick(Discover::network(2))).await;
    let two: Vec<_> = two.unwrap().into_iter().map(|f| f.address).collect();
    assert_eq!(two, [DeviceAddress::new(2, &mac(5)).unwrap()]);
    let every = lab.client.discover(quick(Discover::global())).await;
    assert_eq!(every.unwrap().len(), 2);
    lab.stop().await;
}

/// Cold, learned and explicit next hops carry the same station address.
#[tokio::test(start_paused = true)]
async fn directed_who_is_reaches_only_the_named_remote_station() {
    use bacnet_client::client::Station;
    use bacnet_encoding::apdu::Apdu;
    use bacnet_services::who_is::WhoIsRequest;

    let lab = Routed::new(|near| near.port(1)).await;
    let station = DeviceAddress::new(2, &mac(5)).unwrap();
    for (to, broadcast) in [
        (Station::remote(2, &mac(5)), true),
        (Station::remote(2, &mac(5)), false),
        (Station::routed(&mac(254), 2, &mac(5)), false),
    ] {
        lab.sent.clear();
        let request = Discover::directed(to)
            .range(N..=N)
            .wait(Duration::from_millis(500));
        let (found, ()) = tokio::join!(lab.client.discover(request), async {
            settle().await;
            lab.near.i_am_from(254, Some((3, &mac(6))), N);
        });
        let found = found.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].address, station);
        let frames = super::discover_tests::who_is_frames(&lab.sent);
        let [frame] = frames.as_slice() else {
            panic!("one Who-Is: {frames:?}")
        };
        assert_eq!(frame.broadcast, broadcast);
        if !broadcast {
            assert_eq!(frame.mac.as_slice(), mac(254));
        }
        let to = frame.decode_npdu().destination.unwrap();
        assert_eq!(to.network, 2);
        assert_eq!(to.mac_address.as_slice(), mac(5));
        let Apdu::UnconfirmedRequest(apdu) = frame.apdu() else {
            panic!("Who-Is")
        };
        let range = WhoIsRequest::decode(&apdu.service_request)
            .unwrap()
            .range
            .unwrap();
        assert!(range.contains(N) && !range.contains(N - 1) && !range.contains(N + 1));
    }
    // Unrelated I-Ams still enter the client's inventory.
    assert!(
        lab.client
            .low_level()
            .get_device_at(3, &mac(6))
            .await
            .is_some()
    );
    lab.stop().await;
}

/// A bad explicit next hop fails before the first network-number question.
#[tokio::test(start_paused = true)]
async fn directed_who_is_validates_the_router_before_preflight() {
    use bacnet_client::client::Station;
    let port = TestTransport::builder()
        .local_mac(&mac(1))
        .number_controls()
        .data_link(bacnet_types::data_link::DataLink::Bip)
        .build();
    let (client, sent) = client_on(port).await;
    for router in [&[][..], &[255, 255, 255, 255, 0xba, 0xc0]] {
        let target = Station::routed(router, 2, &mac(5));
        let error = client
            .low_level()
            .who_is_directed(&target, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unicast router"), "{error}");
        assert!(sent.is_empty());
    }
}

/// A search that names no scope stays on this network as a discovery does
/// (#2596): neither routed device hears its Who-Has, so it comes back
/// empty, though one has the object. Network 2 named finds station 5's,
/// and every network finds the object both have, each at its station on
/// its network.
#[tokio::test(start_paused = true)]
async fn the_default_search_stays_on_this_network() {
    let lab = Routed::new(|near| near.port(1)).await;
    let find = |request: Find| lab.client.find(request.wait(Duration::from_millis(500)));
    let near = find(Find::name("on two")).await.unwrap();
    assert!(near.is_empty(), "a routed device answered: {near:?}");
    let two: Vec<_> = find(Find::name("on two").within(Scope::Network(2)))
        .await
        .unwrap()
        .into_iter()
        .map(|found| (found.device, found.address))
        .collect();
    assert_eq!(two, [(N, DeviceAddress::new(2, &mac(5)).unwrap())]);
    // By identifier, which both devices have.
    let ai = bacnet_types::primitives::ObjectIdentifier::new(
        bacnet_types::enums::ObjectType::ANALOG_INPUT,
        1,
    )
    .unwrap();
    let every = find(Find::object(ai).within(Scope::Global)).await;
    let mut every: Vec<_> = every.unwrap().into_iter().map(|f| f.address).collect();
    every.sort();
    let stations = [(2, mac(5)), (3, mac(6))].map(|(n, m)| DeviceAddress::new(n, &m).unwrap());
    assert_eq!(every, stations);
    lab.stop().await;
}

/// Once the client knows its network's number, a station's I-Have heard
/// both locally and as a router's copy from that network (SNET 1, SADR the
/// station) is one answer at the local station, not two (#2596).
#[tokio::test(start_paused = true)]
async fn a_routed_copy_of_a_local_answer_is_one_answer() {
    let lab = Routed::new(|near| near.numbered_port(1)).await;
    let low = lab.client.low_level();
    low.what_is_network_number(None).await.unwrap();
    for _ in 0..50 {
        if low.network_number().is_some() {
            break;
        }
        settle().await;
    }
    assert_eq!(low.network_number(), Some(1), "the router said");
    let search = Find::name("near").wait(Duration::from_millis(500));
    let (found, ()) = tokio::join!(lab.client.find(search), async {
        settle().await;
        lab.near.i_have(2, None, 2002, "near");
        lab.near.i_have(254, Some((1, &mac(2))), 2002, "near");
    });
    let found: Vec<_> = found.unwrap().into_iter().map(|f| f.address).collect();
    assert_eq!(found, [DeviceAddress::local(&mac(2)).unwrap()]);
    lab.stop().await;
}

/// Handles following one instance on two networks stay apart: making the
/// second moves nothing, and each reads its own network's device (the
/// reviewer's probe A).
#[tokio::test(start_paused = true)]
async fn following_handles_on_two_networks_stay_apart() {
    let lab = Routed::new(|near| near.port(1)).await;
    let client = &lab.client;
    lab.discover().await;
    let mut events = client.events();
    let three = client.device("3:1001").await.unwrap();
    assert_eq!(name_of(&three).await.unwrap(), "on three");
    let two = client.device("2:1001").await.unwrap();
    assert_eq!(name_of(&two).await.unwrap(), "on two");
    assert!(events.try_recv().is_err(), "nothing moved");
    assert_eq!(three.address().network(), 3);
    assert_eq!(name_of(&three).await.unwrap(), "on three");
    lab.stop().await;
}

/// A routed following handle whose row was purged, while its instance
/// came back at another station on its network, finds it there: no route
/// is reason to look again, as a timeout is (the reviewer's probe C).
#[tokio::test(start_paused = true)]
async fn a_routed_following_handle_finds_its_device_after_a_purge() {
    let mut lab = Routed::new(|near| near.port(1)).await;
    lab.discover().await;
    let following = lab.client.device("3:1001").await.unwrap();
    assert_eq!(name_of(&following).await.unwrap(), "on three");
    // Station 6 leaves; station 7 on network 3 holds N now.
    lab.three.silence(6);
    lab.servers
        .push(server(lab.three.port(7), N, "new on three").await);
    lab.client.low_level().clear_devices().await;
    lab.discover().await;
    assert_eq!(name_of(&following).await.unwrap(), "new on three");
    assert_eq!(following.address(), DeviceAddress::new(3, &mac(7)).unwrap());
    lab.stop().await;
}

/// Once the client knows its own network's number, an address written
/// with it is the local station: no Who-Is goes to find it, no routed row
/// appears beside the local one, and the two names share one gate (the
/// reviewer's probe H).
#[tokio::test(start_paused = true)]
async fn an_address_on_the_own_network_is_local() {
    let mut lab = Routed::new(|near| near.numbered_port(1)).await;
    lab.servers
        .push(server(lab.near.port(2), 2002, "near").await);
    let low = lab.client.low_level();
    low.what_is_network_number(None).await.unwrap();
    for _ in 0..50 {
        if low.network_number().is_some() {
            break;
        }
        settle().await;
    }
    assert_eq!(low.network_number(), Some(1), "the router said");
    lab.discover().await;
    let local = lab.client.device("10.0.0.2:47808").await.unwrap();
    let before = lab.sent.len();
    let aliased = lab.client.device("1:0x0A000002BAC0").await.unwrap();
    assert_eq!(lab.sent.len(), before, "nothing sent to find it");
    assert_eq!(aliased.address(), local.address());
    assert_eq!(aliased.instance(), Some(2002));
    assert_eq!(name_of(&aliased).await.unwrap(), "near");
    let rows = lab.client.devices().await;
    let claims = rows
        .iter()
        .filter(|row| row.object_identifier.instance_number() == 2002)
        .count();
    assert_eq!(claims, 1, "no routed row beside the local one: {rows:?}");
    assert_eq!(
        lab.client.device(2002).await.unwrap().address(),
        local.address()
    );
    let held = lab.client.limits().gate(&local.address()).acquire_owned();
    let held = held.await.unwrap();
    let read = tokio::time::timeout(Duration::from_secs(10), name_of(&aliased)).await;
    assert!(read.is_err(), "the aliased name went past the local gate");
    drop(held);
    // The routed instance form on the own network is local too.
    let routed = lab.client.device("1:2002").await.unwrap();
    assert_eq!(routed.address(), local.address());
    lab.stop().await;
}

/// Handles named with the own network's number before the client learns
/// it reach the local station once it does, at its gate, and the instance
/// alone finds that one station (the reviewer's probe P5').
#[tokio::test(start_paused = true)]
async fn handles_named_before_the_number_is_learned_are_local_after() {
    use bacnet_client::discovery::RoutedDeviceConfig;
    use bacnet_types::enums::Segmentation;

    let mut lab = Routed::new(|near| near.numbered_port(1)).await;
    lab.servers
        .push(server(lab.near.port(2), 2002, "near").await);
    let low = Arc::clone(lab.client.low_level());
    low.add_routed_device(RoutedDeviceConfig {
        instance: 2002,
        router_mac: mac(254).to_vec(),
        remote_network: 1,
        remote_mac: mac(2).to_vec(),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        max_segments_accepted: None,
    })
    .await
    .unwrap();
    assert_eq!(low.network_number(), None, "the premise: not learned yet");
    let pinned = lab.client.device("1:0x0A000002BAC0").await.unwrap();
    let following = lab.client.device("1:2002").await.unwrap();
    let routed = DeviceAddress::new(1, &mac(2)).unwrap();
    assert_eq!(pinned.address(), routed, "the number isn't known yet");
    assert_eq!(following.address(), routed);
    low.what_is_network_number(None).await.unwrap();
    for _ in 0..50 {
        if low.network_number().is_some() {
            break;
        }
        settle().await;
    }
    assert_eq!(low.network_number(), Some(1), "the router said");
    let local = lab.client.device("10.0.0.2:47808").await.unwrap();
    assert_eq!(pinned.address(), local.address());
    assert_eq!(following.address(), local.address());
    let by_instance = lab.client.device(2002).await.unwrap();
    assert_eq!(by_instance.address(), local.address());
    let held = lab.client.limits().gate(&local.address()).acquire_owned();
    let held = held.await.unwrap();
    for handle in [&pinned, &following] {
        let read = tokio::time::timeout(Duration::from_secs(10), name_of(handle)).await;
        assert!(read.is_err(), "{handle:?} went past the local gate");
    }
    drop(held);
    assert_eq!(name_of(&pinned).await.unwrap(), "near");
    assert_eq!(name_of(&following).await.unwrap(), "near");
    lab.stop().await;
}

/// A directed Who-Is to an address written in routed form on the client's
/// own network, once it knows the number, goes to that station as a local
/// unicast: no destination network, and nothing through the router.
#[tokio::test(start_paused = true)]
async fn a_directed_who_is_on_the_own_network_is_a_local_unicast() {
    let lab = Routed::new(|near| near.numbered_port(1)).await;
    let low = lab.client.low_level();
    low.what_is_network_number(None).await.unwrap();
    for _ in 0..50 {
        if low.network_number().is_some() {
            break;
        }
        settle().await;
    }
    assert_eq!(low.network_number(), Some(1), "the router said");
    lab.sent.clear();
    let station = DeviceAddress::new(1, &mac(9)).unwrap();
    lab.client
        .discover(Discover::directed(station.into()).wait(Duration::ZERO))
        .await
        .unwrap();
    let frames = super::discover_tests::who_is_frames(&lab.sent);
    let [frame] = frames.as_slice() else {
        panic!("one Who-Is, not {frames:?}");
    };
    assert!(!frame.broadcast);
    assert_eq!(frame.mac.as_slice(), mac(9));
    assert_eq!(frame.decode_npdu().destination, None);
    lab.stop().await;
}

/// A following handle on a device behind the router, made from its bare
/// instance on a client resolving on every network, asks for it again on
/// that device's network after a timeout, never on every network.
#[tokio::test(start_paused = true)]
async fn a_routed_device_is_asked_for_again_on_its_network() {
    let mut lab = Routed::new(|near| near.port(1)).await;
    lab.servers
        .push(server(lab.three.port(7), 2002, "on three").await);
    let wide = lab.client.with_resolve_scope(ResolveScope::Global);
    let device = wide.device(2002).await.unwrap();
    assert_eq!(device.address().network(), 3);
    assert_eq!(name_of(&device).await.unwrap(), "on three");
    lab.sent.clear();
    lab.three.silence(7);
    let error = name_of(&device).await.unwrap_err();
    assert!(error.timed_out(), "{error}");
    assert_eq!(
        super::discover_tests::the_who_is(&lab.sent),
        (super::discover_tests::remote(3), true)
    );
    lab.stop().await;
}

/// A target naming the client's own network keeps its text and says that
/// network was asked, with no hint to widen the resolve scope: only a bare
/// instance gets that. Both go out as local broadcasts.
#[tokio::test(start_paused = true)]
async fn a_miss_on_the_own_network_keeps_the_target_as_written() {
    let lab = Routed::new(|near| near.numbered_port(1)).await;
    let low = lab.client.low_level();
    low.what_is_network_number(None).await.unwrap();
    for _ in 0..50 {
        if low.network_number().is_some() {
            break;
        }
        settle().await;
    }
    assert_eq!(low.network_number(), Some(1), "the router said");
    lab.sent.clear();
    let named = lab.client.device("1:4242").await.unwrap_err();
    assert!(
        matches!(
            &named,
            crate::OpsError::UnknownDevice { target, asked: Some(crate::Scope::Network(1)) }
                if target == "1:4242"
        ),
        "{named:?}"
    );
    assert!(!named.to_string().contains("resolve scope"), "{named}");
    assert_eq!(super::discover_tests::the_who_is(&lab.sent), (None, true));
    let bare = lab.client.device(4242).await.unwrap_err();
    assert!(bare.to_string().contains("resolve scope"), "{bare}");
    assert_eq!(super::discover_tests::the_who_is(&lab.sent), (None, true));
    lab.stop().await;
}

/// A bare instance whose one row in the table is behind the router follows
/// it there, on the default local resolve scope, sending no Who-Is: the
/// table is asked before the network.
#[tokio::test(start_paused = true)]
async fn a_bare_instance_with_a_remote_row_needs_no_who_is() {
    let mut lab = Routed::new(|near| near.port(1)).await;
    lab.servers
        .push(server(lab.three.port(7), 2002, "on three").await);
    lab.discover().await;
    lab.sent.clear();
    assert_eq!(lab.client.resolve_scope(), ResolveScope::Local);
    let device = lab.client.device(2002).await.unwrap();
    assert_eq!(device.address(), DeviceAddress::new(3, &mac(7)).unwrap());
    assert!(device.follows());
    assert!(
        super::discover_tests::who_is_frames(&lab.sent).is_empty(),
        "no Who-Is"
    );
    lab.stop().await;
}
