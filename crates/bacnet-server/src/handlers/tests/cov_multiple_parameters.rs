use super::*;
use crate::cov::MultipleRefusal;
use bacnet_services::cov_multiple::{COVReference, COVSubscriptionSpecification};
use bacnet_types::constructed::PropertyReference;

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
            bacnet_encoding::constructed::encode_property_reference(
                &mut buf,
                &cov_ref.monitored_property,
            );
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
            .unwrap_err()
            .error;
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
            .unwrap_err()
            .error;
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
fn subscribe_cov_property_multiple_context_delay_is_last_write_wins_per_form() {
    use bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest;

    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let reference = |property_identifier| COVReference {
        monitored_property: PropertyReference {
            property_identifier,
            property_array_index: None,
        },
        cov_increment: None,
        timestamped: false,
    };
    let mut subscribe = |process, confirmed, lifetime, delay, properties: &[PropertyIdentifier]| {
        let mut buf = BytesMut::new();
        SubscribeCOVPropertyMultipleRequest {
            subscriber_process_identifier: process,
            issue_confirmed_notifications: confirmed,
            lifetime: Some(lifetime),
            max_notification_delay: Some(delay),
            list_of_cov_subscription_specifications: if properties.is_empty() {
                Vec::new()
            } else {
                vec![COVSubscriptionSpecification {
                    monitored_object_identifier: oid,
                    list_of_cov_references: properties.iter().copied().map(reference).collect(),
                }]
            },
        }
        .encode(&mut buf)
        .unwrap();
        handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf)
    };
    let pv = PropertyIdentifier::PRESENT_VALUE;
    let flags = PropertyIdentifier::STATUS_FLAGS;
    let accepted = subscribe(1, false, 300, 10, &[pv, flags]).unwrap();
    assert!(accepted
        .iter()
        .all(|sub| sub.max_notification_delay() == Some(10)));
    subscribe(1, true, 600, 20, &[pv]).unwrap();
    // An empty-spec finite request succeeds with no new reference and
    // refreshes only its own form's context; for an unknown context it
    // establishes nothing.
    assert!(subscribe(1, false, 900, 30, &[]).unwrap().is_empty());
    assert!(subscribe(2, false, 900, 30, &[]).unwrap().is_empty());
    // Rejected late input leaves every stored delay unchanged.
    let specs = vec![COVSubscriptionSpecification {
        monitored_object_identifier: oid,
        list_of_cov_references: vec![reference(pv)],
    }];
    for (lifetime, delay) in [(100, 100), (4000, 3601)] {
        let buf = encode_unchecked(&specs, Some(lifetime), Some(delay));
        assert!(matches!(
            handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf),
            Err(MultipleRefusal { error: Error::Protocol { code, .. }, refused: None, .. })
                if code == ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32
        ));
    }
    let mut delays: Vec<_> = table
        .subscriptions_for(&oid)
        .into_iter()
        .map(|sub| {
            (
                sub.issue_confirmed_notifications,
                sub.monitored_property,
                sub.max_notification_delay(),
            )
        })
        .collect();
    delays.sort_by_key(|(confirmed, property, _)| (*confirmed, property.map(|p| p.to_raw())));
    assert_eq!(
        delays,
        vec![
            (false, Some(pv), Some(30)),
            (false, Some(flags), Some(30)),
            (true, Some(pv), Some(20)),
        ]
    );
}

#[test]
fn clockless_timestamped_first_reference_is_refused_by_name_but_can_cancel() {
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
    subscribe.encode(&mut buf).unwrap();

    // The timestamped reference itself is refused (#1102); as the first
    // reference, nothing before it is kept.
    let refusal = handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf)
        .unwrap_err();
    assert_eq!(refusal.refused, Some(0));
    assert!(refusal.committed.is_empty());
    match refusal.error {
        Error::Structured {
            class,
            code,
            detail,
        } => {
            assert_eq!(class, ErrorClass::SERVICES.to_raw() as u32);
            assert_eq!(
                code,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32
            );
            assert_eq!(
                *detail,
                ErrorDetail::FirstFailedSubscription(BACnetObjectPropertyReference::new(
                    oid,
                    PropertyIdentifier::PRESENT_VALUE.to_raw()
                ))
            );
        }
        other => panic!("expected a refusal naming the reference, got {other:?}"),
    }
    assert!(table.is_empty(), "rejection must precede table mutation");

    // Seed through the context owner: clockless wire admission is rejected.
    table
        .admit_for_test(
            CovSubscription {
                subscriber_mac: MacAddr::from_slice(&mac),
                subscriber_network: None,
                subscriber_process_identifier: 1,
                monitored_object_identifier: oid,
                issue_confirmed_notifications: false,
                expires_at: Some(crate::runtime_clock::now() + Duration::from_secs(300)),
                last_notified_observation: None,
                monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
                monitored_property_array_index: None,
                cov_increment: Some(0.5),
                notification_kind: CovNotificationKind::Multiple,
                timestamped: true,
            },
            10,
        )
        .unwrap();

    let cancel = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: false,
        lifetime: None,
        max_notification_delay: None,
        list_of_cov_subscription_specifications: specifications,
    };
    let mut buf = BytesMut::new();
    cancel.encode(&mut buf).unwrap();
    let initial =
        handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf).unwrap();
    assert!(initial.is_empty());
    assert!(table.is_empty(), "clockless cancellation remains usable");
}

/// Valid for the first read, then present but invalid.
struct ClockTurningInvalid {
    reads: std::sync::atomic::AtomicUsize,
    valid: bacnet_objects::clock::ClockFrame,
}

impl bacnet_objects::clock::ClockReader for ClockTurningInvalid {
    fn read_clock(&self) -> Option<bacnet_objects::clock::ClockFrame> {
        let first = self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
        let mut frame = self.valid;
        if !first {
            frame.local_time.hour = 24;
        }
        Some(frame)
    }
}

#[test]
fn timestamped_admission_captures_the_initial_report_with_the_clock_it_validated() {
    use bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest;
    use bacnet_types::primitives::{Date, Time};

    let valid = bacnet_objects::clock::ClockFrame {
        local_date: Date {
            year: 126,
            month: 9,
            day: 29,
            day_of_week: 2,
        },
        local_time: Time {
            hour: 15,
            minute: 0,
            second: 7,
            hundredths: 0,
        },
        utc_offset: 0,
        daylight_savings_status: false,
    };
    let mut db = make_db_with_ai();
    db.set_clock_reader(Some(std::sync::Arc::new(ClockTurningInvalid {
        reads: std::sync::atomic::AtomicUsize::new(0),
        valid,
    })));
    let mut table = CovSubscriptionTable::new();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let mut buf = BytesMut::new();
    SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: false,
        lifetime: Some(300),
        max_notification_delay: Some(10),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: oid,
            list_of_cov_references: vec![COVReference {
                monitored_property: PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                },
                cov_increment: Some(0.5),
                timestamped: true,
            }],
        }],
    }
    .encode(&mut buf)
    .unwrap();
    let accepted = handle_subscribe_cov_property_multiple_with_initial(
        &mut table,
        &db,
        &[192, 168, 1, 1, 0xBA, 0xC0],
        &buf,
    )
    .unwrap();
    assert_eq!(accepted.len(), 1);
    // The clock the admission check validated stamps the initial report; a
    // second read would find it invalid and capture nothing.
    let (_, initial) = table
        .timed()
        .lock()
        .drain(accepted[0].key(), accepted[0].generation());
    assert_eq!(
        initial
            .iter()
            .map(|change| change.frame())
            .collect::<Vec<_>>(),
        vec![valid]
    );
}
