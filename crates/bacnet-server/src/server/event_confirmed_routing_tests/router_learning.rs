//! The router an address recipient's notifications go through (#2831,
//! decision of 2026-11-26): learned or replaced only through the server's
//! own router query. No answer teaches one, silence forgets none, and while
//! none is known the notifications go by local broadcast.

use super::*;

/// Another station on the link.
const STRANGER: &[u8] = &[10, 0, 0, 9, 0xBA, 0xC0];

impl Harness {
    /// Know `router` for `network` at once, as two of the server's router
    /// queries a minute apart would have it (`router_replacement_tests`
    /// runs the queries themselves).
    pub(super) fn learn_router(&self, network: u16, router: &[u8]) {
        self.learned_routers.learn_for_test(network, router);
        assert_eq!(
            self.learned_routers.router(network).as_deref(),
            Some(router)
        );
    }
}

/// The first notification goes by local broadcast; the router carrying its
/// answer is not learned, so the next goes by broadcast too.
#[tokio::test]
async fn an_answer_to_a_broadcast_attempt_teaches_no_router() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        60_000,
    )
    .await;
    for round in 0..3 {
        harness.distribute().await;
        let broadcasts = harness.broadcast_frames();
        assert_eq!(broadcasts.len(), round + 1);
        assert!(harness.unicast_frames().is_empty(), "round {round}");
        let (_, request) = decode_confirmed(&broadcasts[round]);
        assert!(
            harness
                .ack_routed(ROUTER_A, 1000, RECIPIENT, request.invoke_id)
                .await,
            "any router may carry the answer to a broadcast attempt"
        );
        assert_eq!(harness.learned_routers.router(1000), None);
    }
}

/// A router the server's own query learned carries the next notification,
/// with the same DNET and DADR, by unicast to it.
#[tokio::test]
async fn a_router_the_query_learned_carries_the_next_notification() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        60_000,
    )
    .await;
    harness.learn_router(1000, ROUTER_A);
    harness.distribute().await;
    assert!(harness.broadcast_frames().is_empty());
    let unicasts = harness.unicast_frames();
    assert_eq!(unicasts.len(), 1);
    assert_eq!(unicasts[0].0.as_slice(), ROUTER_A);
    let (npdu, request) = decode_confirmed(&unicasts[0].1);
    assert!(npdu.expecting_reply);
    let destination = npdu.destination.unwrap();
    assert_eq!(destination.network, 1000);
    assert_eq!(destination.mac_address.as_slice(), RECIPIENT);
    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, request.invoke_id)
            .await
    );
}

/// The router stays silent: the retry goes by local broadcast, its answer
/// from another station completes the notification, and the router is
/// neither forgotten nor replaced. The next notification goes to it again.
#[tokio::test(start_paused = true)]
async fn silence_forgets_nothing_and_the_retry_goes_by_broadcast() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        1_000,
    )
    .await;
    harness.learn_router(1000, ROUTER_A);
    harness.distribute().await;
    let (_, request) = decode_confirmed(&harness.unicast_frames()[0].1);
    tokio::time::sleep(Duration::from_millis(1_001)).await;
    assert_eq!(harness.broadcast_frames().len(), 1, "the retry broadcasts");
    assert!(
        harness
            .ack_routed(STRANGER, 1000, RECIPIENT, request.invoke_id)
            .await,
        "any router may carry the answer to a broadcast retry"
    );
    assert_eq!(
        harness.learned_routers.router(1000).as_deref(),
        Some(ROUTER_A)
    );
    harness.distribute().await;
    let unicasts = harness.unicast_frames();
    assert_eq!(unicasts.len(), 2);
    assert_eq!(unicasts[1].0.as_slice(), ROUTER_A);
}

/// With no retries, the one attempt through a silent router ends the
/// notification unanswered and forgets nothing.
#[tokio::test(start_paused = true)]
async fn with_no_retries_a_silent_router_is_kept() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        1_000,
    )
    .await;
    harness.notification_transactions.set_retries(0);
    harness.learn_router(1000, ROUTER_A);
    harness.distribute().await;
    assert_eq!(harness.unicast_frames().len(), 1);
    tokio::time::sleep(Duration::from_millis(1_001)).await;
    assert_eq!(harness.notification_transactions.active_count(), 0);
    assert_eq!(
        harness.learned_routers.router(1000).as_deref(),
        Some(ROUTER_A)
    );
    harness.distribute().await;
    assert!(harness.broadcast_frames().is_empty());
    assert_eq!(harness.unicast_frames().len(), 2);
}

/// An answer from the router itself only keeps it: two recipients on one
/// network, both answered, one through ROUTER_A and one through a stranger,
/// teach nothing while no router is known.
#[tokio::test]
async fn answers_through_two_stations_teach_nothing() {
    let harness = Harness::new(
        vec![
            destination_for(address_recipient(1000, RECIPIENT), true),
            destination_for(address_recipient(1000, &[0x0B; 6]), true),
        ],
        60_000,
    )
    .await;
    harness.distribute().await;
    let broadcasts = harness.broadcast_frames();
    assert_eq!(broadcasts.len(), 2);
    for (frame, router) in broadcasts.iter().zip([ROUTER_A, STRANGER]) {
        let (npdu, request) = decode_confirmed(frame);
        let dadr = npdu.destination.unwrap().mac_address;
        assert!(
            harness
                .ack_routed(router, 1000, &dadr, request.invoke_id)
                .await
        );
    }
    assert_eq!(harness.learned_routers.router(1000), None);
}

/// A stranger's answer under the right invoke ID neither completes a
/// notification sent through the known router nor replaces that router.
#[tokio::test]
async fn a_strangers_answer_does_not_replace_a_known_router() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        60_000,
    )
    .await;
    harness.learn_router(1000, ROUTER_A);
    harness.distribute().await;
    let (_, request) = decode_confirmed(&harness.unicast_frames()[0].1);
    assert!(
        !harness
            .ack_routed(STRANGER, 1000, RECIPIENT, request.invoke_id)
            .await
    );
    assert_eq!(
        harness.learned_routers.router(1000).as_deref(),
        Some(ROUTER_A)
    );
    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, request.invoke_id)
            .await
    );
}

/// A first attempt through the known router whose send failed never
/// reached it, so its retry says nothing of the router and starts no check;
/// one that went out and drew no answer starts the check, a Who-Is to the
/// recipient through the router.
#[tokio::test(start_paused = true)]
async fn only_an_attempt_that_went_out_through_the_router_starts_its_check() {
    use bacnet_network::layer::QueueAdmissionCounters;
    use bacnet_network::router_learning::RouterReports;
    for went_out in [false, true] {
        let harness = Harness::new(
            vec![destination_for(address_recipient(1000, RECIPIENT), true)],
            1_000,
        )
        .await;
        harness.learn_router(1000, ROUTER_A);
        let learner = {
            let (learning, link) = (
                Arc::clone(&harness.learned_routers),
                Arc::clone(&harness.network),
            );
            let reports = RouterReports::new(Some(QueueAdmissionCounters::default()));
            let wait = Duration::from_secs(3);
            tokio::spawn(async move { learning.run(&link, &reports, || true, wait).await })
        };
        if !went_out {
            harness.network.transport().handle().fail_next_send();
        }
        harness.distribute().await;
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        assert!(!harness.broadcast_frames().is_empty(), "the retry went");
        let probes = harness
            .unicast_frames()
            .into_iter()
            .filter(|(mac, frame)| {
                mac.as_slice() == ROUTER_A
                    && bacnet_encoding::npdu::decode_npdu(frame.clone())
                        .unwrap()
                        .payload
                        .as_ref()
                        == [0x10, 0x08]
            })
            .count();
        assert_eq!(probes, usize::from(went_out), "went out: {went_out}");
        learner.abort();
    }
}
