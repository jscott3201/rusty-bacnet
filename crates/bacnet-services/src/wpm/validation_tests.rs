use super::*;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

fn property(priority: Option<u8>) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: Some(0),
        value: vec![0],
        priority,
    }
}
fn request(properties: Vec<BACnetPropertyValue>) -> WritePropertyMultipleRequest {
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            list_of_properties: properties,
        }],
    }
}

#[test]
fn wpm_outbound_late_invalid_priority_preserves_buffer() {
    for priority in [0, 17, 255] {
        let request = request(vec![property(None), property(Some(priority))]);
        let mut buffer = BytesMut::from(&b"prefix"[..]);
        assert!(matches!(
            request.encode(&mut buffer),
            Err(Error::Encoding(_))
        ));
        assert_eq!(&buffer[..], b"prefix");
    }
}

fn assert_invalid(request: &WritePropertyMultipleRequest) {
    assert!(matches!(request.validate(), Err(Error::Encoding(_))));
    let mut buffer = BytesMut::from(&b"existing"[..]);
    assert!(matches!(
        request.encode(&mut buffer),
        Err(Error::Encoding(_))
    ));
    assert_eq!(&buffer[..], b"existing");
}

#[test]
fn wpm_outbound_rejects_empty_lists_selectors_and_later_invalid_objects() {
    assert_invalid(&WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![],
    });
    assert_invalid(&request(vec![]));
    for selector in [
        PropertyIdentifier::ALL,
        PropertyIdentifier::REQUIRED,
        PropertyIdentifier::OPTIONAL,
    ] {
        let mut bad = property(None);
        bad.property_identifier = selector;
        assert_invalid(&request(vec![property(Some(8)), bad.clone()]));
        let mut multiple = request(vec![property(None)]);
        multiple
            .list_of_write_access_specs
            .extend(request(vec![bad]).list_of_write_access_specs);
        assert_invalid(&multiple);
    }
    let mut multiple = request(vec![property(None)]);
    multiple
        .list_of_write_access_specs
        .extend(request(vec![]).list_of_write_access_specs);
    assert_invalid(&multiple);
    for priority in [0, 17, 255] {
        let mut multiple = request(vec![property(None)]);
        multiple
            .list_of_write_access_specs
            .extend(request(vec![property(Some(priority))]).list_of_write_access_specs);
        assert_invalid(&multiple);
    }
}

#[test]
fn wpm_outbound_preserves_valid_priority_index_zero_proprietary_and_empty_value_bytes() {
    for priority in std::iter::once(None).chain((1..=16).map(Some)) {
        for value in [vec![0], vec![]] {
            let mut property = property(priority);
            property.property_identifier = PropertyIdentifier::from_raw(600);
            property.value = value.clone();
            let request = request(vec![property]);
            request.validate().unwrap();
            let mut bytes = BytesMut::from(&b"prefix"[..]);
            request.encode(&mut bytes).unwrap();
            let mut expected = vec![0x0c, 0, 0x40, 0, 1, 0x1e, 0x0a, 2, 0x58, 0x19, 0, 0x2e];
            expected.extend_from_slice(&value);
            expected.push(0x2f);
            if let Some(priority) = priority {
                expected.extend_from_slice(&[0x39, priority]);
            }
            expected.push(0x1f);
            assert_eq!(&bytes[..6], b"prefix");
            assert_eq!(&bytes[6..], expected);
            assert_eq!(
                WritePropertyMultipleRequest::decode(&bytes[6..]).unwrap(),
                request
            );
        }
    }
}
