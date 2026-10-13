//! Real-clock runs in both execution modes, over loopback and (from the
//! serial tests) native pseudo-terminals: byte-for-byte output, the task's
//! thread, turnaround after chunked input, application answers, and
//! teardown.

use std::time::Duration;

use bacnet_mstp_codec::{decode_frame, FrameType};
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::test_support::{config, encode};
use super::{LoopbackSerial, MstpConfig, MstpExecutionMode, MstpTransport, SerialPort};
use crate::port::TransportPort;

/// Microseconds of T_turnaround (40 bit times, rounded up) per baud rate.
pub(crate) const TURNAROUNDS: [(u32, u64); 3] = [(9600, 4167), (38400, 1042), (76800, 521)];

/// A port that reports each read and write with its time and thread.
struct ExecutionObserved<S> {
    inner: S,
    reads: mpsc::UnboundedSender<(Instant, std::thread::ThreadId, usize)>,
    writes: mpsc::UnboundedSender<(Instant, std::thread::ThreadId, Vec<u8>)>,
}

impl<S: SerialPort> SerialPort for ExecutionObserved<S> {
    async fn read(&self, buf: &mut [u8]) -> Result<usize, bacnet_types::error::Error> {
        let count = self.inner.read(buf).await?;
        if count > 0 {
            let _ = self
                .reads
                .send((Instant::now(), std::thread::current().id(), count));
        }
        Ok(count)
    }

    async fn write(&self, data: &[u8]) -> Result<(), bacnet_types::error::Error> {
        let _ = self
            .writes
            .send((Instant::now(), std::thread::current().id(), data.to_vec()));
        self.inner.write(data).await
    }

    async fn drain(&self) -> Result<(), bacnet_types::error::Error> {
        self.inner.drain().await
    }
}

/// Shared host-timing and byte-order harness, also used by the native
/// serial tests. Station 3 queues two NPDUs and receives a Token (whole, or
/// with a late tail); it sends both, then polls for a successor. Returns the
/// bytes it wrote, which must match in every mode and backend.
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
                ..config(3, 127)
            },
        );
        if let Some(mode) = mode {
            transport = transport.with_execution_mode(mode);
        }
        let mut npdus = transport.start().await.unwrap();
        assert_eq!(npdus.max_capacity(), 64);
        transport.send_unicast(&[1, 0, 0x10], &[7]).await.unwrap();
        transport.send_unicast(&[1, 0, 0x20], &[8]).await.unwrap();
        let incoming = encode(FrameType::TOKEN, 3, 7, &[]);
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
        let turnaround = TURNAROUNDS
            .iter()
            .find(|(rate, _)| *rate == baud)
            .unwrap()
            .1;
        let mut expected_wire = Vec::new();
        let mut wire = Vec::new();
        for (index, (kind, destination, data)) in [
            (
                FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                7,
                &[1, 0, 0x10][..],
            ),
            (
                FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                8,
                &[1, 0, 0x20][..],
            ),
            (FrameType::POLL_FOR_MASTER, 4, &[][..]),
        ]
        .into_iter()
        .enumerate()
        {
            let (at, thread, bytes) = writes.recv().await.unwrap();
            assert_eq!(thread, worker_thread);
            if index == 0 {
                assert!(at >= last_read + Duration::from_micros(turnaround));
            }
            let (frame, consumed) = decode_frame(&bytes).unwrap();
            assert_eq!(consumed, bytes.len(), "one frame per write");
            assert_eq!(
                (
                    frame.header.frame_type,
                    frame.header.destination,
                    frame.header.source
                ),
                (kind, destination, 3)
            );
            assert_eq!(frame.data, data);
            expected_wire.extend(encode(kind, destination, 3, data));
            wire.extend_from_slice(&bytes);
        }
        assert_eq!(wire, expected_wire);
        let mut received_wire = Vec::new();
        while received_wire.len() < wire.len() {
            let mut buf = [0; 128];
            // Read only this exchange; later polls remain valid traffic.
            let remaining = (wire.len() - received_wire.len()).min(buf.len());
            let count = peer.read(&mut buf[..remaining]).await.unwrap();
            assert!(count > 0);
            received_wire.extend_from_slice(&buf[..count]);
        }
        assert_eq!(received_wire, wire, "actual backend output differs");
        transport.stop().await.unwrap();
        assert!(transport.send_broadcast(&[1, 0, 0x30]).await.is_err());
        assert!(npdus.recv().await.is_none());
        wire
    })
    .await
    .expect("execution harness stalled")
}

#[tokio::test]
async fn execution_modes_write_the_same_bytes() {
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
async fn execution_modes_answer_requests_in_time() {
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
                    config(3, 127),
                )
                .with_execution_mode(mode);
                let mut npdus = transport.start().await.unwrap();
                let request = encode(FrameType::BACNET_DATA_EXPECTING_REPLY, 3, 7, &[1, 4, 0x10]);
                peer.write(&request).await.unwrap();
                let (received_at, _, _) = reads.recv().await.unwrap();
                let reply_tx = npdus.recv().await.unwrap().reply_tx.unwrap();
                if ready {
                    reply_tx.send(Bytes::from_static(&[1, 0, 0x30])).unwrap();
                }
                // Otherwise the sender stays alive until the deadline.
                let (sent_at, _, bytes) = writes.recv().await.unwrap();
                let (frame, _) = decode_frame(&bytes).unwrap();
                assert_eq!(frame.header.destination, 7);
                let (kind, minimum) = if ready {
                    (
                        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                        Duration::from_micros(4167),
                    )
                } else {
                    (FrameType::REPLY_POSTPONED, Duration::from_millis(225))
                };
                assert_eq!(frame.header.frame_type, kind);
                assert!(sent_at >= received_at + minimum);
                assert!(sent_at < received_at + Duration::from_millis(250));
                transport.stop().await.unwrap();
            })
            .await
            .expect("application reply stalled");
        }
    }
}

/// Aborting or dropping the transport releases the serial port, the NPDU
/// channel and a pending request's reply channel. The task drops the reply
/// channels first, so by the time the application sees the NPDU channel
/// close or the port go, an answer is refused.
async fn abort_and_drop_release_serial_and_pending_reply(mode: MstpExecutionMode) {
    for abort in [true, false] {
        for pending_reply in [true, false] {
            tokio::time::timeout(Duration::from_secs(2), async {
                let (serial, peer) = LoopbackSerial::pair();
                let mut transport =
                    MstpTransport::new(serial, config(3, 127)).with_execution_mode(mode);
                let mut npdus = transport.start().await.unwrap();
                let reply_tx = if pending_reply {
                    let request =
                        encode(FrameType::BACNET_DATA_EXPECTING_REPLY, 3, 7, &[1, 4, 0x10]);
                    peer.write(&request).await.unwrap();
                    npdus.recv().await.unwrap().reply_tx
                } else {
                    None
                };
                if abort {
                    transport.abort();
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
            .expect("the task retained resources");
        }
    }
}

#[tokio::test]
async fn dedicated_abort_and_drop_release_serial_and_pending_reply() {
    abort_and_drop_release_serial_and_pending_reply(MstpExecutionMode::DedicatedThread).await;
}

/// On a multi-thread runtime the aborted task is dropped on a worker
/// thread, racing the test as the dedicated thread does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tokio_abort_and_drop_release_serial_and_pending_reply() {
    abort_and_drop_release_serial_and_pending_reply(MstpExecutionMode::Tokio).await;
}
