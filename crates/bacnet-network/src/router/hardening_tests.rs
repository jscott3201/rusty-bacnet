//! The hosted router passes the #1698 hardening options through to its
//! engine: next-hop rejects and duplicate global-broadcast suppression.

use bacnet_npdu::Mac;
use bacnet_router_core::table::Reachability;
use bacnet_router_core::PortId;

use super::priority_harness::*;
use super::{BACnetRouter, Duplicates, RouterOptions};

/// Wait until `done` holds for the router.
async fn until(router: &BACnetRouter, done: impl Fn(&BACnetRouter) -> bool) {
    for _ in 0..100_000 {
        if done(router) {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("never got there: {:?}", router.counters());
}

/// A Who-Is to every network.
const GLOBAL_WHO_IS: [u8; 8] = [1, 0x20, 0xFF, 0xFF, 0, 255, 0x10, 0x08];

#[tokio::test]
async fn a_repeated_global_broadcast_is_dropped_when_asked() {
    let options = RouterOptions::new().duplicates(Duplicates::DEFAULT);
    let (mut router, mut apdus, mut peers) = start_with(2, options).await;
    receive(&peers[0], GLOBAL_WHO_IS.to_vec(), 0).await;
    let relayed = sending(&mut peers[1]).await;
    assert_eq!(relayed.to, None);
    relayed.complete.send(Ok(())).unwrap();
    assert_eq!(apdus.recv().await.unwrap().apdu.as_ref(), [0x10, 0x08]);
    receive(&peers[0], GLOBAL_WHO_IS.to_vec(), 0).await;
    until(&router, |router| router.counters().duplicate_drops == 1).await;
    assert!(peers[1].sent.try_recv().is_err(), "not relayed again");
    assert!(apdus.try_recv().is_err(), "not delivered again");
    router.stop().await;
}

#[tokio::test]
async fn by_default_a_repeat_goes_through() {
    let (mut router, mut apdus, mut peers) = start(2).await;
    for _ in 0..2 {
        receive(&peers[0], GLOBAL_WHO_IS.to_vec(), 0).await;
        sending(&mut peers[1]).await.complete.send(Ok(())).unwrap();
        assert_eq!(apdus.recv().await.unwrap().apdu.as_ref(), [0x10, 0x08]);
    }
    assert_eq!(router.counters().duplicate_drops, 0);
    router.stop().await;
}

/// Network 3000 learned through node 7 on port 1, then a reject naming it
/// from node 7 on port 0: another router than its next.
async fn off_path_reject(options: RouterOptions) -> (Option<Reachability>, u64) {
    let (mut router, _apdus, peers) = start_with(2, options).await;
    let now = router.now();
    assert!(router
        .table()
        .add_learned(3000, PortId(1), Mac::from_array([7]), now));
    let reject = vec![1, 0x80, 0x03, 1, 0x0B, 0xB8];
    receive(&peers[0], reject, 0).await;
    until(&router, |router| {
        let unreachable = router
            .table()
            .lookup(3000)
            .is_some_and(|route| route.reachability() == Reachability::Unreachable);
        unreachable || router.counters().not_next_hop > 0
    })
    .await;
    let reachability = router
        .table()
        .lookup(3000)
        .map(|route| route.reachability());
    let counted = router.counters().not_next_hop;
    router.stop().await;
    (reachability, counted)
}

#[tokio::test]
async fn next_hop_rejects_leave_an_off_path_route_alone_when_asked() {
    assert_eq!(
        off_path_reject(RouterOptions::new().next_hop_rejects()).await,
        (Some(Reachability::Reachable), 1)
    );
    assert_eq!(
        off_path_reject(RouterOptions::new()).await,
        (Some(Reachability::Unreachable), 0)
    );
}
