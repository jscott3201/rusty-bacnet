//! The hosted router's own Router-Busy and Router-Available around a
//! clogged port, end to end through its queues and sender tasks (#1697).

use bacnet_router_core::router::Congestion;

use super::priority_harness::*;
use super::send_queue::CAPACITY;
use super::{BACnetRouter, RouterOptions};

/// A unicast for station 9 on network 2000, port 1's own, from node 7 on
/// network 1000.
fn for_port_1(tag: u8) -> Vec<u8> {
    vec![1, 0x20, 0x07, 0xD0, 1, 9, 255, 0x10, tag]
}

/// Wait until the router has refused `refused` sends in all.
async fn refused(router: &BACnetRouter, refused: u64) {
    for _ in 0..100_000 {
        if router.counters().refused >= refused {
            return;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(router.counters().refused, refused);
}

#[tokio::test(start_paused = true)]
async fn a_clogged_port_announces_busy_rejects_with_reason_2_then_available() {
    let options = RouterOptions::new().congestion(Congestion::DEFAULT);
    let (router, _apdus, mut peers) = start_with(2, options).await;
    // One send in progress on port 1, then its queue fills up.
    receive(&peers[0], for_port_1(0), 0).await;
    let in_progress = sending(&mut peers[1]).await;
    for tag in 1..=CAPACITY {
        receive(&peers[0], for_port_1(tag as u8), 0).await;
    }
    // One more is refused: full, so the wait for the clog starts.
    receive(&peers[0], for_port_1(0xF0), 0).await;
    refused(&router, 1).await;
    assert!(peers[0].sent.try_recv().is_err(), "not clogged yet");

    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    receive(&peers[0], for_port_1(0xF1), 0).await;
    refused(&router, 2).await;
    // Reason 2 back to node 7, the Busy for network 2000 on port 0.
    let reject = sending(&mut peers[0]).await;
    assert_eq!(reject.to, Some(vec![7]));
    assert_eq!(reject.npdu, [1, 0x80, 0x03, 2, 0x07, 0xD0]);
    reject.complete.send(Ok(())).unwrap();
    let busy = sending(&mut peers[0]).await;
    assert_eq!(busy.to, None);
    assert_eq!(busy.npdu, [1, 0x80, 0x04, 0x07, 0xD0]);
    assert!(busy.attributes.is_empty());
    busy.complete.send(Ok(())).unwrap();
    assert_eq!(router.counters().busy_announced, 1);

    // Port 1 drains; under half its capacity, Router-Available.
    in_progress.complete.send(Ok(())).unwrap();
    let mut completed = 1;
    let available = loop {
        let send = sending(&mut peers[1]).await;
        send.complete.send(Ok(())).unwrap();
        completed += 1;
        tokio::task::yield_now().await;
        if let Ok(available) = peers[0].sent.try_recv() {
            break available;
        }
        assert!(completed <= CAPACITY + 1, "never announced available");
    };
    assert_eq!(available.npdu, [1, 0x80, 0x05, 0x07, 0xD0]);
    assert_eq!(available.to, None);
    // Of the CAPACITY + 1 sends, `left` aren't done: one the sender has
    // started and the rest queued. Available comes once fewer than half
    // the capacity wait.
    let left = CAPACITY + 1 - completed;
    assert!(
        left <= CAPACITY / 2 && left + 2 >= CAPACITY / 2,
        "{left} left"
    );
    available.complete.send(Ok(())).unwrap();
    let counts = router.counters();
    assert_eq!((counts.available_announced, counts.rejects), (1, 1));
}

#[tokio::test(start_paused = true)]
async fn off_by_default_a_full_port_only_counts_refusals() {
    let (router, _apdus, mut peers) = start(2).await;
    receive(&peers[0], for_port_1(0), 0).await;
    let _in_progress = sending(&mut peers[1]).await;
    for tag in 1..=CAPACITY {
        receive(&peers[0], for_port_1(tag as u8), 0).await;
    }
    receive(&peers[0], for_port_1(0xF0), 0).await;
    tokio::time::advance(std::time::Duration::from_secs(3)).await;
    receive(&peers[0], for_port_1(0xF1), 0).await;
    refused(&router, 2).await;
    tokio::task::yield_now().await;
    assert!(peers[0].sent.try_recv().is_err(), "no reject, no Busy");
    let counts = router.counters();
    assert_eq!((counts.busy_announced, counts.rejects), (0, 0));
}
