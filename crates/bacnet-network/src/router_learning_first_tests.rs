//! A first router, learned only from two of the learner's own queries a
//! minute apart, and the bound on the networks a learner tracks (#2831).

use super::*;
use crate::router_learning::IDLE;

/// Another single router answering the second query starts over: it is a
/// first answer of its own, and the query a minute later learns it.
#[tokio::test(start_paused = true)]
async fn another_router_the_second_time_starts_over() {
    let site = Site::new(Script {
        routers: vec![OLD.to_vec()],
        ..Script::default()
    })
    .await;
    site.learning.unrouted(DNET);
    sleep(WAIT + TICK).await;
    site.script.locked().routers = vec![NEW.to_vec()];
    sleep(STRIKE_GAP + WAIT).await;
    assert_eq!(site.queries(), 2);
    assert_eq!(site.router(), None, "two routers, one answer each");
    sleep(STRIKE_GAP).await;
    assert_eq!(site.queries(), 3);
    sleep(WAIT).await;
    assert_eq!(site.router(), Some(MacAddr::from_slice(&NEW)));
}

/// Several routers, or none, answering the second query learn nothing and
/// clear the first answer: no third query follows without a request, and
/// the next one alone is a first answer again.
#[tokio::test(start_paused = true)]
async fn several_or_none_the_second_time_learn_nothing() {
    for second in [vec![], vec![OLD.to_vec(), NEW.to_vec()]] {
        let site = Site::new(Script {
            routers: vec![OLD.to_vec()],
            ..Script::default()
        })
        .await;
        site.learning.unrouted(DNET);
        sleep(WAIT + TICK).await;
        site.script.locked().routers = second;
        sleep(STRIKE_GAP + WAIT).await;
        assert_eq!(site.queries(), 2);
        assert_eq!(site.router(), None);
        sleep(2 * STRIKE_GAP).await;
        assert_eq!(site.queries(), 2, "no query without a request");
        site.script.locked().routers = vec![OLD.to_vec()];
        site.learning.unrouted(DNET);
        sleep(WAIT + TICK).await;
        assert_eq!(site.queries(), 3);
        assert_eq!(site.router(), None, "the first answer was cleared");
    }
}

/// A full learner takes no new network while the others are in use: the
/// new one is never asked about, and requests to it go by broadcast. Once
/// one has gone unused for an hour, the new one takes the place of the
/// network used least recently; one still in use keeps its router.
#[tokio::test(start_paused = true)]
async fn a_full_learner_makes_room_only_from_an_idle_network() {
    let site = Site::new(Script::default()).await;
    let networks = 1..=u16::try_from(MAX_NETWORKS).unwrap();
    let new = u16::try_from(MAX_NETWORKS).unwrap() + 1;
    for network in networks.clone() {
        site.learning.learn_for_test(network, &OLD);
    }
    site.learning.unrouted(new);
    sleep(WAIT + TICK).await;
    assert!(
        site.sent().is_empty(),
        "a network past the cap was asked about"
    );
    for network in networks {
        assert!(site.learning.router(network).is_some(), "{network}");
    }
    sleep(IDLE - Duration::from_secs(60)).await;
    // A request to network 1 asks for its router: it is in use.
    assert!(site.learning.router(1).is_some());
    sleep(Duration::from_secs(60)).await;
    site.learning.unrouted(new);
    sleep(TICK).await;
    assert_eq!(site.sent(), [Sent::RouterQuery(new)]);
    assert_eq!(site.learning.router(2), None, "the idle one gave way");
    assert!(site.learning.router(1).is_some());
    assert!(site.learning.router(3).is_some());
}

/// An owner's timeout of a millisecond still has each query wait three
/// seconds: a router answering two seconds after each query is learned.
#[tokio::test(start_paused = true)]
async fn each_query_waits_three_seconds_at_least() {
    let site = Site::with(Script::default(), Counted::Own, Duration::from_millis(1)).await;
    site.learning.unrouted(DNET);
    sleep(TICK).await;
    assert_eq!(site.queries(), 1);
    sleep(Duration::from_secs(2)).await;
    site.hear(&OLD, announcement(&[DNET])).await;
    // The second query a minute after the first window closed, at 3 s.
    sleep(STRIKE_GAP + Duration::from_secs(1)).await;
    assert_eq!(site.queries(), 2);
    sleep(Duration::from_secs(2)).await;
    site.hear(&OLD, announcement(&[DNET])).await;
    sleep(Duration::from_secs(2)).await;
    assert_eq!(site.router(), Some(MacAddr::from_slice(&OLD)));
}
