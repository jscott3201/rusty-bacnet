use super::*;
use bacnet_types::enums::ObjectType;

const MAX_WITH_LEADING_ZERO: &[u8] = &[0, 0xff, 0xff, 0xff, 0xff];
const ONE: &[u8] = &[1];

fn encode_context_value(buf: &mut BytesMut, number: u8, value: &[u8]) {
    tags::encode_tag(buf, number, tags::TagClass::Context, value.len() as u32);
    buf.extend_from_slice(value);
}

fn subscribe_cov_multiple(
    process_id: &[u8],
    lifetime: Option<&[u8]>,
    max_notification_delay: Option<&[u8]>,
) -> BytesMut {
    let mut buf = BytesMut::new();
    encode_context_value(&mut buf, 0, process_id);
    primitives::encode_ctx_boolean(&mut buf, 1, true);
    if let Some(lifetime) = lifetime {
        encode_context_value(&mut buf, 2, lifetime);
    }
    if let Some(delay) = max_notification_delay {
        encode_context_value(&mut buf, 3, delay);
    }
    tags::encode_opening_tag(&mut buf, 4);
    tags::encode_closing_tag(&mut buf, 4);
    buf
}

fn cov_notification_multiple(
    process_id: &[u8],
    time_remaining: &[u8],
    property_id: &[u8],
    array_index: Option<&[u8]>,
) -> BytesMut {
    let mut buf = BytesMut::new();
    encode_context_value(&mut buf, 0, process_id);
    primitives::encode_ctx_object_id(
        &mut buf,
        1,
        &ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
    );
    encode_context_value(&mut buf, 2, time_remaining);
    primitives::encode_timestamp(&mut buf, 3, &BACnetTimeStamp::SequenceNumber(1)).unwrap();
    tags::encode_opening_tag(&mut buf, 4);
    primitives::encode_ctx_object_id(
        &mut buf,
        0,
        &ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
    );
    tags::encode_opening_tag(&mut buf, 1);
    encode_context_value(&mut buf, 0, property_id);
    if let Some(array_index) = array_index {
        encode_context_value(&mut buf, 1, array_index);
    }
    tags::encode_opening_tag(&mut buf, 2);
    buf.extend_from_slice(&[0]);
    tags::encode_closing_tag(&mut buf, 2);
    tags::encode_closing_tag(&mut buf, 1);
    tags::encode_closing_tag(&mut buf, 4);
    buf
}

#[test]
fn cov_multiple_unsigned_fields_accept_u32_max_with_leading_zero() {
    let decoded = SubscribeCOVPropertyMultipleRequest::decode(&subscribe_cov_multiple(
        MAX_WITH_LEADING_ZERO,
        Some(MAX_WITH_LEADING_ZERO),
        Some(MAX_WITH_LEADING_ZERO),
    ))
    .unwrap();
    assert_eq!(decoded.subscriber_process_identifier, u32::MAX);
    assert_eq!(decoded.lifetime, Some(u32::MAX));
    assert_eq!(decoded.max_notification_delay, Some(u32::MAX));

    let decoded = COVNotificationMultipleRequest::decode(&cov_notification_multiple(
        MAX_WITH_LEADING_ZERO,
        MAX_WITH_LEADING_ZERO,
        MAX_WITH_LEADING_ZERO,
        Some(MAX_WITH_LEADING_ZERO),
    ))
    .unwrap();
    assert_eq!(decoded.subscriber_process_identifier, u32::MAX);
    assert_eq!(decoded.time_remaining, u32::MAX);
    let value = &decoded.list_of_cov_notifications[0].list_of_values[0];
    assert_eq!(value.property_identifier.to_raw(), u32::MAX);
    assert_eq!(value.property_array_index, Some(u32::MAX));
}

#[test]
fn cov_multiple_unsigned_fields_reject_u32_overflow() {
    for value in [&[1, 0, 0, 0, 0][..], &[0xff; 8][..]] {
        assert!(
            SubscribeCOVPropertyMultipleRequest::decode(
                &subscribe_cov_multiple(value, None, None,)
            )
            .is_err()
        );
        assert!(
            SubscribeCOVPropertyMultipleRequest::decode(&subscribe_cov_multiple(
                ONE,
                Some(value),
                None,
            ))
            .is_err()
        );
        assert!(
            SubscribeCOVPropertyMultipleRequest::decode(&subscribe_cov_multiple(
                ONE,
                None,
                Some(value),
            ))
            .is_err()
        );

        assert!(
            COVNotificationMultipleRequest::decode(&cov_notification_multiple(
                value, ONE, ONE, None,
            ))
            .is_err()
        );
        assert!(
            COVNotificationMultipleRequest::decode(&cov_notification_multiple(
                ONE, value, ONE, None,
            ))
            .is_err()
        );
        assert!(
            COVNotificationMultipleRequest::decode(&cov_notification_multiple(
                ONE, ONE, value, None,
            ))
            .is_err()
        );
        assert!(
            COVNotificationMultipleRequest::decode(&cov_notification_multiple(
                ONE,
                ONE,
                ONE,
                Some(value),
            ))
            .is_err()
        );
    }
}

#[test]
fn cov_multiple_decoders_reject_wrong_tags_malformed_booleans_and_trailing_data() {
    let mut wrong_tag = subscribe_cov_multiple(ONE, None, None);
    wrong_tag[0] = 0x21;
    assert!(SubscribeCOVPropertyMultipleRequest::decode(&wrong_tag).is_err());

    let mut malformed_boolean = BytesMut::new();
    encode_context_value(&mut malformed_boolean, 0, ONE);
    encode_context_value(&mut malformed_boolean, 1, &[2]);
    tags::encode_opening_tag(&mut malformed_boolean, 4);
    tags::encode_closing_tag(&mut malformed_boolean, 4);
    assert!(SubscribeCOVPropertyMultipleRequest::decode(&malformed_boolean).is_err());

    let mut subscription = subscribe_cov_multiple(ONE, None, None);
    subscription.extend_from_slice(&[0]);
    assert!(SubscribeCOVPropertyMultipleRequest::decode(&subscription).is_err());

    let mut notification = cov_notification_multiple(ONE, ONE, ONE, None);
    notification.extend_from_slice(&[0]);
    assert!(COVNotificationMultipleRequest::decode(&notification).is_err());
}
