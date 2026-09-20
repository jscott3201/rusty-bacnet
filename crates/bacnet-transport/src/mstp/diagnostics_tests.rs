use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use bytes::BytesMut;
use tokio::sync::Mutex;
use tokio::time::{advance, timeout, Duration, Instant};

use super::*;
use crate::mstp_frame::{decode_frame, encode_frame};
use crate::port::TransportPort;

struct Serial {
    input: Mutex<mpsc::Receiver<Result<Vec<u8>, Error>>>,
    output: mpsc::Sender<Vec<u8>>,
    fail_write: Arc<AtomicBool>,
    read_calls: Arc<AtomicU64>,
}

impl SerialPort for Serial {
    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        self.read_calls.fetch_add(1, Ordering::Relaxed);
        let bytes = self.input.lock().await.recv().await.unwrap()?;
        buf[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }

    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        if self.fail_write.load(Ordering::Relaxed) {
            return Err(io_error());
        }
        self.output.send(data.to_vec()).await.unwrap();
        Ok(())
    }
}

fn io_error() -> Error {
    Error::Transport(std::io::Error::other("simulated serial failure"))
}

struct Harness {
    transport: MstpTransport<Serial>,
    counts: MstpDiagnostics,
    input: mpsc::Sender<Result<Vec<u8>, Error>>,
    output: mpsc::Receiver<Vec<u8>>,
    npdus: mpsc::Receiver<ReceivedNpdu>,
    fail_write: Arc<AtomicBool>,
    read_calls: Arc<AtomicU64>,
}

fn frame(kind: FrameType, data: &[u8]) -> MstpFrame {
    MstpFrame {
        frame_type: kind,
        destination: 3,
        source: 7,
        data: Bytes::copy_from_slice(data),
    }
}

fn encoded(frame: &MstpFrame) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_frame(&mut bytes, frame).unwrap();
    bytes.to_vec()
}

async fn until(mut ready: impl FnMut() -> bool) {
    timeout(Duration::from_secs(2), async {
        while !ready() {
            // Also lets a paused clock reach the failure timeout; a perpetual
            // yield loop would stay ready and prevent virtual-time progress.
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("diagnostic event not observed");
}

impl Harness {
    async fn new(mode: MstpExecutionMode) -> Self {
        let (input, rx) = mpsc::channel(128);
        let (tx, output) = mpsc::channel(128);
        let fail_write = Arc::new(AtomicBool::new(false));
        let read_calls = Arc::new(AtomicU64::new(0));
        let mut transport = MstpTransport::new(
            Serial {
                input: Mutex::new(rx),
                output: tx,
                fail_write: fail_write.clone(),
                read_calls: read_calls.clone(),
            },
            MstpConfig {
                this_station: 3,
                max_master: 7,
                max_info_frames: 1,
                baud_rate: 76800,
            },
        )
        .with_execution_mode(mode);
        let counts = transport.diagnostics();
        assert_eq!(counts.snapshot(), MstpDiagnosticsSnapshot::default());
        let npdus = transport.start().await.unwrap();
        {
            let mut node = transport.node_state().unwrap().lock().await;
            node.next_station = 7;
            node.token_count = 0;
        }
        Self {
            transport,
            counts,
            input,
            output,
            npdus,
            fail_write,
            read_calls,
        }
    }

    async fn receive(&self, kind: FrameType, data: &[u8]) {
        self.input
            .send(Ok(encoded(&frame(kind, data))))
            .await
            .unwrap();
    }

    async fn written(&mut self, kind: FrameType, data: &[u8]) {
        let bytes = timeout(Duration::from_secs(2), self.output.recv())
            .await
            .unwrap()
            .unwrap();
        let expected = MstpFrame {
            source: 3,
            destination: 7,
            ..frame(kind, data)
        };
        assert_eq!(bytes, encoded(&expected), "wire bytes/order changed");
        assert_eq!(decode_frame(&bytes).unwrap().0, expected);
    }
}

#[tokio::test]
async fn mstp_diagnostics_frame_counts_and_owned_lifecycle_match_execution_modes() {
    for mode in [MstpExecutionMode::Tokio, MstpExecutionMode::DedicatedThread] {
        let mut h = Harness::new(mode).await;
        let observer = h.counts.clone();

        h.receive(FrameType::BACnetDataExpectingReply, &[1, 4, 0x10])
            .await;
        h.npdus
            .recv()
            .await
            .unwrap()
            .reply_tx
            .unwrap()
            .send(Bytes::from_static(&[1, 0, 0x30]))
            .unwrap();
        h.written(FrameType::BACnetDataNotExpectingReply, &[1, 0, 0x30])
            .await;
        until(|| observer.snapshot().dner_tx_direct == 1).await;

        // A dropped sender produces ReplyPostponed; the later queue item is not
        // claimed to be correlated by the transport, even in this known scenario.
        h.receive(FrameType::BACnetDataExpectingReply, &[1, 4, 0x10])
            .await;
        drop(h.npdus.recv().await.unwrap());
        h.written(FrameType::ReplyPostponed, &[]).await;
        h.transport.send_unicast(&[1, 0, 0x30], &[7]).await.unwrap();
        h.receive(FrameType::Token, &[]).await;
        h.written(FrameType::BACnetDataNotExpectingReply, &[1, 0, 0x30])
            .await;
        h.written(FrameType::Token, &[]).await;

        h.transport.send_unicast(&[1, 4, 0x10], &[7]).await.unwrap();
        h.receive(FrameType::Token, &[]).await;
        h.written(FrameType::BACnetDataExpectingReply, &[1, 4, 0x10])
            .await;
        h.receive(FrameType::ReplyPostponed, &[]).await;
        h.written(FrameType::Token, &[]).await;
        h.receive(FrameType::BACnetDataNotExpectingReply, &[1, 0, 0x30])
            .await;
        assert_eq!(h.npdus.recv().await.unwrap().npdu.as_ref(), &[1, 0, 0x30]);

        h.transport.send_unicast(&[1, 4, 0x20], &[7]).await.unwrap();
        h.receive(FrameType::Token, &[]).await;
        h.written(FrameType::BACnetDataExpectingReply, &[1, 4, 0x20])
            .await;
        h.receive(FrameType::BACnetDataNotExpectingReply, &[1, 0, 0x40])
            .await;
        h.written(FrameType::Token, &[]).await;
        assert_eq!(h.npdus.recv().await.unwrap().npdu.as_ref(), &[1, 0, 0x40]);
        until(|| observer.snapshot().der_tx == 2).await;

        h.transport.stop().await.unwrap();
        let expected = MstpDiagnosticsSnapshot {
            der_tx: 2,
            der_rx: 2,
            dner_tx_direct: 1,
            dner_tx_queued: 1,
            dner_rx: 2,
            reply_postponed_tx: 1,
            reply_postponed_rx: 1,
            ..Default::default()
        };
        assert_eq!(observer.snapshot(), expected);
        drop(h.transport);
        assert_eq!(observer.snapshot(), expected);
    }
}

#[tokio::test(start_paused = true)]
async fn mstp_diagnostics_reply_deadline_timeout_and_rejections() {
    let mut h = Harness::new(MstpExecutionMode::Tokio).await;
    h.receive(FrameType::BACnetDataExpectingReply, &[1, 4])
        .await;
    let reply = h.npdus.recv().await.unwrap().reply_tx.unwrap();
    let started = Instant::now();
    h.written(FrameType::ReplyPostponed, &[]).await;
    assert!(started.elapsed() < Duration::from_millis(250));
    assert!(reply.send(Bytes::new()).is_err());
    assert_eq!(h.counts.snapshot().reply_postponed_tx, 1);

    h.transport.send_unicast(&[1, 4, 0x10], &[7]).await.unwrap();
    h.receive(FrameType::Token, &[]).await;
    h.written(FrameType::BACnetDataExpectingReply, &[1, 4, 0x10])
        .await;
    let sent = Instant::now();
    advance(Duration::from_millis(254)).await;
    tokio::task::yield_now().await;
    assert_eq!(h.counts.snapshot().wait_for_reply_timeouts, 0);
    assert!(h.output.try_recv().is_err());
    h.written(FrameType::Token, &[]).await;
    assert!(sent.elapsed() >= Duration::from_millis(255));
    assert!(sent.elapsed() < Duration::from_millis(257));
    assert_eq!(h.counts.snapshot().wait_for_reply_timeouts, 1);

    let oversize = vec![0; MAX_STANDARD_MPDU_DATA + 1];
    assert!(matches!(
        h.transport.send_broadcast(&oversize).await,
        Err(Error::Encoding(_))
    ));
    assert!(matches!(
        h.transport.send_unicast(&oversize, &[7]).await,
        Err(Error::Encoding(_))
    ));
    for _ in 0..MAX_TX_QUEUE_DEPTH {
        h.transport.send_broadcast(&[1, 0]).await.unwrap();
    }
    for broadcast in [true, false] {
        let error = if broadcast {
            h.transport.send_broadcast(&[1, 0]).await
        } else {
            h.transport.send_unicast(&[1, 0], &[7]).await
        }
        .unwrap_err();
        assert!(
            matches!(error, Error::Transport(ref e) if e.kind() == std::io::ErrorKind::WouldBlock)
        );
    }
    // Oversize remains the first rejection even with a full queue.
    assert!(matches!(
        h.transport.send_broadcast(&oversize).await,
        Err(Error::Encoding(_))
    ));
    assert_eq!(
        h.transport
            .node_state()
            .unwrap()
            .lock()
            .await
            .tx_queue
            .len(),
        MAX_TX_QUEUE_DEPTH
    );

    h.receive(FrameType::BACnetDataExpectingReply, &[1, 4])
        .await;
    h.npdus
        .recv()
        .await
        .unwrap()
        .reply_tx
        .unwrap()
        .send(Bytes::from(oversize))
        .unwrap();
    until(|| h.counts.snapshot().outbound_oversize == 4).await;
    assert!(
        h.output.try_recv().is_err(),
        "oversize direct reply must not write"
    );
    assert_eq!(h.counts.snapshot().outbound_queue_full, 2);
    assert_eq!(h.counts.snapshot().dner_tx_direct, 0);
    h.transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn mstp_diagnostics_invalid_and_stale_host_assembly_are_distinct() {
    let mut h = Harness::new(MstpExecutionMode::Tokio).await;
    // One invalid header, followed by a known valid DNER: invalid is not RX.
    let mut bad = encoded(&frame(FrameType::BACnetDataNotExpectingReply, &[1, 0]));
    bad[7] ^= 1;
    h.input.send(Ok(bad)).await.unwrap();
    h.receive(FrameType::BACnetDataNotExpectingReply, &[1, 0])
        .await;
    h.npdus.recv().await.unwrap();
    assert!(h.counts.snapshot().invalid_frame_discards > 0);
    assert_eq!(h.counts.snapshot().dner_rx, 1);
    assert_eq!(h.counts.snapshot().stale_partial_resets, 0);
    let invalid_before = h.counts.snapshot().invalid_frame_discards;

    let partial = encoded(&frame(FrameType::BACnetDataNotExpectingReply, &[1, 0]));
    let calls = h.read_calls.load(Ordering::Relaxed);
    h.input.send(Ok(partial[..5].to_vec())).await.unwrap();
    until(|| h.read_calls.load(Ordering::Relaxed) > calls).await;
    // The next read is polled only after processing the partial chunk.
    advance(Duration::from_micros(
        calculate_host_stale_partial_timeout_us(76800) + 1,
    ))
    .await;
    h.receive(FrameType::BACnetDataNotExpectingReply, &[1, 0])
        .await;
    h.npdus.recv().await.unwrap();
    assert_eq!(h.counts.snapshot().stale_partial_resets, 1);
    assert_eq!(h.counts.snapshot().invalid_frame_discards, invalid_before);
    assert_eq!(h.counts.snapshot().dner_rx, 2);
    h.transport.stop().await.unwrap();
}

#[tokio::test]
async fn mstp_diagnostics_ingress_drops_and_io_errors_match_execution_modes() {
    for mode in [MstpExecutionMode::Tokio, MstpExecutionMode::DedicatedThread] {
        let mut h = Harness::new(mode).await;
        for _ in 0..65 {
            h.receive(FrameType::BACnetDataNotExpectingReply, &[1, 0])
                .await;
        }
        until(|| h.counts.snapshot().ingress_full == 1).await;
        assert_eq!(h.npdus.len(), 64);
        h.npdus.close();
        h.receive(FrameType::BACnetDataNotExpectingReply, &[1, 0])
            .await;
        until(|| h.counts.snapshot().ingress_closed == 1).await;
        h.receive(FrameType::BACnetDataExpectingReply, &[1, 4])
            .await;
        h.written(FrameType::ReplyPostponed, &[]).await;
        until(|| h.counts.snapshot().reply_postponed_tx == 1).await;
        assert_eq!(h.counts.snapshot().ingress_closed, 2);

        h.fail_write.store(true, Ordering::Relaxed);
        h.transport.send_unicast(&[1, 4], &[7]).await.unwrap();
        h.receive(FrameType::Token, &[]).await;
        until(|| h.counts.snapshot().serial_write_errors == 1).await;
        assert_eq!(
            h.counts.snapshot().der_tx,
            0,
            "failed write is not transmitted"
        );
        h.input.send(Err(io_error())).await.unwrap();
        until(|| h.counts.snapshot().serial_read_errors == 1).await;
        h.transport.stop().await.unwrap();
        let snapshot = h.counts.snapshot();
        assert_eq!(snapshot.dner_rx, 66);
        assert_eq!(snapshot.der_rx, 1);
        assert_eq!(snapshot.ingress_full, 1);
        assert_eq!(snapshot.serial_write_errors, 1);
        drop(h.transport);
        assert_eq!(h.counts.snapshot(), snapshot);
    }
}

#[tokio::test]
async fn mstp_diagnostics_drop_without_stop_keeps_counts_not_serial_owner() {
    for mode in [MstpExecutionMode::Tokio, MstpExecutionMode::DedicatedThread] {
        let mut h = Harness::new(mode).await;
        h.receive(FrameType::BACnetDataExpectingReply, &[1, 4])
            .await;
        let reply = h.npdus.recv().await.unwrap().reply_tx.unwrap();
        let observer = h.counts.clone();
        drop(h.transport);
        timeout(Duration::from_secs(2), h.input.closed())
            .await
            .unwrap();
        assert!(reply.send(Bytes::new()).is_err());
        assert_eq!(observer.snapshot().der_rx, 1);
        assert_eq!(observer.snapshot().dner_tx_direct, 0);
    }
}

#[tokio::test]
async fn mstp_diagnostics_timer_and_reply_write_failures_match_execution_modes() {
    for mode in [MstpExecutionMode::Tokio, MstpExecutionMode::DedicatedThread] {
        for direct in [true, false] {
            let mut h = Harness::new(mode).await;
            h.fail_write.store(true, Ordering::Relaxed);
            h.receive(FrameType::BACnetDataExpectingReply, &[1, 4])
                .await;
            let reply = h.npdus.recv().await.unwrap().reply_tx.unwrap();
            if direct {
                reply.send(Bytes::from_static(&[1, 0, 0x30])).unwrap();
            }
            // Otherwise retain the sender through the timer-generated postponed
            // write, exercising the third serial write site without a real bus.
            until(|| h.counts.snapshot().serial_write_errors == 1).await;
            h.transport.stop().await.unwrap();
            assert_eq!(
                h.counts.snapshot(),
                MstpDiagnosticsSnapshot {
                    der_rx: 1,
                    serial_write_errors: 1,
                    ..Default::default()
                }
            );
        }

        let mut h = Harness::new(mode).await;
        h.transport.send_unicast(&[1, 4], &[7]).await.unwrap();
        h.receive(FrameType::Token, &[]).await;
        h.written(FrameType::BACnetDataExpectingReply, &[1, 4])
            .await;
        h.written(FrameType::Token, &[]).await;
        until(|| h.counts.snapshot().wait_for_reply_timeouts == 1).await;
        h.transport.stop().await.unwrap();
        assert_eq!(
            h.counts.snapshot(),
            MstpDiagnosticsSnapshot {
                der_tx: 1,
                wait_for_reply_timeouts: 1,
                ..Default::default()
            }
        );
    }
}

#[tokio::test(start_paused = true)]
async fn mstp_diagnostics_counts_foreign_frames_but_not_npdu_delivery() {
    let mut h = Harness::new(MstpExecutionMode::Tokio).await;
    let mut chunk = Vec::new();
    for (kind, data) in [
        (FrameType::BACnetDataExpectingReply, &[1, 4][..]),
        (FrameType::BACnetDataNotExpectingReply, &[1, 0][..]),
        (FrameType::ReplyPostponed, &[][..]),
    ] {
        chunk.extend(encoded(&MstpFrame {
            destination: 6,
            ..frame(kind, data)
        }));
    }
    h.input.send(Ok(chunk)).await.unwrap();
    until(|| h.counts.snapshot().reply_postponed_rx == 1).await;
    h.transport.stop().await.unwrap();
    assert!(h.npdus.try_recv().is_err());
    assert_eq!(
        h.counts.snapshot(),
        MstpDiagnosticsSnapshot {
            der_rx: 1,
            dner_rx: 1,
            reply_postponed_rx: 1,
            ..Default::default()
        }
    );
}
