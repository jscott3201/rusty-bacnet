//! Observe cache boundaries through terminal dispatch and subsequent sends.

use super::*;
use bacnet_objects::notification_class::MAX_RECIPIENT_LIST_DESTINATIONS;

const ROUTER_B: &[u8] = &[10, 0, 0, 2, 0xBA, 0xC0];

/// Route one transition per chunk of at most `MAX_RECIPIENT_LIST_DESTINATIONS`
/// DNETs, one confirmed destination per DNET in `networks`, in order. Routing
/// refuses any class past the cap (#1124), so a span wider than the cap takes
/// several transitions; Notification Class 0 is swapped for one holding each
/// chunk before its transition.
async fn distribute_to_networks(harness: &Harness, networks: std::ops::Range<u16>) {
    let networks: Vec<u16> = networks.collect();
    let class = ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 0).unwrap();
    for chunk in networks.chunks(MAX_RECIPIENT_LIST_DESTINATIONS) {
        {
            let mut nc = NotificationClass::new(0, "NC-0").unwrap();
            for &network in chunk {
                nc.add_destination(destination_for(address_recipient(network, RECIPIENT), true))
                    .unwrap();
            }
            let mut db = harness.db.write().await;
            db.remove(&class).unwrap();
            db.add(Box::new(nc)).unwrap();
        }
        harness.distribute().await;
    }
}

async fn wait_for_frames(harness: &Harness, expected: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while harness.broadcast_frames().len() + harness.unicast_frames().len() < expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("notification workers reach the recording transport");
    assert_eq!(
        harness.broadcast_frames().len() + harness.unicast_frames().len(),
        expected,
        "no extra attempts before the long retry deadline"
    );
}

#[tokio::test]
async fn admitted_routes_stop_at_64_and_a_known_router_is_kept() {
    // One destination per DNET, one more than the cache holds.
    const DNETS: std::ops::Range<u16> = 1000..1065;
    let harness = Harness::new(Vec::new(), 60_000).await;
    distribute_to_networks(&harness, DNETS).await;
    wait_for_frames(&harness, 65).await;
    assert!(harness.unicast_frames().is_empty());
    let mut first = harness.broadcast_frames();
    first.sort_by_key(|frame| decode_confirmed(frame).0.destination.unwrap().network);
    for frame in &first {
        let (npdu, request) = decode_confirmed(frame);
        assert!(
            harness
                .ack_routed(
                    ROUTER_A,
                    npdu.destination.unwrap().network,
                    RECIPIENT,
                    request.invoke_id,
                )
                .await
        );
    }
    assert_eq!(harness.notification_transactions.active_count(), 0);

    distribute_to_networks(&harness, DNETS).await;
    wait_for_frames(&harness, 130).await;
    let unicasts = harness.unicast_frames();
    assert_eq!(unicasts.len(), 64, "exactly 64 admitted routes were cached");
    let broadcasts = harness.broadcast_frames();
    assert_eq!(broadcasts.len(), 66);
    let (uncached, request) = decode_confirmed(&broadcasts[65]);
    assert_eq!(uncached.destination.unwrap().network, 1064);
    for (router, frame) in &unicasts {
        assert_eq!(router.as_slice(), ROUTER_A);
        let (npdu, request) = decode_confirmed(frame);
        let network = npdu.destination.unwrap().network;
        // Another router's answer to a request sent through ROUTER_A
        // completes nothing (#2831); ROUTER_A's own does.
        if network == 1000 {
            assert!(
                !harness
                    .ack_routed(ROUTER_B, network, RECIPIENT, request.invoke_id)
                    .await
            );
        }
        assert!(
            harness
                .ack_routed(ROUTER_A, network, RECIPIENT, request.invoke_id)
                .await
        );
    }
    // The 65th network's request went by broadcast, so any router's answer
    // completes it, but the full cache learns nothing from it.
    assert!(
        harness
            .ack_routed(ROUTER_B, 1064, RECIPIENT, request.invoke_id)
            .await
    );

    distribute_to_networks(&harness, DNETS).await;
    wait_for_frames(&harness, 195).await;
    let unicasts = harness.unicast_frames();
    assert_eq!(unicasts.len(), 128);
    for (router, frame) in &unicasts[64..] {
        let (npdu, request) = decode_confirmed(frame);
        let destination = npdu.destination.unwrap();
        assert_eq!(destination.mac_address.as_slice(), RECIPIENT);
        assert_eq!(router.as_slice(), ROUTER_A, "the known router is kept");
        assert!(
            harness
                .ack_routed(ROUTER_A, destination.network, RECIPIENT, request.invoke_id)
                .await
        );
    }
    let broadcasts = harness.broadcast_frames();
    assert_eq!(broadcasts.len(), 67, "the 65th DNET still broadcasts");
    let (npdu, request) = decode_confirmed(&broadcasts[66]);
    assert_eq!(npdu.destination.unwrap().network, 1064);
    assert!(
        harness
            .ack_routed(ROUTER_B, 1064, RECIPIENT, request.invoke_id)
            .await
    );
    assert_eq!(harness.notification_transactions.active_count(), 0);
}

#[tokio::test]
async fn unmatched_routed_terminal_cannot_teach_or_replace_a_router() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        60_000,
    )
    .await;
    assert!(!harness.ack_routed(ROUTER_B, 1000, RECIPIENT, 7).await);
    harness.distribute().await;
    wait_for_frames(&harness, 1).await;
    assert!(
        harness.unicast_frames().is_empty(),
        "unmatched response did not teach"
    );
    let (_, request) = decode_confirmed(&harness.broadcast_frames()[0]);
    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, request.invoke_id)
            .await
    );
    assert!(
        !harness
            .ack_routed(ROUTER_B, 1000, RECIPIENT, request.invoke_id)
            .await
    );
    harness.distribute().await;
    wait_for_frames(&harness, 2).await;
    let unicasts = harness.unicast_frames();
    assert_eq!(unicasts.len(), 1);
    assert_eq!(
        unicasts[0].0.as_slice(),
        ROUTER_A,
        "late duplicate did not replace the router"
    );
    let (_, request) = decode_confirmed(&unicasts[0].1);
    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, request.invoke_id)
            .await
    );
}

#[tokio::test]
async fn admitted_empty_router_or_routed_address_does_not_teach() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        60_000,
    )
    .await;
    let (operation, receiver) = harness
        .notification_transactions
        .reserve(
            canonical_direct_peer(ROUTER_B),
            ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        )
        .unwrap();
    harness
        .dispatch_terminal(
            ROUTER_B,
            Some(NpduAddress {
                network: 1000,
                mac_address: MacAddr::new(),
            }),
            Apdu::SimpleAck(SimpleAck {
                invoke_id: operation.invoke_id(),
                service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
            }),
        )
        .await;
    assert_eq!(
        receiver.await.unwrap(),
        CovAckResult::Ack,
        "empty SADR uses ordinary direct admission"
    );
    drop(operation);
    harness.distribute().await;
    wait_for_frames(&harness, 1).await;
    assert!(
        harness.unicast_frames().is_empty(),
        "empty SADR cannot teach a DNET"
    );
    let (_, request) = decode_confirmed(&harness.broadcast_frames()[0]);
    assert!(
        harness
            .ack_routed(&[], 1000, RECIPIENT, request.invoke_id)
            .await
    );

    harness.distribute().await;
    wait_for_frames(&harness, 2).await;
    assert!(
        harness.unicast_frames().is_empty(),
        "empty immediate MAC cannot become a router"
    );
    let (_, request) = decode_confirmed(&harness.broadcast_frames()[1]);
    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, request.invoke_id)
            .await
    );
    harness.distribute().await;
    wait_for_frames(&harness, 3).await;
    let unicasts = harness.unicast_frames();
    assert_eq!(unicasts.len(), 1, "a valid routed terminal still teaches");
    assert_eq!(unicasts[0].0.as_slice(), ROUTER_A);
    let (_, request) = decode_confirmed(&unicasts[0].1);
    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, request.invoke_id)
            .await
    );
}
