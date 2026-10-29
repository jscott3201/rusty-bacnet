//! Deterministic retained-join and failed-shutdown admission checks.

use super::*;
use std::net::Ipv4Addr;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::timeout;

async fn running_client() -> ClientInner {
    pyo3::Python::initialize();
    let state = Arc::new(ClientState::default());
    *state.client.write().await = Some(
        bacnet_client::client::BACnetClient::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .broadcast_address(Ipv4Addr::LOCALHOST)
            .port(0)
            .build_into()
            .await
            .unwrap(),
    );
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
        Err(Error::Encoding("injected shutdown failure".into()))
    }));
    let error = close(Arc::clone(&state)).await.unwrap_err();
    assert!(error.to_string().contains("injected shutdown failure"));
    assert!(state.client.read().await.is_some());
    assert!(state.closing.load(Ordering::Acquire));
    assert!(started(&state).await.is_err());
    assert!(starting(&state).await.is_err());
    timeout(Duration::from_secs(3), close(Arc::clone(&state)))
        .await
        .unwrap()
        .unwrap();
    assert!(state.client.read().await.is_none());
    assert!(!state.closing.load(Ordering::Acquire));
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
