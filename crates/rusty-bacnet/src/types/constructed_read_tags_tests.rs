use super::super::read_value::decode_read_value;
use super::super::rpm_wpm::read_access_result_to_py;
use super::*;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::color::ColorObject;
use bacnet_objects::object_profile::ObjectProfile;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::rpm::{ReadAccessResult, ReadResultElement};
use bacnet_types::constructed::BACnetNameValue;
use bytes::BytesMut;
use pyo3::types::{PyDict, PyList};

const SEMANTIC: &[u8] = &[0x0a, 0, b'a'];
const NULL: &[u8] = &[0x0a, 0, b'a', 0];

#[test]
fn tags_typed_mapping_preserves_none_null_original_bytes_and_local_read_parity() {
    Python::initialize();
    Python::attach(|py| {
        let wire = [SEMANTIC, NULL].concat();
        let decoded =
            decode_read_value(ObjectType::COLOR, PropertyIdentifier::TAGS, None, &wire).unwrap();
        assert_eq!(decoded.element, Some(Element::NameValue));
        let mut encoded = BytesMut::new();
        encode_property_value(&mut encoded, &decoded.inner).unwrap();
        assert_eq!(encoded.as_ref(), wire);
        let value = Bound::new(py, decoded).unwrap();
        assert_eq!(
            value.getattr("tag").unwrap().extract::<String>().unwrap(),
            "list"
        );
        let mappings = value.getattr("value").unwrap();
        let mappings = mappings.cast::<PyList>().unwrap();
        let first = mappings.get_item(0).unwrap();
        assert!(first
            .cast::<PyDict>()
            .unwrap()
            .get_item("value")
            .unwrap()
            .unwrap()
            .is_none());
        let second = mappings.get_item(1).unwrap();
        let primitive = second
            .cast::<PyDict>()
            .unwrap()
            .get_item("value")
            .unwrap()
            .unwrap();
        assert_eq!(
            primitive
                .extract::<PyRef<'_, PyPropertyValue>>()
                .unwrap()
                .inner,
            PropertyValue::Null
        );
        let mut object = ColorObject::new(1, "local-tags").unwrap();
        object
            .set_profile(ObjectProfile {
                tags: Some(vec![
                    BACnetNameValue::semantic("a"),
                    BACnetNameValue::valued("a", PropertyValue::Null),
                ]),
                ..ObjectProfile::default()
            })
            .unwrap();
        let served = object
            .read_property(PropertyIdentifier::TAGS, None)
            .unwrap();
        let mut bytes = BytesMut::new();
        encode_property_value(&mut bytes, &served).unwrap();
        assert_eq!(bytes.as_ref(), wire);
        let local =
            decode_read_value(ObjectType::COLOR, PropertyIdentifier::TAGS, None, &bytes).unwrap();
        assert!(Bound::new(py, local).unwrap().eq(&value).unwrap());
    });
}

#[test]
fn tags_failed_typed_decode_preserves_generic_fallback_and_rpm_framing_errors() {
    for (index, bytes) in [
        (None, [SEMANTIC, &[0x0a, 0, 255]].concat()),
        (Some(1), [SEMANTIC, NULL].concat()),
    ] {
        let value =
            decode_read_value(ObjectType::COLOR, PropertyIdentifier::TAGS, index, &bytes).unwrap();
        assert_eq!(value.element, None);
        assert_eq!(value.inner, PropertyValue::ApplicationData(bytes));
    }
    let broken = vec![0x0d];
    assert!(decode_read_value(ObjectType::COLOR, PropertyIdentifier::TAGS, None, &broken).is_err());
    Python::initialize();
    Python::attach(|py| {
        let result = ReadAccessResult {
            object_identifier: ObjectIdentifier::new(ObjectType::COLOR, 1).unwrap(),
            list_of_results: vec![ReadResultElement {
                property_identifier: PropertyIdentifier::TAGS,
                property_array_index: None,
                property_value: Some(broken.clone()),
                error: None,
            }],
        };
        let result = read_access_result_to_py(py, result).unwrap();
        let rows = result.get_item("results").unwrap().unwrap();
        let row = rows.cast::<PyList>().unwrap().get_item(0).unwrap();
        let row = row.cast::<PyDict>().unwrap();
        assert_eq!(
            row.get_item("value")
                .unwrap()
                .unwrap()
                .cast::<PyBytes>()
                .unwrap()
                .as_bytes(),
            broken
        );
        assert!(row.get_item("error").unwrap().unwrap().is_none());
    });
}

#[test]
fn tags_empty_and_count_reads_keep_existing_generic_shapes() {
    let read = |index, bytes: &[u8]| {
        decode_read_value(ObjectType::COLOR, PropertyIdentifier::TAGS, index, bytes).unwrap()
    };
    assert_eq!(read(None, &[]).inner, PropertyValue::List(vec![]));
    assert_eq!(read(Some(0), &[0x21, 0]).inner, PropertyValue::Unsigned(0));
    assert_eq!(read(Some(1), &[]).element, None);
    assert_eq!(read(Some(0), SEMANTIC).element, None);
    assert_eq!(read(Some(0), &[0x21, 1, 0x21, 2]).element, None);
}
