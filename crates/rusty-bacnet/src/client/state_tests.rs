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
            .build_any::<crate::mstp_py::PySerial>()
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
