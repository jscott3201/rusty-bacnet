//! Causal transport cleanup barriers for router ownership.

use super::owned_fixture::port;
use super::{BACnetRouter, RouterOptions, RouterStopFailure};
use bacnet_types::error::Error;
use std::sync::atomic::Ordering;
use std::time::Duration;

async fn cleanup_barrier(pending_send: bool) {
    let (port, mut probe) = port(pending_send);
    let mut started = BACnetRouter::start(vec![port], RouterOptions::new())
        .await
        .unwrap();
    probe.send_entered.await.unwrap();
    let stop = started.router.stop();
    tokio::pin!(stop);
    tokio::select! {
        biased;
        _ = &mut stop => panic!("router stop returned without invoking and awaiting transport.stop"),
        entered = &mut probe.stop_entered => entered.expect("transport.stop must be invoked"),
        _ = tokio::time::sleep(Duration::from_secs(5)) => panic!("transport.stop was not entered"),
    }
    assert_eq!(probe.stopped.load(Ordering::SeqCst), 1);
    assert_eq!(probe.dropped.load(Ordering::SeqCst), 0);
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(stop.as_mut().poll(cx).is_pending()))
            .await
    );
    probe.stop_release.send(()).unwrap();
    stop.await.unwrap();
    assert_eq!(probe.aborted.load(Ordering::SeqCst), 0);
    assert_eq!(probe.dropped.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn idle_stop_awaits_transport_child_cleanup() {
    cleanup_barrier(false).await;
}

#[tokio::test(start_paused = true)]
async fn pending_send_stop_awaits_transport_child_cleanup() {
    cleanup_barrier(true).await;
}

#[tokio::test(start_paused = true)]
async fn cancelled_stop_wait_keeps_every_port_and_replays_success() {
    let (a, mut first) = port(true);
    let (mut b, mut second) = port(true);
    b.network_number = 2;
    let mut started = BACnetRouter::start(vec![a, b], RouterOptions::new())
        .await
        .unwrap();
    first.send_entered.await.unwrap();
    second.send_entered.await.unwrap();
    {
        let stop = started.router.stop();
        tokio::pin!(stop);
        tokio::select! {
            result = &mut stop => panic!("cleanup completed before release: {result:?}"),
            _ = async { (&mut first.stop_entered).await.unwrap(); (&mut second.stop_entered).await.unwrap(); } => {}
        }
        assert_eq!(first.dropped.load(Ordering::SeqCst), 0);
        assert_eq!(second.dropped.load(Ordering::SeqCst), 0);
    }
    first.stop_release.send(()).unwrap();
    second.stop_release.send(()).unwrap();
    started.router.stop().await.unwrap();
    first.completed.await.unwrap();
    second.completed.await.unwrap();
    started.router.stop().await.unwrap();
    for probe in [
        &first.stopped,
        &second.stopped,
        &first.dropped,
        &second.dropped,
    ] {
        assert_eq!(probe.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn cancelled_failed_stop_preserves_original_errors_and_attempts_later_ports() {
    let mut ports = Vec::new();
    let mut probes = Vec::new();
    for index in 1..=3 {
        let (mut port, probe) = port(true);
        port.network_number = index;
        if index == 1 {
            port.transport.stop_error =
                Some(Error::Transport(std::io::Error::from_raw_os_error(13)));
        }
        if index == 2 {
            port.transport.stop_error = Some(Error::Encoding("second cleanup failure".into()));
        }
        ports.push(port);
        probes.push(probe);
    }
    let mut started = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    {
        let stop = started.router.stop();
        tokio::pin!(stop);
        tokio::select! {
            result = &mut stop => panic!("cleanup completed before release: {result:?}"),
            _ = async { for probe in &mut probes { (&mut probe.stop_entered).await.unwrap(); } } => {}
        }
    }
    let mut counts = Vec::new();
    for probe in probes {
        counts.push((probe.stopped, probe.dropped));
        probe.stop_release.send(()).unwrap();
    }
    let first = started.router.stop().await.unwrap_err();
    let second = started.router.stop().await.unwrap_err();
    assert!(std::ptr::eq(first.failures(), second.failures()));
    assert_eq!(first.failures().len(), 2);
    assert!(
        matches!(&first.failures()[0], RouterStopFailure::Transport {
        network_number: 1, error: Error::Transport(error),
    } if error.raw_os_error() == Some(13))
    );
    assert!(
        matches!(&first.failures()[1], RouterStopFailure::Transport {
        network_number: 2, error: Error::Encoding(detail),
    } if detail == "second cleanup failure")
    );
    for (stopped, dropped) in counts {
        assert_eq!(stopped.load(Ordering::SeqCst), 1);
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn dropping_router_initiates_cleanup_without_a_stop_waiter() {
    let (port, probe) = port(true);
    let started = BACnetRouter::start(vec![port], RouterOptions::new())
        .await
        .unwrap();
    probe.send_entered.await.unwrap();
    drop(started.router);
    probe.stop_entered.await.unwrap();
    assert_eq!(probe.dropped.load(Ordering::SeqCst), 0);
    probe.stop_release.send(()).unwrap();
    probe.completed.await.unwrap();
    assert_eq!(probe.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(probe.stopped.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn sender_panic_is_a_retained_failure_and_still_cleans_transport() {
    let (mut port, probe) = port(false);
    port.transport.panic_send = true;
    let mut started = BACnetRouter::start(vec![port], RouterOptions::new())
        .await
        .unwrap();
    probe.stop_entered.await.unwrap();
    probe.stop_release.send(()).unwrap();
    let error = started.router.stop().await.unwrap_err();
    assert!(matches!(&error.failures()[0], RouterStopFailure::Worker {
        network_number: Some(1), worker: "sender", detail,
    } if detail == "named sender panic"));
    assert_eq!(probe.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(probe.stopped.load(Ordering::SeqCst), 1);
    assert!(std::ptr::eq(
        error.failures(),
        started.router.stop().await.unwrap_err().failures()
    ));
}

#[tokio::test(start_paused = true)]
async fn unpolled_sender_owner_loss_is_failure_and_runs_abort_fallback() {
    let (port, probe) = port(false);
    let mut started = BACnetRouter::start(vec![port], RouterOptions::new())
        .await
        .unwrap();
    // On this current-thread runtime startup contains no pending await; this
    // abort precedes the sender owner's first poll.
    started.router.lifecycle.workers.as_mut().unwrap().senders[0]
        .1
        .abort();
    let error = started.router.stop().await.unwrap_err();
    assert!(matches!(
        &error.failures()[0],
        RouterStopFailure::Worker {
            network_number: Some(1),
            worker: "sender owner",
            ..
        }
    ));
    probe.resource_drop.await.unwrap();
    assert_eq!(probe.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(probe.stopped.load(Ordering::SeqCst), 0);
    assert_eq!(probe.aborted.load(Ordering::SeqCst), 1);
    assert!(probe.stop_entered.await.is_err());
}

#[tokio::test(start_paused = true)]
async fn transport_stop_panic_is_failure_and_does_not_skip_another_owner() {
    let (mut a, mut first) = port(false);
    let (mut b, mut second) = port(false);
    a.transport.panic_stop = true;
    b.network_number = 2;
    let mut started = BACnetRouter::start(vec![a, b], RouterOptions::new())
        .await
        .unwrap();
    let stop = started.router.stop();
    tokio::pin!(stop);
    tokio::select! {
        result = &mut stop => panic!("cleanup completed before release: {result:?}"),
        _ = async { (&mut first.stop_entered).await.unwrap(); (&mut second.stop_entered).await.unwrap(); } => {}
    }
    first.stop_release.send(()).unwrap();
    second.stop_release.send(()).unwrap();
    let error = stop.await.unwrap_err();
    assert_eq!(error.failures().len(), 1);
    assert!(matches!(&error.failures()[0], RouterStopFailure::Worker {
        network_number: Some(1), worker: "transport stop", detail,
    } if detail == "named transport stop panic"));
    first.resource_drop.await.unwrap();
    assert_eq!(first.stopped.load(Ordering::SeqCst), 1);
    assert_eq!(second.stopped.load(Ordering::SeqCst), 1);
    assert_eq!(second.dropped.load(Ordering::SeqCst), 1);
}
