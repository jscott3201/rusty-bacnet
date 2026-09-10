use super::deadline_test_support::*;
use super::*;
use std::time::Duration;

#[tokio::test]
async fn hub_stop_releases_established_peer_before_returning() {
    let tls = TestTls::new();
    let mut hub = ScHub::start("127.0.0.1:0", tls.hub_config.clone(), [0x10; 6])
        .await
        .unwrap();
    let address = hub.local_addr().unwrap();
    let mut peer = tls.websocket(address).await;
    peer.send(request([0x42; 6], [0x42; 16])).await.unwrap();
    assert!(matches!(poll_io(peer.next()).await,
        Some(Ok(Message::Binary(data))) if data[0] == 7));

    hub.stop().await;
    assert!(
        matches!(
            tokio::time::timeout(Duration::from_millis(250), peer.next()).await,
            Ok(None | Some(Err(_)) | Some(Ok(Message::Close(_))))
        ),
        "stop returned with an established connection still alive"
    );
    let _rebound = TcpListener::bind(address).await.unwrap();
    hub.stop().await;
}

use super::deadline_capacity_tests::CountedHub;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

async fn registered(tls: &TestTls, hub: &CountedHub, id: u8) -> ClientWs {
    let mut ws = tls.websocket(hub.address).await;
    ws.send(request([id; 6], [id; 16])).await.unwrap();
    assert!(matches!(poll_io(ws.next()).await,
        Some(Ok(Message::Binary(data))) if data[0] == 7));
    ws
}

async fn closed(ws: &mut ClientWs) {
    assert!(matches!(
        poll_io(ws.next()).await,
        None | Some(Err(_)) | Some(Ok(Message::Close(_)))
    ));
}

#[tokio::test]
async fn hub_stop_drains_every_accepted_phase_and_registry() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut tls_pending = TcpStream::connect(hub.address).await.unwrap();
    until(|| hub.active.load(Ordering::Acquire) == 1).await;
    let mut upgrade_pending = tls
        .connect_tls(TcpStream::connect(hub.address).await.unwrap())
        .await;
    let mut connect_pending = tls.websocket(hub.address).await;
    let mut established = registered(&tls, &hub, 0x42).await;
    until(|| hub.active.load(Ordering::Acquire) == 4).await;
    assert_eq!(hub.clients.lock().await.len(), 1);
    let socket = Arc::downgrade(&hub.clients.lock().await.get(&[0x42; 6]).unwrap().sink);

    poll_io(hub.hub.stop()).await;
    assert_eq!(hub.active.load(Ordering::Acquire), 0);
    assert_eq!(hub.hub.tasks.len(), 0);
    assert!(hub.clients.lock().await.is_empty());
    assert!(socket.upgrade().is_none(), "hub retained established sink");
    let mut byte = [0];
    assert!(matches!(
        poll_io(tls_pending.read(&mut byte)).await,
        Ok(0) | Err(_)
    ));
    assert!(matches!(
        poll_io(upgrade_pending.read(&mut byte)).await,
        Ok(0) | Err(_)
    ));
    closed(&mut connect_pending).await;
    closed(&mut established).await;
    let _rebound = TcpListener::bind(hub.address).await.unwrap();
}

#[tokio::test]
async fn cancelled_stop_retains_join_and_resumes_after_cleanup_lock() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut peer = registered(&tls, &hub, 0x42).await;
    let registry = hub.clients.clone();
    let held = registry.lock().await;
    {
        let mut stopping = Box::pin(hub.hub.stop());
        // Force cleanup to remain pending on the externally held registry.
        poll_io(async {
            loop {
                assert!(futures_util::poll!(&mut stopping).is_pending());
                if hub.active.load(Ordering::Acquire) == 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
    }
    assert!(
        hub.hub.listener_task.is_some(),
        "cancelled stop lost its join"
    );
    drop(held);
    poll_io(hub.hub.stop()).await;
    assert!(hub.hub.listener_task.is_none());
    assert!(registry.lock().await.is_empty());
    assert_eq!(hub.active.load(Ordering::Acquire), 0);
    closed(&mut peer).await;
    hub.hub.stop().await;
}

#[tokio::test]
async fn drop_requests_eventual_cleanup_on_live_runtime() {
    let tls = TestTls::new();
    let hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut peer = registered(&tls, &hub, 0x42).await;
    let address = hub.address;
    let active = hub.active.clone();
    let clients = hub.clients.clone();
    let tasks = hub.hub.tasks.clone();
    drop(hub);
    until(|| active.load(Ordering::Acquire) == 0).await;
    poll_io(async {
        while !clients.lock().await.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(tasks.len(), 0);
    closed(&mut peer).await;
    let _rebound = TcpListener::bind(address).await.unwrap();
}

#[tokio::test]
async fn stop_and_drop_before_supervisor_first_poll_are_sticky() {
    let tls = TestTls::new();
    for stop in [true, false] {
        let mut hub = ScHub::start("127.0.0.1:0", tls.hub_config.clone(), [0x10; 6])
            .await
            .unwrap();
        let address = hub.local_addr().unwrap();
        let task = hub.listener_task.as_ref().unwrap().abort_handle();
        if stop {
            hub.stop().await;
        }
        drop(hub);
        until(|| task.is_finished()).await;
        let _rebound = TcpListener::bind(address).await.unwrap();
    }
}

#[tokio::test]
async fn supervisor_reaps_completed_connections_while_running() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    for _ in 0..40 {
        let peer = TcpStream::connect(hub.address).await.unwrap();
        until(|| hub.active.load(Ordering::Acquire) == 1).await;
        drop(peer);
        until(|| hub.active.load(Ordering::Acquire) == 0).await;
        until(|| hub.hub.tasks.len() == 1).await; // only the live heartbeat worker
    }
    hub.hub.stop().await;
    assert_eq!(hub.hub.tasks.len(), 0);
}
