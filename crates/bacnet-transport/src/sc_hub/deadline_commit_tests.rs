use super::deadline_test_support::*;
use super::heartbeat_test_support::{clients, LiveClient};
use super::*;
use std::time::Duration;

#[tokio::test]
async fn connect_deadline_releases_held_registry_without_evicting_uuid_owner() {
    let clients = clients();
    let mut old = LiveClient::connect(clients.clone(), [0x22; 6]).await;
    let old_sink = old.sink().await;
    let mut peer = DeadlinePeer::new(clients.clone(), Duration::from_secs(1)).await;
    tokio::time::pause();
    let map = clients.lock().await;
    peer.ws.send(request([0x42; 6], [0x22; 16])).await.unwrap();
    until(|| peer.deadline.admission_started.load(Ordering::Acquire)).await;
    // Tokio Sleep rounds to a millisecond tick. Leave two ticks for wakeup;
    // the separate readiness test below exercises the exact Instant boundary.
    tokio::time::advance(Duration::from_millis(1002)).await;
    poll_io(&mut peer.task).await.unwrap(); // must finish while registry stays locked
    assert_eq!(peer.active.load(Ordering::Acquire), 0);
    let incumbent = map.get(&old.vmac).unwrap();
    assert_eq!(map.len(), 1);
    assert!(Arc::ptr_eq(&incumbent.sink, &old_sink));
    assert!(!incumbent.closed.load(Ordering::Acquire));
    assert_eq!(
        (
            incumbent.device_uuid,
            incumbent.max_bvlc,
            incumbent.max_npdu
        ),
        ([0x22; 16], 1476, 1476)
    );
    assert!(!peer.deadline.is_committed());
    drop(map);
    assert!(matches!(peer.next().await, Message::Close(_)));
    tokio::time::resume();
    old.send(super::heartbeat_test_support::frame(
        ScFunction::HeartbeatRequest,
        77,
    ))
    .await;
    assert_eq!(old.recv().await.function, ScFunction::HeartbeatAck);
}

#[tokio::test]
async fn connect_deadline_exact_expiry_beats_ready_registry_and_request() {
    let clients = clients();
    let mut peer = DeadlinePeer::new(clients.clone(), Duration::from_secs(1)).await;
    tokio::time::pause();
    let map = clients.lock().await;
    peer.ws.send(request([0x42; 6], [0x42; 16])).await.unwrap();
    until(|| peer.deadline.admission_started.load(Ordering::Acquire)).await;
    tokio::time::advance(peer.deadline.expires() - tokio::time::Instant::now()).await;
    assert_eq!(tokio::time::Instant::now(), peer.deadline.expires());
    drop(map);
    poll_io(&mut peer.task).await.unwrap();
    assert!(clients.lock().await.is_empty());
    assert!(!peer.deadline.is_committed());
    assert!(matches!(peer.next().await, Message::Close(_)));
}

#[tokio::test]
async fn connect_commit_survives_ready_expiry_and_blocked_accept_then_cleans_up() {
    let clients = clients();
    let (server, mut ws, address, _) = TestTls::new().pair().await;
    let (write, read) = server.split();
    let sink = Arc::new(Mutex::new(write));
    let held = sink.clone().lock_owned().await;
    tokio::time::pause();
    let deadline = Arc::new(super::deadlines::ConnectDeadline::new(
        tokio::time::Instant::now() + Duration::from_secs(1),
    ));
    let mut handler = Box::pin(super::deadlines::serve(
        address,
        ([0x10; 6], [0x10; 16]),
        read,
        sink,
        clients.clone(),
        deadline.clone(),
        || {},
    ));
    ws.send(request([0x42; 6], [0x42; 16])).await.unwrap();
    tokio::time::advance(Duration::from_millis(999)).await;
    let started = std::time::Instant::now();
    loop {
        assert!(futures_util::poll!(&mut handler).is_pending());
        if clients.try_lock().unwrap().contains_key(&[0x42; 6]) {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(5));
        tokio::task::yield_now().await;
    }
    // The handler is not spawned: no poll can consume commit notification
    // before this advance makes both notification and expiry ready.
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(
        futures_util::poll!(&mut handler).is_pending(),
        "old deadline cancelled committed handler"
    );
    let map = clients.lock().await;
    let client = map.get(&[0x42; 6]).unwrap();
    assert!(!client.closed.load(Ordering::Acquire));
    assert_eq!(client.device_uuid, [0x42; 16]);
    drop(map);
    drop(held);
    let task = tokio::spawn(handler);
    let accept = poll_io(ws.next()).await.unwrap().unwrap();
    assert!(matches!(accept, Message::Binary(data) if data[0..4] == [7, 0, 0x22, 0x33]));
    ws.close(None).await.unwrap();
    poll_io(task).await.unwrap();
    assert!(
        clients.lock().await.is_empty(),
        "ordinary identity cleanup was bypassed"
    );
}

#[tokio::test]
async fn connect_deadline_bounds_preregistration_output_and_close_lock_waits() {
    for message in [
        Message::Binary(vec![0x0A, 0, 0, 1].into()),
        Message::Binary(vec![6, 0, 0, 1].into()),
        Message::Text("invalid text".into()),
    ] {
        let clients = clients();
        let mut peer = DeadlinePeer::new(clients.clone(), Duration::from_secs(1)).await;
        let held = peer.sink.clone().lock_owned().await;
        tokio::time::pause();
        peer.ws.send(message).await.unwrap();
        until(|| peer.deadline.received.load(Ordering::Acquire) != 0).await;
        tokio::time::advance(Duration::from_millis(1002)).await;
        until(|| peer.deadline.close_started.load(Ordering::Acquire)).await;
        assert!(
            !peer.task.is_finished(),
            "Close grace did not include the held sink"
        );
        tokio::time::advance(Duration::from_millis(1002)).await;
        poll_io(&mut peer.task).await.unwrap();
        assert_eq!(
            peer.active.load(Ordering::Acquire),
            0,
            "held output retained admission after Close grace"
        );
        assert!(!peer.deadline.is_committed());
        assert!(clients.lock().await.is_empty());
        drop(held);
        tokio::time::resume();
    }
}

#[tokio::test]
async fn connect_deadline_ignores_nonqualifying_traffic_without_restart_or_starvation() {
    let clients = clients();
    let mut peer = DeadlinePeer::new(clients.clone(), Duration::from_secs(1)).await;
    tokio::time::pause();
    for id in 1..=9u8 {
        tokio::time::advance(Duration::from_millis(100)).await;
        for message in [
            Message::Binary(vec![0xff].into()),
            Message::Binary(vec![0; 1477].into()),
            Message::Ping(vec![id].into()),
            Message::Binary(vec![0x0A, 0, 0, id, 0x42].into()),
        ] {
            peer.ws.send(message).await.unwrap();
        }
        loop {
            match peer.next().await {
                Message::Pong(_) => {}
                Message::Binary(data) => {
                    assert_eq!(&data[..], &[0, 0, 0, id, 0x0A, 1, 0, 0, 7, 0, 7]);
                    break;
                }
                other => panic!("unexpected response during flood: {other:?}"),
            }
        }
        assert!(!peer.deadline.is_committed());
    }
    tokio::time::advance(Duration::from_millis(102)).await;
    poll_io(&mut peer.task).await.unwrap();
    assert!(matches!(peer.next().await, Message::Close(_)));
    assert!(clients.lock().await.is_empty());
}
