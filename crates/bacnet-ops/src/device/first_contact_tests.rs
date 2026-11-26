//! A handle made from an instance is placed only where a device confirmed
//! the instance to the client's own question (#2824), reads and writes
//! alike. The threat: anyone on the link can send an I-Am for any instance
//! from a station of their own, and run a responder there that answers as
//! any device; nobody can send from another station's address. Device N
//! answers at station 2; station 3 is the other station.

use std::sync::Arc;

use bacnet_server::server::BACnetServer;
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::enums::ConfirmedServiceChoice;

use super::test_bus::{Bus, N, TurnAway, at, client, mac, name_of, requests, server, settle, stop};
use super::*;

const WRITE: ConfirmedServiceChoice = ConfirmedServiceChoice::WRITE_PROPERTY;
const READ: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;

/// Device N at station 2, which hasn't announced itself; with `responder`,
/// a device answering as N at station 3.
async fn lab(
    responder: bool,
) -> (
    Arc<Bus>,
    Client<TestTransport>,
    SendLog,
    Vec<BACnetServer<TestTransport>>,
) {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let mut servers = vec![server(bus.port(2), N, "A").await];
    if responder {
        servers.push(server(bus.port(3), N, "impostor").await);
    }
    settle().await;
    (bus, client, sent, servers)
}

/// Writes sent anywhere.
fn writes(sent: &SendLog) -> usize {
    sent.unicasts()
        .iter()
        .filter(|frame| {
            matches!(frame.apdu(), bacnet_encoding::apdu::Apdu::ConfirmedRequest(request)
                if request.service_choice == WRITE)
        })
        .count()
}

/// An I-Am for N from station 3, where nothing answers: at first sight,
/// and after station 2's own row aged out. A handle by instance, named
/// either way, fails as not confirmed at station 3 after one question
/// there; nothing is read or written anywhere.
#[tokio::test(start_paused = true)]
async fn a_forged_claim_takes_no_first_request() {
    for aged in [false, true] {
        for text in [false, true] {
            let (bus, client, sent, servers) = lab(false).await;
            if aged {
                bus.i_am(2, N);
                settle().await;
                client.low_level().clear_devices().await;
            }
            bus.i_am(3, N);
            settle().await;
            let made = match text {
                true => client.device("1001").await,
                false => client.device(N).await,
            };
            let error = made.unwrap_err();
            assert!(
                matches!(
                    &error,
                    OpsError::NotConfirmed { instance: N, at: here, candidate: None }
                        if *here == at(3)
                ),
                "aged {aged}, text {text}: {error:?}"
            );
            assert_eq!(requests(&sent, READ, 3), 1, "the question alone");
            assert_eq!(requests(&sent, READ, 2) + writes(&sent), 0);
            stop(servers).await;
        }
    }
}

/// The residual: a responder at station 3 answers the question as N while
/// the device at station 2 has never announced itself. Nothing tells the
/// two apart, so the handle goes there; the question is the one request
/// more.
#[tokio::test(start_paused = true)]
async fn a_station_answering_as_an_unheard_device_is_placed() {
    let (bus, client, sent, servers) = lab(true).await;
    bus.i_am(3, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    assert_eq!(device.address(), at(3));
    assert_eq!(requests(&sent, READ, 3), 1, "the question");
    assert_eq!(name_of(&device).await.unwrap(), "impostor");
    assert_eq!(requests(&sent, READ, 2), 0);
    stop(servers).await;
}

/// Station 2 announced N, then station 3 did too, where nothing answers:
/// both are asked, station 2 alone confirms, and the handle's first read
/// and first write go there. Station 3 hears the question alone.
#[tokio::test(start_paused = true)]
async fn a_shared_claim_is_asked_and_the_device_that_answers_is_used() {
    let (bus, client, sent, servers) = lab(false).await;
    bus.i_am(2, N);
    bus.i_am(3, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    assert_eq!(device.address(), at(2));
    assert_eq!(name_of(&device).await.unwrap(), "A");
    device.write("ai:1 oos", "true", None).await.unwrap();
    assert_eq!(requests(&sent, WRITE, 2), 1);
    assert_eq!(requests(&sent, READ, 3), 1, "the question alone");
    assert_eq!(requests(&sent, WRITE, 3), 0);
    stop(servers).await;
}

/// Both stations claim N and both answer the question as N: the handle
/// fails as ambiguous, naming both, and nothing but the two questions is
/// sent.
#[tokio::test(start_paused = true)]
async fn two_claims_that_both_answer_are_ambiguous() {
    let (bus, client, sent, servers) = lab(true).await;
    bus.i_am(2, N);
    bus.i_am(3, N);
    settle().await;
    let error = client.device(N).await.unwrap_err();
    assert!(
        matches!(
            &error,
            OpsError::Ambiguous { instance: N, candidates } if *candidates == [at(2), at(3)]
        ),
        "{error:?}"
    );
    assert_eq!(requests(&sent, READ, 2), 1);
    assert_eq!(requests(&sent, READ, 3), 1);
    assert_eq!(writes(&sent), 0);
    stop(servers).await;
}

/// A claim that turns the question away (a Reject, an Abort) says nothing
/// of which device it is: beside one that confirms, the choice stays open,
/// whichever station it is, so the handle fails as ambiguous; alone, as
/// not confirmed.
#[tokio::test(start_paused = true)]
async fn a_turned_away_question_leaves_the_choice_open() {
    for how in [TurnAway::Reject, TurnAway::Abort] {
        for busy in [2u8, 3] {
            let (bus, client, sent, servers) = lab(true).await;
            bus.i_am(2, N);
            bus.i_am(3, N);
            settle().await;
            bus.turn_away(busy, 2, how);
            let error = client.device(N).await.unwrap_err();
            assert!(
                matches!(
                    &error,
                    OpsError::Ambiguous { candidates, .. } if *candidates == [at(2), at(3)]
                ),
                "{how:?} at {busy}: {error:?}"
            );
            assert_eq!(writes(&sent), 0);
            stop(servers).await;
        }
        let (bus, client, _sent, servers) = lab(false).await;
        bus.i_am(2, N);
        settle().await;
        bus.turn_away(2, 2, how);
        let error = client.device(N).await.unwrap_err();
        assert!(
            matches!(&error, OpsError::NotConfirmed { at: here, .. } if *here == at(2)),
            "{how:?} alone: {error:?}"
        );
        stop(servers).await;
    }
}

/// The first handle costs one question; later ones, while it lives and
/// after it is dropped, cost nothing more, and neither do its reads and
/// writes.
#[tokio::test(start_paused = true)]
async fn a_confirmed_device_costs_nothing_more() {
    let (bus, client, sent, servers) = lab(false).await;
    bus.i_am(2, N);
    settle().await;
    let first = client.device(N).await.unwrap();
    assert_eq!(requests(&sent, READ, 2), 1, "the question");
    let again = client.device("1001").await.unwrap();
    assert_eq!(requests(&sent, READ, 2), 1, "joined, asking nothing");
    drop((first, again));
    let later = client.device(N).await.unwrap();
    assert_eq!(requests(&sent, READ, 2), 1, "remembered, asking nothing");
    assert_eq!(name_of(&later).await.unwrap(), "A");
    later.write("ai:1 oos", "true", None).await.unwrap();
    assert_eq!(requests(&sent, READ, 2), 2, "the read alone");
    assert_eq!(requests(&sent, WRITE, 2), 1);
    stop(servers).await;
}

/// A device that turns the read by the wildcard instance away is asked for
/// its own Device object instead, and confirmed so: two questions the
/// first time, none for a handle made after the first is dropped.
#[tokio::test(start_paused = true)]
async fn a_device_refusing_the_wildcard_is_confirmed_by_its_device_object() {
    let (bus, client, sent, servers) = lab(false).await;
    bus.i_am(2, N);
    settle().await;
    bus.turn_away(2, 1, TurnAway::Reject);
    let device = client.device(N).await.unwrap();
    assert_eq!(device.address(), at(2));
    assert_eq!(requests(&sent, READ, 2), 2, "refused, then by its instance");
    drop(device);
    let again = client.device(N).await.unwrap();
    assert_eq!(again.address(), at(2));
    assert_eq!(requests(&sent, READ, 2), 2, "remembered");
    stop(servers).await;
}

/// The device moved from station 2 (silent now, its row still held) to
/// station 3, where it announced itself: both are asked, and the handle
/// goes where it answers, once station 2's question timed out.
#[tokio::test(start_paused = true)]
async fn a_device_that_moved_is_placed_where_it_answers() {
    let (bus, client, sent, servers) = lab(true).await;
    bus.i_am(2, N);
    bus.i_am(3, N);
    settle().await;
    bus.silence(2);
    let device = client.device(N).await.unwrap();
    assert_eq!(device.address(), at(3));
    assert_eq!(name_of(&device).await.unwrap(), "impostor");
    assert_eq!(requests(&sent, READ, 2), 1, "asked, silent");
    stop(servers).await;
}

/// Handles follow N at station 2, confirmed there; then another device
/// announces itself at station 2 and N is claimed at station 3. A handle
/// made now asks station 2 first, as the handles' next read would: still
/// N, or turning the question away, it joins them there and station 3 is
/// never asked; silent, it is placed where station 3 answers as N, and
/// the handles move with it, as a read would move them (#2795).
#[tokio::test(start_paused = true)]
async fn a_new_handle_asks_its_place_before_joining_it() {
    for station_two in ["answers", "turns away", "silent"] {
        let (bus, client, sent, servers) = lab(true).await;
        bus.i_am(2, N);
        settle().await;
        let first = client.device(N).await.unwrap();
        bus.i_am(2, N + 1);
        bus.i_am(3, N);
        settle().await;
        match station_two {
            "turns away" => bus.turn_away(2, 2, TurnAway::Abort),
            "silent" => bus.silence(2),
            _ => {}
        }
        let mut events = client.events();
        let again = client.device(N).await.unwrap();
        let (to, asked_three) = match station_two {
            "silent" => (at(3), 1),
            _ => (at(2), 0),
        };
        assert_eq!(again.address(), to, "{station_two}");
        assert_eq!(first.address(), to, "{station_two}: they share it");
        assert_eq!(requests(&sent, READ, 3), asked_three, "{station_two}");
        assert_eq!(events.try_recv().is_ok(), to == at(3), "{station_two}");
        stop(servers).await;
    }
}

/// More claims than [`MAX_ASKED`] are refused as ambiguous with nothing
/// sent.
#[tokio::test(start_paused = true)]
async fn too_many_claims_are_refused_unasked() {
    let (bus, client, sent, servers) = lab(false).await;
    let count = u8::try_from(MAX_ASKED).unwrap() + 1;
    for station in 3..3 + count {
        bus.i_am(station, N);
    }
    settle().await;
    let error = client.device(N).await.unwrap_err();
    assert!(
        matches!(&error, OpsError::Ambiguous { candidates, .. } if candidates.len() == MAX_ASKED + 1),
        "{error:?}"
    );
    assert!(sent.unicasts().is_empty(), "nothing asked");
    stop(servers).await;
}

/// A watch following N made its place at station 3 from the table alone (a
/// forged claim): a handle made after asks station 3 rather than joining
/// that place unasked, and fails as not confirmed there.
#[tokio::test(start_paused = true)]
async fn a_handle_never_joins_a_place_no_device_confirmed() {
    use crate::watch::{Binding, Point, Watched};
    let (bus, client, sent, servers) = lab(false).await;
    bus.i_am(3, N);
    settle().await;
    let point = Point {
        device: Binding::Follows {
            instance: N,
            network: None,
        },
        object: bacnet_types::primitives::ObjectIdentifier::new(
            bacnet_types::enums::ObjectType::ANALOG_INPUT,
            1,
        )
        .unwrap(),
        watched: Watched::Object,
    };
    let watch = client.watch([point]).unwrap();
    let error = client.device(N).await.unwrap_err();
    assert!(
        matches!(&error, OpsError::NotConfirmed { at: here, .. } if *here == at(3)),
        "{error:?}"
    );
    assert_eq!(writes(&sent), 0);
    drop(watch);
    settle().await;
    stop(servers).await;
}

/// A routed I-Am for N from station 3 on the client's own link, naming
/// station 7 on network 2 (the device sits at station 5 there, behind the
/// router at station 254, unheard): `2:1001` asks the claimed station
/// through station 3, which doesn't answer, and fails as not confirmed;
/// nothing else goes through station 3.
#[tokio::test(start_paused = true)]
async fn a_forged_routed_claim_takes_no_first_request() {
    use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
    let (near, two) = (Bus::new(), Bus::new());
    let ports = [(&near, 1), (&two, 2)]
        .into_iter()
        .map(|(bus, network_number)| RouterPort::new(bus.port(254), network_number))
        .collect();
    let StartedRouter { mut router, .. } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    let servers = vec![server(two.port(5), N, "on two").await];
    let (client, sent) = super::test_bus::client_on(near.port(1)).await;
    settle().await;
    near.i_am_from(3, Some((2, &mac(7))), N);
    settle().await;
    let error = client.device("2:1001").await.unwrap_err();
    let claimed = DeviceAddress::new(2, &mac(7)).unwrap();
    assert!(
        matches!(&error, OpsError::NotConfirmed { at: here, .. } if *here == claimed),
        "{error:?}"
    );
    let through = |n: u8| {
        sent.unicasts()
            .iter()
            .filter(|frame| frame.mac.as_slice() == mac(n))
            .count()
    };
    assert_eq!(through(3), 1, "the question alone");
    assert_eq!(writes(&sent), 0);
    stop(servers).await;
    router.stop().await.unwrap();
}

/// Who answers decides, in every combination: one confirmation with
/// nothing left open places; none fails as not confirmed (one claim) or
/// ambiguous (several); an unclear answer or a second confirmation leaves
/// the choice open.
#[test]
fn the_answers_decide_the_place() {
    use Answer::{Failed, Has, Is, Not, Silent, Unclear};
    let (a, b) = (at(2), at(3));
    let place = |asked: &[(DeviceAddress, Answer)]| placed(N, asked);
    assert_eq!(place(&[(a.clone(), Is(N))]).unwrap(), a);
    assert_eq!(place(&[(a.clone(), Has(N))]).unwrap(), a);
    assert_eq!(
        place(&[(a.clone(), Silent), (b.clone(), Is(N))]).unwrap(),
        b
    );
    assert_eq!(
        place(&[(a.clone(), Is(N + 1)), (b.clone(), Has(N))]).unwrap(),
        b
    );
    assert_eq!(place(&[(a.clone(), Not), (b.clone(), Is(N))]).unwrap(), b);
    for alone in [Silent, Not, Is(N + 1), Has(N + 1), Unclear, Failed] {
        assert!(
            matches!(
                place(&[(a.clone(), alone)]),
                Err(OpsError::NotConfirmed {
                    candidate: None,
                    ..
                })
            ),
            "{alone:?}"
        );
    }
    for other in [Unclear, Failed, Is(N), Has(N)] {
        assert!(
            matches!(
                place(&[(a.clone(), Is(N)), (b.clone(), other)]),
                Err(OpsError::Ambiguous { candidates, .. }) if candidates.len() == 2
            ),
            "{other:?}"
        );
    }
    assert!(matches!(
        place(&[(a.clone(), Silent), (b.clone(), Silent)]),
        Err(OpsError::Ambiguous { candidates, .. }) if candidates == [a.clone(), b.clone()]
    ));
    assert!(matches!(place(&[]), Err(OpsError::UnknownDevice { .. })));
}
