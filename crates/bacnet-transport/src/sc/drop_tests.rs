use super::*;
use tokio::time::timeout;

async fn hub_accept(ws_hub: &LoopbackWebSocket, hub_vmac: Vmac) {
    let data = ws_hub.recv().await.unwrap();
    let req = decode_sc_message(&data).unwrap();
    assert_eq!(req.function, ScFunction::ConnectRequest);

    let mut accept_payload = Vec::with_capacity(26);
    accept_payload.extend_from_slice(&hub_vmac);
    accept_payload.extend_from_slice(&[0x33; 16]);
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());

    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(accept_payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &accept);
    ws_hub.send(&buf).await.unwrap();
}

async fn assert_closed_without_post_drop_heartbeat(ws_hub: &LoopbackWebSocket, context: &str) {
    timeout(Duration::from_secs(1), async {
        while let Ok(data) = ws_hub.recv().await {
            let msg = decode_sc_message(&data).unwrap();
            assert_ne!(
                msg.function,
                ScFunction::HeartbeatAck,
                "{context} must not leave SC answering heartbeats"
            );
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{context} did not close the loopback WebSocket"));

    let heartbeat = ScMessage {
        function: ScFunction::HeartbeatRequest,
        message_id: 0x66,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &heartbeat);
    assert!(
        ws_hub.send(&buf).await.is_err(),
        "{context} must reject post-drop Heartbeat-Request on the closed socket"
    );
}

async fn start_loopback_sc(
    client_vmac: Vmac,
    hub_vmac: Vmac,
) -> (ScTransport<LoopbackWebSocket>, LoopbackWebSocket) {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, client_vmac).with_device_uuid([1; 16]);
    let hub_task = tokio::spawn(async move {
        hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let _rx = transport.start().await.unwrap();
    (transport, hub_task.await.unwrap())
}

#[tokio::test]
async fn sc_drop_aborts_recv_task_and_closes_loopback_socket() {
    let client_vmac = [0x01; 6];
    let hub_vmac = [0x10; 6];
    let (transport, ws_hub) = start_loopback_sc(client_vmac, hub_vmac).await;
    let conn = transport.connection().unwrap().clone();
    let abort_handle = transport
        .recv_task
        .lock()
        .unwrap()
        .as_ref()
        .expect("SC start spawns recv task")
        .abort_handle();

    assert!(!abort_handle.is_finished());
    drop(transport);

    timeout(Duration::from_secs(1), async {
        while !abort_handle.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropping SC transport aborts recv task");

    assert_eq!(conn.lock().await.state, ScConnectionState::Disconnected);

    assert_closed_without_post_drop_heartbeat(&ws_hub, "dropped SC transport").await;

    let (replacement, _replacement_hub) = start_loopback_sc(client_vmac, hub_vmac).await;
    replacement.stop().await.unwrap();
}

#[tokio::test]
async fn sc_stop_sends_disconnect_and_drop_after_stop_is_idempotent() {
    let (transport, ws_hub) = start_loopback_sc([0x01; 6], [0x10; 6]).await;
    let abort_handle = transport
        .recv_task
        .lock()
        .unwrap()
        .as_ref()
        .expect("SC start spawns recv task")
        .abort_handle();

    transport.stop().await.unwrap();

    let data = timeout(Duration::from_secs(1), ws_hub.recv())
        .await
        .expect("SC stop sends Disconnect-Request before closing socket")
        .expect("hub socket receives Disconnect-Request");
    let msg = decode_sc_message(&data).unwrap();
    assert_eq!(msg.function, ScFunction::DisconnectRequest);
    assert!(abort_handle.is_finished());

    drop(transport);
}

/// A stop through a shared reference, with another holder alive (#2617):
/// the hub hears the Disconnect-Request, the hub socket leaves the send
/// slot, every later send fails with the stopped error, the receiver ends,
/// and a stop again or at once beside it changes nothing.
#[tokio::test]
async fn sc_stop_through_a_shared_reference_fails_every_later_send() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6]).with_device_uuid([1; 16]);
    let hub = tokio::spawn(async move {
        hub_accept(&ws_hub, [0x10; 6]).await;
        ws_hub
    });
    let mut rx = transport.start().await.unwrap();
    let ws_hub = hub.await.unwrap();
    let transport = Arc::new(transport);
    let other = Arc::clone(&transport);
    transport
        .send_unicast(&[0x01, 0x00], &[0x20; 6])
        .await
        .unwrap();
    ws_hub.recv().await.unwrap();

    let (first, second) = tokio::join!(transport.stop(), other.stop());
    first.unwrap();
    second.unwrap();
    let data = timeout(Duration::from_secs(1), ws_hub.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        decode_sc_message(&data).unwrap().function,
        ScFunction::DisconnectRequest
    );
    assert!(
        other.ws_shared.as_ref().unwrap().lock().await.is_none(),
        "the hub socket left the send slot"
    );
    for sent in [
        other.send_unicast(&[0x01, 0x00], &[0x20; 6]).await,
        other.send_broadcast(&[0x01, 0x00]).await,
        other
            .send_unicast_with_data_attributes(&[0x01, 0x00], &[0x20; 6], &[])
            .await,
    ] {
        assert!(
            matches!(&sent, Err(Error::State(Lifecycle::Closed, _))),
            "{sent:?}"
        );
    }
    assert!(
        timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .is_none()
    );
    other.stop().await.unwrap();
}

/// A start's error once the transport was stopped says so.
fn stopped_start<T: std::fmt::Debug>(started: Result<T, Error>) -> bool {
    let error = started.expect_err("a stopped transport started");
    let message = error.to_string();
    matches!(error, Error::State(Lifecycle::Closed, _))
        && message.contains("was stopped")
        && message.contains("cannot start again")
}

/// A send before start is refused as one to a transport not running yet,
/// which a start would let through (#1833): nothing was sent, and it is not
/// the link's failure.
#[tokio::test]
async fn sc_send_before_start_is_refused_as_not_running() {
    let (ws_client, _ws_hub) = LoopbackWebSocket::pair();
    let transport = ScTransport::new(ws_client, [0x01; 6]).with_device_uuid([1; 16]);
    for sent in [
        transport.send_unicast(&[0x01, 0x00], &[0x20; 6]).await,
        transport.send_broadcast(&[0x01, 0x00]).await,
    ] {
        assert!(
            matches!(&sent, Err(Error::State(Lifecycle::NotRunning, _))),
            "{sent:?}"
        );
    }
}

/// A stop before start lets go of the socket it was given, as a drop
/// would, and a later start fails as one after a stop does (#2617).
#[tokio::test]
async fn sc_stop_before_start_lets_go_of_the_socket() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6]).with_device_uuid([1; 16]);
    transport.stop().await.unwrap();
    let closed = timeout(Duration::from_secs(1), ws_hub.recv())
        .await
        .unwrap();
    assert!(closed.is_err(), "the socket closed");
    // A send finds it closed for good, as the start does (#1833).
    let sent = transport.send_unicast(&[0x01, 0x00], &[0x20; 6]).await;
    assert!(
        matches!(&sent, Err(Error::State(Lifecycle::Closed, _))),
        "{sent:?}"
    );
    assert!(stopped_start(transport.start().await));
}

/// A transport stopped after it ran doesn't start again either, and says
/// why rather than that it is already running.
#[tokio::test]
async fn sc_start_after_a_stop_says_the_transport_was_stopped() {
    let (mut transport, _ws_hub) = start_loopback_sc([0x01; 6], [0x10; 6]).await;
    transport.stop().await.unwrap();
    assert!(stopped_start(transport.start().await));
}

/// An abort through a shared reference, with another holder alive (#2617):
/// nothing is said to the hub, the hub socket leaves the send slot at once
/// and closes as the runtime drops the aborted receive task, every later
/// send fails with the stopped error, and the transport does not start
/// again.
#[tokio::test]
async fn sc_abort_through_a_shared_reference_closes_the_hub_socket_without_a_goodbye() {
    let (transport, ws_hub) = start_loopback_sc([0x01; 6], [0x10; 6]).await;
    let transport = Arc::new(transport);
    let other = Arc::clone(&transport);
    other.abort();
    assert!(
        transport
            .ws_shared
            .as_ref()
            .unwrap()
            .try_lock()
            .unwrap()
            .is_none(),
        "the hub socket left the send slot"
    );
    for sent in [
        transport.send_unicast(&[0x01, 0x00], &[0x20; 6]).await,
        transport.send_broadcast(&[0x01, 0x00]).await,
    ] {
        assert!(
            matches!(&sent, Err(Error::State(Lifecycle::Closed, _))),
            "{sent:?}"
        );
    }
    timeout(Duration::from_secs(1), async {
        while let Ok(data) = ws_hub.recv().await {
            let function = decode_sc_message(&data).unwrap().function;
            assert_ne!(function, ScFunction::DisconnectRequest, "no goodbye");
        }
    })
    .await
    .expect("the hub socket closed");
    drop(other);
    let mut transport = Arc::into_inner(transport).unwrap();
    assert!(stopped_start(transport.start().await));
}

/// An abort from a thread with no runtime, while a send holds the hub
/// slot, still fails every later send at once, and the socket leaves the
/// slot on the transport's runtime once the holder lets go (#2617).
#[tokio::test]
async fn sc_abort_off_the_runtime_beside_a_held_hub_slot_lets_go_once_it_is_free() {
    let (transport, ws_hub) = start_loopback_sc([0x01; 6], [0x10; 6]).await;
    let transport = Arc::new(transport);
    let slot = Arc::clone(transport.ws_shared.as_ref().unwrap());
    let held = slot.lock().await;
    let other = Arc::clone(&transport);
    std::thread::spawn(move || other.abort()).join().unwrap();
    drop(held);
    // Before the runtime runs anything: the socket is still in the slot.
    let sent = transport.send_unicast(&[0x01, 0x00], &[0x20; 6]).await;
    assert!(
        matches!(&sent, Err(Error::State(Lifecycle::Closed, _))),
        "{sent:?}"
    );
    timeout(Duration::from_secs(1), async {
        while ws_hub.recv().await.is_ok() {}
    })
    .await
    .expect("the hub socket closed");
    assert!(slot.lock().await.is_none(), "the socket left the slot");
}

/// A socket the receive task publishes as it is aborted, after the abort
/// emptied the slot, leaves the slot as the task drops (#2617).
#[tokio::test]
async fn sc_a_socket_published_as_the_abort_lands_leaves_the_slot_with_the_task() {
    let (transport, _ws_hub) = start_loopback_sc([0x01; 6], [0x10; 6]).await;
    let slot = Arc::clone(transport.ws_shared.as_ref().unwrap());
    let late = slot.lock().await.clone();
    transport.abort();
    // The task's late publish, before the runtime has dropped it.
    let mut published = slot.try_lock().expect("the abort let go of the slot");
    assert!(published.is_none(), "the abort emptied the slot");
    *published = late;
    drop(published);
    timeout(Duration::from_secs(1), async {
        while slot.lock().await.is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the task's drop took the late socket out");
}

/// A stop after an abort joins the receive task the abort left (#2617).
#[tokio::test]
async fn sc_a_stop_after_an_abort_joins_the_receive_task() {
    let (transport, _ws_hub) = start_loopback_sc([0x01; 6], [0x10; 6]).await;
    let receive = transport
        .recv_task
        .lock()
        .unwrap()
        .as_ref()
        .expect("SC start spawns recv task")
        .abort_handle();
    transport.abort();
    assert!(
        transport.recv_task.lock().unwrap().is_some(),
        "kept for a stop to join"
    );
    transport.stop().await.unwrap();
    assert!(transport.recv_task.lock().unwrap().is_none());
    assert!(receive.is_finished(), "the stop joined it");
}

/// With direct connections on, an abort lets go of the NPDU sender they
/// deliver through, so the receiver ends, as after a stop (#2617).
#[cfg(feature = "sc-tls")]
#[tokio::test]
async fn sc_an_abort_with_direct_connections_on_ends_the_receiver() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6])
        .with_device_uuid([1; 16])
        .with_direct_discovery(true);
    let hub = tokio::spawn(async move {
        hub_accept(&ws_hub, [0x10; 6]).await;
        ws_hub
    });
    let mut rx = transport.start().await.unwrap();
    let _ws_hub = hub.await.unwrap();
    transport.abort();
    let ended = timeout(Duration::from_secs(1), rx.recv()).await;
    assert!(ended.expect("the receiver ends").is_none());
}
