//! Where a search's Who-Has goes (#2596): a local broadcast unless the
//! request names another scope, on the shared test transport's send log,
//! and what the I-Haves that answer it say. Behind a router, the routed lab
//! (`routed_tests`) shows what each scope reaches.

use std::time::Duration;

use bacnet_encoding::apdu::Apdu;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_services::who_has::WhoHasObject;
use bacnet_transport::test_transport::{SendLog, SentFrame};
use bacnet_types::enums::{ObjectType, UnconfirmedServiceChoice};
use bacnet_types::primitives::ObjectIdentifier;

use super::discover_tests::remote;
use super::test_bus::{Bus, N, at, client, client_on, mac, server, settle, stop};
use crate::OpsError;
use crate::client::{FIND_WAIT, Find, FoundObject, Scope};

/// The Who-Has frames in `log`, broadcast or not.
fn who_has_frames(log: &SendLog) -> Vec<SentFrame> {
    log.frames()
        .into_iter()
        .filter(|frame| {
            matches!(frame.apdu(), Apdu::UnconfirmedRequest(request)
                if request.service_choice == UnconfirmedServiceChoice::WHO_HAS)
        })
        .collect()
}

/// The one Who-Has in `log` (cleared after): where its NPDU sends it, and
/// whether the link broadcast it.
pub(super) fn the_who_has(log: &SendLog) -> (Option<NpduAddress>, bool) {
    let frames = who_has_frames(log);
    log.clear();
    let [frame] = frames.as_slice() else {
        panic!("one Who-Has, not {frames:?}");
    };
    (frame.decode_npdu().destination, frame.broadcast)
}

fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

/// What the test bus's device at station 2 answers for its analog input.
fn at_two(name: &str) -> FoundObject {
    FoundObject {
        device: N,
        object: ai1(),
        name: name.into(),
        address: at(2),
    }
}

/// A search that names no scope sends one local broadcast, which no
/// router passes on: no destination network in its NPDU, so neither a
/// remote network nor every network (65535). It still finds the object on
/// this network, by name or by identifier.
#[tokio::test(start_paused = true)]
async fn a_search_with_no_scope_is_a_local_broadcast() {
    let quick = |request: Find| request.wait(Duration::from_millis(500));
    assert_eq!(Find::name("A").scope(), &Scope::Local);
    assert_eq!(Find::name("A"), Find::name("A").within(Scope::Local));
    assert_eq!(Find::name("A"), Find::name("A").wait(FIND_WAIT));
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let lab = server(bus.port(2), N, "A").await;
    let found = client.find(quick(Find::name("A"))).await.unwrap();
    assert_eq!(found, [at_two("A")]);
    assert_eq!(the_who_has(&sent), (None, true));
    let found = client.find(quick(Find::object(ai1()))).await.unwrap();
    assert_eq!(found, [at_two("A")]);
    assert_eq!(the_who_has(&sent), (None, true));
    // Nothing has this name: an empty list once the wait is over.
    let missed = client.find(quick(Find::name("B"))).await.unwrap();
    assert!(missed.is_empty(), "{missed:?}");
    // Its range goes out as given, and a device outside it doesn't answer.
    let outside = client
        .find(quick(Find::name("A").range(1..=2)))
        .await
        .unwrap();
    assert!(outside.is_empty(), "{outside:?}");
    let frames = who_has_frames(&sent);
    assert_eq!(frames.len(), 2);
    assert!(frames.iter().all(|f| f.decode_npdu().destination.is_none()));
    stop(vec![lab]).await;
}

/// A named network and every network are explicit choices, and reach the
/// wire as such: a broadcast to that network's number, or to 65535. A
/// directed one is a unicast to the station, with no destination network,
/// and its device answers it.
#[tokio::test(start_paused = true)]
async fn an_explicit_scope_reaches_the_wire_as_named() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let lab = server(bus.port(2), N, "A").await;
    let quick = |scope: Scope| Find::name("A").within(scope).wait(Duration::ZERO);
    client.find(quick(Scope::Network(5))).await.unwrap();
    assert_eq!(the_who_has(&sent), (remote(5), true));
    client.find(quick(Scope::Global)).await.unwrap();
    assert_eq!(the_who_has(&sent), (remote(0xFFFF), true));
    let found = client
        .find(quick(Scope::Directed(at(2).into())).wait(Duration::from_millis(500)))
        .await
        .unwrap();
    assert_eq!(found, [at_two("A")]);
    let frames = who_has_frames(&sent);
    assert_eq!(frames.len(), 1);
    assert!(!frames[0].broadcast);
    assert_eq!(frames[0].mac.as_slice(), mac(2));
    assert_eq!(frames[0].decode_npdu().destination, None);
    stop(vec![lab]).await;
}

/// A scope or range that can't be sent is refused before anything goes,
/// as text to fix, naming the scope to use instead: network 0 (this one is
/// local), 65535 (every one is global) and a directed address behind a
/// router (its network's scope); an empty range or one past the highest
/// instance.
#[tokio::test(start_paused = true)]
async fn a_refused_scope_or_range_sends_nothing() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let routed = super::DeviceAddress::new(2, &[5]).unwrap();
    let (low, high) = (5, 3);
    let find = || Find::name("A");
    for (request, what, says) in [
        (
            find().within(Scope::Network(0)),
            "network",
            "Find::within(Scope::Local)",
        ),
        (
            find().within(Scope::Network(0xFFFF)),
            "network",
            "Find::within(Scope::Global)",
        ),
        (
            find().within(Scope::Directed(routed.into())),
            "address",
            "a directed Who-Has goes to one station on this network: \
             Find::within(Scope::Network(2))",
        ),
        (find().range(low..=high), "range", "5"),
        (find().range(0..=4_194_304), "range", "4194303"),
    ] {
        let refused = client.find(request.clone()).await.unwrap_err();
        let OpsError::Parse { what: given, .. } = &refused else {
            panic!("{request:?}: {refused:?}");
        };
        assert_eq!(*given, what, "{request:?}");
        assert!(refused.to_string().contains(says), "{request:?}: {refused}");
    }
    assert!(who_has_frames(&sent).is_empty(), "nothing was sent");
}

/// A front end on the low-level client sends through the same scope
/// (`raw::who_has`, `raw::who_has_replies`, no rate cap): the default is a
/// local broadcast there too, and the replies are the device's I-Have.
#[tokio::test(start_paused = true)]
async fn the_low_level_sends_take_the_same_default() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let lab = server(bus.port(2), N, "A").await;
    let low = client.low_level();
    let name = WhoHasObject::Name("A".into());
    crate::raw::who_has(low, &Scope::default(), &name, None)
        .await
        .unwrap();
    assert_eq!(the_who_has(&sent), (None, true));
    crate::raw::who_has(low, &Scope::Global, &name, None)
        .await
        .unwrap();
    assert_eq!(the_who_has(&sent), (remote(0xFFFF), true));
    let wait = Duration::from_millis(500);
    let replies = crate::raw::who_has_replies(low, &Scope::default(), &name, None, wait)
        .await
        .unwrap();
    assert_eq!(the_who_has(&sent), (None, true));
    let [reply] = replies.as_slice() else {
        panic!("one I-Have, not {replies:?}");
    };
    assert_eq!(reply.i_have.object_identifier, ai1());
    assert_eq!(reply.source_mac.as_slice(), mac(2));
    for refused in [
        crate::raw::who_has(low, &Scope::Network(0), &name, None).await,
        crate::raw::who_has_replies(low, &Scope::Network(0xFFFF), &name, None, wait)
            .await
            .map(drop),
    ] {
        assert!(
            matches!(refused, Err(OpsError::Parse { .. })),
            "{refused:?}"
        );
    }
    assert!(who_has_frames(&sent).is_empty());
    stop(vec![lab]).await;
}

/// The instance of the device at station `n` in the scope tests.
fn device_at(n: u8) -> u32 {
    3000 + u32::from(n)
}

/// An I-Have for "A" from a station, routed (SNET, SADR) or not.
type Heard = (u8, Option<(u16, [u8; 1])>);

/// What `client` finds of the object named "A" within `scope` while
/// `bus` hears `heard`, sent as the search listens.
async fn found_within(
    client: &crate::Client<bacnet_transport::test_transport::TestTransport>,
    bus: &Bus,
    scope: Scope,
    heard: &[Heard],
) -> Vec<(u32, super::DeviceAddress)> {
    let search = Find::name("A")
        .within(scope)
        .wait(Duration::from_millis(500));
    let (found, ()) = tokio::join!(client.find(search), async {
        super::test_bus::settle().await;
        for (n, routed) in heard {
            let routed = routed.as_ref().map(|(network, sadr)| (*network, &sadr[..]));
            bus.i_have(*n, routed, device_at(*n), "A");
        }
    });
    let mut found: Vec<_> = found
        .unwrap()
        .into_iter()
        .map(|f| (f.device, f.address))
        .collect();
    found.sort();
    found
}

/// Only an answer from the station a directed search asked is a result
/// (#2596): station 3's I-Have, heard during a directed search of station 2,
/// unasked or another's answer, is not listed.
#[tokio::test(start_paused = true)]
async fn a_directed_search_lists_only_its_station() {
    let bus = Bus::new();
    let (client, _sent) = client(&bus).await;
    let found = found_within(
        &client,
        &bus,
        Scope::Directed(at(2).into()),
        &[(3, None), (2, None)],
    )
    .await;
    assert_eq!(found, [(device_at(2), at(2))]);
}

/// A local search lists the answers from this network alone: an I-Have a
/// router passed on from network 5 during it is not a result.
#[tokio::test(start_paused = true)]
async fn a_local_search_lists_only_this_network() {
    let bus = Bus::new();
    let (client, _sent) = client(&bus).await;
    let found = found_within(
        &client,
        &bus,
        Scope::Local,
        &[(9, Some((5, [7]))), (2, None)],
    )
    .await;
    assert_eq!(found, [(device_at(2), at(2))]);
}

/// A search of network 5 lists the answers from network 5 alone: not a
/// local station's I-Have, and not one a router passed on from network 6.
#[tokio::test(start_paused = true)]
async fn a_network_search_lists_only_that_network() {
    let bus = Bus::new();
    let (client, _sent) = client(&bus).await;
    let heard = [(2, None), (9, Some((6, [8]))), (9, Some((5, [7])))];
    let found = found_within(&client, &bus, Scope::Network(5), &heard).await;
    let station = super::DeviceAddress::new(5, &[7]).unwrap();
    assert_eq!(found, [(device_at(9), station)]);
    // Every network takes them all.
    let found = found_within(&client, &bus, Scope::Global, &heard).await;
    assert_eq!(found.len(), 3, "{found:?}");
}

/// What a search within `scope` finds of the object named "near" when a
/// router's copy of station 2's answer (SNET 1) comes before the client
/// learns that this network is 1, and the station's own answer after:
/// through [`Client::find`](crate::Client::find), the stations it lists;
/// with `raw`, through `raw::who_has_replies`, as many stations, one per
/// answer it returns.
async fn learned_mid_wait(scope: Scope, raw: bool) -> Vec<super::DeviceAddress> {
    let bus = Bus::new();
    let (client, _sent) = client_on(bus.numbered_port(1)).await;
    let low = client.low_level();
    let wait = Duration::from_millis(2000);
    let search = async {
        if raw {
            let name = WhoHasObject::Name("near".into());
            let heard = crate::raw::who_has_replies(low, &scope, &name, None, wait).await;
            return heard.unwrap().iter().map(|_| at(2)).collect();
        }
        let found = client.find(Find::name("near").within(scope.clone()).wait(wait));
        found
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.address)
            .collect()
    };
    let (found, ()) = tokio::join!(search, async {
        settle().await;
        assert_eq!(low.network_number(), None, "the premise: not learned yet");
        bus.i_have(254, Some((1, &mac(2))), 2002, "near");
        settle().await;
        bus.number_is(254, 1);
        settle().await;
        assert_eq!(low.network_number(), Some(1), "the router said");
        bus.i_have(2, None, 2002, "near");
    });
    found
}

/// The two copies are one answer at the local station, whether the
/// search asked every network or network 1 by its number, and through the
/// low-level collector too (#2596): the list is folded on the number known
/// at the end, not only as each answer came.
#[tokio::test(start_paused = true)]
async fn a_number_learned_mid_wait_folds_the_routed_copy() {
    for scope in [Scope::Global, Scope::Network(1)] {
        for raw in [false, true] {
            let found = learned_mid_wait(scope.clone(), raw).await;
            assert_eq!(found, [at(2)], "{scope:?}, raw {raw}");
        }
    }
}
