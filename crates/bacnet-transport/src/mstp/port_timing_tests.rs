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
