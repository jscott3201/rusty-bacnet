use super::mutation_tests::{apdu, oid, Fixture};
use super::*;
use bacnet_encoding::{primitives, tags};
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_services::wpm::WritePropertyMultipleError;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use std::sync::atomic::AtomicUsize;

fn write_value(bytes: &mut BytesMut, value: f32, index: Option<u32>) {
    primitives::encode_ctx_unsigned(bytes, 0, PropertyIdentifier::PRESENT_VALUE.to_raw() as u64);
    if let Some(index) = index {
        primitives::encode_ctx_unsigned(bytes, 1, index as u64);
    }
    tags::encode_opening_tag(bytes, 2);
    primitives::encode_app_real(bytes, value);
    tags::encode_closing_tag(bytes, 2);
}

fn request(prefix: bool, index: Option<u32>, priority: &[u8], suffix: bool) -> Bytes {
    let mut bytes = BytesMut::new();
    primitives::encode_ctx_object_id(&mut bytes, 0, &oid(ObjectType::ANALOG_OUTPUT, 7));
    tags::encode_opening_tag(&mut bytes, 1);
    if prefix {
        write_value(&mut bytes, 10.0, None);
        primitives::encode_ctx_unsigned(&mut bytes, 3, 8);
    }
    write_value(&mut bytes, 20.0, index);
    bytes.extend_from_slice(priority);
    if suffix {
        write_value(&mut bytes, 99.0, None);
        primitives::encode_ctx_unsigned(&mut bytes, 3, 1);
        tags::encode_closing_tag(&mut bytes, 1);
    }
    bytes.freeze()
}

async fn fixture() -> (Fixture, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let fixture = Fixture::new(Some(Arc::new(move |_| {
        seen.fetch_add(1, Ordering::Relaxed);
        true
    })));
    fixture
        .db
        .write()
        .await
        .add(Box::new(AnalogOutputObject::new(7, "AO", 62).unwrap()))
        .unwrap();
    (fixture, calls)
}

async fn assert_state(fixture: &Fixture, calls: &AtomicUsize, prefix: bool) {
    let db = fixture.db.read().await;
    let object = db.get(&oid(ObjectType::ANALOG_OUTPUT, 7)).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(if prefix { 10.0 } else { 0.0 })
    );
    for slot in 1..=16 {
        assert_eq!(
            object
                .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(slot))
                .unwrap(),
            if prefix && slot == 8 {
                PropertyValue::Real(10.0)
            } else {
                PropertyValue::Null
            },
            "slot {slot}"
        );
    }
    assert_eq!(
        calls.load(Ordering::Relaxed),
        usize::from(prefix),
        "only the valid prefix reaches policy"
    );
}

fn assert_error(response: Bytes, index: Option<u32>, code: ErrorCode) {
    let Apdu::Error(error) = apdu(response) else {
        panic!("expected formal WPM Error")
    };
    assert_eq!(error.invoke_id, 71);
    assert_eq!(
        error.service_choice,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE
    );
    assert_eq!(
        WritePropertyMultipleError::from_error_pdu(&error).unwrap(),
        WritePropertyMultipleError {
            error_class: ErrorClass::SERVICES,
            error_code: code,
            first_failed_write_attempt: BACnetObjectPropertyReference {
                object_identifier: oid(ObjectType::ANALOG_OUTPUT, 7),
                property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
                property_array_index: index,
            },
        }
    );
}

#[tokio::test]
async fn wpm_priority_range_wire_error_is_semantic_before_and_after_prefix() {
    for priority in [0, 17, 257] {
        for prefix in [false, true] {
            for index in [None, Some(4)] {
                let mut encoded = BytesMut::new();
                primitives::encode_ctx_unsigned(&mut encoded, 3, priority);
                let (fixture, calls) = fixture().await;
                let response = fixture
                    .dispatch(
                        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                        request(prefix, index, &encoded, true),
                        71,
                    )
                    .await
                    .unwrap();
                assert_error(response, index, ErrorCode::PARAMETER_OUT_OF_RANGE);
                assert_state(&fixture, &calls, prefix).await;
            }
        }
    }
}

#[tokio::test]
async fn wpm_priority_syntax_wire_result_retains_reject_and_prefix_invalid_tag() {
    for malformed in [&[0x39][..], &[0x3a, 1], &[0x38]] {
        for prefix in [false, true] {
            let (fixture, calls) = fixture().await;
            let response = fixture
                .dispatch(
                    ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                    request(prefix, None, malformed, false),
                    71,
                )
                .await
                .unwrap();
            if prefix {
                assert_error(response, None, ErrorCode::INVALID_TAG);
            } else {
                let Apdu::Reject(reject) = apdu(response) else {
                    panic!("expected initial syntax Reject")
                };
                assert_eq!(reject.invoke_id, 71);
                assert_eq!(reject.reject_reason, RejectReason::INVALID_DATA_ENCODING);
            }
            assert_state(&fixture, &calls, prefix).await;
        }
    }
}
