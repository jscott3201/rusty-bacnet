//! Failed and cancelled sequential startup retains every attempted transport.

use super::owned_fixture::port;
use super::{BACnetRouter, RouterOptions};
use bacnet_types::error::Error;
use std::sync::atomic::Ordering;

async fn rejected_start(invalid_mac: bool) {
    let (a, mut first) = port(false);
    let (mut b, mut second) = port(false);
    let (mut c, third) = port(false);
    b.network_number = 2;
    c.network_number = 3;
    if invalid_mac {
        b.transport.mac = vec![2; bacnet_npdu::MAX_MAC_LEN + 1];
    } else {
        b.transport.start_error = Some(Error::Transport(std::io::Error::from_raw_os_error(13)));
    }
    // Secondary cleanup failure must not replace the primary startup error,
    // nor skip cleanup of the second attempted transport.
    let mut a = a;
    a.transport.stop_error = Some(Error::Encoding("rollback secondary".into()));
    let start = BACnetRouter::start(vec![a, b, c], RouterOptions::new());
    tokio::pin!(start);
    tokio::select! {
        _ = &mut start => panic!("startup returned before attempted rollback completed"),
        entered = &mut first.stop_entered => entered.unwrap(),
    }
    assert_eq!(third.starts.load(Ordering::SeqCst), 0);
    assert_eq!(first.dropped.load(Ordering::SeqCst), 0);
    first.stop_release.send(()).unwrap();
    tokio::select! {
        _ = &mut start => panic!("startup returned before second rollback completed"),
        entered = &mut second.stop_entered => entered.unwrap(),
    }
    assert_eq!(first.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(second.dropped.load(Ordering::SeqCst), 0);
    second.stop_release.send(()).unwrap();
    let error = match start.await {
        Err(error) => error,
        Ok(_) => panic!("invalid startup succeeded"),
    };
    if invalid_mac {
        assert!(
            matches!(error, Error::Encoding(ref text) if text.contains("network 2") && text.contains("MAC longer"))
        );
    } else {
        assert!(matches!(error, Error::Transport(ref error) if error.raw_os_error() == Some(13)));
    }
    for counter in [
        &first.starts,
        &second.starts,
        &first.stopped,
        &second.stopped,
        &first.dropped,
        &second.dropped,
    ] {
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
    assert_eq!(third.starts.load(Ordering::SeqCst), 0);
    assert_eq!(third.stopped.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn later_start_error_awaits_every_attempted_rollback_and_preserves_primary() {
    rejected_start(false).await;
}

#[tokio::test(start_paused = true)]
async fn invalid_post_start_mac_awaits_every_attempted_rollback() {
    rejected_start(true).await;
}

#[tokio::test(start_paused = true)]
async fn cancelled_multiport_start_retains_running_and_partial_owners() {
    let (a, first) = port(false);
    let (mut b, mut second) = port(false);
    let (mut c, third) = port(false);
    b.network_number = 2;
    b.transport.pending_start = true;
    c.network_number = 3;
    {
        let start = BACnetRouter::start(vec![a, b, c], RouterOptions::new());
        tokio::pin!(start);
        tokio::select! {
            _ = &mut start => panic!("held second start returned"),
            entered = &mut second.start_entered => entered.unwrap(),
        }
        assert_eq!(first.starts.load(Ordering::SeqCst), 1);
        assert_eq!(second.starts.load(Ordering::SeqCst), 1);
        assert_eq!(third.starts.load(Ordering::SeqCst), 0);
    }
    // Dropping the waiter initiates cleanup; completion is observed through
    // independent probes, not inferred from the cancellation itself.
    first.stop_entered.await.unwrap();
    assert!(second.start_release.is_closed());
    first.stop_release.send(()).unwrap();
    first.completed.await.unwrap();
    second.stop_entered.await.unwrap();
    second.stop_release.send(()).unwrap();
    second.completed.await.unwrap();
    assert_eq!(first.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(second.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(third.starts.load(Ordering::SeqCst), 0);
    assert_eq!(third.stopped.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn cancellation_while_awaiting_failed_start_rollback_keeps_cleanup_running() {
    let (a, mut first) = port(false);
    let (mut b, second) = port(false);
    b.network_number = 2;
    b.transport.start_error = Some(Error::Encoding("named startup error".into()));
    {
        let start = BACnetRouter::start(vec![a, b], RouterOptions::new());
        tokio::pin!(start);
        tokio::select! {
            _ = &mut start => panic!("startup returned before rollback release"),
            entered = &mut first.stop_entered => entered.unwrap(),
        }
    }
    first.stop_release.send(()).unwrap();
    first.completed.await.unwrap();
    second.stop_entered.await.unwrap();
    second.stop_release.send(()).unwrap();
    second.completed.await.unwrap();
    assert_eq!(first.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(second.dropped.load(Ordering::SeqCst), 1);
}
