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
use super::test_bus::{Bus, N, at, client, mac, server, stop};
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
        .find(quick(Scope::Directed(at(2))).wait(Duration::from_millis(500)))
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
            find().within(Scope::Directed(routed)),
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
