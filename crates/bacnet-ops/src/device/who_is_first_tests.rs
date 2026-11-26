//! A first placement by instance sends one Who-Is for it when no claim is
//! confirmed already (decision of 2026-11-26 on #2824), and gives its
//! answers the settle time before the claims are asked: about half a
//! second; with no claim at all, the resolve wait for a first answer.
//! Resolutions of one instance run one at a time and share the Who-Is. On
//! the handle tests' bus, with paused time.

use std::time::Duration;

use bacnet_transport::test_transport::TestTransport;
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType};
use bacnet_types::primitives::ObjectIdentifier;
use tokio::time::Instant;

use super::first_contact_tests::{lab_with, writes};
use super::test_bus::{N, at, requests, server, settle, stop, who_is};
use super::*;
use crate::client::RESOLVE_WAIT;
use crate::watch::{Binding, Point, State, Watched};

const READ: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;

/// The device announced itself, and nothing confirmed it yet: the first
/// handle sends one Who-Is, waits the settle time, and asks once, so it
/// costs about half a second. A second handle, made while the first
/// lives, joins it, sending nothing. One made after the last is dropped
/// is placed again: the Who-Is and its settle time once more, but no
/// question, the device's answer being remembered.
#[tokio::test(start_paused = true)]
async fn a_first_contact_costs_about_half_a_second() {
    let (bus, client, sent, servers) = lab_with(true, false).await;
    bus.i_am(2, N);
    settle().await;
    let from = Instant::now();
    let first = client.device(N).await.unwrap();
    let took = from.elapsed();
    assert!(
        (super::resolve::SETTLE..Duration::from_millis(600)).contains(&took),
        "{took:?}"
    );
    assert_eq!((who_is(&sent), requests(&sent, READ, 2)), (1, 1));
    let from = Instant::now();
    let joined = client.device(N).await.unwrap();
    assert!(from.elapsed() < Duration::from_millis(50));
    assert_eq!((who_is(&sent), requests(&sent, READ, 2)), (1, 1), "joined");
    drop((first, joined));
    let from = Instant::now();
    client.device(N).await.unwrap();
    let took = from.elapsed();
    assert!(
        (super::resolve::SETTLE..Duration::from_millis(600)).contains(&took),
        "{took:?}"
    );
    assert_eq!(
        (who_is(&sent), requests(&sent, READ, 2)),
        (2, 1),
        "the Who-Is again, the answer remembered"
    );
    stop(servers).await;
}

/// No address claims the instance and nothing answers its Who-Is: the
/// handle fails as unknown after the resolve wait, about 3 s, saying where
/// it asked.
#[tokio::test(start_paused = true)]
async fn nothing_answering_fails_after_the_resolve_wait() {
    let (_bus, client, sent, servers) = lab_with(false, false).await;
    let from = Instant::now();
    let error = client.device(N).await.unwrap_err();
    let took = from.elapsed();
    assert!(
        matches!(&error, OpsError::UnknownDevice { asked: Some(_), .. }),
        "{error:?}"
    );
    assert!(
        (RESOLVE_WAIT..RESOLVE_WAIT + Duration::from_millis(100)).contains(&took),
        "{took:?}"
    );
    assert_eq!(who_is(&sent), 1);
    stop(servers).await;
}

/// Two handles for one instance made at once: one resolution runs, the
/// other waits its turn and joins the place, so one Who-Is goes out and the
/// device is asked once.
#[tokio::test(start_paused = true)]
async fn resolutions_of_one_instance_share_their_who_is() {
    let (bus, client, sent, servers) = lab_with(true, false).await;
    bus.i_am(2, N);
    settle().await;
    let (one, two) = tokio::join!(client.device(N), client.device("1001"));
    assert_eq!(
        (one.unwrap().address(), two.unwrap().address()),
        (at(2), at(2))
    );
    assert_eq!((who_is(&sent), requests(&sent, READ, 2)), (1, 1));
    stop(servers).await;
}

/// A watch following N, made with no handle, on a place a forged claim at
/// station 3 gave it (a responder there answers as N), while the device at
/// station 2 is on the network, quiet: the hub settles the place before its
/// first read, the Who-Is draws the device's claim, both answer as N, and
/// the point is unreachable as unconfirmed. Nothing is read at station 3 but
/// the question, and nothing subscribed there.
#[tokio::test(start_paused = true)]
async fn a_watch_by_instance_is_not_placed_on_an_ambiguous_claim() {
    let (bus, client, sent, servers) = lab_with(true, true).await;
    bus.i_am(3, N);
    settle().await;
    let point = Point {
        device: Binding::Follows {
            instance: N,
            network: None,
        },
        object: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        watched: Watched::Object,
    };
    let mut watch = client.watch([point]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let update = loop {
        let update = tokio::time::timeout_at(deadline, watch.next())
            .await
            .expect("an update")
            .expect("the watch goes on");
        if update.state != State::Starting {
            break update;
        }
    };
    assert_eq!(
        update.state,
        State::Unreachable {
            why: crate::watch::Why::Unconfirmed
        },
        "{update:?}"
    );
    assert!(update.value.is_none());
    assert_eq!(requests(&sent, READ, 3), 1, "the question alone");
    let subscribe = ConfirmedServiceChoice::SUBSCRIBE_COV;
    assert_eq!(requests(&sent, subscribe, 3) + writes(&sent), 0);
    drop(watch);
    settle().await;
    stop(servers).await;
}

/// The low-level placement a front end without a `Client` uses
/// ([`place_instance`]) keeps the same rule: a forged claim at station 3,
/// the device at station 2 quiet. With a Who-Is allowed, the device answers
/// it, both are asked, and station 2 is placed, station 3 silent. With
/// none (MCP's case), station 3 alone is asked and refused as not
/// confirmed, saying it was silent, and no Who-Is goes.
#[tokio::test(start_paused = true)]
async fn the_low_level_placement_keeps_the_rule() {
    use crate::Scope;
    use crate::device::{Look, place_instance};
    use crate::limits::RequestLimits;
    for look in [true, false] {
        let (bus, client, sent, servers) = lab_with(true, false).await;
        bus.i_am(3, N);
        settle().await;
        let limits = RequestLimits::new(0);
        let look = look.then_some(Look {
            scope: Scope::Local,
            wait: RESOLVE_WAIT,
        });
        let asking = look.is_some();
        let placement = place_instance(client.low_level(), &limits, N, None, look, &|_| None)
            .await
            .unwrap();
        match asking {
            true => {
                assert_eq!(placement.placed.unwrap(), at(2));
                assert!(placement.asked.contains(&(at(3), Answer::Silent)));
                assert_eq!(who_is(&sent), 1);
            }
            false => {
                assert!(
                    matches!(
                        &placement.placed,
                        Err(OpsError::NotConfirmed { at: here, answer: Some(Answer::Silent), .. })
                            if *here == at(3)
                    ),
                    "{placement:?}"
                );
                assert_eq!(who_is(&sent), 0);
                assert_eq!(requests(&sent, READ, 2), 0);
            }
        }
        stop(servers).await;
    }
}

/// Two handles for one instance made at once, the first cancelled while
/// its Who-Is settles: a turn marks its Who-Is shared only once the settle
/// time is over, so the second sends its own and gives it the settle time.
/// The device at station 2 missed the first Who-Is and answers the second,
/// and with the responder at station 3 both answer as N: ambiguous, rather
/// than placed at station 3 on a settle cut short.
#[tokio::test(start_paused = true)]
async fn a_resolution_cancelled_while_it_settles_shares_no_who_is() {
    let (bus, client, sent, servers) = lab_with(true, true).await;
    bus.i_am(3, N);
    settle().await;
    bus.silence(2);
    let place = |client: Client<TestTransport>| async move {
        client.device(N).await.map(|device| device.address())
    };
    let first = tokio::spawn(place(client.clone()));
    let second = tokio::spawn(place(client.clone()));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(who_is(&sent), 1, "the first's");
    first.abort();
    bus.unsilence(2);
    assert!(first.await.unwrap_err().is_cancelled());
    let second = second.await.unwrap();
    assert!(
        matches!(
            &second,
            Err(OpsError::Ambiguous { candidates, .. }) if *candidates == [at(2), at(3)]
        ),
        "{second:?}"
    );
    assert_eq!(who_is(&sent), 2, "the second's own");
    stop(servers).await;
}

/// A Who-Is the link fails to send places nothing: a handle by instance and
/// the low-level placement both fail with the link's error, and no claim is
/// asked anything.
#[tokio::test(start_paused = true)]
async fn a_who_is_the_link_cannot_send_places_nothing() {
    use crate::Scope;
    use crate::device::{Look, place_instance};
    use crate::limits::RequestLimits;
    let (bus, client, sent, servers) = lab_with(true, false).await;
    bus.i_am(2, N);
    settle().await;
    client.low_level().transport().handle().fail_next_send();
    let error = client.device(N).await.unwrap_err();
    assert!(matches!(&error, OpsError::Client(_)), "{error:?}");
    client.low_level().transport().handle().fail_next_send();
    let look = Look {
        scope: Scope::Local,
        wait: RESOLVE_WAIT,
    };
    let limits = RequestLimits::new(0);
    let placement = place_instance(client.low_level(), &limits, N, None, Some(look), &|_| None)
        .await
        .unwrap();
    assert!(
        matches!(&placement.placed, Err(OpsError::Client(_))),
        "{placement:?}"
    );
    assert!(placement.asked.is_empty(), "{placement:?}");
    assert_eq!(requests(&sent, READ, 2), 0, "nothing asked");
    stop(servers).await;
}

/// A device slower than the settle time (#2824): the responder at station
/// 3 claims N alone when the Who-Is settles, answers as N, and the handle
/// is placed there, provisionally for the resolve wait. The device at
/// station 2 claims N a second later: that clears the placement's
/// confirmation, so a write through the handle asks again and is refused
/// as ambiguous, writing nothing, and so is a new handle. A claim after
/// the resolve wait changes nothing: the write goes, and a new handle joins
/// the place.
#[tokio::test(start_paused = true)]
async fn a_lone_claim_places_the_handle_for_the_resolve_wait_only_provisionally() {
    for late in [false, true] {
        let (bus, client, sent, mut servers) = lab_with(false, true).await;
        bus.i_am(3, N);
        settle().await;
        let device = client.device(N).await.unwrap();
        assert_eq!(device.address(), at(3));
        let after = match late {
            true => RESOLVE_WAIT + Duration::from_millis(100),
            false => Duration::from_secs(1),
        };
        tokio::time::sleep(after).await;
        servers.push(server(bus.port(2), N, "A").await);
        bus.i_am(2, N);
        settle().await;
        let written = device.write("ai:1 oos", "true", None).await;
        let again = client.device(N).await;
        match late {
            true => {
                written.unwrap();
                assert_eq!(again.unwrap().address(), at(3));
            }
            false => {
                assert!(
                    matches!(&written, Err(OpsError::Ambiguous { .. })),
                    "{written:?}"
                );
                assert!(
                    matches!(&again, Err(OpsError::Ambiguous { .. })),
                    "{again:?}"
                );
                assert_eq!(writes(&sent), 0);
            }
        }
        stop(servers).await;
    }
}
