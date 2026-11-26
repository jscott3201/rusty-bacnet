//! An answer for a routed recipient through a station the notification was
//! not sent to (#2831, the server's half): it completes nothing, and teaches
//! no router.

use super::*;

/// Another station on the link, which answers for the recipient.
const STRANGER: &[u8] = &[10, 0, 0, 9, 0xBA, 0xC0];

/// Teach the harness ROUTER_A as the router to network 1000: the first
/// notification goes by local broadcast, and ROUTER_A carries its answer.
async fn learn_router_a(harness: &Harness) {
    harness.distribute().await;
    let (_, first) = decode_confirmed(&harness.broadcast_frames()[0]);
    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, first.invoke_id)
            .await
    );
}

/// Once the router to the recipient's network is known, the notification
/// goes to it alone, and another station's answer for the recipient under
/// the right invoke ID does not complete it.
#[tokio::test]
async fn a_strangers_answer_for_a_routed_recipient_completes_nothing() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        60_000,
    )
    .await;
    learn_router_a(&harness).await;
    harness.distribute().await;
    let unicasts = harness.unicast_frames();
    assert_eq!(unicasts.len(), 1);
    assert_eq!(unicasts[0].0.as_slice(), ROUTER_A);
    let (_, second) = decode_confirmed(&unicasts[0].1);
    assert!(
        !harness
            .ack_routed(STRANGER, 1000, RECIPIENT, second.invoke_id)
            .await,
        "another station's answer completed the notification"
    );
    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, second.invoke_id)
            .await,
        "the router's own answer still completes it"
    );
}

/// The router a stranger's answer would teach is not learned: the next
/// notification still goes to ROUTER_A.
#[tokio::test]
async fn a_strangers_answer_does_not_replace_a_learned_router() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        60_000,
    )
    .await;
    learn_router_a(&harness).await;
    harness.distribute().await;
    let (_, second) = decode_confirmed(&harness.unicast_frames()[0].1);
    harness
        .ack_routed(STRANGER, 1000, RECIPIENT, second.invoke_id)
        .await;
    let router = harness.learned_routers.lock().await.cached_router(1000);
    assert_eq!(router.as_deref(), Some(ROUTER_A));
}

/// A Device recipient bound through ROUTER_A: another station's answer for
/// it completes nothing.
#[tokio::test]
async fn a_strangers_answer_for_a_bound_routed_device_completes_nothing() {
    let identifier = ObjectIdentifier::new(ObjectType::DEVICE, 91).unwrap();
    let mut bindings = DeviceBindingTable::new();
    bindings
        .insert_configured(
            DeviceBinding::routed(identifier, 1000, RECIPIENT, ROUTER_A).unwrap(),
            |_| false,
        )
        .unwrap();
    let harness = Harness::new_with_bindings(
        vec![destination_for(BACnetRecipient::Device(identifier), true)],
        60_000,
        bindings,
    )
    .await;
    harness.distribute().await;
    let (_, request) = decode_confirmed(&harness.unicast_frames()[0].1);
    assert!(
        !harness
            .ack_routed(STRANGER, 1000, RECIPIENT, request.invoke_id)
            .await,
        "another station's answer completed the notification"
    );
    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, request.invoke_id)
            .await
    );
}

/// Each refused answer is counted, and only those.
#[tokio::test]
async fn refused_answers_are_counted() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        60_000,
    )
    .await;
    learn_router_a(&harness).await;
    assert_eq!(harness.notification_transactions.off_path_answers(), 0);
    harness.distribute().await;
    let (_, request) = decode_confirmed(&harness.unicast_frames()[0].1);
    for _ in 0..2 {
        harness
            .ack_routed(STRANGER, 1000, RECIPIENT, request.invoke_id)
            .await;
    }
    harness
        .ack_routed(ROUTER_A, 1000, RECIPIENT, request.invoke_id)
        .await;
    assert_eq!(harness.notification_transactions.off_path_answers(), 2);
}

/// A learned router that stays silent is forgotten at the first retry,
/// which goes by local broadcast; the router that carries the answer to it
/// is learned in its place, and the next notification goes through it.
#[tokio::test(start_paused = true)]
async fn a_silent_learned_router_is_replaced_by_the_one_that_answers_the_retry() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        1_000,
    )
    .await;
    learn_router_a(&harness).await;
    harness.distribute().await;
    let (_, request) = decode_confirmed(&harness.unicast_frames()[0].1);
    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert_eq!(harness.broadcast_frames().len(), 2, "the retry broadcasts");
    assert_eq!(
        harness.learned_routers.lock().await.cached_router(1000),
        None
    );
    assert!(
        harness
            .ack_routed(STRANGER, 1000, RECIPIENT, request.invoke_id)
            .await,
        "any router may carry the answer to a broadcast retry"
    );
    harness.distribute().await;
    let unicasts = harness.unicast_frames();
    assert_eq!(unicasts.len(), 2);
    assert_eq!(unicasts[1].0.as_slice(), STRANGER);
}

/// An answer to a request sent by broadcast teaches nothing while a router
/// for its network is known: two recipients on one network, both sent by
/// broadcast before either was answered, the first answer through ROUTER_A,
/// the second through another station.
#[tokio::test]
async fn a_broadcast_answer_does_not_replace_a_router_learned_meanwhile() {
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
    let mut answered = Vec::new();
    for (frame, router) in broadcasts.iter().zip([ROUTER_A, STRANGER]) {
        let (npdu, request) = decode_confirmed(frame);
        let dadr = npdu.destination.unwrap().mac_address;
        answered.push(
            harness
                .ack_routed(router, 1000, &dadr, request.invoke_id)
                .await,
        );
    }
    assert_eq!(answered, [true, true]);
    assert_eq!(
        harness
            .learned_routers
            .lock()
            .await
            .cached_router(1000)
            .as_deref(),
        Some(ROUTER_A)
    );
}

/// With no retries, a learned router that stays silent is forgotten when
/// the notification ends unanswered, so the next one goes by broadcast.
#[tokio::test(start_paused = true)]
async fn with_no_retries_a_silent_learned_router_is_forgotten_at_the_end() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        1_000,
    )
    .await;
    harness.notification_transactions.set_retries(0);
    learn_router_a(&harness).await;
    harness.distribute().await;
    assert_eq!(harness.unicast_frames().len(), 1);
    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        harness.learned_routers.lock().await.cached_router(1000),
        None
    );
    harness.distribute().await;
    assert_eq!(harness.broadcast_frames().len(), 2);
}
