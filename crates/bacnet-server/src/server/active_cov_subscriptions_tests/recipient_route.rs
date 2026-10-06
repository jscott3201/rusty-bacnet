use super::*;
use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_services::cov::COVNotificationRequest;

fn router_b() -> Peer {
    Peer {
        mac: vec![0x0A, 0, 0, 9, 0xBA, 0xC0],
        ..routed()
    }
}
fn request(
    single: bool,
    confirmed: Option<bool>,
    lifetime: Option<u32>,
) -> (ConfirmedServiceChoice, BytesMut) {
    if single {
        subscribe_cov_property(av(1), (840, PV, None, Some(0.1), confirmed, lifetime))
    } else {
        subscribe_cov(840, av(1), confirmed, lifetime)
    }
}
// Deliberately bypass the valid-only typed encoder for the zero-lifetime wire case.
fn invalid_single_lifetime() -> (ConfirmedServiceChoice, BytesMut) {
    use bacnet_encoding::{primitives, tags};
    let mut bytes = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut bytes, 0, 840);
    primitives::encode_ctx_object_id(&mut bytes, 1, &av(1));
    primitives::encode_ctx_boolean(&mut bytes, 2, false);
    primitives::encode_ctx_unsigned(&mut bytes, 3, 0);
    tags::encode_opening_tag(&mut bytes, 4);
    primitives::encode_ctx_unsigned(&mut bytes, 0, PV.to_raw() as u64);
    tags::encode_closing_tag(&mut bytes, 4);
    (ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY, bytes)
}

/// Answer every confirmed COV notification sent so far, as its subscriber would.
fn acknowledge_confirmed(wire: &Wire) {
    for frame in wire.sent.frames() {
        let npdu = decode_npdu(frame.npdu.clone()).unwrap();
        if let Ok(Apdu::ConfirmedRequest(request)) = decode_apdu(npdu.payload) {
            wire.server.notification_transactions.admit_terminal(
                &frame.mac,
                npdu.destination.as_ref(),
                None,
                &Apdu::SimpleAck(SimpleAck {
                    invoke_id: request.invoke_id,
                    service_choice: request.service_choice,
                }),
            );
        }
    }
}

/// A confirmed initial notification commits its observation on the Ack (#896).
async fn initial_complete(wire: &Wire) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            acknowledge_confirmed(wire);
            let complete = {
                let mut table = wire.server.cov_table.write().await;
                let entries = table.subscriptions_for(&av(1));
                !entries.is_empty()
                    && entries
                        .iter()
                        .all(|entry| entry.last_notified_observation.is_some())
            };
            if complete {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("initial notification committed its observation");
}
async fn renewal_wire(single: bool) {
    let mut wire = Wire::start(ServerConfig::default()).await;
    for peer in [routed(), router_b()] {
        simple_ack(
            wire.send(&peer, request(single, Some(false), Some(300)))
                .await,
        );
        initial_complete(&wire).await;
    }
    let active = wire.active().await;
    let entries = wire.server.cov_table.read().await.len();
    wire.sent.clear();
    wire.server
        .write_local(
            &av(1),
            PV,
            None,
            PropertyValue::Real(21.0),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    let frames = wire.sent.frames();
    wire.server.stop().await.unwrap();
    assert_eq!(
        entries, 1,
        "one canonical ordinary/Single context through either router"
    );
    assert_eq!(active.len(), 1, "one Device property152 entry");
    assert_eq!(active[0].recipient.recipient, address(&routed()));
    assert_eq!(frames.len(), 1, "one later notification");
    assert_eq!(frames[0].mac.as_slice(), router_b().mac);
    let npdu = decode_npdu(frames[0].npdu.clone()).unwrap();
    assert_eq!(npdu.destination, routed().network);
    let Apdu::UnconfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
        panic!("expected unconfirmed COV notification");
    };
    assert_eq!(
        request.service_choice,
        UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION
    );
    let notification = COVNotificationRequest::decode(&request.service_request).unwrap();
    assert_eq!(notification.subscriber_process_identifier, 840);
    assert_eq!(notification.monitored_object_identifier, av(1));
}
#[tokio::test]
async fn ordinary_two_routers_renew_one_recipient_and_deliver_via_latest_route() {
    renewal_wire(false).await;
}
#[tokio::test]
async fn single_two_routers_renew_one_recipient_and_deliver_via_latest_route() {
    renewal_wire(true).await;
}

async fn live(wire: &Wire) -> crate::cov::CovSubscriptionSnapshot {
    wire.server
        .cov_table
        .write()
        .await
        .subscriptions_for(&av(1))[0]
        .clone()
}
async fn unchanged(wire: &Wire, before: &crate::cov::CovSubscriptionSnapshot) {
    let table = wire.server.cov_table.read().await;
    let current = table.get_subscription(before.key()).unwrap();
    assert!(table.is_current(before));
    assert_eq!(current.endpoint(), before.endpoint());
    assert_eq!(current.expires_at, before.expires_at);
    assert_eq!(
        current.issue_confirmed_notifications,
        before.issue_confirmed_notifications
    );
    assert_eq!(current.cov_increment, before.cov_increment);
    assert_eq!(
        current.last_notified_observation,
        before.last_notified_observation
    );
    assert_eq!(table.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn cov_recipient_wire_refusal_mode_lifetime_and_cross_router_cancel() {
    for single in [false, true] {
        let allowed = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let gate = allowed.clone();
        let mut wire = Wire::start(ServerConfig {
            mutation_authorizer: Some(Arc::new(move |_| gate.load(Ordering::SeqCst))),
            cov_policy: crate::cov::CovPolicy {
                max_subscriptions_per_peer: 1,
                ..Default::default()
            },
            ..Default::default()
        })
        .await;
        simple_ack(
            wire.send(&routed(), request(single, Some(false), Some(300)))
                .await,
        );
        initial_complete(&wire).await;
        let before = live(&wire).await;
        allowed.store(false, Ordering::SeqCst);
        error(
            wire.send(&router_b(), request(single, Some(true), Some(600)))
                .await,
            ErrorClass::SERVICES,
            ErrorCode::SERVICE_REQUEST_DENIED,
        );
        unchanged(&wire, &before).await;
        // Cancellation is authorized before canonical matching too.
        error(
            wire.send(&router_b(), request(single, None, None)).await,
            ErrorClass::SERVICES,
            ErrorCode::SERVICE_REQUEST_DENIED,
        );
        unchanged(&wire, &before).await;
        allowed.store(true, Ordering::SeqCst);
        if single {
            error(
                wire.send(&router_b(), invalid_single_lifetime()).await,
                ErrorClass::SERVICES,
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            unchanged(&wire, &before).await;
        }
        simple_ack(
            wire.send(&router_b(), request(single, Some(true), Some(600)))
                .await,
        );
        initial_complete(&wire).await;
        let current = live(&wire).await;
        assert_eq!(current.key(), before.key());
        assert!(!wire.server.cov_table.read().await.is_current(&before));
        assert_eq!(current.subscriber_mac.as_slice(), router_b().mac);
        let listed = wire.active().await;
        assert_eq!(listed.len(), 1);
        assert!(listed[0].issue_confirmed_notifications);
        assert_eq!(listed[0].time_remaining, 600);
        if !single {
            for lifetime in [None, Some(0)] {
                simple_ack(
                    wire.send(&routed(), request(false, Some(false), lifetime))
                        .await,
                );
                initial_complete(&wire).await;
                assert_eq!(live(&wire).await.expires_at, None);
                let listed = wire.active().await;
                assert_eq!(listed.len(), 1);
                assert_eq!(listed[0].time_remaining, 0);
                assert!(!listed[0].issue_confirmed_notifications);
            }
        }
        // Either immediate router cancels the same recipient key.
        for cancel_peer in [routed(), router_b()] {
            simple_ack(
                wire.send(&router_b(), request(single, Some(false), Some(300)))
                    .await,
            );
            initial_complete(&wire).await;
            simple_ack(wire.send(&cancel_peer, request(single, None, None)).await);
            assert!(wire.active().await.is_empty());
            assert!(wire.server.cov_table.read().await.is_empty());
        }
        wire.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn cov_recipient_wire_cleanup_expiry_and_delete_follow_current_route() {
    for single in [false, true] {
        let mut wire = Wire::start(ServerConfig::default()).await;
        for peer in [routed(), router_b()] {
            simple_ack(
                wire.send(&peer, request(single, Some(false), Some(300)))
                    .await,
            );
            initial_complete(&wire).await;
        }
        assert_eq!(
            wire.server
                .remove_peer_subscriptions(&routed().mac, routed().network.as_ref())
                .await,
            0
        );
        assert_eq!(wire.active().await.len(), 1);
        assert_eq!(
            wire.server
                .remove_peer_subscriptions(&router_b().mac, router_b().network.as_ref())
                .await,
            1
        );
        assert!(wire.active().await.is_empty());
        simple_ack(
            wire.send(&routed(), request(single, Some(false), Some(300)))
                .await,
        );
        initial_complete(&wire).await;
        let old = live(&wire).await;
        let mut expired = (*old).clone();
        expired.subscriber_mac = MacAddr::from_slice(&router_b().mac);
        expired.expires_at = Some(runtime_clock::now());
        wire.server
            .cov_table
            .write()
            .await
            .subscribe(expired)
            .unwrap();
        // Reached deadline supplied through the table seam: real wire projection
        // must omit it without waiting for the periodic purge.
        assert!(wire.active().await.is_empty());
        wire.server.cov_table.write().await.purge_expired();
        assert!(wire.server.cov_table.read().await.is_empty());
        for peer in [routed(), router_b()] {
            simple_ack(
                wire.send(&peer, request(single, Some(false), Some(300)))
                    .await,
            );
            initial_complete(&wire).await;
        }
        let mut bytes = BytesMut::new();
        DeleteObjectRequest {
            object_identifier: av(1),
        }
        .encode(&mut bytes);
        simple_ack(
            wire.send(&direct(), (ConfirmedServiceChoice::DELETE_OBJECT, bytes))
                .await,
        );
        assert!(wire.active().await.is_empty());
        assert!(wire.server.cov_table.read().await.is_empty());
        wire.server.stop().await.unwrap();
    }
}
