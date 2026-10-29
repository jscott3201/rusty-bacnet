//! COV for Tracking_Value and Reliability simulated while Out_Of_Service is
//! TRUE (#1108): a committed simulation write notifies through the same
//! Life Safety snapshots as any other WriteProperty or WritePropertyMultiple.

use super::*;
use bacnet_types::error::RemoteError;

use bacnet_objects::life_safety::LifeSafetyZoneObject;
use bacnet_types::enums::{LifeSafetyState, Reliability};

fn zone_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 1).unwrap()
}

fn on_zone(mut subscription: CovSubscription) -> CovSubscription {
    subscription.monitored_object_identifier = zone_oid();
    subscription
}

fn point_and_zone_db() -> ObjectDatabase {
    let mut db = life_safety_db();
    db.add(Box::new(LifeSafetyZoneObject::new(1, "zone").unwrap()))
        .unwrap();
    db
}

fn encode_write(
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Bytes {
    let mut value_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut value_buf, &value).unwrap();
    let mut encoded = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: value_buf.to_vec(),
        priority: None,
    }
    .encode(&mut encoded)
    .unwrap();
    encoded.freeze()
}

fn encode_write_multiple(
    oid: ObjectIdentifier,
    writes: &[(PropertyIdentifier, PropertyValue)],
) -> Bytes {
    let list_of_properties = writes
        .iter()
        .map(|(property, value)| {
            let mut value_buf = BytesMut::new();
            bacnet_encoding::primitives::encode_property_value(&mut value_buf, value).unwrap();
            BACnetPropertyValue {
                property_identifier: *property,
                property_array_index: None,
                value: value_buf.to_vec(),
                priority: None,
            }
        })
        .collect();
    let mut encoded = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties,
        }],
    }
    .encode(&mut encoded)
    .unwrap();
    encoded.freeze()
}

/// Each notification after the ACK as (process, object, reported properties,
/// first reported value bytes).
fn notifications(apdus: &[Apdu]) -> Vec<(u32, ObjectIdentifier, Vec<PropertyIdentifier>, Vec<u8>)> {
    assert!(
        matches!(apdus.first(), Some(Apdu::SimpleAck(_))),
        "expected the write's SimpleACK first, got {apdus:?}"
    );
    let mut notifications: Vec<_> = apdus[1..]
        .iter()
        .map(|apdu| {
            let Apdu::UnconfirmedRequest(request) = apdu else {
                panic!("expected unconfirmed COV notification, got {apdu:?}");
            };
            let notification = COVNotificationRequest::decode(&request.service_request).unwrap();
            (
                notification.subscriber_process_identifier,
                notification.monitored_object_identifier,
                notification
                    .list_of_values
                    .iter()
                    .map(|value| value.property_identifier)
                    .collect(),
                notification.list_of_values[0].value.clone(),
            )
        })
        .collect();
    notifications.sort_by_key(|notification| notification.0);
    notifications
}

const TRACKING: PropertyIdentifier = PropertyIdentifier::TRACKING_VALUE;
const PRESENT: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
const FLAGS: PropertyIdentifier = PropertyIdentifier::STATUS_FLAGS;

#[tokio::test]
async fn simulated_tracking_value_and_reliability_notify_like_any_committed_write() {
    let fixture = DispatchFixture::new(
        point_and_zone_db(),
        [
            subscription(Some(TRACKING), CovNotificationKind::Single, 1),
            subscription(None, CovNotificationKind::Single, 2),
            on_zone(subscription(Some(TRACKING), CovNotificationKind::Single, 3)),
        ],
    )
    .await;
    let alarm = PropertyValue::Enumerated(LifeSafetyState::ALARM.to_raw());

    fixture
        .dispatch(
            1,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            encode_write(
                point_oid(),
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
            ),
        )
        .await;
    assert_eq!(
        notifications(&fixture.take_apdus()).len(),
        2,
        "OUT_OF_SERVICE flag"
    );

    // A simulated Tracking_Value reaches only its property subscriber.
    fixture
        .dispatch(
            2,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            encode_write(point_oid(), TRACKING, alarm.clone()),
        )
        .await;
    assert_eq!(
        notifications(&fixture.take_apdus()),
        vec![(1, point_oid(), vec![TRACKING, FLAGS], vec![0x91, 2])]
    );

    // A simulated fault flips FAULT, so every Point subscriber hears it.
    fixture
        .dispatch(
            3,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            encode_write(
                point_oid(),
                PropertyIdentifier::RELIABILITY,
                PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
            ),
        )
        .await;
    assert_eq!(
        notifications(&fixture.take_apdus()),
        vec![
            (1, point_oid(), vec![TRACKING, FLAGS], vec![0x91, 2]),
            (2, point_oid(), vec![PRESENT, FLAGS], vec![0x91, 0]),
        ]
    );

    // The return to service restores QUIET and clears both flags: one
    // notification each, carrying the restored value.
    fixture
        .dispatch(
            4,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            encode_write(
                point_oid(),
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(false),
            ),
        )
        .await;
    assert_eq!(
        notifications(&fixture.take_apdus()),
        vec![
            (1, point_oid(), vec![TRACKING, FLAGS], vec![0x91, 0]),
            (2, point_oid(), vec![PRESENT, FLAGS], vec![0x91, 0]),
        ]
    );

    // On the Zone, one WritePropertyMultiple takes it out of service and
    // simulates ALARM; its subscriber gets one notification for both changes.
    fixture
        .dispatch(
            5,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            encode_write_multiple(
                zone_oid(),
                &[
                    (
                        PropertyIdentifier::OUT_OF_SERVICE,
                        PropertyValue::Boolean(true),
                    ),
                    (TRACKING, alarm.clone()),
                ],
            ),
        )
        .await;
    assert_eq!(
        notifications(&fixture.take_apdus()),
        vec![(3, zone_oid(), vec![TRACKING, FLAGS], vec![0x91, 2])]
    );

    // Rewriting the value already simulated commits no change: ACK only.
    fixture
        .dispatch(
            6,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            encode_write(zone_oid(), TRACKING, alarm),
        )
        .await;
    assert!(notifications(&fixture.take_apdus()).is_empty());
}

#[tokio::test]
async fn refused_in_service_tracking_value_write_notifies_nobody() {
    let fixture = DispatchFixture::new(
        point_and_zone_db(),
        [
            subscription(Some(TRACKING), CovNotificationKind::Single, 1),
            on_zone(subscription(Some(TRACKING), CovNotificationKind::Single, 2)),
        ],
    )
    .await;
    for (invoke_id, oid) in [(1, point_oid()), (2, zone_oid())] {
        fixture
            .dispatch(
                invoke_id,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                encode_write(
                    oid,
                    TRACKING,
                    PropertyValue::Enumerated(LifeSafetyState::ALARM.to_raw()),
                ),
            )
            .await;
        let apdus = fixture.take_apdus();
        assert_eq!(apdus.len(), 1, "only the Error, no COV: {apdus:?}");
        let Apdu::Error(error) = &apdus[0] else {
            panic!("expected an Error PDU, got {:?}", apdus[0]);
        };
        assert_eq!(error.error_class, ErrorClass::PROPERTY);
        assert_eq!(error.error_code, ErrorCode::WRITE_ACCESS_DENIED);
    }
}

async fn write_tracking_value(server: &BACnetServer<TestTransport>) -> Result<(), Error> {
    server
        .write_local(
            &point_oid(),
            TRACKING,
            None,
            PropertyValue::Enumerated(LifeSafetyState::ALARM.to_raw()),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
}

#[tokio::test]
async fn write_local_simulates_tracking_value_only_out_of_service() {
    let (transport, sent) = recording_transport();
    let mut server = BACnetServer::<TestTransport>::generic_builder()
        .transport(transport)
        .database(life_safety_db())
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();
    server
        .cov_table
        .write()
        .await
        .subscribe(subscription(Some(TRACKING), CovNotificationKind::Single, 1))
        .unwrap();
    let refused = write_tracking_value(&server).await.unwrap_err();
    assert!(
        matches!(refused, Error::Remote(RemoteError::Error { class, code })
        if class == ErrorClass::PROPERTY
            && code == ErrorCode::WRITE_ACCESS_DENIED)
    );
    assert!(decode_sent(&sent).is_empty());

    server
        .write_local(
            &point_oid(),
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    sent.clear();
    write_tracking_value(&server).await.unwrap();
    let apdus = decode_sent(&sent);
    assert_eq!(apdus.len(), 1);
    assert_eq!(single_properties(&apdus[0]), vec![TRACKING, FLAGS]);

    server.stop().await.unwrap();
}
