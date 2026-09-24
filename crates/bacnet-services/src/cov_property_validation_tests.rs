use super::*;
use bacnet_types::enums::ObjectType;

#[test]
fn subscribe_cov_property_round_trip() {
    let req = SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 7,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        issue_confirmed_notifications: Some(true),
        lifetime: Some(600),
        monitored_property_identifier: PropertyIdentifier::PRESENT_VALUE,
        monitored_property_array_index: None,
        cov_increment: Some(1.5),
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = SubscribeCOVPropertyRequest::decode(&buf).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn subscribe_cov_property_round_trip_with_array_index() {
    let req = SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 2,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::BINARY_VALUE, 10).unwrap(),
        issue_confirmed_notifications: None,
        lifetime: None,
        monitored_property_identifier: PropertyIdentifier::PRESENT_VALUE,
        monitored_property_array_index: Some(3),
        cov_increment: None,
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = SubscribeCOVPropertyRequest::decode(&buf).unwrap();
    assert_eq!(req, decoded);
    assert!(decoded.is_cancellation());
}

fn request(confirmed: Option<bool>, lifetime: Option<u32>) -> SubscribeCOVPropertyRequest {
    SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        issue_confirmed_notifications: confirmed,
        lifetime,
        monitored_property_identifier: PropertyIdentifier::PRESENT_VALUE,
        monitored_property_array_index: Some(0),
        cov_increment: Some(0.5),
    }
}

#[test]
fn subscribe_cov_property_outbound_pair_validation_is_transactional() {
    for (confirmed, lifetime) in [
        (Some(false), None),
        (Some(true), None),
        (None, Some(1)),
        (None, Some(0)),
        (Some(false), Some(0)),
        (Some(true), Some(0)),
    ] {
        let mut output = BytesMut::from(&b"prefix"[..]);
        assert!(matches!(
            request(confirmed, lifetime).encode(&mut output),
            Err(Error::Encoding(_))
        ));
        assert_eq!(&output[..], b"prefix");
    }
}

#[test]
fn subscribe_cov_property_positive_bounds_and_cancel_have_independent_wire_vectors() {
    let base = [0x09, 1, 0x1c, 0, 0, 0, 3]; // process1, AnalogInput3
    for (confirmed, lifetime, fields) in [
        (Some(false), Some(1), vec![0x29, 0, 0x39, 1]),
        (Some(true), Some(28800), vec![0x29, 1, 0x3a, 0x70, 0x80]),
        (
            Some(false),
            Some(u32::MAX),
            vec![0x29, 0, 0x3c, 0xff, 0xff, 0xff, 0xff],
        ),
        (None, None, vec![]),
    ] {
        let mut expected = base.to_vec();
        expected.extend(fields);
        expected.extend([0x4e, 0x09, 85, 0x19, 0, 0x4f, 0x5c, 0x3f, 0, 0, 0]);
        let req = request(confirmed, lifetime);
        let mut encoded = BytesMut::new();
        req.encode(&mut encoded).unwrap();
        assert_eq!(&encoded[..], expected);
        assert_eq!(SubscribeCOVPropertyRequest::decode(&expected).unwrap(), req);
    }
}

#[test]
fn subscribe_cov_property_decode_retains_invalid_service_values_for_formal_errors() {
    for fields in [vec![0x29, 0], vec![0x39, 0], vec![0x29, 0, 0x39, 0]] {
        let mut raw = vec![0x09, 1, 0x1c, 0, 0, 0, 3];
        raw.extend(fields);
        raw.extend([0x4e, 0x09, 85, 0x4f]);
        let decoded = SubscribeCOVPropertyRequest::decode(&raw).unwrap();
        assert!(decoded.validate().is_err());
    }
}
