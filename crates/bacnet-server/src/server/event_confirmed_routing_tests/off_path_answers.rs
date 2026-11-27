//! An answer for a routed recipient through a station the notification was
//! not sent to (#2831, the server's half): it completes nothing, and teaches
//! no router (no answer does: `router_learning`).

use super::*;

/// Another station on the link, which answers for the recipient.
const STRANGER: &[u8] = &[10, 0, 0, 9, 0xBA, 0xC0];

/// Teach the harness ROUTER_A as the router to network 1000, as the
/// server's own router query does (#2831).
fn learn_router_a(harness: &Harness) {
    harness.learn_router(1000, ROUTER_A);
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
    learn_router_a(&harness);
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
    learn_router_a(&harness);
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

/// A station that has the server send it a routed request, as a COV
/// subscription made from its own address with a network source does, and
/// then acknowledges it, does not become that network's router: an answer
/// to an attempt sent through a station teaches nothing.
#[tokio::test]
async fn an_answer_through_the_station_a_request_went_to_teaches_no_router() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        60_000,
    )
    .await;
    let (operation, receiver) = harness
        .notification_transactions
        .reserve(
            canonical_routed_peer(1000, &[0x0B; 6]),
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        )
        .unwrap();
    operation.route_through(Some(STRANGER));
    let ack = Apdu::SimpleAck(SimpleAck {
        invoke_id: operation.invoke_id(),
        service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
    });
    let subscriber = NpduAddress {
        network: 1000,
        mac_address: MacAddr::from_slice(&[0x0B; 6]),
    };
    harness
        .dispatch_terminal(STRANGER, Some(subscriber), ack)
        .await;
    assert_eq!(receiver.await.unwrap(), CovAckResult::Ack);
    assert_eq!(harness.learned_routers.router(1000), None);
    harness.distribute().await;
    assert!(harness.unicast_frames().is_empty(), "still by broadcast");
}
