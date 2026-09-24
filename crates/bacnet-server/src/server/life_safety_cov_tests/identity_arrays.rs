use super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_services::common::PropertyReference;
use bacnet_services::cov::SubscribeCOVPropertyRequest;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};

fn object() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 8).unwrap()
}

fn multiple(refs: &[(Option<u32>, f32)], cancel: bool) -> Bytes {
    let request = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 8,
        issue_confirmed_notifications: false,
        lifetime: (!cancel).then_some(300),
        max_notification_delay: (!cancel).then_some(5),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: object(),
            list_of_cov_references: refs
                .iter()
                .map(|&(index, increment)| COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
                        property_array_index: index,
                    },
                    cov_increment: Some(increment),
                    timestamped: false,
                })
                .collect(),
        }],
    };
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes).unwrap();
    bytes.freeze()
}

#[tokio::test]
async fn cov_identity_multiple_array_coordinates_duplicates_and_late_invalid_are_atomic() {
    let mut db = clocked_test_database();
    db.add(Box::new(AnalogValueObject::new(8, "array", 95).unwrap()))
        .unwrap();
    let fixture = DispatchFixture::new(db, []).await;
    fixture
        .dispatch(
            1,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            multiple(
                &[
                    (None, 0.1),
                    (Some(0), 0.2),
                    (Some(1), 0.3),
                    (Some(2), 0.4),
                    (Some(1), 0.9),
                ],
                false,
            ),
        )
        .await;
    let apdus = fixture.take_apdus();
    assert_eq!(apdus.len(), 2);
    assert!(matches!(apdus[0], Apdu::SimpleAck(_)));
    let Apdu::UnconfirmedRequest(notification) = &apdus[1] else {
        panic!("Multiple initial");
    };
    let notification =
        COVNotificationMultipleRequest::decode(&notification.service_request).unwrap();
    let values = &notification.list_of_cov_notifications[0].list_of_values;
    assert_eq!(
        values
            .iter()
            .map(|v| v.property_array_index)
            .collect::<Vec<_>>(),
        vec![None, Some(0), Some(2), Some(1)]
    );
    let before = fixture
        .cov_table
        .write()
        .await
        .subscriptions_for(&object())
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(before.len(), 4);
    assert_eq!(
        before
            .iter()
            .find(|s| s.monitored_property_array_index == Some(1))
            .unwrap()
            .cov_increment,
        Some(0.9)
    );
    // Valid early coordinate followed by an unreadable array element cannot refresh or replace.
    fixture
        .dispatch(
            2,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            multiple(&[(Some(1), 1.5), (Some(17), 0.1)], false),
        )
        .await;
    assert!(matches!(fixture.take_apdus().as_slice(), [Apdu::Error(_)]));
    {
        let table = fixture.cov_table.read().await;
        for original in &before {
            assert!(table.is_current(original));
            let current = table.get_subscription(original.key()).unwrap();
            assert_eq!(current.expires_at, original.expires_at);
            assert_eq!(current.cov_increment, original.cov_increment);
        }
    }
    fixture
        .dispatch(
            3,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            multiple(&[(Some(1), 2.0)], false),
        )
        .await;
    fixture.take_apdus();
    {
        let table = fixture.cov_table.read().await;
        for original in &before {
            assert_eq!(
                table.is_current(original),
                original.monitored_property_array_index != Some(1)
            );
        }
    }
    fixture
        .dispatch(
            4,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            multiple(&[(Some(0), 0.2)], true),
        )
        .await;
    assert!(matches!(
        fixture.take_apdus().as_slice(),
        [Apdu::SimpleAck(_)]
    ));
    let retained = fixture
        .cov_table
        .write()
        .await
        .subscriptions_for(&object())
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(retained.len(), 3);
    assert!(retained
        .iter()
        .all(|s| s.monitored_property_array_index != Some(0)));
}

#[tokio::test]
async fn cov_identity_single_array_coordinates_cancel_exactly() {
    let mut db = clocked_test_database();
    db.add(Box::new(AnalogValueObject::new(8, "array", 95).unwrap()))
        .unwrap();
    let fixture = DispatchFixture::new(db, []).await;
    for (invoke, index) in [None, Some(0), Some(1), Some(2)].into_iter().enumerate() {
        let request = SubscribeCOVPropertyRequest {
            subscriber_process_identifier: 8,
            monitored_object_identifier: object(),
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
            monitored_property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
            monitored_property_array_index: index,
            cov_increment: None,
        };
        let mut bytes = BytesMut::new();
        request.encode(&mut bytes).unwrap();
        fixture
            .dispatch(
                invoke as u8,
                ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
                bytes.freeze(),
            )
            .await;
        let apdus = fixture.take_apdus();
        assert_eq!(apdus.len(), 2);
        assert!(matches!(apdus[0], Apdu::SimpleAck(_)));
        let Apdu::UnconfirmedRequest(notification) = &apdus[1] else {
            panic!("Single initial")
        };
        let notification = COVNotificationRequest::decode(&notification.service_request).unwrap();
        assert_eq!(
            notification.list_of_values[0].property_identifier,
            PropertyIdentifier::PRIORITY_ARRAY
        );
        assert_eq!(notification.list_of_values[0].property_array_index, index);
    }
    assert_eq!(fixture.cov_table.read().await.len(), 4);
    let cancel = SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 8,
        monitored_object_identifier: object(),
        issue_confirmed_notifications: None,
        lifetime: None,
        monitored_property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
        monitored_property_array_index: Some(1),
        cov_increment: None,
    };
    let mut bytes = BytesMut::new();
    cancel.encode(&mut bytes).unwrap();
    fixture
        .dispatch(
            9,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
            bytes.freeze(),
        )
        .await;
    assert!(matches!(
        fixture.take_apdus().as_slice(),
        [Apdu::SimpleAck(_)]
    ));
    let retained = fixture
        .cov_table
        .write()
        .await
        .subscriptions_for(&object())
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(retained.len(), 3);
    assert!(retained
        .iter()
        .all(|s| s.monitored_property_array_index != Some(1)));
}
