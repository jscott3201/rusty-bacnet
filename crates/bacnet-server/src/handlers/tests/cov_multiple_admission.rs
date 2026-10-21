//! SubscribeCOVPropertyMultiple admission in request order (Clause 13.16.2):
//! the error names the first reference that fails, whether validation or the
//! subscription caps refuse it, and the references before it stay (#1058,
//! #1059).
use super::*;

#[test]
fn subscribe_cov_property_multiple_invalid_property_keeps_the_earlier_reference() {
    use bacnet_services::cov_multiple::{
        COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
    };
    use bacnet_types::constructed::PropertyReference;

    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let request = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: false,
        lifetime: Some(300),
        max_notification_delay: Some(10),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: oid,
            list_of_cov_references: vec![
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    cov_increment: Some(0.5),
                    timestamped: false,
                },
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
                        property_array_index: None,
                    },
                    cov_increment: None,
                    timestamped: false,
                },
            ],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let refusal = handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf)
        .unwrap_err();
    // The error names the failed reference, the second (#1047).
    match refusal.error {
        Error::Structured {
            class,
            code,
            detail,
        } => {
            assert_eq!(class, ErrorClass::PROPERTY);
            assert_eq!(code, ErrorCode::UNKNOWN_PROPERTY);
            assert_eq!(
                *detail,
                ErrorDetail::FirstFailedSubscription(BACnetObjectPropertyReference::new(
                    oid,
                    PropertyIdentifier::PRIORITY_ARRAY.to_raw()
                ))
            );
        }
        other => panic!("expected UNKNOWN_PROPERTY structured error, got {other:?}"),
    }
    // Present_Value, processed before the refusal, stays subscribed with its
    // increment and comes back for its initial notification (#1058).
    assert_eq!(refusal.refused, Some(1));
    assert_eq!(table.len(), 1);
    let [kept] = refusal.committed.as_slice() else {
        panic!("one kept reference: {:?}", refusal.committed);
    };
    assert!(table.is_current(kept));
    assert_eq!(
        kept.monitored_property,
        Some(PropertyIdentifier::PRESENT_VALUE)
    );
    assert_eq!(kept.cov_increment, Some(0.5));
}

#[test]
fn subscribe_cov_property_multiple_capacity_names_the_overflowing_reference() {
    use bacnet_services::cov_multiple::{
        COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
    };
    use bacnet_types::constructed::PropertyReference;

    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::with_policy(
        crate::cov::CovPolicy {
            max_subscriptions_per_peer: 1024,
            max_indefinite_per_peer: 1024,
            reserved_capacity: 0,
            ..Default::default()
        },
        std::sync::Arc::new(crate::cov::AtomicCovCounters::default()),
    );
    let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    // Another peer fills all but one slot of the table's 1,024.
    for instance in 1000..2023 {
        table
            .subscribe(CovSubscription {
                subscriber_mac: MacAddr::from_slice(&[192, 168, 1, 2, 0xBA, 0xC0]),
                subscriber_network: None,
                subscriber_process_identifier: 99,
                monitored_object_identifier: ObjectIdentifier::new(
                    ObjectType::ANALOG_INPUT,
                    instance,
                )
                .unwrap(),
                issue_confirmed_notifications: false,
                expires_at: None,
                last_notified_observation: None,
                monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
                monitored_property_array_index: None,
                cov_increment: None,
                notification_kind: CovNotificationKind::Single,
                timestamped: false,
            })
            .unwrap();
    }
    assert_eq!(table.len(), 1023);

    let request = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: false,
        lifetime: Some(300),
        max_notification_delay: Some(10),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: oid,
            list_of_cov_references: vec![
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    cov_increment: Some(0.5),
                    timestamped: false,
                },
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::STATUS_FLAGS,
                        property_array_index: None,
                    },
                    cov_increment: None,
                    timestamped: false,
                },
            ],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    // One slot is left: Present_Value takes it and Status_Flags, the
    // reference that does not fit, is named (#1059) while the first stays.
    let refusal = handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf)
        .unwrap_err();
    match refusal.error {
        Error::Structured {
            class,
            code,
            detail,
        } => {
            assert_eq!(class, ErrorClass::RESOURCES);
            assert_eq!(code, ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT);
            assert_eq!(
                *detail,
                ErrorDetail::FirstFailedSubscription(BACnetObjectPropertyReference::new(
                    oid,
                    PropertyIdentifier::STATUS_FLAGS.to_raw()
                ))
            );
        }
        other => panic!("expected NO_SPACE_TO_ADD_LIST_ELEMENT structured error, got {other:?}"),
    }
    assert_eq!(refusal.refused, Some(1));
    assert_eq!(table.len(), 1024);
    let [kept] = refusal.committed.as_slice() else {
        panic!("one kept reference: {:?}", refusal.committed);
    };
    assert_eq!(
        kept.monitored_property,
        Some(PropertyIdentifier::PRESENT_VALUE)
    );
    assert_eq!(
        table.counters().snapshot().subscriptions_rejected_capacity,
        1
    );
}
