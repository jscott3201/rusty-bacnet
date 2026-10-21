//! The hosted sender must choose among queued NPDUs, without preempting I/O.

use super::priority_harness::*;

#[derive(Clone, Copy)]
enum Traffic {
    Apdu,
    Network,
    Global,
}

/// Independent wire vectors: ingress on network 1000, egress on 2000.
fn packet(kind: Traffic, priority: u8, tag: u8) -> (Vec<u8>, Vec<u8>, Option<Vec<u8>>) {
    match kind {
        Traffic::Apdu => (
            vec![1, 0x20 | priority, 0x07, 0xd0, 1, tag, 10, 0x10, tag],
            vec![1, 0x08 | priority, 0x03, 0xe8, 1, 7, 0x10, tag],
            Some(vec![tag]),
        ),
        // A directed Who-Is-Router is routed before local handling.
        Traffic::Network => (
            vec![1, 0xa0 | priority, 0x07, 0xd0, 1, tag, 10, 0, 2, tag],
            vec![1, 0x88 | priority, 0x03, 0xe8, 1, 7, 0, 2, tag],
            Some(vec![tag]),
        ),
        Traffic::Global => (
            vec![1, 0x20 | priority, 0xff, 0xff, 0, 10, 0x10, tag],
            vec![
                1,
                0x28 | priority,
                0xff,
                0xff,
                0,
                0x03,
                0xe8,
                1,
                7,
                9,
                0x10,
                tag,
            ],
            None,
        ),
    }
}

#[tokio::test]
async fn queued_routed_traffic_uses_priority_then_arrival_order() {
    let (mut router, mut apdus, mut peers) = start(2).await;
    let (first, expected, to) = packet(Traffic::Apdu, 0, 0xaa);
    receive(&peers[0], first, 0xaa).await;
    let in_flight = sending(&mut peers[1]).await;
    assert_eq!(in_flight.npdu, expected);
    assert_eq!(in_flight.to, to);
    assert_eq!(in_flight.attributes, attributes(0xaa));

    let arrivals = [
        (Traffic::Apdu, 0, 0x10),
        (Traffic::Network, 1, 0x20),
        (Traffic::Global, 0, 0x11),
        (Traffic::Apdu, 3, 0x30),
        (Traffic::Network, 2, 0x40),
        (Traffic::Global, 3, 0x31),
    ];
    for (kind, priority, tag) in arrivals {
        receive(&peers[0], packet(kind, priority, tag).0, tag).await;
    }
    barrier(&peers[0], &mut apdus).await;
    assert!(peers[1].sent.try_recv().is_err(), "one in-flight send");
    assert_eq!(router.counters().refused, 0);
    in_flight.complete.send(Ok(())).unwrap();

    // LifeSafety A/B, Critical, Urgent, Normal A/B. FIFO spans traffic kinds.
    for index in [3, 5, 4, 1, 0, 2] {
        let (kind, priority, tag) = arrivals[index];
        let (_, expected, to) = packet(kind, priority, tag);
        let sent = sending(&mut peers[1]).await;
        assert_eq!(sent.npdu, expected, "queued item {tag:#x}");
        assert_eq!(sent.to, to);
        assert_eq!(sent.attributes, attributes(tag));
        sent.complete.send(Ok(())).unwrap();
    }
    router.stop().await;
}

#[tokio::test]
async fn full_held_port_refuses_new_work_without_stalling_another_port_or_shutdown() {
    let (mut router, mut apdus, mut peers) = start(3).await;
    receive(&peers[0], packet(Traffic::Apdu, 0, 0xaa).0, 0xaa).await;
    let in_flight = sending(&mut peers[1]).await;
    // The in-flight send has released its slot. All four priorities together
    // can admit 256 more, not 256 apiece and not a second prefetched backlog.
    for index in 0..256 {
        let priority = (index % 4) as u8;
        let tag = index as u8;
        let mut frame = packet(Traffic::Apdu, priority, tag).0;
        // Never address the router itself ([2] on this egress).
        frame[5] = 0x55;
        receive(&peers[0], frame, tag).await;
    }
    barrier(&peers[0], &mut apdus).await;
    assert_eq!(router.counters().refused, 0);
    receive(&peers[0], packet(Traffic::Apdu, 3, 0xbb).0, 0xbb).await;
    barrier(&peers[0], &mut apdus).await;
    assert_eq!(router.counters().refused, 1);
    assert!(peers[1].sent.try_recv().is_err());

    // An independent egress still starts a send while the first is blocked.
    let (mut other, expected, to) = packet(Traffic::Apdu, 0, 0xcc);
    other[2..4].copy_from_slice(&3000u16.to_be_bytes());
    receive(&peers[0], other, 0xcc).await;
    let sent = sending(&mut peers[2]).await;
    assert_eq!(sent.npdu, expected);
    assert_eq!(sent.to, to);
    assert_eq!(sent.attributes, attributes(0xcc));
    sent.complete.send(Ok(())).unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(2), router.stop())
        .await
        .expect("stop must cancel in-flight I/O without draining the queue");
    assert!(
        in_flight.complete.is_closed(),
        "the send future was dropped"
    );
    assert_eq!(router.counters().refused, 1, "stop is not a new refusal");
    for peer in &mut peers {
        assert!(peer.frames.is_closed(), "dispatch was joined");
        assert!(matches!(
            peer.sent.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
        ));
    }
}
