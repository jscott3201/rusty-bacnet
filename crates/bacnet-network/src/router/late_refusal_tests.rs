//! Controlled send completions test return paths and feedback ownership.

use super::priority_harness::*;
use super::BACnetRouter;
use bacnet_types::error::Error;
use std::sync::atomic::Ordering;

fn packet(dnet: u16, source: Option<(u16, &[u8])>, priority: u8, tag: u8) -> Vec<u8> {
    let mut bytes = vec![1, 0x20 | priority | if source.is_some() { 0x08 } else { 0 }];
    bytes.extend_from_slice(&dnet.to_be_bytes());
    bytes.extend_from_slice(&[1, 9]);
    if let Some((network, mac)) = source {
        bytes.extend_from_slice(&network.to_be_bytes());
        bytes.push(mac.len() as u8);
        bytes.extend_from_slice(mac);
    }
    bytes.extend_from_slice(&[10, 0x10, tag]);
    bytes
}

async fn counts(router: &BACnetRouter, expected: (u64, u64, u64)) {
    for _ in 0..10_000 {
        let c = router.counters();
        if (c.egress_too_long, c.refused, c.rejects) == expected {
            return;
        }
        tokio::task::yield_now().await;
    }
    let c = router.counters();
    assert_eq!((c.egress_too_long, c.refused, c.rejects), expected);
}

#[tokio::test]
async fn late_refusal_returns_to_local_routed_and_attached_origins_after_limit_changes_again() {
    for (source, return_port, to, expected) in [
        (None, 0, vec![7], vec![1, 0x80, 3, 4, 7, 0xd0]),
        (
            Some((3000, &[0x42, 0x43][..])),
            0,
            vec![7],
            vec![1, 0xa0, 0x0b, 0xb8, 2, 0x42, 0x43, 255, 3, 4, 7, 0xd0],
        ),
        (
            Some((2000, &[8][..])),
            1,
            vec![8],
            vec![1, 0x80, 3, 4, 7, 0xd0],
        ),
    ] {
        let (mut router, _apdus, mut peers) = start(2).await;
        let mut request = packet(2000, source, 3, 0xa0);
        request.resize(601, 0);
        receive(&peers[0], request, 0xa0).await;
        let send = sending(&mut peers[1]).await;
        assert!(send.npdu.len() > 480);
        // An actual refusal is already decided at the smaller limit. The
        // transport advertises a larger value again before returning it.
        peers[1].limit.store(480, Ordering::SeqCst);
        let error = Error::SendTooLong {
            length: send.npdu.len(),
            limit: 480,
        };
        peers[1].limit.store(1497, Ordering::SeqCst);
        send.complete.send(Err(error)).unwrap();
        let reply = sending(&mut peers[return_port]).await;
        assert_eq!(reply.to, Some(to));
        assert_eq!(reply.npdu, expected);
        assert_eq!(reply.attributes, attributes(0xa0));
        reply.complete.send(Ok(())).unwrap();
        counts(&router, (1, 1, 1)).await;
        assert_eq!(router.counters().undrained, 0);
        router.stop().await;
    }
}

#[tokio::test]
async fn an_io_failure_is_not_reclassified_by_a_smaller_current_limit() {
    let (mut router, _apdus, mut peers) = start(2).await;
    let mut request = packet(2000, None, 0, 0x20);
    request.resize(601, 0);
    receive(&peers[0], request, 0x20).await;
    let send = sending(&mut peers[1]).await;
    peers[1].limit.store(64, Ordering::SeqCst);
    send.complete
        .send(Err(Error::Transport(std::io::Error::other(
            "uncertain write",
        ))))
        .unwrap();
    receive(&peers[0], packet(2000, None, 0, 0x21), 0x21).await;
    let following = sending(&mut peers[1]).await;
    following.complete.send(Ok(())).unwrap();
    counts(&router, (0, 0, 0)).await;
    assert!(peers[0].sent.try_recv().is_err());
    router.stop().await;
}

#[tokio::test]
async fn forwarded_rejects_and_global_or_control_fanouts_are_silent_when_refused() {
    for request in [
        vec![1, 0xa0, 7, 0xd0, 1, 9, 10, 3, 4, 0, 99],
        vec![1, 0x20, 0xff, 0xff, 0, 10, 0x10, 0x55],
        vec![1, 0x80, 1, 0x0b, 0xb8],
    ] {
        let (mut router, _apdus, mut peers) = start(2).await;
        receive(&peers[0], request, 0x55).await;
        let send = sending(&mut peers[1]).await;
        assert_eq!(send.attributes, attributes(0x55));
        send.complete
            .send(Err(Error::SendTooLong {
                length: 1600,
                limit: 1500,
            }))
            .unwrap();
        receive(&peers[0], packet(2000, None, 0, 0x56), 0x56).await;
        let following = sending(&mut peers[1]).await;
        following.complete.send(Ok(())).unwrap();
        counts(&router, (1, 1, 0)).await;
        assert!(peers[0].sent.try_recv().is_err());
        router.stop().await;
    }
}

#[tokio::test]
async fn refusing_the_generated_reject_ends_without_another_reply() {
    let (mut router, _apdus, mut peers) = start(2).await;
    receive(&peers[0], packet(2000, None, 0, 0x50), 0x50).await;
    sending(&mut peers[1])
        .await
        .complete
        .send(Err(Error::SendTooLong {
            length: 1600,
            limit: 1500,
        }))
        .unwrap();
    let reply = sending(&mut peers[0]).await;
    assert_eq!(reply.npdu, [1, 0x80, 3, 4, 7, 0xd0]);
    reply
        .complete
        .send(Err(Error::SendTooLong {
            length: 1600,
            limit: 1500,
        }))
        .unwrap();
    counts(&router, (2, 2, 1)).await;
    assert!(peers[0].sent.try_recv().is_err());
    assert!(peers[1].sent.try_recv().is_err());
    router.stop().await;
}

#[tokio::test]
async fn a_late_reject_respects_the_existing_shared_queue_capacity_and_shutdown() {
    let (mut router, mut apdus, mut peers) = start(2).await;
    receive(&peers[1], packet(1000, None, 0, 0x10), 0x10).await;
    let held = sending(&mut peers[0]).await;
    for index in 0..256 {
        receive(
            &peers[1],
            packet(1000, None, (index % 4) as u8, index as u8),
            0,
        )
        .await;
    }
    barrier(&peers[1], &mut apdus).await;
    receive(&peers[0], packet(2000, None, 0, 0x20), 0x20).await;
    sending(&mut peers[1])
        .await
        .complete
        .send(Err(Error::SendTooLong {
            length: 1600,
            limit: 1500,
        }))
        .unwrap();
    // Original length refusal, then the full return queue refuses its one
    // generated reject. Neither admission evicts an existing queued send.
    counts(&router, (1, 2, 1)).await;
    assert!(peers[0].sent.try_recv().is_err());
    router.stop().await;
    assert!(held.complete.is_closed());
    for peer in &mut peers {
        assert!(peer.frames.is_closed());
        assert!(matches!(
            peer.sent.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
        ));
    }
}

#[tokio::test]
async fn feedback_owners_do_not_keep_closed_ingress_consumers_or_senders_alive() {
    let (mut router, mut apdus, peers) = start(2).await;
    drop(peers);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), apdus.recv())
            .await
            .unwrap()
            .is_none()
    );
    for sender in router.sender_tasks.drain(..) {
        tokio::time::timeout(std::time::Duration::from_secs(2), sender)
            .await
            .unwrap()
            .unwrap();
    }
    router.stop().await;
}

#[tokio::test]
async fn a_late_reject_joins_the_existing_priority_queue_in_fifo_order() {
    let (mut router, mut apdus, mut peers) = start(2).await;
    receive(&peers[1], packet(1000, None, 0, 0x10), 0x10).await;
    let held = sending(&mut peers[0]).await;
    for (priority, tag) in [(0, 0x20), (3, 0x30)] {
        receive(&peers[1], packet(1000, None, priority, tag), tag).await;
    }
    barrier(&peers[1], &mut apdus).await;
    receive(&peers[0], packet(2000, None, 3, 0x77), 0x77).await;
    sending(&mut peers[1])
        .await
        .complete
        .send(Err(Error::SendTooLong {
            length: 1600,
            limit: 1500,
        }))
        .unwrap();
    counts(&router, (1, 1, 1)).await;
    receive(&peers[1], packet(1000, None, 0, 0x40), 0x40).await;
    barrier(&peers[1], &mut apdus).await;
    held.complete.send(Ok(())).unwrap();
    // The generated reply has Normal priority: queued LifeSafety first,
    // then the earlier Normal send, then the reply, then the later Normal.
    for (priority, tag) in [(3, 0x30), (0, 0x20), (0, 0x77), (0, 0x40)] {
        let send = sending(&mut peers[0]).await;
        let (to, expected) = if tag == 0x77 {
            (vec![7], vec![1, 0x80, 3, 4, 7, 0xd0])
        } else {
            (vec![9], vec![1, 0x08 | priority, 7, 0xd0, 1, 7, 0x10, tag])
        };
        assert_eq!(send.to, Some(to));
        assert_eq!(send.npdu, expected);
        assert_eq!(send.attributes, attributes(tag));
        send.complete.send(Ok(())).unwrap();
    }
    router.stop().await;
}
