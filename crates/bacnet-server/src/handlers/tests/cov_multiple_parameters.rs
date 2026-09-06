use super::*;
use bacnet_services::common::PropertyReference;
use bacnet_services::cov_multiple::{COVReference, COVSubscriptionSpecification};

fn encode_unchecked(
    specs: &[COVSubscriptionSpecification],
    lifetime: Option<u32>,
    max_notification_delay: Option<u32>,
) -> BytesMut {
    let mut buf = BytesMut::new();
    bacnet_encoding::primitives::encode_ctx_unsigned(&mut buf, 0, 1);
    bacnet_encoding::primitives::encode_ctx_boolean(&mut buf, 1, false);
    if let Some(lifetime) = lifetime {
        bacnet_encoding::primitives::encode_ctx_unsigned(&mut buf, 2, lifetime as u64);
    }
    if let Some(max_delay) = max_notification_delay {
        bacnet_encoding::primitives::encode_ctx_unsigned(&mut buf, 3, max_delay as u64);
    }
    bacnet_encoding::tags::encode_opening_tag(&mut buf, 4);
    for spec in specs {
        bacnet_encoding::primitives::encode_ctx_object_id(
            &mut buf,
            0,
            &spec.monitored_object_identifier,
        );
        bacnet_encoding::tags::encode_opening_tag(&mut buf, 1);
        for cov_ref in &spec.list_of_cov_references {
            bacnet_encoding::tags::encode_opening_tag(&mut buf, 0);
            cov_ref.monitored_property.encode(&mut buf);
            bacnet_encoding::tags::encode_closing_tag(&mut buf, 0);
            if let Some(increment) = cov_ref.cov_increment {
                bacnet_encoding::primitives::encode_ctx_real(&mut buf, 1, increment);
            }
            bacnet_encoding::primitives::encode_ctx_boolean(&mut buf, 2, cov_ref.timestamped);
        }
        bacnet_encoding::tags::encode_closing_tag(&mut buf, 1);
    }
    bacnet_encoding::tags::encode_closing_tag(&mut buf, 4);
    buf
}

#[test]
fn subscribe_cov_property_multiple_rejects_invalid_service_parameters() {
    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let specs = vec![COVSubscriptionSpecification {
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        list_of_cov_references: vec![COVReference {
            monitored_property: PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            },
            cov_increment: Some(0.5),
            timestamped: false,
        }],
    }];

    for (lifetime, max_notification_delay) in [(Some(300), None), (None, Some(10))] {
        let buf = encode_unchecked(&specs, lifetime, max_notification_delay);
        let err = handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf)
            .unwrap_err();
        assert!(matches!(
            err,
            Error::Reject { reason }
                if reason == RejectReason::INCONSISTENT_PARAMETERS.to_raw()
        ));
        assert!(table.is_empty());
    }

    for (lifetime, max_notification_delay, expected_code) in [
        (Some(0), Some(0), ErrorCode::VALUE_OUT_OF_RANGE),
        (Some(300), Some(300), ErrorCode::VALUE_OUT_OF_RANGE),
        (Some(4000), Some(3601), ErrorCode::VALUE_OUT_OF_RANGE),
    ] {
        let buf = encode_unchecked(&specs, lifetime, max_notification_delay);
        let err = handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf)
            .unwrap_err();
        match err {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::SERVICES.to_raw() as u32);
                assert_eq!(code, expected_code.to_raw() as u32);
            }
            other => panic!("expected service parameter protocol error, got {other:?}"),
        }
        assert!(table.is_empty());
    }
}

#[test]
fn clockless_timestamped_cov_multiple_rejects_atomically_but_can_cancel() {
    use bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest;

    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let specifications = vec![COVSubscriptionSpecification {
        monitored_object_identifier: oid,
        list_of_cov_references: vec![COVReference {
            monitored_property: PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            },
            cov_increment: Some(0.5),
            timestamped: true,
        }],
    }];

    let subscribe = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: false,
        lifetime: Some(300),
        max_notification_delay: Some(10),
        list_of_cov_subscription_specifications: specifications.clone(),
    };
    let mut buf = BytesMut::new();
    subscribe.encode(&mut buf);

    let err = handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf)
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Protocol { class, code }
            if class == ErrorClass::SERVICES.to_raw() as u32
                && code == ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32
    ));
    assert!(table.is_empty(), "rejection must precede table mutation");

    table.subscribe(CovSubscription {
        subscriber_mac: MacAddr::from_slice(&mac),
        subscriber_network: None,
        subscriber_process_identifier: 1,
        monitored_object_identifier: oid,
        issue_confirmed_notifications: false,
        expires_at: None,
        last_notified_value: None,
        monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
        monitored_property_array_index: None,
        cov_increment: Some(0.5),
        notification_kind: CovNotificationKind::Multiple,
        timestamped: true,
    });

    let cancel = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: false,
        lifetime: None,
        max_notification_delay: None,
        list_of_cov_subscription_specifications: specifications,
    };
    let mut buf = BytesMut::new();
    cancel.encode(&mut buf);
    let initial =
        handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf).unwrap();
    assert!(initial.is_empty());
    assert!(table.is_empty(), "clockless cancellation remains usable");
}
