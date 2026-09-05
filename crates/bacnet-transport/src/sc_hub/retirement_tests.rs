use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::heartbeat_test_support::ClockIo;
use super::shutdown_blocked_tests::ControlledPeer;
use super::*;
use std::time::Duration;

#[tokio::test]
async fn heartbeat_retirement_interrupts_blocked_ack_and_releases_admission() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut peer = ControlledPeer::open(&tls, &hub).await;
    peer.connect(0x42).await;
    let held = peer.sink.clone().lock_owned().await;
    peer.ws.send(heartbeat_wire(23)).await.unwrap();
    until(|| peer.deadline.received.load(Ordering::Acquire) == 2).await;
    {
        let mut map = hub.clients.lock().await;
        let client = map.get_mut(&[0x42; 6]).unwrap();
        client.heartbeat.generation = 1;
        client.heartbeat.pending = Some(heartbeat::PendingHeartbeat {
            message_id: 23,
            published_at: 100,
        });
    }
    tokio::time::pause();
    heartbeat::sweep(
        &hub.clients,
        &std::sync::atomic::AtomicU16::new(24),
        &ClockIo(AtomicU64::new(106)),
    )
    .await;
    assert!(!hub.clients.lock().await.contains_key(&[0x42; 6]));
    // Explicitly enter the owner cleanup wait before advancing its budget.
    until(|| peer.deadline.close_started.load(Ordering::Acquire)).await;
    tokio::time::advance(Duration::from_secs(6)).await;
    until(|| hub.active.load(Ordering::Acquire) == 0).await;
    assert!(hub.clients.lock().await.is_empty());
    drop(held);
    hub.hub.stop().await;
}

pub(super) fn unicast_wire(destination: Vmac) -> Message {
    let mut message = super::heartbeat_test_support::frame(ScFunction::EncapsulatedNpdu, 24);
    message.destination_vmac = Some(destination);
    message.payload = Bytes::from_static(&[1, 0]);
    let mut wire = BytesMut::new();
    encode_sc_message(&mut wire, &message);
    Message::Binary(wire.freeze())
}

#[tokio::test]
async fn retired_target_unblocks_healthy_sender_waiting_on_its_sink() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut source = ControlledPeer::open(&tls, &hub).await;
    source.connect(0x42).await;
    let mut target = ControlledPeer::open(&tls, &hub).await;
    target.connect(0x43).await;
    let held = target.sink.clone().lock_owned().await;
    source.ws.send(unicast_wire([0x43; 6])).await.unwrap();
    until(|| source.deadline.received.load(Ordering::Acquire) == 2).await;
    let attempt = heartbeat::Attempt {
        vmac: [0x43; 6],
        sink: target.sink.clone(),
        generation: 0,
    };
    assert!(
        heartbeat::retire(
            &hub.clients,
            &attempt,
            heartbeat::Retirement::SendFailed,
            &ClockIo(AtomicU64::new(106))
        )
        .await
    );
    source.ws.send(heartbeat_wire(25)).await.unwrap();
    assert!(
        matches!(tokio::time::timeout(Duration::from_millis(250), source.ws.next()).await,
        Ok(Some(Ok(Message::Binary(data)))) if data[0..4] == [0x0B, 0, 0, 25]),
        "retired target stranded or closed its healthy sender"
    );
    assert!(hub.clients.lock().await.contains_key(&[0x42; 6]));
    drop(held);
    hub.hub.stop().await;
}

#[tokio::test]
async fn terminal_relay_writes_retire_failed_target_only() {
    for kind in 0..3 {
        let tls = TestTls::new();
        let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
        let mut source = ControlledPeer::open(&tls, &hub).await;
        source.connect(0x42).await;
        let mut target = ControlledPeer::open(&tls, &hub).await;
        target.connect(0x43).await;
        // Put the real target sink in closing state without polling its peer to
        // echo Close. The next relay write returns Protocol(SendAfterClosing).
        target
            .sink
            .lock()
            .await
            .send(Message::Close(None))
            .await
            .unwrap();
        let mut healthy = ControlledPeer::open(&tls, &hub).await;
        healthy.connect(0x44).await;
        let wire = match kind {
            0 => unicast_wire([0x43; 6]),
            1 => unicast_wire([0xFF; 6]),
            _ => {
                let mut frame = super::heartbeat_test_support::frame(ScFunction::Result, 24);
                frame.destination_vmac = Some([0x43; 6]);
                frame.payload = Bytes::from_static(&[1, 0]); // ACK for Encapsulated-NPDU
                encode(frame)
            }
        };
        source.ws.send(wire).await.unwrap();
        source.ws.send(heartbeat_wire(25)).await.unwrap();
        assert!(matches!(poll_io(source.ws.next()).await,
        Some(Ok(Message::Binary(data))) if data[0..4] == [0x0B, 0, 0, 25]));
        assert!(
            !hub.clients.lock().await.contains_key(&[0x43; 6]),
            "terminal relay failure kept target registered"
        );
        until(|| hub.active.load(Ordering::Acquire) == 2).await;
        assert!(hub.clients.lock().await.contains_key(&[0x42; 6]));
        if kind == 1 {
            assert!(matches!(poll_io(healthy.ws.next()).await,
            Some(Ok(Message::Binary(data))) if decode_sc_message(&data).unwrap().payload.as_ref() == [1, 0]));
        }
        healthy.ws.send(heartbeat_wire(26)).await.unwrap();
        assert!(matches!(poll_io(healthy.ws.next()).await,
        Some(Ok(Message::Binary(data))) if data[0..4] == [0x0B, 0, 0, 26]));
        hub.hub.stop().await;
    }
}

pub(super) fn encode(message: ScMessage) -> Message {
    let mut wire = BytesMut::new();
    encode_sc_message(&mut wire, &message);
    Message::Binary(wire.freeze())
}

pub(super) fn heartbeat_wire(id: u16) -> Message {
    encode(super::heartbeat_test_support::frame(
        ScFunction::HeartbeatRequest,
        id,
    ))
}
