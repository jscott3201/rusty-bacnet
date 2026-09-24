use super::*;
use bacnet_types::enums::ObjectType;

fn request(bytes: Vec<u8>, index: Option<u32>) -> ListElementRequest {
    ListElementRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap(),
        property_identifier: PropertyIdentifier::from_raw(600),
        property_array_index: index,
        list_of_elements: bytes,
    }
}

#[test]
fn list_outbound_invalid_tail_preserves_existing_buffer() {
    let request = request(vec![0x21, 42, 0x22, 1], None);
    let mut buffer = BytesMut::from(&b"prefix"[..]);
    assert!(matches!(
        request.encode(&mut buffer),
        Err(Error::Encoding(_))
    ));
    assert_eq!(&buffer[..], b"prefix");
}

#[test]
fn list_outbound_empty_zero_index_and_bad_framing_are_transactional() {
    let malformed = [
        vec![],
        vec![0x1f],
        vec![0x3f],
        vec![0x1e],
        vec![0x1e, 0x2f],
        vec![0xf9],
        vec![0x65],
        vec![0x65, 255, 0, 0x10, 0, 1],
        vec![0x22, 1],
        vec![0x06],
    ];
    for bytes in malformed {
        let request = request(bytes, None);
        assert!(request.validate().is_err());
        let mut output = BytesMut::from(&b"unchanged"[..]);
        assert!(matches!(
            request.encode(&mut output),
            Err(Error::Encoding(_))
        ));
        assert_eq!(&output[..], b"unchanged");
    }
    let mut output = BytesMut::from(&b"index"[..]);
    assert!(request(vec![0], Some(0)).encode(&mut output).is_err());
    assert_eq!(&output[..], b"index");
}

#[test]
fn list_outbound_structural_only_preserves_empty_values_vendor_context_and_boolean() {
    let bodies = [
        vec![0],
        vec![0x60],
        vec![0x08],
        vec![0x0e, 0x0f],
        vec![0x10, 0x11, 0x21, 7],
        vec![0xf8, 254], // Empty vendor context value.
        vec![0x0e, 0x21, 7, 0x1e, 0x08, 0x1f, 0x0f],
        // Framing is valid: primitive widths, charset and application tag
        // interpretation remain the target's responsibility.
        vec![0x40],
        vec![0x71, 255],
        vec![0xd1, 0],
        vec![0x32, 0, 1],
    ];
    for body in bodies {
        for index in [None, Some(1), Some(u32::MAX)] {
            let request = request(body.clone(), index);
            request.validate().unwrap();
            let mut bytes = BytesMut::from(&b"prefix"[..]);
            request.encode(&mut bytes).unwrap();
            let mut expected = vec![0x0c, 0x03, 0xc0, 0, 1, 0x1a, 2, 0x58];
            match index {
                None => {}
                Some(1) => expected.extend_from_slice(&[0x29, 1]),
                Some(_) => expected.extend_from_slice(&[0x2c, 255, 255, 255, 255]),
            }
            expected.push(0x3e);
            expected.extend_from_slice(&body);
            expected.push(0x3f);
            assert_eq!(&bytes[..6], b"prefix");
            assert_eq!(&bytes[6..], expected);
            assert_eq!(ListElementRequest::decode(&bytes[6..]).unwrap(), request);
        }
    }
}

#[test]
fn list_outbound_depth_budget_includes_service_wrapper_and_tag_length_limit() {
    for depth in [
        tags::MAX_CONTEXT_NESTING_DEPTH - 1,
        tags::MAX_CONTEXT_NESTING_DEPTH,
    ] {
        let mut body = vec![0x0e; depth];
        body.extend(vec![0x0f; depth]);
        let request = request(body, None);
        let mut bytes = BytesMut::from(&b"prefix"[..]);
        if depth == tags::MAX_CONTEXT_NESTING_DEPTH - 1 {
            request.encode(&mut bytes).unwrap();
            assert_eq!(ListElementRequest::decode(&bytes[6..]).unwrap(), request);
        } else {
            assert!(request.encode(&mut bytes).is_err());
            assert_eq!(&bytes[..], b"prefix");
        }
    }
    // Existing decode_tag sanity limit is 1 MiB per tag; no service-specific
    // primitive whitelist or new item-count policy is introduced.
    let mut max = vec![0x65, 255, 0, 0x10, 0, 0];
    max.resize(max.len() + 1_048_576, 0);
    request(max, None).validate().unwrap();
    assert!(request(vec![0x65, 255, 0, 0x10, 0, 1], None)
        .validate()
        .is_err());
}
