//! The diagnostics handle: transport-boundary counts and the core's
//! counters, through a run and after it.

use bacnet_mstp_codec::FrameType;
use bytes::Bytes;
use tokio::time::Duration;

use super::test_support::{config, encode, Bus, Sent, T_REPLY_TIMEOUT};
use super::{
    LoopbackSerial, MstpDiagnosticsSnapshot, MstpExecutionMode, MstpTransport, SerialPort,
};
use crate::port::TransportPort;

/// As station 7, send `frame_type` to the station once T_turnaround has
/// passed after `after`.
async fn answer(bus: &mut Bus, after: &Sent, frame_type: FrameType, data: &[u8]) {
    bus.until(after.end + bus.turnaround).await;
    bus.send(frame_type, 3, 7, data);
}

/// Frames are counted by kind as they complete on the wire: a direct
/// answer, Reply Postponed, queued data and a request, with the core's
/// receive counts beside them. Counts outlive the transport.
#[tokio::test(start_paused = true)]
async fn frames_are_counted_by_kind_and_outlive_the_transport() {
    let mut bus = Bus::start(config(3, 7)).await;
    let observer = bus.transport.diagnostics();
    let token = bus.join(7).await;

    // A request answered directly, then one postponed.
    answer(
        &mut bus,
        &token,
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        &[1, 4, 0x10],
    )
    .await;
    let reply = bus.npdus.recv().await.unwrap().reply_tx.unwrap();
    reply.send(Bytes::from_static(&[1, 0, 0x30])).unwrap();
    let direct = bus.next().await;
    assert_eq!(
        direct.frame_type,
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
    );
    answer(
        &mut bus,
        &direct,
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        &[1, 4, 0x11],
    )
    .await;
    drop(bus.npdus.recv().await.unwrap());
    let postponed = bus.next().await;
    assert_eq!(postponed.frame_type, FrameType::REPLY_POSTPONED);

    // Queued data with one token, a request with the next.
    bus.transport
        .send_unicast(&[1, 0, 0x30], &[7])
        .await
        .unwrap();
    bus.transport
        .send_unicast(&[1, 4, 0x40], &[7])
        .await
        .unwrap();
    answer(&mut bus, &postponed, FrameType::TOKEN, &[]).await;
    let queued = bus.next().await;
    assert_eq!(
        queued.frame_type,
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
    );
    let token = bus.next().await;
    assert_eq!(
        token.frame_type,
        FrameType::TOKEN,
        "one frame per token use"
    );
    answer(&mut bus, &token, FrameType::TOKEN, &[]).await;
    let request = bus.next().await;
    assert_eq!(request.frame_type, FrameType::BACNET_DATA_EXPECTING_REPLY);
    answer(&mut bus, &request, FrameType::REPLY_POSTPONED, &[]).await;
    let last = bus.next().await;
    assert_eq!(last.frame_type, FrameType::TOKEN);
    // Counted once its last octet has left.
    bus.until(last.end + Duration::from_millis(1)).await;

    bus.stop().await;
    let snapshot = observer.snapshot();
    assert_eq!(snapshot.der_tx, 1);
    assert_eq!(snapshot.dner_tx_direct, 1);
    assert_eq!(snapshot.dner_tx_queued, 1);
    assert_eq!(snapshot.reply_postponed_tx, 1);
    assert_eq!(snapshot.wait_for_reply_timeouts, 0);
    assert_eq!(snapshot.mac.npdus_delivered, 2);
    assert_eq!(snapshot.mac.frames_received, 7);
    assert_eq!(snapshot.mac.unicast.enqueued, 2);
    assert_eq!(snapshot.mac.unicast.dequeued, 2);
    assert_eq!(snapshot.mac.token_rotations, 3);
    assert_eq!(snapshot.mac.adapter_errors, 0);
    assert_eq!(snapshot.mac.internal_errors, 0);
    assert_eq!(
        observer.snapshot(),
        snapshot,
        "counts survive the transport"
    );
}

/// The handle publishes the node's place in the ring: after joining a
/// ring with station 7 the station passes its token there and is no sole
/// master, and the last state stays readable after the transport stops.
#[tokio::test(start_paused = true)]
async fn the_node_state_follows_the_ring_and_outlives_the_transport() {
    let mut bus = Bus::start(config(3, 7)).await;
    let observer = bus.transport.diagnostics();
    let token = bus.join(7).await;
    assert_eq!(token.destination, 7);
    let node = observer.node_state().expect("published after an event");
    assert_eq!(node.next_station, 7);
    assert!(!node.sole_master);
    bus.stop().await;
    assert_eq!(observer.node_state(), Some(node));
}

/// A wait for a reply that ends in T_reply_timeout is counted.
#[tokio::test(start_paused = true)]
async fn reply_timeouts_are_counted() {
    let mut bus = Bus::start(config(3, 4)).await;
    let token = bus.join(4).await;
    bus.transport
        .send_unicast(&[1, 4, 0x10], &[4])
        .await
        .unwrap();
    bus.return_token(&token, 4).await;
    let request = bus.next().await;
    assert_eq!(request.frame_type, FrameType::BACNET_DATA_EXPECTING_REPLY);
    let token = bus.next().await;
    assert_eq!(token.frame_type, FrameType::TOKEN);
    assert!(token.start >= request.end + T_REPLY_TIMEOUT);
    assert!(
        token.start <= request.end + T_REPLY_TIMEOUT + bus.turnaround + Duration::from_millis(1)
    );
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.wait_for_reply_timeouts, 1);
    assert_eq!(counts.der_tx, 1);
    bus.stop().await;
}

/// NPDUs that find the receive channel full or closed are counted and
/// dropped; a request among them still gets Reply Postponed.
#[tokio::test(start_paused = true)]
async fn ingress_drops_are_counted() {
    let mut bus = Bus::start(config(3, 127)).await;
    for _ in 0..65 {
        bus.send(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 3, 7, &[1, 0]);
    }
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(bus.transport.diagnostics().snapshot().ingress_full, 1);
    assert_eq!(bus.npdus.len(), 64);
    bus.npdus.close();
    bus.send(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 3, 7, &[1, 0]);
    bus.send(FrameType::BACNET_DATA_EXPECTING_REPLY, 3, 7, &[1, 4]);
    let postponed = bus.next().await;
    assert_eq!(postponed.frame_type, FrameType::REPLY_POSTPONED);
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.ingress_closed, 2);
    assert_eq!(counts.mac.npdus_delivered, 67);
    bus.stop().await;
}

/// Counts are kept the same way in both execution modes, and a dropped
/// transport keeps them while its task and port go away.
#[tokio::test]
async fn execution_modes_share_the_counts() {
    for mode in [
        MstpExecutionMode::Tokio,
        MstpExecutionMode::dedicated_thread(),
    ] {
        let (serial, peer) = LoopbackSerial::pair();
        let mut transport =
            MstpTransport::new(serial, config(3, 127)).with_execution_mode(mode.clone());
        let observer = transport.diagnostics();
        assert_eq!(observer.snapshot(), MstpDiagnosticsSnapshot::default());
        let mut npdus = transport.start().await.unwrap();
        let wire = encode(FrameType::BACNET_DATA_EXPECTING_REPLY, 3, 7, &[1, 4]);
        peer.write(&wire).await.unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(2), npdus.recv())
            .await
            .unwrap()
            .unwrap()
            .reply_tx
            .unwrap();
        reply.send(Bytes::from_static(&[1, 0, 0x30])).unwrap();
        let mut buf = [0u8; 64];
        tokio::time::timeout(Duration::from_secs(2), peer.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while observer.snapshot().dner_tx_direct == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the answer was not counted");
        drop(transport);
        let closed = tokio::time::timeout(Duration::from_secs(2), npdus.recv()).await;
        assert!(
            matches!(closed, Ok(None)),
            "{mode:?}: the task kept running"
        );
        let snapshot = observer.snapshot();
        assert_eq!(snapshot.dner_tx_direct, 1, "{mode:?}");
        assert_eq!(snapshot.mac.npdus_delivered, 1, "{mode:?}");
    }
}
