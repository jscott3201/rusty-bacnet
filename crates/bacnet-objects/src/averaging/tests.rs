use super::*;
use bacnet_types::enums::ObjectType;
use bacnet_types::error::RemoteError;

#[test]
fn averaging_create() {
    let avg = AveragingObject::new(1, "AVG-1").unwrap();
    assert_eq!(
        avg.read_property(PropertyIdentifier::OBJECT_NAME, None)
            .unwrap(),
        PropertyValue::CharacterString("AVG-1".into())
    );
    assert_eq!(
        avg.read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(ObjectType::AVERAGING.to_raw())
    );
    // The starting statistics are pinned in window_tests.rs.
    assert!(matches!(
        avg.read_property(PropertyIdentifier::AVERAGE_VALUE, None)
            .unwrap(),
        PropertyValue::Real(v) if v.is_nan()
    ));
}

#[test]
fn averaging_add_samples() {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    avg.add_sample(10.0).unwrap();
    avg.add_sample(20.0).unwrap();
    avg.add_sample(30.0).unwrap();

    assert_eq!(
        avg.read_property(PropertyIdentifier::ATTEMPTED_SAMPLES, None)
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
    assert_eq!(
        avg.read_property(PropertyIdentifier::VALID_SAMPLES, None)
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
}

#[test]
fn averaging_min_max() {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    avg.add_sample(15.0).unwrap();
    avg.add_sample(5.0).unwrap();
    avg.add_sample(25.0).unwrap();

    assert_eq!(
        avg.read_property(PropertyIdentifier::MINIMUM_VALUE, None)
            .unwrap(),
        PropertyValue::Real(5.0)
    );
    assert_eq!(
        avg.read_property(PropertyIdentifier::MAXIMUM_VALUE, None)
            .unwrap(),
        PropertyValue::Real(25.0)
    );
}

#[test]
fn averaging_average_value() {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    avg.add_sample(10.0).unwrap();
    avg.add_sample(20.0).unwrap();
    avg.add_sample(30.0).unwrap();

    let val = avg
        .read_property(PropertyIdentifier::AVERAGE_VALUE, None)
        .unwrap();
    if let PropertyValue::Real(v) = val {
        assert!((v - 20.0).abs() < 0.001);
    } else {
        panic!("Expected Real");
    }
}

#[test]
fn averaging_property_list() {
    let avg = AveragingObject::new(1, "AVG-1").unwrap();
    let props = avg.property_list();
    assert!(props.contains(&PropertyIdentifier::MINIMUM_VALUE));
    assert!(props.contains(&PropertyIdentifier::MAXIMUM_VALUE));
    assert!(props.contains(&PropertyIdentifier::AVERAGE_VALUE));
    assert!(props.contains(&PropertyIdentifier::ATTEMPTED_SAMPLES));
    assert!(props.contains(&PropertyIdentifier::VALID_SAMPLES));
    assert!(props.contains(&PropertyIdentifier::OBJECT_PROPERTY_REFERENCE));
    assert!(props.contains(&PropertyIdentifier::WINDOW_INTERVAL));
    assert!(props.contains(&PropertyIdentifier::WINDOW_SAMPLES));
    // Table 12-5 defines none of these (#1064).
    for absent in [
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::STATUS_FLAGS,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyIdentifier::RELIABILITY,
        PropertyIdentifier::EVENT_STATE,
    ] {
        assert!(!props.contains(&absent), "{absent:?}");
    }
}

/// The unset form an Averaging object without a reference reads as (#1417):
/// [0] analog-input 4194303, [1] present-value.
fn unset() -> PropertyValue {
    PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x3F, 0xFF, 0xFF, 0x19, 0x55])
}

#[test]
fn averaging_object_property_reference_default_unset() {
    let avg = AveragingObject::new(1, "AVG-1").unwrap();
    assert_eq!(
        avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
            .unwrap(),
        unset()
    );
}

#[test]
fn averaging_set_object_property_reference() {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 5).unwrap();
    let pv_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
    avg.set_object_property_reference(Some(BACnetObjectPropertyReference::new(oid, pv_raw)));

    // BACnetDeviceObjectPropertyReference with no index or Device (#1182).
    assert_eq!(
        avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
            .unwrap(),
        PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x00, 0x00, 0x05, 0x19, 0x55])
    );
}

#[test]
fn averaging_write_object_property_reference() {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    // [0] analog-input 3, [1] present-value: what a read serves comes back.
    let framed = PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x00, 0x00, 0x03, 0x19, 0x55]);
    avg.write_property(
        PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
        None,
        framed.clone(),
        None,
    )
    .unwrap();
    assert_eq!(
        avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
            .unwrap(),
        framed
    );
}

#[test]
fn averaging_write_of_the_unset_form_clears_reference() {
    let set = || {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        avg.set_object_property_reference(Some(BACnetObjectPropertyReference::new(
            oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )));
        avg
    };
    // The form a read serves, another object type at the reserved instance,
    // and a Device member at it all clear the reference, the last although
    // a Device member is otherwise refused.
    for octets in [
        vec![0x0C, 0x00, 0x3F, 0xFF, 0xFF, 0x19, 0x55],
        vec![0x0C, 0x00, 0xBF, 0xFF, 0xFF, 0x19, 0x55],
        vec![
            0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x3C, 0x02, 0x3F, 0xFF, 0xFF,
        ],
    ] {
        let mut avg = set();
        avg.write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            PropertyValue::ApplicationData(octets),
            None,
        )
        .unwrap();
        assert_eq!(
            avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
                .unwrap(),
            unset()
        );
    }
    // So does the setter given the unset form.
    let mut avg = set();
    avg.set_object_property_reference(Some(BACnetObjectPropertyReference::new(
        ObjectIdentifier::new(
            ObjectType::ANALOG_VALUE,
            ObjectIdentifier::WILDCARD_INSTANCE,
        )
        .unwrap(),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    assert_eq!(
        avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
            .unwrap(),
        unset()
    );
}

#[test]
fn averaging_null_reference_write_is_another_datatype() {
    // A NULL is no BACnetDeviceObjectPropertyReference: INVALID_DATA_TYPE,
    // which the server turns into the Clause 15.9.2 no-op (#1417). The
    // reference and the samples stay.
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    avg.set_object_property_reference(Some(BACnetObjectPropertyReference::new(
        oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    avg.add_sample(4.0).unwrap();
    let before = avg
        .read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
        .unwrap();
    let err = avg
        .write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            PropertyValue::Null,
            None,
        )
        .unwrap_err();
    assert!(
        matches!(err, Error::Remote(RemoteError::Error { class, code })
            if class == bacnet_types::enums::ErrorClass::PROPERTY
                && code == bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE),
        "{err:?}"
    );
    assert_eq!(
        avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
            .unwrap(),
        before
    );
    assert_eq!(
        avg.read_property(PropertyIdentifier::VALID_SAMPLES, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
}

#[test]
fn averaging_write_present_value_denied() {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    let result = avg.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(42.0),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn averaging_description_read_write() {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    assert_eq!(
        avg.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString(String::new())
    );
    avg.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        PropertyValue::CharacterString("Zone temperature averaging".into()),
        None,
    )
    .unwrap();
    assert_eq!(
        avg.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("Zone temperature averaging".into())
    );
}

#[test]
fn averaging_single_sample() {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    avg.add_sample(42.0).unwrap();

    assert_eq!(
        avg.read_property(PropertyIdentifier::MINIMUM_VALUE, None)
            .unwrap(),
        PropertyValue::Real(42.0)
    );
    assert_eq!(
        avg.read_property(PropertyIdentifier::MAXIMUM_VALUE, None)
            .unwrap(),
        PropertyValue::Real(42.0)
    );
    assert_eq!(
        avg.read_property(PropertyIdentifier::AVERAGE_VALUE, None)
            .unwrap(),
        PropertyValue::Real(42.0)
    );
}

// --- #182 adversary blocker: strict shared decode on the reference arm ---

#[test]
fn averaging_reference_write_refuses_the_flat_list_forms() {
    // Reads serve the context-tagged form (#1182), so a flat
    // application-tagged reference is a value of another datatype.
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap();
    let pv_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
    for members in [
        vec![
            PropertyValue::ObjectIdentifier(oid),
            PropertyValue::Unsigned(pv_raw as u64),
        ],
        vec![
            PropertyValue::ObjectIdentifier(oid),
            PropertyValue::Enumerated(pv_raw),
        ],
        vec![
            PropertyValue::ObjectIdentifier(oid),
            PropertyValue::Unsigned(pv_raw as u64),
            PropertyValue::Unsigned(4),
        ],
    ] {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        let err = avg
            .write_property(
                PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
                None,
                PropertyValue::List(members.clone()),
                None,
            )
            .unwrap_err();
        assert!(
            matches!(err, Error::Remote(RemoteError::Error { class, code })
                if class == bacnet_types::enums::ErrorClass::PROPERTY
                    && code == bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE),
            "{members:?}: {err:?}"
        );
        assert_eq!(
            avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
                .unwrap(),
            unset()
        );
    }
}

#[test]
fn averaging_reference_write_rejects_bad_shapes_and_preserves_state() {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap();
    let pv_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
    let baseline = PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x00, 0x00, 0x07, 0x19, 0x55]);

    // A framed device-qualified write ([3] device-identifier): the
    // Clause 12.5 typing is BACnetDeviceObjectPropertyReference, so the
    // encoding is valid, but the remote-sample path is the standard's
    // OPTIONAL branch, unmodeled here. The object can't tell which Device
    // holds it, so any Device member is OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED
    // rather than silently local-ized (#1153; the server drops one naming
    // its own Device first).
    let mut framed_device = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_object_property_reference(
        &mut framed_device,
        &BACnetObjectPropertyReference::new(oid, pv_raw),
    );
    bacnet_encoding::primitives::encode_ctx_object_id(
        &mut framed_device,
        3,
        &ObjectIdentifier::new(ObjectType::DEVICE, 42).unwrap(),
    );

    let cases: Vec<(PropertyValue, bacnet_types::enums::ErrorCode, &str)> = vec![
        (
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(oid),
                PropertyValue::Unsigned(pv_raw as u64),
                PropertyValue::Unsigned(2),
                PropertyValue::Unsigned(9), // 4th member: silently dropped pre-fix
            ]),
            bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE,
            "4-member list",
        ),
        (
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(oid),
                PropertyValue::Unsigned(pv_raw as u64),
                PropertyValue::Real(2.0), // non-Unsigned 3rd: retyped to no-index pre-fix
            ]),
            bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE,
            "wrong-typed third member",
        ),
        (
            PropertyValue::List(vec![
                PropertyValue::Unsigned(1),
                PropertyValue::Unsigned(pv_raw as u64),
            ]),
            bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE,
            "non-ObjectIdentifier first member",
        ),
        (
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(oid),
                PropertyValue::Unsigned(u64::MAX), // > 4 octets: `as u32` truncated pre-fix
            ]),
            bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE,
            "oversized Unsigned property member",
        ),
        (
            PropertyValue::ApplicationData(framed_device.to_vec()),
            bacnet_types::enums::ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            "device-qualified framed reference",
        ),
        (
            // [3] names analog-input 42, which is no Device (#1182).
            PropertyValue::ApplicationData(
                [
                    &framed_device[..framed_device.len() - 5],
                    &[0x3C, 0x00, 0x00, 0x00, 0x2A],
                ]
                .concat(),
            ),
            bacnet_types::enums::ErrorCode::VALUE_OUT_OF_RANGE,
            "Device member that is no Device",
        ),
        (
            PropertyValue::ApplicationData(
                [framed_device.to_vec(), framed_device.to_vec()].concat(),
            ),
            bacnet_types::enums::ErrorCode::INVALID_DATA_ENCODING,
            "two framed references",
        ),
        (
            PropertyValue::ApplicationData(framed_device[..framed_device.len() - 1].to_vec()),
            bacnet_types::enums::ErrorCode::INVALID_DATA_ENCODING,
            "truncated device member",
        ),
    ];

    for (value, expected_code, label) in cases {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        avg.write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            baseline.clone(),
            None,
        )
        .unwrap();
        let err = avg
            .write_property(
                PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
                None,
                value,
                None,
            )
            .expect_err(label);
        match err {
            Error::Remote(RemoteError::Error { class, code }) => {
                assert_eq!(
                    class,
                    bacnet_types::enums::ErrorClass::PROPERTY,
                    "{label}: wrong class"
                );
                assert_eq!(code, expected_code, "{label}: wrong code");
            }
            other => panic!("{label}: expected Property error, got {other:?}"),
        }
        assert_eq!(
            avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
                .unwrap(),
            baseline,
            "{label}: refused write must leave the stored reference untouched"
        );
    }
}

#[test]
fn averaging_reference_write_accepts_the_framed_local_form() {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap();
    let pv_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
    let mut framed = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_object_property_reference(
        &mut framed,
        &BACnetObjectPropertyReference::new_indexed(oid, pv_raw, 2),
    );
    avg.write_property(
        PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
        None,
        PropertyValue::ApplicationData(framed.to_vec()),
        None,
    )
    .unwrap();
    // The index member [2] comes back after [0] and [1].
    assert_eq!(
        avg.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
            .unwrap(),
        PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x00, 0x00, 0x07, 0x19, 0x55, 0x29, 0x02])
    );
}
