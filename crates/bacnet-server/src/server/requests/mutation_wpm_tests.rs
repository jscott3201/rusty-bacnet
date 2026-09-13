use super::mutation_tests::{apdu, assert_denied, oid, value, wpm, Fixture};
use super::*;
use crate::mutation::MutationTarget;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleError};
use std::sync::{atomic::AtomicUsize, Mutex as StdMutex};

fn description(object: ObjectIdentifier, text: &str) -> WriteAccessSpecification {
    WriteAccessSpecification {
        object_identifier: object,
        list_of_properties: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::DESCRIPTION,
            property_array_index: None,
            value: value(PropertyValue::CharacterString(text.into())),
            priority: None,
        }],
    }
}

#[tokio::test]
async fn wpm_per_element_denial_or_panic_keeps_prefix_and_skips_suffix() {
    let first = oid(ObjectType::BINARY_VALUE, 1);
    let second = oid(ObjectType::BINARY_VALUE, 2);
    for panics in [false, true] {
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let seen = calls.clone();
        let fixture = Fixture::new(Some(Arc::new(move |context| {
            let MutationTarget::WritePropertyMultiple(attempt) = &context.target else {
                panic!("wrong target");
            };
            seen.lock().unwrap().push(attempt.clone());
            if attempt.reference.object_identifier == second {
                assert!(!panics, "second element policy panic");
                false
            } else {
                true
            }
        })));
        let initial_second = fixture.read(second, PropertyIdentifier::DESCRIPTION).await;
        let bytes = wpm(vec![
            description(first, "prefix"),
            description(second, "denied"),
            description(first, "suffix"),
        ]);
        let response = fixture
            .dispatch(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, bytes, 3)
            .await
            .unwrap();
        assert_denied(
            response.clone(),
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            3,
        );
        let Apdu::Error(error) = apdu(response) else {
            unreachable!()
        };
        let detailed = WritePropertyMultipleError::from_error_pdu(&error).unwrap();
        assert_eq!(
            detailed.first_failed_write_attempt.object_identifier,
            second
        );
        assert_eq!(
            detailed.first_failed_write_attempt.property_identifier,
            PropertyIdentifier::DESCRIPTION.to_raw()
        );
        assert_eq!(
            detailed.first_failed_write_attempt.property_array_index,
            None
        );
        assert_eq!(
            fixture.read(first, PropertyIdentifier::DESCRIPTION).await,
            PropertyValue::CharacterString("prefix".into())
        );
        assert_eq!(
            fixture.read(second, PropertyIdentifier::DESCRIPTION).await,
            initial_second
        );
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].reference.object_identifier, first);
        assert_eq!(
            calls[0].value,
            value(PropertyValue::CharacterString("prefix".into()))
        );
        assert_eq!(calls[1].reference.object_identifier, second);
    }
}

#[tokio::test]
async fn wpm_malformed_suffix_preserves_authorized_prefix_and_existing_error_bytes() {
    let first = oid(ObjectType::BINARY_VALUE, 1);
    for exact_coordinate in [false, true] {
        let mut bytes = BytesMut::from(wpm(vec![description(first, "prefix")]).as_ref());
        if exact_coordinate {
            // Incomplete indexed property value in the same object's list.
            bytes.truncate(bytes.len() - 1);
            bacnet_encoding::primitives::encode_ctx_unsigned(
                &mut bytes,
                0,
                PropertyIdentifier::OBJECT_TYPE.to_raw() as u64,
            );
            bacnet_encoding::primitives::encode_ctx_unsigned(&mut bytes, 1, 4);
        } else {
            // An undecodable next object coordinate after a complete prefix.
            bytes.extend_from_slice(&[0x0c]);
        }
        let bytes = bytes.freeze();
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let allow = Fixture::new(Some(Arc::new(move |_| {
            seen.fetch_add(1, Ordering::Relaxed);
            true
        })));
        let absent = Fixture::new(None);
        let baseline = absent
            .dispatch(
                ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                bytes.clone(),
                1,
            )
            .await
            .unwrap();
        let response = allow
            .dispatch(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, bytes, 1)
            .await
            .unwrap();
        assert_eq!(response, baseline);
        let Apdu::Error(error) = apdu(response) else {
            panic!("prefix requires Error, not Reject")
        };
        let detailed = WritePropertyMultipleError::from_error_pdu(&error).unwrap();
        assert_eq!(detailed.error_class, ErrorClass::SERVICES);
        assert_eq!(detailed.error_code, ErrorCode::INVALID_TAG);
        if exact_coordinate {
            assert_eq!(detailed.first_failed_write_attempt.object_identifier, first);
            assert_eq!(
                detailed.first_failed_write_attempt.property_identifier,
                PropertyIdentifier::OBJECT_TYPE.to_raw()
            );
            assert_eq!(
                detailed.first_failed_write_attempt.property_array_index,
                Some(4)
            );
        } else {
            assert_eq!(
                detailed
                    .first_failed_write_attempt
                    .object_identifier
                    .instance_number(),
                ObjectIdentifier::MAX_INSTANCE
            );
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            allow.read(first, PropertyIdentifier::DESCRIPTION).await,
            PropertyValue::CharacterString("prefix".into())
        );
    }
}

#[tokio::test]
async fn wpm_allow_all_preserves_order_index_priority_and_raw_value() {
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let seen = calls.clone();
    let allow = Fixture::new(Some(Arc::new(move |context| {
        seen.lock().unwrap().push(context.target.clone());
        true
    })));
    let absent = Fixture::new(None);
    let mut spec = description(oid(ObjectType::BINARY_VALUE, 1), "first");
    spec.list_of_properties.push(BACnetPropertyValue {
        property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
        property_array_index: Some(8),
        value: value(PropertyValue::Enumerated(1)),
        priority: Some(4),
    });
    let bytes = wpm(vec![
        spec,
        description(oid(ObjectType::BINARY_VALUE, 2), "third"),
    ]);
    let baseline = absent
        .dispatch(
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            bytes.clone(),
            1,
        )
        .await
        .unwrap();
    let response = allow
        .dispatch(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, bytes, 1)
        .await
        .unwrap();
    assert_eq!(baseline, response);
    assert!(matches!(apdu(response), Apdu::SimpleAck(_)));
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    let MutationTarget::WritePropertyMultiple(second) = &calls[1] else {
        unreachable!()
    };
    assert_eq!(
        second.reference.object_identifier,
        oid(ObjectType::BINARY_VALUE, 1)
    );
    assert_eq!(
        second.reference.property_identifier,
        PropertyIdentifier::PRIORITY_ARRAY.to_raw()
    );
    assert_eq!(second.reference.property_array_index, Some(8));
    assert_eq!(second.priority, Some(4));
    assert_eq!(second.value, value(PropertyValue::Enumerated(1)));
}

#[tokio::test]
async fn wpm_empty_request_never_calls_policy() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let fixture = Fixture::new(Some(Arc::new(move |_| {
        seen.fetch_add(1, Ordering::Relaxed);
        false
    })));
    let response = fixture
        .dispatch(
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            Bytes::new(),
            1,
        )
        .await
        .unwrap();
    assert!(matches!(apdu(response), Apdu::SimpleAck(_)));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}
