use super::*;
use bacnet_types::enums::ObjectType;

fn request(mode: Option<bool>, lifetime: Option<u32>) -> SubscribeCOVRequest {
    SubscribeCOVRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        issue_confirmed_notifications: mode,
        lifetime,
    }
}

#[test]
fn subscribe_cov_lifetime_only_encode_is_transactional_and_decode_is_structural() {
    for lifetime in [0, 1, 28800, u32::MAX] {
        let request = request(None, Some(lifetime));
        let mut buffer = BytesMut::from(&b"prefix"[..]);
        assert!(matches!(
            request.encode(&mut buffer),
            Err(Error::Encoding(_))
        ));
        assert_eq!(&buffer[..], b"prefix");
        let mut raw = BytesMut::from(&[0x09, 1, 0x1c, 0, 0, 0, 3][..]);
        primitives::encode_ctx_unsigned(&mut raw, 3, u64::from(lifetime));
        assert_eq!(SubscribeCOVRequest::decode(&raw).unwrap(), request);
    }
}

#[test]
fn subscribe_cov_valid_finite_indefinite_and_cancel_independent_vectors() {
    for (mode, lifetime, fields) in [
        (None, None, vec![]),
        (Some(false), None, vec![0x29, 0]),
        (Some(true), None, vec![0x29, 1]),
        (Some(false), Some(0), vec![0x29, 0, 0x39, 0]),
        (Some(true), Some(0), vec![0x29, 1, 0x39, 0]),
        (Some(false), Some(1), vec![0x29, 0, 0x39, 1]),
        (Some(true), Some(28800), vec![0x29, 1, 0x3a, 0x70, 0x80]),
        (
            Some(true),
            Some(u32::MAX),
            vec![0x29, 1, 0x3c, 0xff, 0xff, 0xff, 0xff],
        ),
    ] {
        let mut expected = vec![0x09, 1, 0x1c, 0, 0, 0, 3];
        expected.extend(fields);
        let req = request(mode, lifetime);
        let mut buffer = BytesMut::new();
        req.encode(&mut buffer).unwrap();
        assert_eq!(&buffer[..], expected);
        assert_eq!(SubscribeCOVRequest::decode(&expected).unwrap(), req);
    }
}
