//! Answers that come after the core has finished their request: kept in a
//! bounded set and sent with the token (Clause 9.5.6.9 DeferredReply), or
//! given up and counted.

use bacnet_mstp_codec::FrameType;
use bytes::Bytes;
use tokio::sync::oneshot;
use tokio::time::{Duration, Instant};

use super::replies::DEFERRED_REPLIES;
use super::test_support::{config, Bus, Sent};

const REQUEST: &[u8] = &[1, 0x04, 0x10];
const ANSWER: &[u8] = &[1, 0x00, 0x30];

/// Station `source` asks station 3 for a reply, once `after` has ended and
/// T_turnaround passed; the application keeps the reply channel. Returns
/// the station's Reply Postponed and the channel.
async fn postponed(bus: &mut Bus, after: Instant, source: u8) -> (Sent, oneshot::Sender<Bytes>) {
    bus.until(after + bus.turnaround).await;
    bus.send(FrameType::BACNET_DATA_EXPECTING_REPLY, 3, source, REQUEST);
    let reply = bus.npdus.recv().await.unwrap().reply_tx.unwrap();
    let sent = bus.next().await;
    assert_eq!(
        (sent.frame_type, sent.destination),
        (FrameType::REPLY_POSTPONED, source)
    );
    (sent, reply)
}

/// After Reply Postponed the application's answer is still taken, and goes
/// out as a data frame to the requester when the station next holds the
/// token, as a deferred reply does.
#[tokio::test(start_paused = true)]
async fn an_answer_after_reply_postponed_goes_out_with_the_token() {
    let mut bus = Bus::start(config(3, 4)).await;
    let token = bus.join(4).await;
    let (sent, reply) = postponed(&mut bus, token.end, 4).await;
    reply
        .send(Bytes::from_static(ANSWER))
        .expect("the channel is kept after Reply Postponed");
    bus.until(sent.end + Duration::from_millis(5)).await;
    bus.send(FrameType::TOKEN, 3, 4, &[]);
    let answer = bus.next().await;
    assert_eq!(
        (
            answer.frame_type,
            answer.destination,
            answer.data.as_slice()
        ),
        (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 4, ANSWER)
    );
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.late_answers_queued, 1);
    assert_eq!(counts.reply_postponed_tx, 1);
    bus.stop().await;
}

/// An answer older than the queue's maximum age (5 s by default) would be
/// refused as expired once queued, so the channel is given up then: the
/// answer never goes out, and the give-up is counted.
#[tokio::test(start_paused = true)]
async fn an_answer_after_the_queue_age_is_dropped() {
    let mut bus = Bus::start(config(3, 4)).await;
    let token = bus.join(4).await;
    let (sent, reply) = postponed(&mut bus, token.end, 4).await;
    bus.until(sent.end + Duration::from_secs(5) + Duration::from_millis(30))
        .await;
    assert!(reply.is_closed(), "the channel outlived the queue age");
    assert!(reply.send(Bytes::from_static(ANSWER)).is_err());
    // The station runs on alone; none of its frames carries the answer.
    let until = Instant::now() + Duration::from_millis(500);
    while Instant::now() < until {
        let Ok(sent) = tokio::time::timeout_at(until, bus.next()).await else {
            break;
        };
        assert_ne!(sent.data, ANSWER, "an expired answer went out");
    }
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.late_answers_expired, 1);
    assert_eq!(counts.late_answers_queued, 0);
    bus.stop().await;
}

/// The channel goes at its expiry even when nothing else happens then: the
/// task wakes for it. Traffic between other stations keeps the station in
/// IDLE until shortly before the expiry, then the line falls quiet, so the
/// station's own next deadline (T_no_token after that traffic, 500 ms) comes
/// well after the expiry.
#[tokio::test(start_paused = true)]
async fn a_channel_expires_on_time_on_a_quiet_line() {
    const TICK: Duration = Duration::from_millis(1);
    let mut bus = Bus::start(config(3, 4)).await;
    let token = bus.join(4).await;
    bus.until(token.end + bus.turnaround).await;
    let arrived = bus.send(FrameType::BACNET_DATA_EXPECTING_REPLY, 3, 4, REQUEST);
    let reply = bus.npdus.recv().await.unwrap().reply_tx.unwrap();
    let sent = bus.next().await;
    assert_eq!(sent.frame_type, FrameType::REPLY_POSTPONED);
    let expires = arrived + Duration::from_secs(5);
    let mut at = sent.end + Duration::from_millis(300);
    while at + Duration::from_millis(100) <= expires {
        bus.until(at).await;
        bus.send(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 9, 4, &[1, 0, 1]);
        at += Duration::from_millis(300);
    }
    bus.until(expires - TICK).await;
    assert!(!reply.is_closed(), "the channel went before its expiry");
    bus.until(expires + TICK).await;
    assert!(reply.is_closed(), "the channel outlived its expiry");
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.late_answers_expired, 1);
    bus.stop().await;
}

/// At most eight finished requests keep their channels: a ninth drops the
/// oldest, counted, whose sender then fails; the newest still works.
#[tokio::test(start_paused = true)]
async fn a_full_set_drops_the_oldest() {
    let mut bus = Bus::start(config(3, 127)).await;
    let mut after = Instant::now();
    let mut replies = Vec::new();
    for source in 0..=DEFERRED_REPLIES as u8 {
        let (sent, reply) = postponed(&mut bus, after, 10 + source).await;
        after = sent.end;
        replies.push(reply);
    }
    tokio::time::sleep(Duration::from_millis(1)).await;
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.late_answers_dropped, 1);
    assert!(replies[0].is_closed(), "the oldest was kept");
    let newest = replies.pop().unwrap();
    newest.send(Bytes::from_static(ANSWER)).unwrap();
    // Taken once the last Reply Postponed has left.
    bus.until(after + Duration::from_millis(1)).await;
    assert_eq!(
        bus.transport.diagnostics().snapshot().late_answers_queued,
        1
    );
    bus.stop().await;
}

/// A channel whose sender is dropped leaves the set at once: after eight
/// dropped ones, eight more requests fit without dropping any.
#[tokio::test(start_paused = true)]
async fn a_dropped_sender_leaves_nothing_behind() {
    let mut bus = Bus::start(config(3, 127)).await;
    let mut after = Instant::now();
    for round in 0..2u8 {
        let mut replies = Vec::new();
        for source in 0..DEFERRED_REPLIES as u8 {
            let (sent, reply) = postponed(&mut bus, after, 10 + source).await;
            after = sent.end;
            replies.push(reply);
        }
        if round == 0 {
            drop(replies);
        }
    }
    tokio::time::sleep(Duration::from_millis(1)).await;
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.late_answers_dropped, 0);
    assert_eq!(counts.late_answers_expired, 0);
    bus.stop().await;
}

/// A late answer keeps its request's expiry in the queue: its request's
/// arrival plus the queue's maximum age (5 s), not that age after the
/// answer. One that comes just before then, with no token until after it,
/// is dropped from the queue and never sent.
#[tokio::test(start_paused = true)]
async fn a_late_answer_expires_with_its_request() {
    let mut bus = Bus::start(config(3, 4)).await;
    let token = bus.join(4).await;
    bus.until(token.end + bus.turnaround).await;
    let arrived = bus.send(FrameType::BACNET_DATA_EXPECTING_REPLY, 3, 4, REQUEST);
    let reply = bus.npdus.recv().await.unwrap().reply_tx.unwrap();
    let sent = bus.next().await;
    assert_eq!(sent.frame_type, FrameType::REPLY_POSTPONED);
    // Traffic between other stations keeps this one in IDLE, without the
    // token, until the answer has expired.
    let age = Duration::from_secs(5);
    let answer_at = arrived + age - Duration::from_millis(10);
    let mut at = sent.end + Duration::from_millis(300);
    while at < answer_at {
        bus.until(at).await;
        bus.send(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 9, 4, &[1, 0, 1]);
        at += Duration::from_millis(300);
    }
    bus.until(answer_at).await;
    reply.send(Bytes::from_static(ANSWER)).unwrap();
    bus.until(arrived + age + Duration::from_millis(10)).await;
    bus.send(FrameType::TOKEN, 3, 4, &[]);
    let next = bus.next().await;
    assert_eq!(
        (next.frame_type, next.destination),
        (FrameType::TOKEN, 4),
        "the expired answer went out"
    );
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.late_answers_queued, 1);
    assert_eq!(counts.mac.unicast.drops_expired, 1);
    bus.stop().await;
}
