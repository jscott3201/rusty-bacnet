//! Deterministic retained-join and failed-shutdown admission checks.

use super::*;
use std::net::Ipv4Addr;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::timeout;

pub(super) async fn running_client() -> ClientInner {
    pyo3::Python::initialize();
    let state = Arc::new(ClientState::default());
    *state.client.write().await = Some(Started::new(
        bacnet_client::client::BACnetClient::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .broadcast_address(Ipv4Addr::LOCALHOST)
            .port(0)
            .build_into()
            .await
            .unwrap(),
        bacnet_ops::WritePolicy::default(),
        bacnet_ops::ResolveScope::Local,
    ));
    state
}

#[tokio::test]
async fn cancelled_waiter_retains_join_after_native_stop_completes() {
    let state = running_client().await;
    let native = Arc::clone(&state);
    let (stopped_tx, stopped_rx) = oneshot::channel();
    let (finish_tx, finish_rx) = oneshot::channel();
    state.closing.store(true, Ordering::Release);
    *state.shutdown.lock().await = Some(tokio::spawn(async move {
        // Run the actual native shutdown, then hold task completion at a
        // deterministic test-only gate. This tests retained join ownership,
        // not a claimed cancellation point inside a lower-layer stop method.
        stop_native(native).await?;
        stopped_tx.send(()).unwrap();
        finish_rx.await.unwrap();
        Ok(())
    }));
    timeout(Duration::from_secs(3), stopped_rx)
        .await
        .unwrap()
        .unwrap();
    let waiter = tokio::spawn(close(Arc::clone(&state)));
    timeout(Duration::from_secs(3), async {
        while state.shutdown.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert!(state.shutdown.lock().await.is_some());
    assert!(state.closing.load(Ordering::Acquire));
    assert!(started(&state).await.is_err());
    assert!(starting(&state).await.is_err());
    finish_tx.send(()).unwrap();
    timeout(Duration::from_secs(3), close(Arc::clone(&state)))
        .await
        .unwrap()
        .unwrap();
    assert!(state.shutdown.lock().await.is_none());
    assert!(state.client.read().await.is_none());
    assert!(!state.closing.load(Ordering::Acquire));
}

#[tokio::test]
async fn failed_shutdown_keeps_client_closed_to_operations_until_retry() {
    let state = running_client().await;
    state.closing.store(true, Ordering::Release);
    *state.shutdown.lock().await = Some(tokio::spawn(async {
        Err(Error::Transport(std::io::Error::other(
            "injected shutdown failure",
        )))
    }));
    let error = close(Arc::clone(&state)).await.unwrap_err();
    assert!(error.to_string().contains("injected shutdown failure"));
    assert!(state.client.read().await.is_some());
    assert!(state.closing.load(Ordering::Acquire));
    // An endpoint asked for a client learns why, not that it is stopping
    // forever (#2617).
    assert!(
        matches!(state.liveness(), Liveness::Failed(cause) if cause.contains("injected shutdown failure"))
    );
    assert!(started(&state).await.is_err());
    assert!(starting(&state).await.is_err());
    timeout(Duration::from_secs(3), close(Arc::clone(&state)))
        .await
        .unwrap()
        .unwrap();
    assert!(state.client.read().await.is_none());
    assert!(!state.closing.load(Ordering::Acquire));
    assert_eq!(state.liveness(), Liveness::Stopped);
}

/// A call through the high-level client holds the same lease as any other,
/// and its handle lets go of the low-level client when it ends, so the stop
/// that follows owns the client and stops it (#2561).
#[tokio::test]
async fn a_stop_after_a_high_level_call_owns_the_client() {
    let state = running_client().await;
    {
        let high = high_level(&state).await.unwrap();
        let station = bacnet_ops::DeviceAddress::local(&[127, 0, 0, 1, 0xBA, 0xC0]).unwrap();
        let device = high.device(station).await.unwrap();
        assert!(!device.follows());
    }
    timeout(Duration::from_secs(3), close(Arc::clone(&state)))
        .await
        .unwrap()
        .unwrap();
    assert!(state.client.read().await.is_none());
    assert!(high_level(&state).await.is_err());
}

/// Poll `future` once, so a lock acquisition in it joins the lock's queue.
async fn poll_once<F: std::future::Future + Unpin>(future: &mut F) -> bool {
    std::future::poll_fn(|cx| {
        std::task::Poll::Ready(std::pin::Pin::new(&mut *future).poll(cx).is_pending())
    })
    .await
}

/// An operation and an entry queued behind a pending entry before close is
/// admitted fail with "closing" once it is, instead of waiting for the entry
/// to finish (#2030: the flake hunt's probe hung behind a held entry).
#[tokio::test]
async fn queued_work_behind_a_pending_entry_learns_of_close_at_once() {
    pyo3::Python::initialize();
    let state = Arc::new(ClientState::default());
    // The pending entry: an exclusive hold on the slot, as `starting` keeps.
    let entry = state.client.write().await;
    let mut operation = Box::pin(started(&state));
    let mut second_entry = Box::pin(starting(&state));
    assert!(poll_once(&mut operation).await, "queued behind the entry");
    assert!(
        poll_once(&mut second_entry).await,
        "queued behind the entry"
    );
    // Close is admitted at once; its native stop waits for the entry.
    let closing = tokio::spawn(close(Arc::clone(&state)));
    let error = timeout(Duration::from_secs(3), operation)
        .await
        .expect("the queued operation sees the close")
        .err()
        .expect("refused");
    assert!(error.to_string().contains("closing"), "{error}");
    let error = timeout(Duration::from_secs(3), second_entry)
        .await
        .expect("the queued entry sees the close")
        .err()
        .expect("refused");
    assert!(error.to_string().contains("closing"), "{error}");
    assert!(!closing.is_finished(), "close still waits for the entry");
    drop(entry);
    timeout(Duration::from_secs(3), closing)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!state.closing.load(Ordering::Acquire));
}

/// A Python `Device` that goes away takes its handle out of the started
/// client's map, so the map holds only handles someone can still call.
#[tokio::test]
async fn a_dropped_tie_forgets_its_handle_while_the_client_runs() {
    let state = running_client().await;
    let tie = crate::client::DeviceTie::new(Arc::clone(&state));
    let other = crate::client::DeviceTie::new(Arc::clone(&state));
    for tie in [&tie, &other] {
        let held = tie
            .hold(|client| async move {
                let station =
                    bacnet_ops::DeviceAddress::local(&[127, 0, 0, 1, 0xBA, 0xC0]).unwrap();
                Ok(client.device(station).await.unwrap())
            })
            .await
            .unwrap();
        drop(held);
        assert!(tie.peek(|_| ()).is_some());
    }
    assert_eq!(state.handles.lock().unwrap().len(), 2);
    drop(tie);
    // The client is still started, and only the other's handle is left.
    assert!(started(&state).await.is_ok());
    assert_eq!(state.handles.lock().unwrap().len(), 1);
    assert!(other.peek(|_| ()).is_some());
    close(Arc::clone(&state)).await.unwrap();
    assert!(other.peek(|_| ()).is_none(), "a stop drops the rest");
}

/// A native stop that fails is recorded by the stop itself, so a stop()
/// whose waiter went away still leaves the cause (#2617); a later stop,
/// once the stray reference is gone, finishes.
#[tokio::test]
async fn a_failed_native_stop_is_recorded_without_a_waiter() {
    let state = running_client().await;
    let held = state.hold_core_for_test().await.expect("started");
    state.closing.store(true, Ordering::Release);
    let task = tokio::spawn(stop_native(Arc::clone(&state)));
    let error = timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("references remain"), "{error}");
    assert!(
        matches!(state.liveness(), Liveness::Failed(cause) if cause.contains("references remain"))
    );
    drop(held);
    timeout(Duration::from_secs(3), close(Arc::clone(&state)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.liveness(), Liveness::Stopped);
}

/// The `state` of the `BacnetStateError` a lifecycle check raised (#2819).
fn state_of(error: pyo3::PyErr) -> String {
    use pyo3::prelude::*;
    Python::attach(|py| {
        let class = crate::errors::state_error_type(py).unwrap().bind(py);
        assert!(error.value(py).get_type().is(class), "{error}");
        error.value(py).getattr("state").unwrap().extract().unwrap()
    })
}

/// Each of the client's own lifecycle checks raises `BacnetStateError`
/// naming its state (#2819): a second start, a call once a standalone
/// client stopped (it starts again), a call while a stop is under way, and
/// a call on an endpoint's client once it stopped (it never starts again).
#[tokio::test]
async fn lifecycle_refusals_name_their_state() {
    let state = running_client().await;
    assert_eq!(
        state_of(starting(&state).await.err().unwrap()),
        "already_started"
    );
    close(Arc::clone(&state)).await.unwrap();
    assert_eq!(
        state_of(started(&state).await.err().unwrap()),
        "not_running"
    );
    assert_eq!(
        state_of(high_level(&state).await.err().unwrap()),
        "not_running"
    );
    state.closing.store(true, Ordering::Release);
    assert_eq!(state_of(started(&state).await.err().unwrap()), "stopping");
    assert_eq!(state_of(starting(&state).await.err().unwrap()), "stopping");
    let seated: ClientInner = Arc::new(ClientState::seated());
    assert_eq!(state_of(started(&seated).await.err().unwrap()), "closed");
}
