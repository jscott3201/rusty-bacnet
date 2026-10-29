//! Keep-alive on an outbound pooled direct link (#2127), on tokio's paused
//! clock at the smallest valid timing (3 s interval, 6 s timeout), with the
//! #2124 deadline: probe one interval after the last BVLC received, give up
//! one timeout after it. The idle TTL stays a separate reclaim timer.
use super::super::direct_membership::{DirectMembership, DirectRole};
use super::super::loopback::LoopbackWebSocket;
use super::*;
use tokio::sync::{Semaphore, mpsc};
use tokio::time::Instant as TokioInstant;

const INTERVAL: Duration = Duration::from_millis(3_000);
const TIMEOUT: Duration = Duration::from_millis(6_000);

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

struct Link {
    member: Arc<Membership>,
    worker: tokio::task::JoinHandle<()>,
    started: TokioInstant,
}

/// Starts a pooled worker over one end of a loopback pair; returns the peer.
fn start() -> (Link, LoopbackWebSocket) {
    let owner = Arc::new(DirectMembership::default());
    let member = owner
        .reserve([2; 16], [2; 6], [1; 16], [1; 6], DirectRole::Outbound, 16)
        .unwrap()
        .commit();
    let (local, peer) = LoopbackWebSocket::pair();
    let permits = Arc::new(Semaphore::new(1));
    let started = TokioInstant::now();
    let (pooled, worker) = PooledDirect::start(
        DirectSocket::Custom(local),
        member,
        permits.try_acquire_owned().unwrap(),
        #[cfg(feature = "sc-tls")]
        None,
        (1476, 1476),
        (INTERVAL.as_millis() as u64, TIMEOUT.as_millis() as u64),
    );
    (
        Link {
            member: pooled.member,
            worker,
            started,
        },
        peer,
    )
}

/// How a test peer answers a Heartbeat-Request with message ID `id`.
type Reply = fn(u16) -> bytes::BytesMut;

fn ack(id: u16) -> bytes::BytesMut {
    bare_control(ScFunction::HeartbeatAck, id)
}

fn wrong_id_ack(id: u16) -> bytes::BytesMut {
    bare_control(ScFunction::HeartbeatAck, id ^ 0x8000)
}

/// A confused peer's reply to a probe: a negative BVLC-Result.
fn nak(id: u16) -> bytes::BytesMut {
    let mut bytes = bytes::BytesMut::new();
    encode_sc_message(
        &mut bytes,
        &ScMessage {
            function: ScFunction::Result,
            message_id: id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: bytes::Bytes::from_static(&[0x0a, 0x01, 0x00, 0x00, 0x07, 0x00, 0x2b]),
        },
    );
    bytes
}

/// A peer that reports every frame it receives with its arrival time and,
/// when `reply` is set, answers each Heartbeat-Request after its delay.
fn spawn_peer(
    peer: Arc<LoopbackWebSocket>,
    reply: Option<(Duration, Reply)>,
) -> (
    tokio::task::JoinHandle<()>,
    mpsc::UnboundedReceiver<(TokioInstant, ScMessage)>,
) {
    let (seen_tx, seen) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        while let Ok(data) = peer.recv().await {
            let msg = decode_sc_message(&data).unwrap();
            let request = msg.function == ScFunction::HeartbeatRequest;
            let id = msg.message_id;
            let _ = seen_tx.send((TokioInstant::now(), msg));
            let (true, Some((delay, reply))) = (request, reply) else {
                continue;
            };
            tokio::time::sleep(delay).await;
            if peer.send(&reply(id)).await.is_err() {
                return;
            }
        }
    });
    (task, seen)
}

async fn advance_to(target: TokioInstant) {
    let now = TokioInstant::now();
    if target > now {
        tokio::time::advance(target - now).await;
    }
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

async fn next_frame(
    seen: &mut mpsc::UnboundedReceiver<(TokioInstant, ScMessage)>,
    what: &str,
) -> (TokioInstant, ScMessage) {
    tokio::time::timeout(INTERVAL * 3, seen.recv())
        .await
        .unwrap_or_else(|_| panic!("no frame: {what}"))
        .unwrap_or_else(|| panic!("peer gone: {what}"))
}

fn assert_heartbeat(msg: &ScMessage) {
    assert_eq!(msg.function, ScFunction::HeartbeatRequest);
    assert!(msg.originating_vmac.is_none() && msg.destination_vmac.is_none());
    assert!(msg.data_options.is_empty() && msg.payload.is_empty());
}

/// Nothing received: the worker probes after one interval and, with no
/// ACK, closes exactly at the timeout without a Disconnect-Request.
#[tokio::test(start_paused = true)]
async fn direct_pool_idle_link_probes_then_closes_at_the_timeout() {
    let (link, peer) = start();
    let (peer_task, mut seen) = spawn_peer(Arc::new(peer), None);
    let (at, msg) = next_frame(&mut seen, "first heartbeat").await;
    assert_heartbeat(&msg);
    assert_eq!(at - link.started, INTERVAL, "probe after one idle interval");
    advance_to(link.started + TIMEOUT - ms(1)).await;
    assert!(link.member.is_current(), "closed before the timeout");
    assert!(!link.worker.is_finished());
    advance_to(link.started + TIMEOUT).await;
    assert!(
        !link.member.is_current(),
        "an unanswered heartbeat must close the link at the timeout"
    );
    link.worker.await.unwrap();
    assert!(
        seen.try_recv().is_err(),
        "no second probe and no Disconnect-Request to a dead peer"
    );
    peer_task.abort();
}

/// An ACK whose message ID doesn't match the outstanding probe proves
/// nothing: the link still closes at the timeout.
#[tokio::test(start_paused = true)]
async fn direct_pool_ack_with_the_wrong_message_id_is_ignored() {
    let (link, peer) = start();
    let (peer_task, mut seen) = spawn_peer(Arc::new(peer), Some((ms(1), wrong_id_ack)));
    let (_, msg) = next_frame(&mut seen, "first heartbeat").await;
    assert_heartbeat(&msg);
    advance_to(link.started + TIMEOUT - ms(1)).await;
    assert!(link.member.is_current());
    advance_to(link.started + TIMEOUT).await;
    assert!(
        !link.member.is_current(),
        "a mismatched ACK must not reset the heartbeat timeout"
    );
    link.worker.await.unwrap();
    peer_task.abort();
}

/// A peer that ACKs each probe after 1 ms keeps the link open across many
/// intervals even when every timer is seen 2 ms late (the #2124 regression
/// shape). Heartbeats are not use, so the idle TTL still reclaims the link
/// on schedule, with a Disconnect-Request.
#[tokio::test(start_paused = true)]
async fn direct_pool_answered_heartbeats_survive_late_ticks_until_the_idle_ttl() {
    let (link, peer) = start();
    let (peer_task, mut seen) = spawn_peer(Arc::new(peer), Some((ms(1), ack)));
    let mut previous: Option<TokioInstant> = None;
    for cycle in 0..5 {
        let (h, msg) = next_frame(&mut seen, &format!("cycle {cycle}")).await;
        assert_heartbeat(&msg);
        if let Some(previous) = previous {
            assert_eq!(h - previous, INTERVAL + ms(1), "one interval after the ACK");
        }
        for target in [
            h + ms(1),
            h + INTERVAL,
            h + INTERVAL + ms(3),
            h + INTERVAL + ms(4),
            h + INTERVAL * 2 + ms(2),
        ] {
            advance_to(target).await;
            assert!(
                link.member.is_current(),
                "dropped a peer that answered every heartbeat (cycle {cycle})"
            );
        }
        // The late ticks above drew the next probe, itself 2 ms late; take
        // it so the next cycle measures from the right one.
        let (h2, msg) = next_frame(&mut seen, &format!("cycle {cycle} next")).await;
        assert_heartbeat(&msg);
        assert_eq!(h2 - h, INTERVAL + ms(3));
        previous = Some(h2);
    }
    // Sleep rather than jump: the paused clock then visits every timer in
    // order, so each probe gets its 1 ms ACK on the way to the TTL.
    let ttl = link.started + DIRECT_POOL_IDLE_TTL;
    tokio::time::sleep_until(ttl - ms(1)).await;
    advance_to(ttl - ms(1)).await;
    assert!(link.member.is_current(), "reclaimed before the idle TTL");
    advance_to(ttl).await;
    assert!(
        !link.member.is_current(),
        "heartbeats alone must not keep an unused link past the idle TTL"
    );
    link.worker.await.unwrap();
    let mut last = None;
    while let Ok((_, msg)) = seen.try_recv() {
        last = Some(msg.function);
    }
    assert_eq!(last, Some(ScFunction::DisconnectRequest));
    peer_task.abort();
}

/// Traffic received from the peer counts as liveness: it pushes the next
/// probe out to one interval after it.
#[tokio::test(start_paused = true)]
async fn direct_pool_received_traffic_defers_the_probe() {
    let (link, peer) = start();
    let peer = Arc::new(peer);
    let (peer_task, mut seen) = spawn_peer(peer.clone(), None);
    // Let the worker start its timers at `started` before the clock moves.
    advance_to(link.started).await;
    advance_to(link.started + ms(2_000)).await;
    let mut npdu = bytes::BytesMut::new();
    encode_sc_message(
        &mut npdu,
        &ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 7,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: bytes::Bytes::from_static(&[1, 0, 0x30]),
        },
    );
    peer.send(&npdu).await.unwrap();
    let (at, msg) = next_frame(&mut seen, "deferred heartbeat").await;
    assert_heartbeat(&msg);
    assert_eq!(
        at - link.started,
        ms(2_000) + INTERVAL,
        "probe one interval after the last frame received"
    );
    peer_task.abort();
    link.member.retire();
    link.worker.await.unwrap();
}

/// AB.6.3 leaves probing to the initiator, but a peer that probes anyway
/// gets a Heartbeat-ACK echoing its message ID.
#[tokio::test(start_paused = true)]
async fn direct_pool_answers_a_peer_heartbeat_request() {
    let (link, peer) = start();
    let peer = Arc::new(peer);
    let (peer_task, mut seen) = spawn_peer(peer.clone(), None);
    advance_to(link.started).await;
    advance_to(link.started + ms(1_000)).await;
    peer.send(&bare_control(ScFunction::HeartbeatRequest, 0x4242))
        .await
        .unwrap();
    let (at, reply) = next_frame(&mut seen, "heartbeat ack").await;
    assert_eq!(at - link.started, ms(1_000), "answered at once");
    assert_eq!(reply.function, ScFunction::HeartbeatAck);
    assert_eq!(reply.message_id, 0x4242, "the ACK echoes the request's ID");
    assert!(reply.originating_vmac.is_none() && reply.destination_vmac.is_none());
    assert!(reply.data_options.is_empty() && reply.payload.is_empty());
    peer_task.abort();
    link.member.retire();
    link.worker.await.unwrap();
}

/// A peer that answers every probe with a BVLC-Result NAK is alive, so the
/// link survives the heartbeat timeout, but a Result is not use: the idle
/// TTL still reclaims the link on schedule.
#[tokio::test(start_paused = true)]
async fn direct_pool_result_frames_do_not_hold_the_link_past_the_idle_ttl() {
    let (link, peer) = start();
    let (peer_task, mut seen) = spawn_peer(Arc::new(peer), Some((ms(1), nak)));
    let ttl = link.started + DIRECT_POOL_IDLE_TTL;
    tokio::time::sleep_until(ttl - ms(1)).await;
    advance_to(ttl - ms(1)).await;
    assert!(
        link.member.is_current(),
        "a NAK still shows the peer is alive"
    );
    advance_to(ttl).await;
    assert!(
        !link.member.is_current(),
        "Result frames must not keep an unused link past the idle TTL"
    );
    link.worker.await.unwrap();
    let mut probes = 0;
    let mut last = None;
    while let Ok((_, msg)) = seen.try_recv() {
        probes += usize::from(msg.function == ScFunction::HeartbeatRequest);
        last = Some(msg.function);
    }
    assert!(probes > 5, "probes kept going: {probes}");
    assert_eq!(last, Some(ScFunction::DisconnectRequest));
    peer_task.abort();
}
