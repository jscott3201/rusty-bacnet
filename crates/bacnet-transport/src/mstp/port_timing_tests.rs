use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;
use tokio::time::{advance, Duration, Instant};

use super::*;
use crate::mstp_frame::{decode_frame, encode_frame, FrameType, MstpFrame};
use crate::port::TransportPort;

fn expecting_reply_frame() -> MstpFrame {
    MstpFrame {
        frame_type: FrameType::BACnetDataExpectingReply,
        destination: 3,
        source: 7,
        data: Bytes::from_static(&[0x01, 0x04, 0x10]),
    }
}

fn timing_config() -> MstpConfig {
    MstpConfig {
        this_station: 3,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    }
}

#[tokio::test(start_paused = true)]
async fn ready_data_reply_does_not_wait_for_reply_delay() {
    let (serial_transport, serial_peer) = LoopbackSerial::pair();
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut encoded = BytesMut::new();
    encode_frame(&mut encoded, &expecting_reply_frame()).unwrap();
    let started = tokio::time::Instant::now();
    serial_peer.write(&encoded).await.unwrap();

    let received = npdu_rx.recv().await.unwrap();
    received
        .reply_tx
        .unwrap()
        .send(Bytes::from_static(&[0x01, 0x00, 0x30, 0x01]))
        .unwrap();
    let mut response_buf = [0u8; 64];
    let response_len = serial_peer.read(&mut response_buf).await.unwrap();
    let (response, _) = decode_frame(&response_buf[..response_len]).unwrap();

    assert_eq!(response.frame_type, FrameType::BACnetDataNotExpectingReply);
    assert_eq!(response.destination, 7);
    assert!(started.elapsed() < tokio::time::Duration::from_millis(50));
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn reply_postponed_starts_before_reply_delay_limit() {
    let (serial_transport, serial_peer) = LoopbackSerial::pair();
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut encoded = BytesMut::new();
    encode_frame(&mut encoded, &expecting_reply_frame()).unwrap();
    let started = tokio::time::Instant::now();
    serial_peer.write(&encoded).await.unwrap();

    // Keep the application sender alive without replying so the deadline,
    // rather than channel cancellation, selects ReplyPostponed.
    let received = npdu_rx.recv().await.unwrap();
    let _reply_tx = received.reply_tx.unwrap();
    let mut response_buf = [0u8; 64];
    let response_len = serial_peer.read(&mut response_buf).await.unwrap();
    let (response, _) = decode_frame(&response_buf[..response_len]).unwrap();

    assert_eq!(response.frame_type, FrameType::ReplyPostponed);
    assert_eq!(response.destination, 7);
    assert!(started.elapsed() < tokio::time::Duration::from_millis(T_REPLY_DELAY_MS));
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn second_data_request_does_not_extend_first_reply_deadline() {
    let (serial_transport, serial_peer) = LoopbackSerial::pair();
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut first = BytesMut::new();
    encode_frame(&mut first, &expecting_reply_frame()).unwrap();
    let started = tokio::time::Instant::now();
    serial_peer.write(&first).await.unwrap();

    let received = npdu_rx.recv().await.unwrap();
    let _reply_tx = received.reply_tx.unwrap();
    tokio::time::advance(tokio::time::Duration::from_millis(100)).await;

    let mut second = BytesMut::new();
    let second_request = MstpFrame {
        source: 8,
        data: Bytes::from_static(&[0x01, 0x04, 0x20]),
        ..expecting_reply_frame()
    };
    encode_frame(&mut second, &second_request).unwrap();
    serial_peer.write(&second).await.unwrap();
    tokio::task::yield_now().await;
    assert!(npdu_rx.try_recv().is_err());

    let mut response_buf = [0u8; 64];
    let response_len = serial_peer.read(&mut response_buf).await.unwrap();
    let (response, _) = decode_frame(&response_buf[..response_len]).unwrap();
    assert_eq!(response.frame_type, FrameType::ReplyPostponed);
    assert_eq!(response.destination, 7);
    assert!(started.elapsed() < tokio::time::Duration::from_millis(T_REPLY_DELAY_MS));
    transport.stop().await.unwrap();
}

struct ObservedSerial {
    inner: LoopbackSerial,
    reads: mpsc::UnboundedSender<Instant>,
    writes: mpsc::UnboundedSender<(Instant, MstpFrame)>,
}

impl SerialPort for ObservedSerial {
    async fn read(&self, buf: &mut [u8]) -> Result<usize, bacnet_types::error::Error> {
        let count = self.inner.read(buf).await?;
        self.reads.send(Instant::now()).unwrap();
        Ok(count)
    }

    async fn write(&self, data: &[u8]) -> Result<(), bacnet_types::error::Error> {
        let (frame, consumed) = decode_frame(data).unwrap();
        // Each borrowed slice must contain exactly one complete frame, even
        // when multiple frames share reusable transmit storage.
        assert_eq!(consumed, data.len());
        self.writes.send((Instant::now(), frame)).unwrap();
        Ok(())
    }
}

struct TimingHarness {
    transport: MstpTransport<ObservedSerial>,
    peer: LoopbackSerial,
    reads: mpsc::UnboundedReceiver<Instant>,
    writes: mpsc::UnboundedReceiver<(Instant, MstpFrame)>,
    npdus: mpsc::Receiver<crate::port::ReceivedNpdu>,
}

impl TimingHarness {
    async fn new(baud_rate: u32) -> Self {
        let (inner, peer) = LoopbackSerial::pair();
        let (read_tx, reads) = mpsc::unbounded_channel();
        let (write_tx, writes) = mpsc::unbounded_channel();
        let mut transport = MstpTransport::new(
            ObservedSerial {
                inner,
                reads: read_tx,
                writes: write_tx,
            },
            MstpConfig {
                baud_rate,
                ..timing_config()
            },
        );
        let npdus = transport.start().await.unwrap();
        Self {
            transport,
            peer,
            reads,
            writes,
            npdus,
        }
    }

    async fn receive(&mut self, frame: &MstpFrame) -> Instant {
        let mut encoded = BytesMut::new();
        encode_frame(&mut encoded, frame).unwrap();
        self.peer.write(&encoded).await.unwrap();
        self.reads.recv().await.unwrap()
    }
}

// Independent, rounded-up microsecond expectations for forty bit times.
const TURNAROUNDS: [(u32, u64); 3] = [(9600, 4167), (38400, 1042), (76800, 521)];

#[tokio::test(start_paused = true)]
async fn turnaround_deadlines_cover_token_poll_and_data_paths() {
    for (baud, turnaround_us) in TURNAROUNDS {
        // Force processing to finish both before and after the absolute deadline.
        for processing_us in [0, turnaround_us / 2, 8_000] {
            for scenario in 0..5 {
                let mut harness = TimingHarness::new(baud).await;
                let node = harness.transport.node_state().unwrap().clone();
                let mut guard = node.lock().await;
                guard.next_station = 7;
                guard.token_count = 0;
                let (incoming_type, expected_type) = match scenario {
                    0 => (FrameType::PollForMaster, FrameType::ReplyToPollForMaster),
                    1 => (FrameType::Token, FrameType::Token),
                    2 => {
                        guard.next_station = guard.config.this_station;
                        (FrameType::Token, FrameType::PollForMaster)
                    }
                    3 => {
                        guard
                            .queue_npdu(7, Bytes::from_static(&[1, 4, 0x10]))
                            .unwrap();
                        (FrameType::Token, FrameType::BACnetDataExpectingReply)
                    }
                    _ => {
                        guard.config.max_info_frames = 2;
                        guard
                            .queue_npdu(7, Bytes::from_static(&[1, 0, 0x10]))
                            .unwrap();
                        guard
                            .queue_npdu(8, Bytes::from_static(&[1, 0, 0x20]))
                            .unwrap();
                        (FrameType::Token, FrameType::BACnetDataNotExpectingReply)
                    }
                };
                let received_at = harness
                    .receive(&MstpFrame {
                        frame_type: incoming_type,
                        source: 7,
                        destination: 3,
                        data: Bytes::new(),
                    })
                    .await;
                advance(Duration::from_micros(processing_us)).await;
                let released_at = Instant::now();
                drop(guard);
                tokio::task::yield_now().await;
                let (sent_at, frame) = if processing_us > turnaround_us {
                    // A fresh relative sleep would leave this empty.
                    harness
                        .writes
                        .try_recv()
                        .expect("elapsed processing was not credited")
                } else {
                    advance(Duration::from_micros(turnaround_us - processing_us - 1)).await;
                    tokio::task::yield_now().await;
                    assert!(harness.writes.try_recv().is_err(), "TX before turnaround");
                    harness.writes.recv().await.unwrap()
                };
                assert_eq!(frame.frame_type, expected_type);
                assert!(sent_at >= received_at + Duration::from_micros(turnaround_us));
                if processing_us > turnaround_us {
                    assert_eq!(sent_at, released_at);
                } else {
                    // Permit timer quantization, but not a restarted full wait.
                    assert!(sent_at < received_at + Duration::from_micros(turnaround_us + 1_000));
                }
                if scenario == 4 {
                    assert_eq!(frame.data.as_ref(), &[1, 0, 0x10]);
                    let (_, second) = harness.writes.recv().await.unwrap();
                    assert_eq!(second.frame_type, FrameType::BACnetDataNotExpectingReply);
                    assert_eq!(second.destination, 8);
                    assert_eq!(second.data.as_ref(), &[1, 0, 0x20]);
                    let (_, token) = harness.writes.recv().await.unwrap();
                    assert_eq!(token.frame_type, FrameType::Token);
                }
                harness.transport.stop().await.unwrap();
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn application_reply_and_postponement_credit_elapsed_turnaround() {
    for (baud, _) in TURNAROUNDS {
        for ready in [true, false] {
            let mut harness = TimingHarness::new(baud).await;
            harness.receive(&expecting_reply_frame()).await;
            let reply_tx = harness.npdus.recv().await.unwrap().reply_tx.unwrap();
            advance(Duration::from_millis(10)).await;
            let ready_at = Instant::now();
            if ready {
                reply_tx.send(Bytes::from_static(&[1, 0, 0x30])).unwrap();
            } else {
                drop(reply_tx);
            }
            tokio::task::yield_now().await;
            let (sent_at, frame) = harness
                .writes
                .try_recv()
                .expect("unnecessary reply turnaround sleep");
            assert_eq!(sent_at, ready_at);
            assert_eq!(frame.destination, 7);
            assert_eq!(
                frame.frame_type,
                if ready {
                    FrameType::BACnetDataNotExpectingReply
                } else {
                    FrameType::ReplyPostponed
                }
            );
            harness.transport.stop().await.unwrap();
        }
    }
}

#[tokio::test(start_paused = true)]
async fn reply_postponed_timer_does_not_add_another_turnaround() {
    for (baud, turnaround_us) in TURNAROUNDS {
        let mut harness = TimingHarness::new(baud).await;
        let received_at = harness.receive(&expecting_reply_frame()).await;
        let _reply_tx = harness.npdus.recv().await.unwrap().reply_tx.unwrap();
        let (sent_at, frame) = harness.writes.recv().await.unwrap();
        assert_eq!(frame.frame_type, FrameType::ReplyPostponed);
        let decision_ms =
            T_REPLY_DELAY_MS - turnaround_us.div_ceil(1_000) - T_REPLY_TRANSMIT_MARGIN_MS;
        assert!(sent_at >= received_at + Duration::from_millis(decision_ms));
        assert!(sent_at < received_at + Duration::from_millis(decision_ms + 2));
        harness.transport.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn late_usb_tail_anchors_turnaround_to_last_chunk_not_frame_start() {
    for (baud, turnaround_us) in TURNAROUNDS {
        let mut harness = TimingHarness::new(baud).await;
        let mut encoded = BytesMut::new();
        encode_frame(
            &mut encoded,
            &MstpFrame {
                frame_type: FrameType::PollForMaster,
                source: 7,
                destination: 3,
                data: Bytes::new(),
            },
        )
        .unwrap();
        let split = encoded.len() - 1;
        harness.peer.write(&encoded[..split]).await.unwrap();
        let first_chunk_at = harness.reads.recv().await.unwrap();
        advance(Duration::from_millis(40)).await;
        assert!(harness.writes.try_recv().is_err());
        harness.peer.write(&encoded[split..]).await.unwrap();
        let last_chunk_at = harness.reads.recv().await.unwrap();
        assert!(last_chunk_at >= first_chunk_at + Duration::from_millis(40));
        advance(Duration::from_micros(turnaround_us - 1)).await;
        tokio::task::yield_now().await;
        assert!(harness.writes.try_recv().is_err());
        let (sent_at, frame) = harness.writes.recv().await.unwrap();
        assert_eq!(frame.frame_type, FrameType::ReplyToPollForMaster);
        assert!(sent_at >= last_chunk_at + Duration::from_micros(turnaround_us));
        harness.transport.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn later_host_traffic_refreshes_pending_application_reply_turnaround() {
    for (baud, turnaround_us) in TURNAROUNDS {
        let mut harness = TimingHarness::new(baud).await;
        harness.receive(&expecting_reply_frame()).await;
        let reply_tx = harness.npdus.recv().await.unwrap().reply_tx.unwrap();
        advance(Duration::from_millis(40)).await;
        let latest = harness
            .receive(&MstpFrame {
                frame_type: FrameType::BACnetDataNotExpectingReply,
                source: 8,
                destination: 9,
                data: Bytes::from_static(&[1, 0, 0x20]),
            })
            .await;
        reply_tx.send(Bytes::from_static(&[1, 0, 0x30])).unwrap();
        tokio::task::yield_now().await;
        advance(Duration::from_micros(turnaround_us - 1)).await;
        tokio::task::yield_now().await;
        assert!(harness.writes.try_recv().is_err());
        let (sent_at, frame) = harness.writes.recv().await.unwrap();
        assert_eq!(frame.destination, 7);
        assert_eq!(frame.frame_type, FrameType::BACnetDataNotExpectingReply);
        assert!(sent_at >= latest + Duration::from_micros(turnaround_us));
        harness.transport.stop().await.unwrap();
    }
}

// Real-clock harness: unlike the paused-clock tests above, this can cross runtime
// boundaries and exercise the native serial backend as well as chunked loopback.
struct ExecutionObserved<S> {
    inner: S,
    reads: mpsc::UnboundedSender<(Instant, std::thread::ThreadId, usize)>,
    writes: mpsc::UnboundedSender<(Instant, std::thread::ThreadId, Vec<u8>)>,
}

impl<S: SerialPort> SerialPort for ExecutionObserved<S> {
    async fn read(&self, buf: &mut [u8]) -> Result<usize, bacnet_types::error::Error> {
        let count = self.inner.read(buf).await?;
        if count > 0 {
            self.reads
                .send((Instant::now(), std::thread::current().id(), count))
                .unwrap();
        }
        Ok(count)
    }

    async fn write(&self, data: &[u8]) -> Result<(), bacnet_types::error::Error> {
        self.writes
            .send((Instant::now(), std::thread::current().id(), data.to_vec()))
            .unwrap();
        self.inner.write(data).await
    }
}

/// Shared host-timing/byte-order harness, also used by native serial tests.
/// Whole reads and USB-like late tails use the same assertions in both modes.
pub(crate) async fn execution_timing_harness<S: SerialPort, P: SerialPort>(
    serial: S,
    peer: P,
    mode: Option<MstpExecutionMode>,
    baud: u32,
    chunked: bool,
) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let app_thread = std::thread::current().id();
        let (read_tx, mut reads) = mpsc::unbounded_channel();
        let (write_tx, mut writes) = mpsc::unbounded_channel();
        let mut transport = MstpTransport::new(
            ExecutionObserved {
                inner: serial,
                reads: read_tx,
                writes: write_tx,
            },
            MstpConfig {
                baud_rate: baud,
                max_info_frames: 2,
                ..timing_config()
            },
        );
        if let Some(mode) = mode {
            transport = transport.with_execution_mode(mode);
        }
        let mut npdus = transport.start().await.unwrap();
        assert_eq!(npdus.max_capacity(), 64);
        let node = transport.node_state().unwrap().clone();
        {
            let mut node = node.lock().await;
            node.next_station = 7;
            node.token_count = 0;
        }
        transport.send_unicast(&[1, 0, 0x10], &[7]).await.unwrap();
        transport.send_unicast(&[1, 0, 0x20], &[8]).await.unwrap();
        let mut incoming = BytesMut::new();
        encode_frame(
            &mut incoming,
            &MstpFrame {
                frame_type: FrameType::Token,
                destination: 3,
                source: 7,
                data: Bytes::new(),
            },
        )
        .unwrap();
        let split = if chunked {
            incoming.len() - 1
        } else {
            incoming.len()
        };
        peer.write(&incoming[..split]).await.unwrap();
        let mut received = 0;
        let (last_read, worker_thread) = loop {
            let (at, thread, count) = reads.recv().await.unwrap();
            received += count;
            if received == incoming.len() {
                break (at, thread);
            }
            if received == split {
                tokio::time::sleep(Duration::from_millis(10)).await;
                assert!(
                    writes.try_recv().is_err(),
                    "partial token must not transmit"
                );
                peer.write(&incoming[split..]).await.unwrap();
            }
        };
        assert_eq!(
            worker_thread == app_thread,
            mode.unwrap_or_default() == MstpExecutionMode::Tokio
        );
        let mut expected_wire = BytesMut::new();
        let mut wire = Vec::new();
        for (index, (kind, destination, data)) in [
            (FrameType::BACnetDataNotExpectingReply, 7, &[1, 0, 0x10][..]),
            (FrameType::BACnetDataNotExpectingReply, 8, &[1, 0, 0x20][..]),
            (FrameType::Token, 7, &[][..]),
        ]
        .into_iter()
        .enumerate()
        {
            let (at, thread, bytes) = writes.recv().await.unwrap();
            assert_eq!(thread, worker_thread);
            if index == 0 {
                let turnaround = TURNAROUNDS
                    .iter()
                    .find(|(rate, _)| *rate == baud)
                    .unwrap()
                    .1;
                assert!(at >= last_read + Duration::from_micros(turnaround));
            }
            let (frame, consumed) = decode_frame(&bytes).unwrap();
            assert_eq!(consumed, bytes.len(), "one frame per write");
            let expected = MstpFrame {
                frame_type: kind,
                destination,
                source: 3,
                data: Bytes::copy_from_slice(data),
            };
            assert_eq!(frame, expected);
            encode_frame(&mut expected_wire, &expected).unwrap();
            wire.extend_from_slice(&bytes);
        }
        assert_eq!(wire, expected_wire);
        let mut received_wire = Vec::new();
        while received_wire.len() < wire.len() {
            let mut buf = [0; 128];
            // Read only this exchange; later token retries remain valid traffic.
            let remaining = (wire.len() - received_wire.len()).min(buf.len());
            let count = peer.read(&mut buf[..remaining]).await.unwrap();
            assert!(count > 0);
            received_wire.extend_from_slice(&buf[..count]);
        }
        assert_eq!(received_wire, wire, "actual backend output differs");
        transport.send_broadcast(&[1, 0, 0x30]).await.unwrap();
        transport.stop().await.unwrap();
        let node = node.lock().await;
        assert!(node.tx_queue.is_empty());
        assert_eq!(node.state, MasterState::Idle);
        assert_eq!(node.expected_reply_source, None);
        assert!(npdus.recv().await.is_none());
        wire
    })
    .await
    .expect("execution harness stalled")
}

#[tokio::test]
async fn disabled_mode_byte_parity_and_dedicated_timing_share_harness() {
    for (baud, _) in TURNAROUNDS {
        for chunked in [false, true] {
            let mut baseline = None;
            for mode in [
                None,
                Some(MstpExecutionMode::Tokio),
                Some(MstpExecutionMode::DedicatedThread),
            ] {
                let (serial, peer) = LoopbackSerial::pair();
                let wire = execution_timing_harness(serial, peer, mode, baud, chunked).await;
                match &baseline {
                    None => baseline = Some(wire),
                    Some(expected) => assert_eq!(&wire, expected),
                }
            }
        }
    }
}

#[tokio::test]
async fn execution_modes_preserve_application_reply_and_deadline() {
    for mode in [MstpExecutionMode::Tokio, MstpExecutionMode::DedicatedThread] {
        for ready in [true, false] {
            tokio::time::timeout(Duration::from_secs(2), async {
                let (serial, peer) = LoopbackSerial::pair();
                let (read_tx, mut reads) = mpsc::unbounded_channel();
                let (write_tx, mut writes) = mpsc::unbounded_channel();
                let mut transport = MstpTransport::new(
                    ExecutionObserved {
                        inner: serial,
                        reads: read_tx,
                        writes: write_tx,
                    },
                    timing_config(),
                )
                .with_execution_mode(mode);
                let mut npdus = transport.start().await.unwrap();
                let mut incoming = BytesMut::new();
                encode_frame(&mut incoming, &expecting_reply_frame()).unwrap();
                peer.write(&incoming).await.unwrap();
                let (received_at, _, _) = reads.recv().await.unwrap();
                let reply_tx = npdus.recv().await.unwrap().reply_tx.unwrap();
                if ready {
                    reply_tx.send(Bytes::from_static(&[1, 0, 0x30])).unwrap();
                }
                // In the other case the sender remains alive until the deadline.
                let (sent_at, _, bytes) = writes.recv().await.unwrap();
                let (frame, _) = decode_frame(&bytes).unwrap();
                assert_eq!(frame.destination, 7);
                assert_eq!(
                    frame.frame_type,
                    if ready {
                        FrameType::BACnetDataNotExpectingReply
                    } else {
                        FrameType::ReplyPostponed
                    }
                );
                let minimum = if ready {
                    Duration::from_micros(4167)
                } else {
                    Duration::from_millis(T_REPLY_DELAY_MS - 5 - T_REPLY_TRANSMIT_MARGIN_MS)
                };
                assert!(sent_at >= received_at + minimum);
                transport.stop().await.unwrap();
            })
            .await
            .expect("application reply stalled");
        }
    }
}

#[tokio::test]
async fn dedicated_abort_and_drop_release_serial_and_pending_reply() {
    for abort in [true, false] {
        for pending_reply in [true, false] {
            tokio::time::timeout(Duration::from_secs(2), async {
                let (serial, peer) = LoopbackSerial::pair();
                let mut transport = MstpTransport::new(serial, timing_config())
                    .with_execution_mode(MstpExecutionMode::DedicatedThread);
                let mut npdus = transport.start().await.unwrap();
                let reply_tx = if pending_reply {
                    let mut incoming = BytesMut::new();
                    encode_frame(&mut incoming, &expecting_reply_frame()).unwrap();
                    peer.write(&incoming).await.unwrap();
                    npdus.recv().await.unwrap().reply_tx
                } else {
                    None
                };
                if abort {
                    transport.abort();
                    assert!(transport.node_state().is_none());
                    assert!(transport.send_broadcast(&[1, 0]).await.is_err());
                }
                drop(transport);
                assert!(npdus.recv().await.is_none());
                assert!(peer.read(&mut [0; 64]).await.is_err());
                if let Some(reply) = reply_tx {
                    assert!(reply.send(Bytes::from_static(&[1, 0, 0x30])).is_err());
                }
            })
            .await
            .expect("dedicated task retained resources");
        }
    }
}
