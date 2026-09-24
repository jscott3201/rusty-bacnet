use super::*;
use bacnet_services::common::PropertyReference;
use bacnet_services::cov::SubscribeCOVPropertyRequest;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};

fn single(cancel: bool) -> Bytes {
    let request = SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 77,
        monitored_object_identifier: point_oid(),
        issue_confirmed_notifications: (!cancel).then_some(false),
        lifetime: (!cancel).then_some(300),
        monitored_property_identifier: PropertyIdentifier::SILENCED,
        monitored_property_array_index: None,
        cov_increment: None,
    };
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes).unwrap();
    bytes.freeze()
}

fn multiple(cancel: bool) -> Bytes {
    let request = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 77,
        issue_confirmed_notifications: false,
        lifetime: (!cancel).then_some(300),
        max_notification_delay: (!cancel).then_some(5),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: point_oid(),
            list_of_cov_references: vec![COVReference {
                monitored_property: PropertyReference {
                    property_identifier: PropertyIdentifier::SILENCED,
                    property_array_index: None,
                },
                cov_increment: None,
                timestamped: false,
            }],
        }],
    };
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes).unwrap();
    bytes.freeze()
}

async fn fire(fixture: &DispatchFixture) -> Vec<u8> {
    BACnetServer::<RecordingTransport>::fire_life_safety_cov_notifications(
        &fixture.db,
        &fixture.network,
        &fixture.cov_table,
        &fixture.cov_in_flight,
        &fixture.transactions,
        &fixture.comm_state,
        &fixture.config,
        &point_oid(),
        &[PropertyIdentifier::SILENCED],
    )
    .await;
    let mut services: Vec<_> = fixture
        .take_apdus()
        .into_iter()
        .map(|apdu| match apdu {
            Apdu::UnconfirmedRequest(request) => request.service_choice.to_raw(),
            other => panic!("expected COV notification: {other:?}"),
        })
        .collect();
    services.sort_unstable();
    services
}

#[tokio::test]
async fn cov_identity_single_multiple_coexist_on_wire_and_cancel_independently() {
    let fixture = DispatchFixture::new(life_safety_db(), []).await;
    for (invoke, service, request, notification) in [
        (
            1,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
            single(false),
            UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
        ),
        (
            2,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            multiple(false),
            UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
        ),
    ] {
        fixture.dispatch(invoke, service, request).await;
        let apdus = fixture.take_apdus();
        assert_eq!(apdus.len(), 2);
        assert!(matches!(apdus[0], Apdu::SimpleAck(_)));
        assert!(
            matches!(&apdus[1], Apdu::UnconfirmedRequest(request) if request.service_choice == notification)
        );
    }
    assert_eq!(
        fire(&fixture).await,
        vec![2, 11],
        "both accepted families must retain fanout"
    );
    fixture
        .dispatch(
            3,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
            single(true),
        )
        .await;
    assert!(matches!(
        fixture.take_apdus().as_slice(),
        [Apdu::SimpleAck(_)]
    ));
    assert_eq!(fire(&fixture).await, vec![11]);
    fixture
        .dispatch(
            4,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            multiple(true),
        )
        .await;
    assert!(matches!(
        fixture.take_apdus().as_slice(),
        [Apdu::SimpleAck(_)]
    ));
    assert!(fire(&fixture).await.is_empty());
    assert!(fixture.cov_table.read().await.is_empty());
}

fn multiple_form(confirmed: bool, cancel: bool) -> Bytes {
    let mut request = SubscribeCOVPropertyMultipleRequest::decode(&multiple(cancel)).unwrap();
    request.issue_confirmed_notifications = confirmed;
    if cancel {
        request.list_of_cov_subscription_specifications.clear();
    }
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes).unwrap();
    bytes.freeze()
}

async fn take_after_confirmed(fixture: &DispatchFixture, count: usize) -> Vec<Apdu> {
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.sent.lock().unwrap().len() < count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let apdus = fixture.take_apdus();
    for apdu in &apdus {
        if let Apdu::ConfirmedRequest(request) = apdu {
            assert!(fixture.transactions.admit_terminal(
                &fixture.source_mac,
                None,
                &Apdu::SimpleAck(SimpleAck {
                    invoke_id: request.invoke_id,
                    service_choice: request.service_choice,
                })
            ));
        }
    }
    apdus
}

#[tokio::test]
async fn cov_identity_multiple_forms_coexist_refresh_and_cancel_exact_context() {
    let fixture = DispatchFixture::new(life_safety_db(), []).await;
    for (invoke, confirmed) in [(1, false), (2, true)] {
        fixture
            .dispatch(
                invoke,
                ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
                multiple_form(confirmed, false),
            )
            .await;
        let apdus = take_after_confirmed(&fixture, 2).await;
        assert!(matches!(apdus[0], Apdu::SimpleAck(_)));
        assert!(matches!(&apdus[1], Apdu::ConfirmedRequest(_)) == confirmed);
    }
    assert_eq!(fixture.cov_table.read().await.len(), 2);
    fixture
        .dispatch(
            3,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            multiple_form(false, false),
        )
        .await;
    take_after_confirmed(&fixture, 2).await;
    assert_eq!(fixture.cov_table.read().await.len(), 2);
    fixture
        .dispatch(
            4,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            multiple_form(false, true),
        )
        .await;
    assert!(matches!(
        fixture.take_apdus().as_slice(),
        [Apdu::SimpleAck(_)]
    ));
    BACnetServer::<RecordingTransport>::fire_life_safety_cov_notifications(
        &fixture.db,
        &fixture.network,
        &fixture.cov_table,
        &fixture.cov_in_flight,
        &fixture.transactions,
        &fixture.comm_state,
        &fixture.config,
        &point_oid(),
        &[PropertyIdentifier::SILENCED],
    )
    .await;
    let apdus = take_after_confirmed(&fixture, 1).await;
    assert!(matches!(&apdus[0], Apdu::ConfirmedRequest(request)
        if request.service_choice == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE));
    fixture
        .dispatch(
            5,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            multiple_form(true, true),
        )
        .await;
    assert!(matches!(
        fixture.take_apdus().as_slice(),
        [Apdu::SimpleAck(_)]
    ));
    assert!(fixture.cov_table.read().await.is_empty());
    fixture.transactions.close();
    while fixture.transactions.join_next().await.is_some() {}
}

#[tokio::test]
async fn cov_identity_two_router_paths_share_quota_but_not_cleanup_authority() {
    let fixture = DispatchFixture::new(life_safety_db(), []).await;
    let remote = NpduAddress {
        network: 23,
        mac_address: MacAddr::from_slice(&[4, 5]),
    };
    for router in [MacAddr::from_slice(&[1]), MacAddr::from_slice(&[2])] {
        fixture
            .dispatch_at(
                &router,
                Some(&remote),
                router[0],
                ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
                single(false),
            )
            .await;
        let frames = fixture.sent.lock().unwrap().clone();
        assert_eq!(frames.len(), 2);
        for (bytes, destination) in frames {
            assert_eq!(destination, router);
            assert_eq!(
                decode_npdu(bytes).unwrap().destination,
                Some(remote.clone())
            );
        }
        assert!(matches!(fixture.take_apdus()[0], Apdu::SimpleAck(_)));
    }
    let peer = crate::cov::CovPeerKey::from_endpoint(&MacAddr::from_slice(&[1]), Some(&remote));
    {
        let mut table = fixture.cov_table.write().await;
        assert_eq!(table.peer_subscription_count(&peer), 2);
        assert_eq!(table.remove_peer_subscriptions(&[1], Some(&remote)), 1);
        assert_eq!(table.peer_subscription_count(&peer), 1);
    }
    assert_eq!(fire(&fixture).await, vec![2]);
    let remaining = fixture
        .cov_table
        .write()
        .await
        .subscriptions_for(&point_oid())[0]
        .clone();
    assert_eq!(remaining.subscriber_mac, MacAddr::from_slice(&[2]));
    fixture
        .dispatch_at(
            &MacAddr::from_slice(&[1]),
            Some(&remote),
            3,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
            single(true),
        )
        .await;
    assert_eq!(fixture.cov_table.read().await.len(), 1);
}
