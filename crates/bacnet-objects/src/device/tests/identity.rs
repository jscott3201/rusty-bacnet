//! Shared Device identity validation and segmentation-derived declarations.
use super::*;
use bacnet_types::error::RemoteError;

#[test]
fn mode_derived_max_segments_accepted() {
    let cases = [
        ("none", Segmentation::NONE, None),
        ("transmit", Segmentation::TRANSMIT, Some(1)),
        ("receive", Segmentation::RECEIVE, Some(65)),
        ("both", Segmentation::BOTH, Some(65)),
    ];

    for (name, segmentation, expected_max_segments) in cases {
        let dev = DeviceObject::new(DeviceConfig {
            identity: crate::DeviceIdentity::default().with_segmentation(segmentation),
            ..DeviceConfig::default()
        })
        .unwrap();

        assert_eq!(
            dev.read_property(PropertyIdentifier::SEGMENTATION_SUPPORTED, None)
                .unwrap(),
            PropertyValue::Enumerated(segmentation.to_raw() as u32),
            "{name} segmentation readback"
        );

        let PropertyValue::List(property_list) = dev
            .read_property(PropertyIdentifier::PROPERTY_LIST, None)
            .unwrap()
        else {
            panic!("{name} Property_List was not a list");
        };
        assert_eq!(
            property_list.contains(&PropertyValue::Enumerated(
                PropertyIdentifier::MAX_SEGMENTS_ACCEPTED.to_raw(),
            )),
            expected_max_segments.is_some(),
            "{name} Property_List presence"
        );

        let max_segments = dev.read_property(PropertyIdentifier::MAX_SEGMENTS_ACCEPTED, None);
        match expected_max_segments {
            Some(expected) => assert_eq!(
                max_segments.unwrap(),
                PropertyValue::Unsigned(expected),
                "{name} Max_Segments_Accepted"
            ),
            None => assert!(matches!(
                max_segments,
                Err(Error::Remote(RemoteError::Error { class, code }))
                    if class == ErrorClass::PROPERTY
                        && code == ErrorCode::UNKNOWN_PROPERTY
            )),
        }
    }
}

#[test]
fn segment_timeout_absence_custom_value_and_zero_validation() {
    let none = DeviceObject::new(DeviceConfig {
        identity: crate::DeviceIdentity::default().with_apdu_segment_timeout_ms(0),
        ..Default::default()
    })
    .unwrap();
    assert!(
        matches!(none.read_property(PropertyIdentifier::APDU_SEGMENT_TIMEOUT, None),
        Err(Error::Remote(RemoteError::Error { code, .. })) if code == ErrorCode::UNKNOWN_PROPERTY)
    );
    assert!(
        !none
            .property_list()
            .contains(&PropertyIdentifier::APDU_SEGMENT_TIMEOUT)
    );
    for mode in [
        Segmentation::TRANSMIT,
        Segmentation::RECEIVE,
        Segmentation::BOTH,
    ] {
        assert!(
            DeviceObject::new(DeviceConfig {
                identity: crate::DeviceIdentity::default()
                    .with_segmentation(mode)
                    .with_apdu_segment_timeout_ms(0),
                ..Default::default()
            })
            .is_err()
        );
        let device = DeviceObject::new(DeviceConfig {
            identity: crate::DeviceIdentity::default()
                .with_segmentation(mode)
                .with_apdu_segment_timeout_ms(7300),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            device
                .read_property(PropertyIdentifier::APDU_SEGMENT_TIMEOUT, None)
                .unwrap(),
            PropertyValue::Unsigned(7300)
        );
        assert_eq!(
            device
                .read_property(PropertyIdentifier::APDU_TIMEOUT, None)
                .unwrap(),
            PropertyValue::Unsigned(6000)
        );
        assert_eq!(
            device
                .read_property(PropertyIdentifier::NUMBER_OF_APDU_RETRIES, None)
                .unwrap(),
            PropertyValue::Unsigned(3)
        );
        assert!(
            device
                .property_list()
                .contains(&PropertyIdentifier::APDU_SEGMENT_TIMEOUT)
        );
    }
}

#[test]
fn segment_timeout_is_present_for_segmented_devices() {
    for segmentation in [
        Segmentation::TRANSMIT,
        Segmentation::RECEIVE,
        Segmentation::BOTH,
    ] {
        let dev = DeviceObject::new(DeviceConfig {
            identity: crate::DeviceIdentity::default().with_segmentation(segmentation),
            ..DeviceConfig::default()
        })
        .unwrap();
        assert_eq!(
            dev.read_property(PropertyIdentifier::APDU_SEGMENT_TIMEOUT, None)
                .unwrap(),
            PropertyValue::Unsigned(5000)
        );
    }
}

#[test]
fn invalid_segmentation_is_rejected_at_construction() {
    assert!(
        DeviceObject::new(DeviceConfig {
            identity: crate::DeviceIdentity::default()
                .with_segmentation(Segmentation::from_raw(64)),
            ..DeviceConfig::default()
        })
        .is_err()
    );
}
