use super::*;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::wpm::WritePropertyMultipleError;

#[tokio::test]
async fn event_enrollment_prefix_commit_returns_exact_error_through_server_dispatch() {
    let mut db = clocked_test_database();
    let object = EventEnrollmentObject::new(7, "dispatch-ee", 5).unwrap();
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();
    let fixture = DispatchFixture::new(db, std::iter::empty()).await;

    let mut description = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(
        &mut description,
        &PropertyValue::CharacterString("committed through dispatch".into()),
    )
    .unwrap();
    let mut acked = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(
        &mut acked,
        &PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xe0],
        },
    )
    .unwrap();
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::DESCRIPTION,
                    property_array_index: None,
                    value: description.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::ACKED_TRANSITIONS,
                    property_array_index: None,
                    value: acked.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded);

    fixture
        .dispatch(
            0x43,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            encoded.freeze(),
        )
        .await;

    let apdus = fixture.take_apdus();
    assert_eq!(
        apdus.len(),
        1,
        "the no-subscription fixture emits only Result(-)"
    );
    let Apdu::Error(error_pdu) = &apdus[0] else {
        panic!("expected service-16 Error, got {:?}", apdus[0]);
    };
    assert_eq!(
        error_pdu.service_choice,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE
    );
    assert_eq!(error_pdu.error_class, ErrorClass::PROPERTY);
    assert_eq!(error_pdu.error_code, ErrorCode::WRITE_ACCESS_DENIED);
    let formal = WritePropertyMultipleError::from_error_pdu(error_pdu).unwrap();
    assert_eq!(formal.first_failed_write_attempt.object_identifier, oid);
    assert_eq!(
        formal.first_failed_write_attempt.property_identifier,
        PropertyIdentifier::ACKED_TRANSITIONS.to_raw()
    );
    assert_eq!(formal.first_failed_write_attempt.property_array_index, None);
    assert_eq!(
        fixture
            .db
            .read()
            .await
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("committed through dispatch".into())
    );
}

#[tokio::test]
async fn failed_wpm_sends_formal_error_before_committed_prefix_cov() {
    let fixture = DispatchFixture::new(
        life_safety_db(),
        [
            subscription(
                Some(PropertyIdentifier::OPERATION_EXPECTED),
                CovNotificationKind::Single,
                1,
            ),
            subscription(None, CovNotificationKind::Single, 2),
        ],
    )
    .await;
    let mut boolean = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut boolean, &PropertyValue::Boolean(true))
        .unwrap();
    let mut enumerated = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(
        &mut enumerated,
        &PropertyValue::Enumerated(ObjectType::LIFE_SAFETY_POINT.to_raw()),
    )
    .unwrap();
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: point_oid(),
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OUT_OF_SERVICE,
                    property_array_index: None,
                    value: boolean.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_TYPE,
                    property_array_index: None,
                    value: enumerated.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded);

    fixture
        .dispatch(
            0x44,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            encoded.freeze(),
        )
        .await;

    let apdus = fixture.take_apdus();
    assert_eq!(apdus.len(), 3);
    let Apdu::Error(error_pdu) = &apdus[0] else {
        panic!("confirmed WPM Result(-) must precede notifications");
    };
    assert_eq!(error_pdu.error_class, ErrorClass::PROPERTY);
    assert_eq!(error_pdu.error_code, ErrorCode::WRITE_ACCESS_DENIED);
    let formal = WritePropertyMultipleError::from_error_pdu(error_pdu).unwrap();
    assert_eq!(
        formal.first_failed_write_attempt.object_identifier,
        point_oid()
    );
    assert_eq!(
        formal.first_failed_write_attempt.property_identifier,
        PropertyIdentifier::OBJECT_TYPE.to_raw()
    );
    assert_eq!(formal.first_failed_write_attempt.property_array_index, None);
    let payloads: Vec<_> = apdus[1..].iter().map(single_properties).collect();
    assert!(payloads.contains(&vec![
        PropertyIdentifier::OPERATION_EXPECTED,
        PropertyIdentifier::STATUS_FLAGS,
    ]));
    assert!(payloads.contains(&vec![
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::STATUS_FLAGS,
    ]));
    assert_eq!(
        fixture
            .db
            .read()
            .await
            .get(&point_oid())
            .unwrap()
            .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .unwrap(),
        PropertyValue::Boolean(true)
    );
}

#[tokio::test]
async fn failed_wpm_sends_generic_cov_for_non_life_safety_prefix_only() {
    let mut db = clocked_test_database();
    let object = BinaryValueObject::new(1, "binary").unwrap();
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();
    let fixture = DispatchFixture::new(
        db,
        [CovSubscription {
            subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 3]),
            subscriber_network: None,
            subscriber_process_identifier: 3,
            monitored_object_identifier: oid,
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_value: None,
            monitored_property: None,
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        }],
    )
    .await;
    let mut on = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut on, &PropertyValue::Enumerated(1))
        .unwrap();
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: on.to_vec(),
                    priority: Some(8),
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_TYPE,
                    property_array_index: None,
                    value: on.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded);

    fixture
        .dispatch(
            0x45,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            encoded.freeze(),
        )
        .await;

    let apdus = fixture.take_apdus();
    assert_eq!(apdus.len(), 2, "one Error and one committed-prefix COV");
    assert!(matches!(apdus[0], Apdu::Error(_)));
    assert_eq!(
        single_properties(&apdus[1]),
        vec![
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
        ]
    );
}
