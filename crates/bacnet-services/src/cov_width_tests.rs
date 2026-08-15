use super::*;
use bacnet_types::enums::ObjectType;

const MAX_WITH_LEADING_ZERO: &[u8] = &[0, 0xff, 0xff, 0xff, 0xff];
const ONE: &[u8] = &[1];

fn object_identifier() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

fn encode_context_value(buf: &mut BytesMut, number: u8, value: &[u8]) {
    tags::encode_tag(buf, number, tags::TagClass::Context, value.len() as u32);
    buf.extend_from_slice(value);
}

fn subscribe_cov(process_id: &[u8], lifetime: Option<&[u8]>) -> BytesMut {
    let mut buf = BytesMut::new();
    encode_context_value(&mut buf, 0, process_id);
    primitives::encode_ctx_object_id(&mut buf, 1, &object_identifier());
    primitives::encode_ctx_boolean(&mut buf, 2, true);
    if let Some(lifetime) = lifetime {
        encode_context_value(&mut buf, 3, lifetime);
    }
    buf
}

fn subscribe_cov_property(
    process_id: &[u8],
    lifetime: &[u8],
    property_id: &[u8],
    array_index: Option<&[u8]>,
) -> BytesMut {
    let mut buf = BytesMut::new();
    encode_context_value(&mut buf, 0, process_id);
    primitives::encode_ctx_object_id(&mut buf, 1, &object_identifier());
    primitives::encode_ctx_boolean(&mut buf, 2, true);
    encode_context_value(&mut buf, 3, lifetime);
    tags::encode_opening_tag(&mut buf, 4);
    encode_context_value(&mut buf, 0, property_id);
    if let Some(array_index) = array_index {
        encode_context_value(&mut buf, 1, array_index);
    }
    tags::encode_closing_tag(&mut buf, 4);
    buf
}

fn cov_notification(process_id: &[u8], time_remaining: &[u8]) -> BytesMut {
    let mut buf = BytesMut::new();
    encode_context_value(&mut buf, 0, process_id);
    primitives::encode_ctx_object_id(
        &mut buf,
        1,
        &ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
    );
    primitives::encode_ctx_object_id(&mut buf, 2, &object_identifier());
    encode_context_value(&mut buf, 3, time_remaining);
    tags::encode_opening_tag(&mut buf, 4);
    BACnetPropertyValue {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        value: vec![0],
        priority: None,
    }
    .encode(&mut buf);
    tags::encode_closing_tag(&mut buf, 4);
    buf
}

#[test]
fn cov_unsigned_fields_accept_u32_max_with_leading_zero() {
    let decoded = SubscribeCOVRequest::decode(&subscribe_cov(
        MAX_WITH_LEADING_ZERO,
        Some(MAX_WITH_LEADING_ZERO),
    ))
    .unwrap();
    assert_eq!(decoded.subscriber_process_identifier, u32::MAX);
    assert_eq!(decoded.lifetime, Some(u32::MAX));

    let decoded = SubscribeCOVPropertyRequest::decode(&subscribe_cov_property(
        MAX_WITH_LEADING_ZERO,
        MAX_WITH_LEADING_ZERO,
        MAX_WITH_LEADING_ZERO,
        Some(MAX_WITH_LEADING_ZERO),
    ))
    .unwrap();
    assert_eq!(decoded.subscriber_process_identifier, u32::MAX);
    assert_eq!(decoded.lifetime, Some(u32::MAX));
    assert_eq!(decoded.monitored_property_identifier.to_raw(), u32::MAX);
    assert_eq!(decoded.monitored_property_array_index, Some(u32::MAX));

    let decoded = COVNotificationRequest::decode(&cov_notification(
        MAX_WITH_LEADING_ZERO,
        MAX_WITH_LEADING_ZERO,
    ))
    .unwrap();
    assert_eq!(decoded.subscriber_process_identifier, u32::MAX);
    assert_eq!(decoded.time_remaining, u32::MAX);
}

#[test]
fn cov_unsigned_fields_reject_u32_overflow() {
    for value in [&[1, 0, 0, 0, 0][..], &[0xff; 8][..]] {
        assert!(SubscribeCOVRequest::decode(&subscribe_cov(value, Some(ONE))).is_err());
        assert!(SubscribeCOVRequest::decode(&subscribe_cov(ONE, Some(value))).is_err());

        assert!(SubscribeCOVPropertyRequest::decode(&subscribe_cov_property(
            value, ONE, ONE, None,
        ))
        .is_err());
        assert!(SubscribeCOVPropertyRequest::decode(&subscribe_cov_property(
            ONE, value, ONE, None,
        ))
        .is_err());
        assert!(SubscribeCOVPropertyRequest::decode(&subscribe_cov_property(
            ONE, ONE, value, None,
        ))
        .is_err());
        assert!(SubscribeCOVPropertyRequest::decode(&subscribe_cov_property(
            ONE,
            ONE,
            ONE,
            Some(value),
        ))
        .is_err());

        assert!(COVNotificationRequest::decode(&cov_notification(value, ONE)).is_err());
        assert!(COVNotificationRequest::decode(&cov_notification(ONE, value)).is_err());
    }
}

#[test]
fn cov_decoders_reject_wrong_tags_malformed_booleans_and_trailing_data() {
    let mut wrong_tag = subscribe_cov(ONE, Some(ONE));
    wrong_tag[0] = 0x21;
    assert!(SubscribeCOVRequest::decode(&wrong_tag).is_err());

    let mut malformed_boolean = BytesMut::new();
    encode_context_value(&mut malformed_boolean, 0, ONE);
    primitives::encode_ctx_object_id(&mut malformed_boolean, 1, &object_identifier());
    encode_context_value(&mut malformed_boolean, 2, &[2]);
    encode_context_value(&mut malformed_boolean, 3, ONE);
    assert!(SubscribeCOVRequest::decode(&malformed_boolean).is_err());

    let mut subscription = subscribe_cov(ONE, Some(ONE));
    subscription.extend_from_slice(&[0]);
    assert!(SubscribeCOVRequest::decode(&subscription).is_err());

    let mut property = subscribe_cov_property(ONE, ONE, ONE, None);
    property.extend_from_slice(&[0]);
    assert!(SubscribeCOVPropertyRequest::decode(&property).is_err());

    let mut notification = cov_notification(ONE, ONE);
    notification.extend_from_slice(&[0]);
    assert!(COVNotificationRequest::decode(&notification).is_err());
}
