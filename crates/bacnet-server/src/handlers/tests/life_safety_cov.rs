use super::*;

use bacnet_objects::life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject};
use bacnet_services::common::PropertyReference;
use bacnet_services::cov::SubscribeCOVPropertyRequest;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};

fn life_safety_db() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(LifeSafetyPointObject::new(1, "point").unwrap()))
        .unwrap();
    db.add(Box::new(LifeSafetyZoneObject::new(1, "zone").unwrap()))
        .unwrap();
    db
}

fn point_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap()
}

fn zone_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 1).unwrap()
}

fn property_request(oid: ObjectIdentifier, property: PropertyIdentifier) -> BytesMut {
    let request = SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 7,
        monitored_object_identifier: oid,
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
        monitored_property_identifier: property,
        monitored_property_array_index: None,
        cov_increment: None,
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded);
    encoded
}

fn assert_protocol(error: Error, class: ErrorClass, code: ErrorCode) {
    assert!(matches!(
        error,
        Error::Protocol {
            class: actual_class,
            code: actual_code,
        } if actual_class == class.to_raw() as u32 && actual_code == code.to_raw() as u32
    ));
}

#[test]
fn life_safety_single_property_cov_uses_explicit_capability_and_error_taxonomy() {
    let db = life_safety_db();
    let mut table = CovSubscriptionTable::new();
    let mac = [1, 2, 3];

    for (oid, property) in [
        (point_oid(), PropertyIdentifier::TRACKING_VALUE),
        (point_oid(), PropertyIdentifier::SILENCED),
        (zone_oid(), PropertyIdentifier::OPERATION_EXPECTED),
    ] {
        handle_subscribe_cov_property(&mut table, &db, &mac, &property_request(oid, property))
            .unwrap();
    }

    let error = handle_subscribe_cov_property(
        &mut table,
        &db,
        &mac,
        &property_request(zone_oid(), PropertyIdentifier::TRACKING_VALUE),
    )
    .unwrap_err();
    assert_protocol(error, ErrorClass::PROPERTY, ErrorCode::NOT_COV_PROPERTY);

    let error = handle_subscribe_cov_property(
        &mut table,
        &db,
        &mac,
        &property_request(point_oid(), PropertyIdentifier::MODE),
    )
    .unwrap_err();
    assert_protocol(error, ErrorClass::PROPERTY, ErrorCode::NOT_COV_PROPERTY);

    let error = handle_subscribe_cov_property(
        &mut table,
        &db,
        &mac,
        &property_request(point_oid(), PropertyIdentifier::from_raw(4_000)),
    )
    .unwrap_err();
    assert_protocol(error, ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY);
    assert_eq!(table.len(), 3);
}

#[test]
fn life_safety_multiple_property_cov_rejection_is_atomic() {
    let db = life_safety_db();
    for (oid, property, code) in [
        (
            point_oid(),
            PropertyIdentifier::MODE,
            ErrorCode::NOT_COV_PROPERTY,
        ),
        (
            zone_oid(),
            PropertyIdentifier::TRACKING_VALUE,
            ErrorCode::NOT_COV_PROPERTY,
        ),
        (
            point_oid(),
            PropertyIdentifier::from_raw(4_000),
            ErrorCode::UNKNOWN_PROPERTY,
        ),
    ] {
        let mut table = CovSubscriptionTable::new();
        let request = SubscribeCOVPropertyMultipleRequest {
            subscriber_process_identifier: 8,
            issue_confirmed_notifications: false,
            lifetime: Some(300),
            max_notification_delay: Some(10),
            list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
                monitored_object_identifier: oid,
                list_of_cov_references: vec![
                    COVReference {
                        monitored_property: PropertyReference {
                            property_identifier: PropertyIdentifier::SILENCED,
                            property_array_index: None,
                        },
                        cov_increment: None,
                        timestamped: false,
                    },
                    COVReference {
                        monitored_property: PropertyReference {
                            property_identifier: property,
                            property_array_index: None,
                        },
                        cov_increment: None,
                        timestamped: false,
                    },
                ],
            }],
        };

        let error = handle_subscribe_cov_property_multiple_request_endpoint(
            &mut table,
            &db,
            &[1, 2, 3],
            None,
            request,
        )
        .unwrap_err();

        assert_protocol(error, ErrorClass::PROPERTY, code);
        assert!(table.is_empty());
    }
}

#[test]
fn life_safety_property_cancellation_bypasses_current_capability_checks() {
    let db = life_safety_db();
    let mut table = CovSubscriptionTable::new();
    let request = SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 9,
        monitored_object_identifier: zone_oid(),
        issue_confirmed_notifications: None,
        lifetime: None,
        monitored_property_identifier: PropertyIdentifier::TRACKING_VALUE,
        monitored_property_array_index: None,
        cov_increment: None,
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded);

    let initial =
        handle_subscribe_cov_property_with_initial(&mut table, &db, &[1], &encoded).unwrap();

    assert!(initial.is_empty());
    assert!(table.is_empty());
}
