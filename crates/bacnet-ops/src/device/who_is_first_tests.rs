//! A first placement by instance sends one Who-Is for it when no claim is
//! confirmed already (decision of 2026-11-26 on #2824), and gives its
//! answers the settle time before the claims are asked: about half a
//! second; with no claim at all, the resolve wait for a first answer.
//! Resolutions of one instance run one at a time and share the Who-Is. On
//! the handle tests' bus, with paused time.

use std::time::Duration;

use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType};
use bacnet_types::primitives::ObjectIdentifier;
use tokio::time::Instant;

use super::first_contact_tests::{lab_with, writes};
use super::test_bus::{N, at, requests, settle, stop, who_is};
use super::*;
use crate::client::RESOLVE_WAIT;
use crate::watch::{Binding, Point, State, Watched};

const READ: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;

/// The device announced itself, and nothing confirmed it yet: the first
/// handle sends one Who-Is, waits the settle time, and asks once, so it
/// costs about half a second. A second handle, made while the first
/// lives, or after it is dropped, sends no Who-Is: the device's answer is
/// remembered.
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
    drop(first);
    let from = Instant::now();
    client.device(N).await.unwrap();
    assert!(from.elapsed() < Duration::from_millis(50));
    assert_eq!(
        (who_is(&sent), requests(&sent, READ, 2)),
        (1, 1),
        "remembered"
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
        let placement = place_instance(client.low_level(), &limits, N, None, look).await;
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
