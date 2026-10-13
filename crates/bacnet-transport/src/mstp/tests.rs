//! Transport-level behaviour that is not a Clause 9 transition: the
//! TransportPort surface, configuration, queueing and the receive path.

use bacnet_mstp_codec::{FrameType, BROADCAST_MAC, MAX_STANDARD_MPDU_DATA};
use tokio::time::{Duration, Instant};

use super::test_support::{config, encode, Bus};
use super::*;
use crate::port::TransportPort;

#[test]
fn mstp_max_apdu_length() {
    let (s1, _s2) = LoopbackSerial::pair();
    let transport = MstpTransport::new(s1, MstpConfig::default());
    assert_eq!(transport.egress_apdu_limit(), 480);
    assert_eq!(transport.local_receive_apdu_capacity(), 480);
}

#[test]
fn local_mac_is_one_byte_and_broadcast_is_xff() {
    let (serial_a, _serial_b) = LoopbackSerial::pair();
    let transport = MstpTransport::new(serial_a, config(42, 127));
    assert_eq!(transport.local_mac(), &[42]);
    assert!(transport.is_broadcast_mac(&[0xFF]));
    assert!(!transport.is_broadcast_mac(&[0x01]));
    assert!(!transport.is_broadcast_mac(&[]));
}

#[test]
fn configuration_is_validated_by_the_core_rules() {
    assert!(MstpConfig::default().validate().is_ok());
    let cases = [
        (
            MstpConfig {
                max_master: 128,
                ..MstpConfig::default()
            },
            "max_master 128 exceeds MAX_MASTER",
        ),
        (
            config(9, 7),
            "this_station 9 exceeds configured max_master (7)",
        ),
        (
            MstpConfig {
                max_info_frames: 0,
                ..MstpConfig::default()
            },
            "max_info_frames must be in 1..=255",
        ),
        (
            MstpConfig {
                baud_rate: 0,
                ..MstpConfig::default()
            },
            "baud_rate must not be zero",
        ),
        (
            MstpConfig {
                baud_rate: 2_400,
                ..MstpConfig::default()
            },
            "baud_rate 2400 is outside",
        ),
    ];
    for (config, message) in cases {
        let error = config.validate().unwrap_err();
        assert!(
            matches!(&error, Error::Encoding(text) if text.contains(message)),
            "{config:?}: {error}"
        );
    }
    for baud in [9_600, 19_200, 38_400, 57_600, 76_800, 115_200] {
        let config = MstpConfig {
            baud_rate: baud,
            ..MstpConfig::default()
        };
        assert!(config.validate().is_ok(), "{baud}");
    }
}

#[tokio::test]
async fn loopback_serial_pair_and_echo() {
    let (a, b) = LoopbackSerial::pair();
    a.write(&[0x55, 0xFF, 0x01]).await.unwrap();
    let mut buf = [0u8; 16];
    let n = b.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], &[0x55, 0xFF, 0x01]);
    b.write(&[0xAA, 0xBB]).await.unwrap();
    let n = a.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], &[0xAA, 0xBB]);
    assert_eq!(a.echo(), Echo::None);

    // An echoing port hears its own write first, then its peer's.
    let a = a.with_echo(Echo::full());
    assert_eq!(a.echo(), Echo::full());
    a.write(&[1, 2]).await.unwrap();
    b.write(&[3]).await.unwrap();
    let n = a.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], &[1, 2]);
    let n = a.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], &[3]);
    let n = b.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], &[1, 2]);

    // The echo does not keep a port's line open once its peer is gone.
    drop(b);
    assert!(a.read(&mut buf).await.is_err());
}

/// An empty write puts nothing on the line: the peer's next read returns
/// the next octets written, never nothing, which would mean end of file.
#[tokio::test]
async fn loopback_serial_reads_never_return_nothing() {
    let (a, b) = LoopbackSerial::pair();
    let a = a.with_echo(Echo::full());
    a.write(&[]).await.unwrap();
    a.write(&[7]).await.unwrap();
    let mut buf = [0u8; 4];
    assert_eq!(b.read(&mut buf).await.unwrap(), 1);
    assert_eq!(buf[0], 7);
    assert_eq!(a.read(&mut buf).await.unwrap(), 1, "the echo too");
}

#[tokio::test]
async fn transport_start_stop_and_sends_outside_a_run() {
    let (serial_a, _serial_b) = LoopbackSerial::pair();
    let mut transport = MstpTransport::new(serial_a, config(0, 127));
    assert!(transport.send_broadcast(&[1, 0]).await.is_err());
    let _rx = transport.start().await.unwrap();
    transport.send_broadcast(&[1, 0]).await.unwrap();
    transport.send_unicast(&[1, 0], &[5]).await.unwrap();
    let error = transport.send_unicast(&[1], &[1, 2, 3, 4, 5, 6]).await;
    assert!(matches!(error, Err(Error::Encoding(_))));
    assert!(transport.start().await.is_err(), "a second start");
    transport.stop().await.unwrap();
    assert!(transport.send_broadcast(&[1, 0]).await.is_err());
}

#[tokio::test]
async fn a_failed_read_ends_the_task_and_later_sends() {
    let mut transport = MstpTransport::new(NoSerial, config(0, 127));
    let diagnostics = transport.diagnostics();
    let mut npdus = transport.start().await.unwrap();
    assert!(npdus.recv().await.is_none(), "the task ended");
    assert_eq!(diagnostics.snapshot().serial_read_errors, 1);
    let error = transport.send_broadcast(&[1, 0]).await.unwrap_err();
    assert!(
        matches!(error, Error::Transport(ref io) if io.kind() == std::io::ErrorKind::NotConnected)
    );
}

/// A port whose reads return nothing has reached its end of file: the third
/// such read in a row ends the task as a read error does, counted, the NPDU
/// channel closes, later sends are refused, and stopping returns.
#[tokio::test]
async fn three_empty_reads_in_a_row_are_the_end_of_the_port() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct Closed(Arc<AtomicUsize>);

    impl SerialPort for Closed {
        async fn write(&self, _data: &[u8]) -> Result<(), Error> {
            Ok(())
        }

        async fn read(&self, _buf: &mut [u8]) -> Result<usize, Error> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(0)
        }
    }

    let reads = Arc::new(AtomicUsize::new(0));
    let mut transport = MstpTransport::new(Closed(reads.clone()), config(0, 127));
    let diagnostics = transport.diagnostics();
    let mut npdus = transport.start().await.unwrap();
    assert!(npdus.recv().await.is_none(), "the task ended");
    assert_eq!(reads.load(Ordering::SeqCst), 3);
    assert_eq!(diagnostics.snapshot().serial_read_errors, 1);
    let error = transport.send_broadcast(&[1, 0]).await.unwrap_err();
    assert!(
        matches!(error, Error::Transport(ref io) if io.kind() == std::io::ErrorKind::NotConnected)
    );
    tokio::time::timeout(Duration::from_secs(2), transport.stop())
        .await
        .expect("stop hung")
        .unwrap();
}

/// Empty reads short of three in a row are not the end of the port, and
/// octets read between them start the count again: a port that returns
/// nothing twice before each frame keeps running and delivers every frame.
#[tokio::test]
async fn empty_reads_between_frames_are_not_the_end_of_the_port() {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// Returns its chunks in turn, an empty one as an empty read, then
    /// waits for ever.
    struct Scripted(Mutex<VecDeque<Vec<u8>>>);

    impl SerialPort for Scripted {
        async fn write(&self, _data: &[u8]) -> Result<(), Error> {
            Ok(())
        }

        async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
            let Some(chunk) = self.0.lock().unwrap().pop_front() else {
                return std::future::pending().await;
            };
            buf[..chunk.len()].copy_from_slice(&chunk);
            Ok(chunk.len())
        }
    }

    let fills = [0x10u8, 0x20, 0x30];
    let mut script = VecDeque::new();
    for fill in fills {
        script.extend([Vec::new(), Vec::new()]);
        script.push_back(encode(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            3,
            7,
            &[1, 0, fill],
        ));
    }
    let mut transport = MstpTransport::new(Scripted(Mutex::new(script)), config(3, 127));
    let diagnostics = transport.diagnostics();
    let mut npdus = transport.start().await.unwrap();
    for fill in fills {
        let npdu = tokio::time::timeout(Duration::from_secs(2), npdus.recv())
            .await
            .expect("the frame was delivered")
            .expect("the task runs");
        assert_eq!(npdu.npdu.as_ref(), &[1, 0, fill]);
    }
    assert_eq!(diagnostics.snapshot().serial_read_errors, 0);
    transport.stop().await.unwrap();
}

/// A unicast NPDU whose control octet asks for a reply goes out as Data
/// Expecting Reply; one that does not, and every broadcast, as Data Not
/// Expecting Reply.
#[tokio::test(start_paused = true)]
async fn frame_type_follows_the_npdu_and_destination() {
    let mut bus = Bus::start(MstpConfig {
        max_info_frames: 3,
        ..config(3, 4)
    })
    .await;
    let token = bus.join(4).await;
    bus.transport
        .send_unicast(&[1, 0x04, 0xA1], &[4])
        .await
        .unwrap();
    bus.return_token(&token, 4).await;
    let first = bus.next().await;
    assert_eq!(
        (first.frame_type, first.destination, first.data.as_slice()),
        (
            FrameType::BACNET_DATA_EXPECTING_REPLY,
            4,
            &[1, 0x04, 0xA1][..]
        )
    );
    // Answer it, then check the not-expecting unicast and the broadcast.
    bus.until(first.end + bus.turnaround).await;
    bus.send(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        3,
        4,
        &[1, 0, 0xB1],
    );
    let reply = bus.npdus.recv().await.unwrap();
    assert_eq!(reply.npdu.as_ref(), &[1, 0, 0xB1]);
    let token = bus.next().await;
    assert_eq!(token.frame_type, FrameType::TOKEN);

    bus.transport
        .send_unicast(&[1, 0, 0xA2], &[4])
        .await
        .unwrap();
    bus.transport
        .send_broadcast(&[1, 0x04, 0xA3])
        .await
        .unwrap();
    bus.return_token(&token, 4).await;
    let mut kinds = Vec::new();
    for _ in 0..2 {
        let sent = bus.next().await;
        kinds.push((sent.frame_type, sent.destination, sent.data));
    }
    kinds.sort_by_key(|(_, destination, _)| *destination);
    assert_eq!(
        kinds,
        [
            (
                FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                4,
                vec![1, 0, 0xA2]
            ),
            (
                FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                BROADCAST_MAC,
                vec![1, 0x04, 0xA3]
            ),
        ]
    );
    bus.stop().await;
}

/// Queued NPDUs go out by network priority, life safety first.
#[tokio::test(start_paused = true)]
async fn higher_priority_npdus_go_first() {
    let mut bus = Bus::start(config(3, 4)).await;
    let token = bus.join(4).await;
    bus.transport
        .send_unicast(&[1, 0x00, 0x01], &[4])
        .await
        .unwrap();
    bus.transport
        .send_unicast(&[1, 0x03, 0x02], &[4])
        .await
        .unwrap();
    bus.return_token(&token, 4).await;
    assert_eq!(bus.next().await.data, [1, 0x03, 0x02]);
    bus.stop().await;
}

/// The queues refuse an oversized NPDU before a full queue, and each class
/// holds its own 256 NPDUs.
#[tokio::test(start_paused = true)]
async fn queue_bounds_are_reported_at_once() {
    let bus = Bus::start(config(3, 4)).await;
    let oversize = vec![0; MAX_STANDARD_MPDU_DATA + 1];
    for _ in 0..super::queues::QUEUE_CAPACITY {
        bus.transport.send_broadcast(&[1, 0]).await.unwrap();
    }
    let full = bus.transport.send_broadcast(&[1, 0]).await.unwrap_err();
    assert!(
        matches!(full, Error::Transport(ref io) if io.kind() == std::io::ErrorKind::WouldBlock)
    );
    assert!(matches!(
        bus.transport.send_broadcast(&oversize).await,
        Err(Error::Encoding(_))
    ));
    assert!(matches!(
        bus.transport.send_unicast(&oversize, &[4]).await,
        Err(Error::Encoding(_))
    ));
    bus.transport.send_unicast(&[1, 0], &[4]).await.unwrap();
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.outbound_queue_full, 1);
    assert_eq!(counts.outbound_oversize, 2);
    assert_eq!(counts.mac.broadcast.drops_full, 1);
    assert_eq!(counts.mac.broadcast.depth, 256);
    assert_eq!(counts.mac.unicast.depth, 1);
    bus.stop().await;
}

/// One read may hold several frames and noise; each frame goes up.
#[tokio::test(start_paused = true)]
async fn coalesced_frames_and_noise_in_one_read_are_all_delivered() {
    let mut bus = Bus::start(config(3, 127)).await;
    let mut chunk = vec![0x00, 0x55, 0x13];
    for fill in [0x10u8, 0x20, 0x30] {
        chunk.extend(encode(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            3,
            7,
            &[1, 0, fill],
        ));
    }
    chunk.extend(encode(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        3,
        7,
        &[1; 501],
    ));
    bus.send_raw(&chunk);
    for fill in [0x10u8, 0x20, 0x30] {
        let npdu = bus.npdus.recv().await.unwrap();
        assert_eq!(npdu.npdu.as_ref(), &[1, 0, fill]);
        assert_eq!(npdu.source_mac.as_slice(), &[7]);
        assert!(!npdu.link_layer_group);
        assert!(npdu.reply_tx.is_none());
        assert!(npdu.provenance.is_unverified());
    }
    assert_eq!(bus.npdus.recv().await.unwrap().npdu.len(), 501);
    bus.stop().await;
}

/// A frame whose tail arrives in a later read, as from a USB adapter that
/// holds it back, is still one frame: the host's T_frame_abort covers the
/// gap. A gap longer than that drops the partial frame.
#[tokio::test(start_paused = true)]
async fn a_late_tail_completes_its_frame_within_the_host_frame_abort() {
    let mut bus = Bus::start(config(3, 127)).await;
    let frame = encode(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        3,
        7,
        &[1, 0, 0x44],
    );
    let split = frame.len() - 1;
    for (gap, delivered) in [(40, true), (HOST_FRAME_ABORT_MS + 1, false)] {
        bus.send_raw(&frame[..split]);
        tokio::time::sleep(Duration::from_millis(gap)).await;
        bus.send_raw(&frame[split..]);
        let npdu = tokio::time::timeout(Duration::from_millis(5), bus.npdus.recv()).await;
        assert_eq!(npdu.is_ok(), delivered, "gap {gap} ms");
    }
    let counts = bus.transport.diagnostics().snapshot().mac;
    assert_eq!(counts.frames_received, 1);
    assert_eq!(counts.frame_aborts, 1);
    bus.stop().await;
}

/// The station's first frame after a silence waits only for T_turnaround
/// after the last octet it heard, counted from the read that brought it.
#[tokio::test(start_paused = true)]
async fn turnaround_counts_from_the_last_chunk_read() {
    let mut bus = Bus::start(config(3, 127)).await;
    let frame = encode(FrameType::POLL_FOR_MASTER, 3, 7, &[]);
    bus.send_raw(&frame[..4]);
    tokio::time::sleep(Duration::from_millis(30)).await;
    let tail = bus.send_raw(&frame[4..]);
    let reply = bus.next().await;
    assert_eq!(
        (reply.frame_type, reply.destination),
        (FrameType::REPLY_TO_POLL_FOR_MASTER, 7)
    );
    assert!(reply.start >= tail + bus.turnaround);
    assert!(reply.start < tail + bus.turnaround + Duration::from_millis(2));
    let _ = Instant::now();
    bus.stop().await;
}

/// An echo with no grace could never be matched: start refuses it.
#[tokio::test]
async fn a_zero_echo_grace_is_refused() {
    assert!(Echo::full().validate().is_ok());
    assert!(Echo::None.validate().is_ok());
    let zero = Echo::Full {
        grace: Duration::ZERO,
    };
    assert!(matches!(zero.validate(), Err(Error::Encoding(text)) if text.contains("echo grace")));
    let (serial, _peer) = LoopbackSerial::pair();
    let mut transport = MstpTransport::new(serial.with_echo(zero), config(3, 127));
    assert!(transport.start().await.is_err());
}
