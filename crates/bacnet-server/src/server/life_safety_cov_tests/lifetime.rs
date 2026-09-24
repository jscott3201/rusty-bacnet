use super::*;
use bacnet_services::cov::SubscribeCOVPropertyRequest;

#[tokio::test]
async fn cov_lifetime_one_second_initial_wire_remains_positive() {
    let fixture = DispatchFixture::new(life_safety_db(), []).await;
    let request = SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 91,
        monitored_object_identifier: point_oid(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(1),
        monitored_property_identifier: PropertyIdentifier::SILENCED,
        monitored_property_array_index: None,
        cov_increment: None,
    };
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes).unwrap();
    fixture
        .dispatch(
            91,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
            bytes.freeze(),
        )
        .await;
    let apdus = fixture.take_apdus();
    assert_eq!(apdus.len(), 2);
    assert!(matches!(apdus[0], Apdu::SimpleAck(_)));
    let Apdu::UnconfirmedRequest(notification) = &apdus[1] else {
        panic!("expected initial COV");
    };
    let notification = COVNotificationRequest::decode(&notification.service_request).unwrap();
    assert_eq!(
        notification.time_remaining, 1,
        "a live finite subscription must never advertise indefinite zero"
    );
}

use bacnet_services::common::PropertyReference;
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};

#[derive(Clone, Copy)]
enum Family {
    Ordinary,
    Property,
    Multiple,
}

fn encoded(
    family: Family,
    confirmed: bool,
    lifetime: Option<u32>,
    cancel: bool,
) -> (ConfirmedServiceChoice, Bytes) {
    let mut bytes = BytesMut::new();
    let service = match family {
        Family::Ordinary => {
            SubscribeCOVRequest {
                subscriber_process_identifier: 92,
                monitored_object_identifier: point_oid(),
                issue_confirmed_notifications: (!cancel).then_some(confirmed),
                lifetime: if cancel { None } else { lifetime },
            }
            .encode(&mut bytes)
            .unwrap();
            ConfirmedServiceChoice::SUBSCRIBE_COV
        }
        Family::Property => {
            SubscribeCOVPropertyRequest {
                subscriber_process_identifier: 92,
                monitored_object_identifier: point_oid(),
                issue_confirmed_notifications: (!cancel).then_some(confirmed),
                lifetime: if cancel { None } else { lifetime },
                monitored_property_identifier: PropertyIdentifier::SILENCED,
                monitored_property_array_index: None,
                cov_increment: None,
            }
            .encode(&mut bytes)
            .unwrap();
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY
        }
        Family::Multiple => {
            SubscribeCOVPropertyMultipleRequest {
                subscriber_process_identifier: 92,
                issue_confirmed_notifications: confirmed,
                lifetime: if cancel { None } else { lifetime },
                max_notification_delay: (!cancel).then_some(0),
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
            }
            .encode(&mut bytes)
            .unwrap();
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE
        }
    };
    (service, bytes.freeze())
}

async fn take(fixture: &DispatchFixture, count: usize, family: Family, expected: u32) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.sent.lock().unwrap().len() < count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let apdus = fixture.take_apdus();
    assert_eq!(apdus.len(), count);
    for apdu in apdus {
        let bytes = match apdu {
            Apdu::SimpleAck(_) => continue,
            Apdu::UnconfirmedRequest(request) => request.service_request,
            Apdu::ConfirmedRequest(request) => {
                assert!(fixture.transactions.admit_terminal(
                    &fixture.source_mac,
                    None,
                    &Apdu::SimpleAck(SimpleAck {
                        invoke_id: request.invoke_id,
                        service_choice: request.service_choice
                    })
                ));
                request.service_request
            }
            other => panic!("unexpected {other:?}"),
        };
        let remaining = match family {
            Family::Multiple => {
                COVNotificationMultipleRequest::decode(&bytes)
                    .unwrap()
                    .time_remaining
            }
            _ => {
                COVNotificationRequest::decode(&bytes)
                    .unwrap()
                    .time_remaining
            }
        };
        assert_eq!(remaining, expected);
    }
}

async fn wire_case(family: Family, confirmed: bool, lifetime: Option<u32>, expected: u32) {
    let fixture = DispatchFixture::new(life_safety_db(), []).await;
    let (service, request) = encoded(family, confirmed, lifetime, false);
    fixture.dispatch(92, service, request).await;
    take(&fixture, 2, family, expected).await;
    BACnetServer::<RecordingTransport>::fire_life_safety_cov_notifications(
        &fixture.db,
        &fixture.network,
        &fixture.cov_table,
        &fixture.cov_in_flight,
        &fixture.transactions,
        &fixture.comm_state,
        &fixture.config,
        &point_oid(),
        &[PropertyIdentifier::STATUS_FLAGS],
    )
    .await;
    take(&fixture, 1, family, expected).await;
    let (service, request) = encoded(family, confirmed, lifetime, true);
    fixture.dispatch(93, service, request).await;
    assert!(matches!(
        fixture.take_apdus().as_slice(),
        [Apdu::SimpleAck(_)]
    ));
    assert!(fixture.cov_table.read().await.is_empty());
    fixture.transactions.close();
    while fixture.transactions.join_next().await.is_some() {}
    assert_eq!(fixture.cov_in_flight.available_permits(), 255);
}

#[tokio::test]
async fn cov_lifetime_all_owned_wire_families_initial_and_fanout_are_positive() {
    for family in [Family::Ordinary, Family::Property, Family::Multiple] {
        for confirmed in [false, true] {
            wire_case(family, confirmed, Some(1), 1).await;
        }
    }
}

#[tokio::test]
async fn cov_lifetime_ordinary_omitted_and_zero_remain_indefinite() {
    for lifetime in [None, Some(0)] {
        for confirmed in [false, true] {
            wire_case(Family::Ordinary, confirmed, lifetime, 0).await;
        }
    }
}
