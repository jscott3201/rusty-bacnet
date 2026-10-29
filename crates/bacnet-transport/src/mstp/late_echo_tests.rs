//! Echo that arrives short, late or split across the grace. The transport
//! must never drop a peer's octets, and never feed its own echo to the core
//! after the frame's completion, where it would read as another station's
//! traffic.

use std::sync::atomic::Ordering;

use bacnet_mstp_codec::FrameType;
use bytes::Bytes;
use tokio::time::{Duration, Instant};

use super::Echo;
use super::test_support::{Bus, DrainMode, Port, Sent, T_USAGE_TIMEOUT, config, encode};
use crate::port::TransportPort;

const TICK: Duration = Duration::from_millis(1);
const GRACE: Duration = Duration::from_millis(5);

/// A port whose echo is due within 5 ms; it echoes until a test turns that
/// off to script the echo itself.
const SHORT_GRACE: Port = Port {
    echo: Echo::Full { grace: GRACE },
    echoes: true,
    drain: DrainMode::Immediate,
};

/// The wire octets of a frame the station sent.
fn octets(sent: &Sent) -> Vec<u8> {
    encode(sent.frame_type, sent.destination, sent.source, &sent.data)
}

/// Station 3 in a ring with station 4, joined with the echo on, then left
/// to the test to echo. Returns the Token it passed on joining.
async fn ring(port: Port) -> (Bus, Sent) {
    let mut bus = Bus::start_with(config(3, 4), port).await;
    let token = bus.join(4).await;
    bus.echoing.store(false, Ordering::SeqCst);
    (bus, token)
}

/// A short echo, missing its first octets, followed at once by the peer's
/// reply in the same read: the echo cannot be matched, so all of it reaches
/// the core (as noise), and the reply after it is still a reply. WAIT_FOR_REPLY
/// takes it and passes the token (ReceivedReply, Clause 9.5.6.4).
#[tokio::test(start_paused = true)]
async fn a_short_echo_does_not_swallow_the_reply_behind_it() {
    let (mut bus, token) = ring(Port {
        echo: Echo::full(),
        ..SHORT_GRACE
    })
    .await;
    bus.transport
        .send_unicast(&[1, 4, 0x10], &[4])
        .await
        .unwrap();
    bus.return_token(&token, 4).await;
    let request = bus.next().await;
    assert_eq!(request.frame_type, FrameType::BACNET_DATA_EXPECTING_REPLY);
    bus.until(request.end + TICK).await;
    let mut chunk = octets(&request)[3..].to_vec();
    chunk.extend(encode(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        3,
        4,
        &[1, 0, 0x20],
    ));
    let heard = bus.send_raw(&chunk);
    let reply = bus.npdus.recv().await.unwrap();
    assert_eq!(reply.npdu.as_ref(), &[1, 0, 0x20]);
    let token = bus.next().await;
    assert_eq!((token.frame_type, token.destination), (FrameType::TOKEN, 4));
    assert!(token.start >= heard + bus.turnaround);
    assert!(token.start <= heard + bus.turnaround + TICK);
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.echo_mismatches, 1);
    assert_eq!(counts.wait_for_reply_timeouts, 0);
    assert_eq!(counts.mac.own_address_frames, 0);
    bus.stop().await;
}

/// With no echo at all, a peer's answer inside the grace starts the way
/// the station's frame did (the preamble); those octets are held, then
/// released with the rest at the first difference, so the answer arrives
/// whole: the polled station becomes the successor (ReceivedReplyToPFM,
/// Clause 9.5.6.8).
#[tokio::test(start_paused = true)]
async fn a_peer_frame_where_the_echo_should_be_reaches_the_core_whole() {
    let mut bus = Bus::start_with(
        config(3, 7),
        Port {
            echo: Echo::full(),
            echoes: false,
            drain: DrainMode::Immediate,
        },
    )
    .await;
    bus.send(FrameType::TOKEN, 3, 6, &[]);
    let poll = bus.next().await;
    assert_eq!(poll.destination, 4);
    bus.until(poll.end + bus.turnaround).await;
    bus.send(FrameType::REPLY_TO_POLL_FOR_MASTER, 3, 4, &[]);
    let token = bus.next().await;
    assert_eq!((token.frame_type, token.destination), (FrameType::TOKEN, 4));
    bus.stop().await;
}

/// An echo that arrives entirely after the grace, while the station waits
/// in PASS_TOKEN: the frame was completed at the grace, but the filter still
/// recognizes the echo and drops it, so it is not taken for the successor
/// using the token (SawTokenUser, Clause 9.5.6.6) and the station retries.
#[tokio::test(start_paused = true)]
async fn a_late_echo_in_pass_token_is_not_the_successor() {
    let (mut bus, token) = ring(SHORT_GRACE).await;
    bus.return_token(&token, 4).await;
    let passed = bus.next().await;
    assert_eq!(passed.frame_type, FrameType::TOKEN);
    bus.until(passed.end + GRACE + Duration::from_millis(3))
        .await;
    bus.send_raw(&octets(&passed));
    let retry = bus.next().await;
    assert_eq!((retry.frame_type, retry.destination), (FrameType::TOKEN, 4));
    assert!(retry.start >= passed.end + T_USAGE_TIMEOUT);
    assert!(retry.start <= passed.end + T_USAGE_TIMEOUT + TICK);
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.echo_missing, 1);
    assert_eq!(counts.echo_mismatches, 0);
    assert_eq!(counts.mac.own_address_frames, 0);
    assert_eq!(counts.mac.token_retries, 1);
    bus.stop().await;
}

/// A late echo in WAIT_FOR_REPLY is dropped too: fed, it would be an
/// unexpected frame and drop the token (Clause 9.5.6.4). The reply after it
/// ends the wait, and the station passes the token.
#[tokio::test(start_paused = true)]
async fn a_late_echo_in_wait_for_reply_keeps_the_token() {
    let (mut bus, token) = ring(SHORT_GRACE).await;
    bus.transport
        .send_unicast(&[1, 4, 0x10], &[4])
        .await
        .unwrap();
    bus.return_token(&token, 4).await;
    let request = bus.next().await;
    assert_eq!(request.frame_type, FrameType::BACNET_DATA_EXPECTING_REPLY);
    bus.until(request.end + GRACE + Duration::from_millis(3))
        .await;
    bus.send_raw(&octets(&request));
    bus.until(request.end + Duration::from_millis(15)).await;
    let heard = bus.send(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        3,
        4,
        &[1, 0, 0x20],
    );
    let token = bus.next().await;
    assert_eq!((token.frame_type, token.destination), (FrameType::TOKEN, 4));
    assert!(token.start <= heard + bus.turnaround + TICK, "{token:?}");
    assert_eq!(bus.npdus.recv().await.unwrap().npdu.as_ref(), &[1, 0, 0x20]);
    assert_eq!(
        bus.transport
            .diagnostics()
            .snapshot()
            .mac
            .own_address_frames,
        0
    );
    bus.stop().await;
}

/// A late echo in POLL_FOR_MASTER is dropped too: fed, it would be an
/// unexpected frame and drop the token (Clause 9.5.6.8). The station polls on
/// after T_usage_timeout.
#[tokio::test(start_paused = true)]
async fn a_late_echo_in_poll_for_master_keeps_the_token() {
    let mut bus = Bus::start_with(
        config(3, 7),
        Port {
            echoes: false,
            ..SHORT_GRACE
        },
    )
    .await;
    bus.send(FrameType::TOKEN, 3, 6, &[]);
    let poll = bus.next().await;
    assert_eq!(poll.destination, 4);
    bus.until(poll.end + GRACE + Duration::from_millis(3)).await;
    bus.send_raw(&octets(&poll));
    let next = bus.next().await;
    assert_eq!(
        (next.frame_type, next.destination),
        (FrameType::POLL_FOR_MASTER, 5)
    );
    assert!(next.start >= poll.end + T_USAGE_TIMEOUT);
    assert!(next.start <= poll.end + T_USAGE_TIMEOUT + TICK);
    bus.stop().await;
}

/// The grace runs out half way through the echo: the frame is completed
/// then, and the rest of the echo, arriving later, is still dropped.
#[tokio::test(start_paused = true)]
async fn the_rest_of_an_echo_cut_by_the_grace_is_still_dropped() {
    let (mut bus, token) = ring(SHORT_GRACE).await;
    bus.return_token(&token, 4).await;
    let passed = bus.next().await;
    let echo = octets(&passed);
    bus.until(passed.end + TICK).await;
    bus.send_raw(&echo[..4]);
    bus.until(passed.end + GRACE + Duration::from_millis(3))
        .await;
    bus.send_raw(&echo[4..]);
    let retry = bus.next().await;
    assert_eq!((retry.frame_type, retry.destination), (FrameType::TOKEN, 4));
    assert!(retry.start >= passed.end + T_USAGE_TIMEOUT);
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!((counts.echo_missing, counts.echo_mismatches), (0, 0));
    assert_eq!(counts.mac.own_address_frames, 0);
    assert_eq!(counts.mac.token_retries, 1);
    bus.stop().await;
}

/// A stray octet or two before the echo (a peer's trailing 0xFF pad, say,
/// or a lone preamble octet) pass on to the core, and the echo behind them
/// is still found and dropped: the station's own Token is not taken for the
/// successor using the token (SawTokenUser, Clause 9.5.6.6), so it retries.
#[tokio::test(start_paused = true)]
async fn stray_octets_before_the_echo_do_not_hide_it() {
    for stray in [&[0xFF][..], &[0x55], &[0x55, 0xFF]] {
        let (mut bus, token) = ring(Port {
            echo: Echo::full(),
            ..SHORT_GRACE
        })
        .await;
        bus.return_token(&token, 4).await;
        let passed = bus.next().await;
        assert_eq!(passed.frame_type, FrameType::TOKEN);
        bus.until(passed.end + TICK).await;
        let mut chunk = stray.to_vec();
        chunk.extend(octets(&passed));
        bus.send_raw(&chunk);
        let retry = bus.next().await;
        assert_eq!(
            (retry.frame_type, retry.destination),
            (FrameType::TOKEN, 4),
            "stray {stray:02x?}"
        );
        assert!(
            retry.start >= passed.end + T_USAGE_TIMEOUT,
            "stray {stray:02x?}"
        );
        let counts = bus.transport.diagnostics().snapshot();
        assert_eq!(counts.mac.own_address_frames, 0, "stray {stray:02x?}");
        assert_eq!(counts.mac.token_retries, 1, "stray {stray:02x?}");
        assert_eq!(counts.echo_mismatches, 1, "stray {stray:02x?}");
        bus.stop().await;
    }
}

/// A stray octet, then a peer's reply, with no echo at all: both reach the
/// core, and WAIT_FOR_REPLY takes the reply (ReceivedReply, Clause 9.5.6.4).
#[tokio::test(start_paused = true)]
async fn a_stray_octet_and_a_reply_without_echo_reach_the_core() {
    let (mut bus, token) = ring(Port {
        echo: Echo::full(),
        ..SHORT_GRACE
    })
    .await;
    bus.transport
        .send_unicast(&[1, 4, 0x10], &[4])
        .await
        .unwrap();
    bus.return_token(&token, 4).await;
    let request = bus.next().await;
    assert_eq!(request.frame_type, FrameType::BACNET_DATA_EXPECTING_REPLY);
    bus.until(request.end + bus.turnaround).await;
    let mut chunk = vec![0xFF];
    chunk.extend(encode(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        3,
        4,
        &[1, 0, 0x20],
    ));
    let heard = bus.send_raw(&chunk);
    assert_eq!(bus.npdus.recv().await.unwrap().npdu.as_ref(), &[1, 0, 0x20]);
    let token = bus.next().await;
    assert_eq!((token.frame_type, token.destination), (FrameType::TOKEN, 4));
    assert!(token.start <= heard + bus.turnaround + TICK);
    assert_eq!(
        bus.transport
            .diagnostics()
            .snapshot()
            .mac
            .own_address_frames,
        0
    );
    bus.stop().await;
}

/// The rest of an echo cut by the grace never comes: what was held of it
/// is dropped when the station's next start is confirmed (here the token
/// retry, the core having heard nothing), and counted.
#[tokio::test(start_paused = true)]
async fn an_unfinished_echo_is_dropped_and_counted_at_the_next_start() {
    let (mut bus, token) = ring(SHORT_GRACE).await;
    bus.return_token(&token, 4).await;
    let passed = bus.next().await;
    bus.until(passed.end + TICK).await;
    bus.send_raw(&octets(&passed)[..4]);
    let retry = bus.next().await;
    assert_eq!((retry.frame_type, retry.destination), (FrameType::TOKEN, 4));
    assert!(retry.start >= passed.end + T_USAGE_TIMEOUT);
    bus.until(retry.end).await;
    bus.send_raw(&octets(&retry));
    bus.until(retry.end + TICK).await;
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.echo_fragments_dropped, 1);
    assert_eq!((counts.echo_missing, counts.echo_mismatches), (0, 0));
    assert_eq!(counts.mac.own_address_frames, 0);
    bus.stop().await;
}

/// One stray octet where the echo should be, then nothing until the grace
/// runs out: the frame counts as a mismatch, and not also as missing.
#[tokio::test(start_paused = true)]
async fn a_mismatch_is_not_also_counted_missing() {
    let (mut bus, token) = ring(SHORT_GRACE).await;
    bus.return_token(&token, 4).await;
    let passed = bus.next().await;
    bus.until(passed.end + TICK).await;
    bus.send_raw(&[0xFF]);
    bus.until(passed.end + GRACE + TICK).await;
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!((counts.echo_mismatches, counts.echo_missing), (1, 0));
    bus.stop().await;
}

/// A start the core refuses leaves the echo hunt going. The echo of the
/// station's answer breaks off after the grace (the core hears a bad
/// header); when the next answer is handed over, the drain reads a pad
/// octet, which defers the start, and a peer's preamble, which the hunt
/// holds as a possible echo. The preamble still reaches the core with the
/// rest of its frame.
#[tokio::test(start_paused = true)]
async fn a_refused_start_keeps_a_peer_preamble_held_after_the_grace() {
    let mut bus = Bus::start_with(
        config(3, 127),
        Port {
            echoes: false,
            ..SHORT_GRACE
        },
    )
    .await;
    bus.send(FrameType::BACNET_DATA_EXPECTING_REPLY, 3, 7, &[1, 4, 0x10]);
    let first = bus.npdus.recv().await.unwrap().reply_tx.unwrap();
    first.send(Bytes::from_static(&[1, 0, 0x30])).unwrap();
    let answer = bus.next().await;
    let mut echo = octets(&answer);
    bus.until(answer.end + TICK).await;
    bus.send_raw(&echo[..3]);
    bus.until(answer.end + GRACE + TICK).await;
    echo[3] ^= 0x40;
    bus.send_raw(&echo[3..8]);
    bus.until(answer.end + GRACE + TICK * 2).await;
    bus.send(FrameType::BACNET_DATA_EXPECTING_REPLY, 3, 7, &[1, 4, 0x11]);
    let second = bus.npdus.recv().await.unwrap().reply_tx.unwrap();
    bus.until(Instant::now() + Duration::from_millis(10)).await;
    let peer = encode(FrameType::TOKEN, 10, 9, &[]);
    bus.send_raw(&[0xFF, peer[0], peer[1]]);
    second.send(Bytes::from_static(&[1, 0, 0x31])).unwrap();
    bus.until(Instant::now() + TICK).await;
    let heard = bus.send_raw(&peer[2..]);
    let sent = bus.next().await;
    assert_eq!(
        (sent.frame_type, sent.data),
        (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, vec![1, 0, 0x31])
    );
    assert!(sent.start >= heard + bus.turnaround);
    bus.until(sent.end + GRACE + TICK).await;
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.mac.deferred_starts, 1, "the start was not refused");
    assert_eq!(
        counts.mac.frames_received, 3,
        "two requests and the peer's Token"
    );
    assert_eq!(counts.echo_fragments_dropped, 0);
    bus.stop().await;
}

/// The probe after a grace with no echo ends at its own deadline: a copy of
/// the station's frame read after it is no longer taken for the echo and
/// reaches the core, as a frame from another station with this address
/// would (`own_address_frames`). Read inside the probe, it is the late echo.
#[tokio::test(start_paused = true)]
async fn the_probe_ends_at_its_deadline() {
    for (after_probe, fed) in [(false, false), (true, true)] {
        let (mut bus, token) = ring(SHORT_GRACE).await;
        bus.return_token(&token, 4).await;
        let passed = bus.next().await;
        let at = if after_probe {
            passed.end + GRACE * 2 + TICK * 2
        } else {
            passed.end + GRACE + TICK * 2
        };
        bus.until(at).await;
        bus.send_raw(&octets(&passed));
        tokio::time::sleep(TICK).await;
        let counts = bus.transport.diagnostics().snapshot();
        assert_eq!(
            counts.mac.own_address_frames,
            u64::from(fed),
            "after the probe: {after_probe}"
        );
        bus.stop().await;
    }
}

/// At a baud rate whose octet takes a whole millisecond, every instant the
/// test and the station use falls on the timer's millisecond grid, so a copy
/// of the station's frame can arrive at exactly the probe's end. Its deadline
/// is inclusive: the probe is over then, and the copy reaches the core as a
/// frame from another station with this address would. A millisecond before,
/// it is still the late echo.
#[tokio::test(start_paused = true)]
async fn the_probe_is_over_at_exactly_its_deadline() {
    for (before_the_end, fed) in [(TICK, false), (Duration::ZERO, true)] {
        let config = super::MstpConfig {
            baud_rate: 10_000,
            ..config(3, 4)
        };
        let mut bus = Bus::start_with(config, SHORT_GRACE).await;
        assert_eq!(bus.octet, TICK, "an octet takes a whole millisecond");
        let token = bus.join(4).await;
        bus.echoing.store(false, Ordering::SeqCst);
        bus.return_token(&token, 4).await;
        let passed = bus.next().await;
        // The grace ends with no echo, and the probe runs one more grace.
        bus.until(passed.end + GRACE * 2 - before_the_end).await;
        bus.send_raw(&octets(&passed));
        tokio::time::sleep(TICK).await;
        let counts = bus.transport.diagnostics().snapshot();
        assert_eq!(counts.echo_missing, 1);
        assert_eq!(
            counts.mac.own_address_frames,
            u64::from(fed),
            "{before_the_end:?} before the probe's end"
        );
        bus.stop().await;
    }
}

/// A port that reaches its end of file while the station waits for the echo
/// of its first frame: every read after the frame returns nothing at once.
/// The task ends at the third empty read, well inside the grace, so the
/// frame is never reported complete and no echo is counted missing, and
/// stopping the transport returns. Real time, so that if the echo wait went
/// on reading, the reads would not hold a paused clock still: the grace would
/// end and the missing echo show.
#[tokio::test]
async fn an_end_of_file_in_the_echo_wait_ends_the_task() {
    use super::{MstpTransport, SerialPort};

    /// Reads wait until the station writes, then return nothing.
    struct ClosesAfterWrite(tokio::sync::watch::Sender<bool>);

    impl SerialPort for ClosesAfterWrite {
        async fn write(&self, _data: &[u8]) -> Result<(), bacnet_types::error::Error> {
            self.0.send_replace(true);
            Ok(())
        }

        async fn read(&self, _buf: &mut [u8]) -> Result<usize, bacnet_types::error::Error> {
            let mut written = self.0.subscribe();
            let _ = written.wait_for(|written| *written).await;
            Ok(0)
        }

        fn echo(&self) -> Echo {
            Echo::Full {
                grace: Duration::from_millis(200),
            }
        }
    }

    let (written, wrote) = tokio::sync::watch::channel(false);
    let mut transport = MstpTransport::new(ClosesAfterWrite(written), config(3, 7));
    let diagnostics = transport.diagnostics();
    let mut npdus = transport.start().await.unwrap();
    assert!(npdus.recv().await.is_none(), "the task ended");
    assert!(*wrote.borrow(), "the station sent its first frame");
    let counts = diagnostics.snapshot();
    assert_eq!(counts.serial_read_errors, 1);
    assert_eq!(counts.echo_missing, 0, "the frame was completed");
    tokio::time::timeout(Duration::from_secs(2), transport.stop())
        .await
        .expect("stop hung")
        .unwrap();
}
