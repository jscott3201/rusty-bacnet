//! A client's watches and its stop (#2561, #2603): a watch's hub, one of
//! the high-level client's pooled hubs, holds the client while a watch on
//! it lives, so a stop ends every watch left and shuts every hub before it
//! takes the low-level client, and a watch dropped while the client runs
//! ends and is forgotten on its own. A watch of no points, which waits for
//! points while its control lives, stands in: its hub's notification
//! receiver and its own clone hold the client all the same.

use super::*;
use std::time::Duration;

use tokio::time::timeout;

use super::tests::running_client;
use crate::client::WatchTie;
use crate::high_level::{Ended, WatchShared};

/// A watch on the started client, made under a lease as `Device.watch`
/// makes one, and kept by the client.
async fn watch_on(state: &ClientInner) -> (Arc<WatchShared>, WatchTie) {
    let high = high_level(state).await.unwrap();
    let watch = high.watch(Vec::new()).unwrap();
    let shared = Arc::new(WatchShared::new(watch));
    let tie = WatchTie::new(Arc::clone(state), Arc::clone(&shared));
    (shared, tie)
}

#[tokio::test]
async fn a_stop_ends_a_live_watch_before_it_takes_the_client() {
    let state = running_client().await;
    let (watch, tie) = watch_on(&state).await;
    assert_eq!(state.live_watches(), 1);
    timeout(Duration::from_secs(10), close(Arc::clone(&state)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.liveness(), Liveness::Stopped);
    assert_eq!(watch.ended_why(), Some(Ended::Stopped));
    assert!(!watch.watching());
    assert_eq!(state.live_watches(), 0);
    // Its end after the stop finds nothing left to do.
    drop(tie);
}

#[tokio::test]
async fn a_dropped_watch_ends_and_is_forgotten_while_the_client_runs() {
    let state = running_client().await;
    let (watch, tie) = watch_on(&state).await;
    let (other, _kept) = watch_on(&state).await;
    assert_eq!(state.live_watches(), 2);
    drop(tie);
    // The iterator ends at once; the watch's end runs on its own.
    assert_eq!(watch.ended_why(), Some(Ended::Closed));
    timeout(Duration::from_secs(10), async {
        while state.live_watches() != 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the dropped watch is forgotten");
    assert!(started(&state).await.is_ok(), "the client still runs");
    timeout(Duration::from_secs(10), close(Arc::clone(&state)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(other.ended_why(), Some(Ended::Stopped));
}
