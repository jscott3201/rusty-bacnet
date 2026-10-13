//! Answering a Data Expecting Reply: the application's answer, Reply
//! Postponed, and the turnaround before either (Clause 9.5.6.9).

use bacnet_mstp_codec::{FrameType, MAX_STANDARD_MPDU_DATA};
use bytes::Bytes;
use tokio::time::{Duration, Instant};

use super::test_support::{config, Bus};

/// T_reply_delay less the core's default reply transmit margin (25 ms):
/// when the core stops waiting for the application.
const DECISION: Duration = Duration::from_millis(225);
const TICK: Duration = Duration::from_millis(1);

const REQUEST: &[u8] = &[1, 0x04, 0x10];

/// Station 7 asks station 3 for a reply.
async fn requested(bus: &mut Bus) -> (Instant, tokio::sync::oneshot::Sender<Bytes>) {
    let at = bus.send(FrameType::BACNET_DATA_EXPECTING_REPLY, 3, 7, REQUEST);
    let npdu = bus.npdus.recv().await.unwrap();
    assert_eq!(npdu.npdu.as_ref(), REQUEST);
    assert!(!npdu.link_layer_group);
    (
        at,
        npdu.reply_tx.expect("a request carries a reply channel"),
    )
}

/// An answer that is ready goes out as soon as T_turnaround has passed,
/// without waiting for T_reply_delay; a dropped channel gets Reply
/// Postponed at once instead.
#[tokio::test(start_paused = true)]
async fn a_ready_answer_or_a_dropped_channel_goes_out_after_turnaround() {
    for ready in [true, false] {
        let mut bus = Bus::start(config(3, 127)).await;
        let (at, reply) = requested(&mut bus).await;
        if ready {
            reply.send(Bytes::from_static(&[1, 0, 0x30])).unwrap();
        } else {
            drop(reply);
        }
        let sent = bus.next().await;
        assert_eq!(sent.destination, 7);
        if ready {
            assert_eq!(sent.frame_type, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY);
            assert_eq!(sent.data, [1, 0, 0x30]);
        } else {
            assert_eq!(sent.frame_type, FrameType::REPLY_POSTPONED);
        }
        assert!(sent.start >= at + bus.turnaround);
        assert!(sent.start <= at + bus.turnaround + TICK);
        bus.stop().await;
    }
}

/// An answer that comes after the turnaround has passed goes out at once.
#[tokio::test(start_paused = true)]
async fn a_later_answer_is_not_held_for_another_turnaround() {
    let mut bus = Bus::start(config(3, 127)).await;
    let (at, reply) = requested(&mut bus).await;
    bus.until(at + Duration::from_millis(10)).await;
    let ready = Instant::now();
    reply.send(Bytes::from_static(&[1, 0, 0x30])).unwrap();
    let sent = bus.next().await;
    assert_eq!(sent.frame_type, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY);
    assert!(sent.start <= ready + TICK);
    bus.until(sent.end + TICK).await;
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.dner_tx_direct, 1);
    bus.stop().await;
}

/// Without an answer the core sends Reply Postponed at its decision
/// deadline, inside T_reply_delay. The channel stays open for a late answer,
/// which goes out with the token (see `late_answer_tests`).
#[tokio::test(start_paused = true)]
async fn no_answer_gets_reply_postponed_before_the_reply_delay() {
    let mut bus = Bus::start(config(3, 127)).await;
    let (at, reply) = requested(&mut bus).await;
    let sent = bus.next().await;
    assert_eq!(
        (sent.frame_type, sent.destination),
        (FrameType::REPLY_POSTPONED, 7)
    );
    assert!(sent.start >= at + DECISION);
    assert!(sent.start <= at + DECISION + TICK);
    assert!(sent.start < at + Duration::from_millis(250));
    tokio::task::yield_now().await;
    assert!(reply.send(Bytes::from_static(&[1, 0, 0x30])).is_ok());
    bus.stop().await;
}

/// A second request while answering the first is dropped (the station can
/// answer one at a time, Clause 9.5.6.9) and does not move the first one's
/// deadline.
#[tokio::test(start_paused = true)]
async fn a_second_request_does_not_extend_the_first_ones_deadline() {
    let mut bus = Bus::start(config(3, 127)).await;
    let (at, _reply) = requested(&mut bus).await;
    bus.until(at + Duration::from_millis(100)).await;
    bus.send(FrameType::BACNET_DATA_EXPECTING_REPLY, 3, 8, &[1, 4, 0x20]);
    let sent = bus.next().await;
    assert_eq!(
        (sent.frame_type, sent.destination),
        (FrameType::REPLY_POSTPONED, 7)
    );
    assert!(sent.start <= at + DECISION + TICK);
    assert!(bus.npdus.try_recv().is_err(), "the second request went up");
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.mac.discarded_while_busy, 1);
    bus.stop().await;
}

/// Other traffic heard while the answer is being prepared moves the
/// answer's turnaround: it starts T_turnaround after that traffic.
#[tokio::test(start_paused = true)]
async fn later_traffic_moves_the_answers_turnaround() {
    let mut bus = Bus::start(config(3, 127)).await;
    let (at, reply) = requested(&mut bus).await;
    bus.until(at + Duration::from_millis(40)).await;
    let heard = bus.send(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        9,
        8,
        &[1, 0, 0x20],
    );
    reply.send(Bytes::from_static(&[1, 0, 0x30])).unwrap();
    let sent = bus.next().await;
    assert_eq!(sent.frame_type, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY);
    assert!(sent.start >= heard + bus.turnaround);
    assert!(sent.start <= heard + bus.turnaround + TICK);
    bus.stop().await;
}

/// An answer too long for a standard frame cannot be sent: the request is
/// postponed instead, and the answer counted as oversize.
#[tokio::test(start_paused = true)]
async fn an_oversized_answer_is_postponed_and_counted() {
    let mut bus = Bus::start(config(3, 127)).await;
    let (_at, reply) = requested(&mut bus).await;
    reply
        .send(Bytes::from(vec![0x55; MAX_STANDARD_MPDU_DATA + 1]))
        .unwrap();
    let sent = bus.next().await;
    assert_eq!(
        (sent.frame_type, sent.destination),
        (FrameType::REPLY_POSTPONED, 7)
    );
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.outbound_oversize, 1);
    assert_eq!(counts.dner_tx_direct, 0);
    bus.stop().await;
}

/// An answer handed over around the decision deadline is never lost. Before
/// it, the answer is the reply. After it, Reply Postponed has gone out and
/// the answer follows with the station's next token. Exactly at it, either
/// may happen, depending on which the task sees first; the test checks the
/// branch that ran.
#[tokio::test(start_paused = true)]
async fn an_answer_around_the_deadline_is_never_lost() {
    for (offset, expect_direct) in [(-1i32, Some(true)), (0, None), (1, Some(false))] {
        let mut bus = Bus::start(config(3, 4)).await;
        let token = bus.join(4).await;
        bus.until(token.end + bus.turnaround).await;
        let at = bus.send(FrameType::BACNET_DATA_EXPECTING_REPLY, 3, 4, REQUEST);
        let reply = bus.npdus.recv().await.unwrap().reply_tx.unwrap();
        let when = if offset < 0 {
            at + DECISION - TICK
        } else {
            at + DECISION + TICK * offset.unsigned_abs()
        };
        bus.until(when).await;
        reply
            .send(Bytes::from_static(&[1, 0, 0x31]))
            .expect("the channel is open on both sides of the deadline");
        let first = bus.next().await;
        assert_eq!(first.destination, 4);
        let direct = match first.frame_type {
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY => {
                assert_eq!(first.data, [1, 0, 0x31]);
                true
            }
            FrameType::REPLY_POSTPONED => {
                bus.until(first.end + bus.turnaround).await;
                bus.send(FrameType::TOKEN, 3, 4, &[]);
                let late = bus.next().await;
                assert_eq!(late.frame_type, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY);
                assert_eq!(late.data, [1, 0, 0x31]);
                bus.until(late.end + TICK).await;
                false
            }
            other => panic!("offset {offset}: {other:?} answered the request"),
        };
        if let Some(expected) = expect_direct {
            assert_eq!(direct, expected, "offset {offset} ms");
        }
        let counts = bus.transport.diagnostics().snapshot();
        assert_eq!(
            counts.late_answers_queued,
            u64::from(!direct),
            "offset {offset} ms"
        );
        assert_eq!(
            counts.reply_postponed_tx,
            u64::from(!direct),
            "offset {offset} ms"
        );
        bus.stop().await;
    }
}

/// A port whose read is always ready (a line that never falls quiet) cannot
/// hold back the core's deadlines: reads win the task's select, but a
/// deadline already due is serviced after each one. Here noise follows the
/// request without a pause, so no answer can get the quiet T_turnaround it
/// needs; the core still decides on Reply Postponed at its deadline, and
/// gives the request up, counted, once the requester may have stopped
/// waiting. Were the deadline starved, the core would hold the request for
/// as long as the noise lasts.
///
/// The test runs in real time: a paused clock advances only when every task
/// is idle, and the always-ready read keeps the station's task busy. Each
/// read spends Tokio's cooperative budget, as a real port's read does, so
/// the task still yields now and then and can be stopped. The bound is loose
/// (the request is given up about 253 ms after it arrived, the bound 2 s)
/// because the bug it guards against starves the deadline for ever, and a
/// loose bound keeps a busy machine from failing it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_always_readable_port_cannot_hold_back_a_deadline() {
    use super::{MstpTransport, SerialPort};
    use crate::port::TransportPort;

    struct Babbling {
        request: std::sync::Mutex<Option<Vec<u8>>>,
    }

    impl SerialPort for Babbling {
        async fn write(&self, _data: &[u8]) -> Result<(), bacnet_types::error::Error> {
            Ok(())
        }

        async fn read(&self, buf: &mut [u8]) -> Result<usize, bacnet_types::error::Error> {
            tokio::task::consume_budget().await;
            // After the request, noise: octets that start no frame.
            let octets = self
                .request
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| vec![0x00; 4]);
            buf[..octets.len()].copy_from_slice(&octets);
            Ok(octets.len())
        }
    }

    let request =
        super::test_support::encode(FrameType::BACNET_DATA_EXPECTING_REPLY, 3, 7, REQUEST);
    let babbling = Babbling {
        request: std::sync::Mutex::new(Some(request)),
    };
    let mut transport = MstpTransport::new(babbling, config(3, 127));
    let diagnostics = transport.diagnostics();
    let mut npdus = transport.start().await.unwrap();
    let _reply = npdus.recv().await.unwrap().reply_tx.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while diagnostics.snapshot().mac.expired_requests == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("a due deadline was starved by reads");
    assert_eq!(diagnostics.snapshot().serial_read_errors, 0);
    transport.stop().await.unwrap();
}
