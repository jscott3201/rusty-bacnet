use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event_enrollment::{AlertEnrollmentObject, EventEnrollmentObject};
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::{PicsConfig, PicsGenerator};
use crate::server::ServerConfig;

#[test]
fn acked_transitions_network_policy_is_uniform_on_all_supported_types() {
    let alert_source = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();
    let cases: Vec<(&str, ObjectType, Box<dyn BACnetObject>)> = vec![
        (
            "Analog Input",
            ObjectType::ANALOG_INPUT,
            Box::new(AnalogInputObject::new(1, "AI-1", 95).unwrap()),
        ),
        (
            "Analog Output",
            ObjectType::ANALOG_OUTPUT,
            Box::new(AnalogOutputObject::new(1, "AO-1", 95).unwrap()),
        ),
        (
            "Analog Value",
            ObjectType::ANALOG_VALUE,
            Box::new(AnalogValueObject::new(1, "AV-1", 95).unwrap()),
        ),
        (
            "Binary Input",
            ObjectType::BINARY_INPUT,
            Box::new(BinaryInputObject::new(1, "BI-1").unwrap()),
        ),
        (
            "Binary Output",
            ObjectType::BINARY_OUTPUT,
            Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()),
        ),
        (
            "Binary Value",
            ObjectType::BINARY_VALUE,
            Box::new(BinaryValueObject::new(1, "BV-1").unwrap()),
        ),
        (
            "Multi-state Input",
            ObjectType::MULTI_STATE_INPUT,
            Box::new(MultiStateInputObject::new(1, "MSI-1", 2).unwrap()),
        ),
        (
            "Multi-state Output",
            ObjectType::MULTI_STATE_OUTPUT,
            Box::new(MultiStateOutputObject::new(1, "MSO-1", 2).unwrap()),
        ),
        (
            "Multi-state Value",
            ObjectType::MULTI_STATE_VALUE,
            Box::new(MultiStateValueObject::new(1, "MSV-1", 2).unwrap()),
        ),
        (
            "Event Enrollment",
            ObjectType::EVENT_ENROLLMENT,
            Box::new(EventEnrollmentObject::new(1, "EE-1", 0).unwrap()),
        ),
        (
            "Alert Enrollment",
            ObjectType::ALERT_ENROLLMENT,
            Box::new(AlertEnrollmentObject::new(1, "AE-1", alert_source).unwrap()),
        ),
    ];

    let mut database = ObjectDatabase::new();
    let mut object_types = Vec::with_capacity(cases.len());
    for (label, object_type, mut object) in cases {
        let property_list = object
            .read_property(PropertyIdentifier::PROPERTY_LIST, None)
            .unwrap_or_else(|error| panic!("{label}: Property_List read failed: {error:?}"));
        assert!(
            matches!(
                property_list,
                PropertyValue::List(ref properties)
                    if properties.contains(&PropertyValue::Enumerated(
                        PropertyIdentifier::ACKED_TRANSITIONS.to_raw()
                    ))
            ),
            "{label}: Property_List must expose Acked_Transitions"
        );

        let before = object
            .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap_or_else(|error| panic!("{label}: Acked_Transitions read failed: {error:?}"));
        assert!(
            matches!(
                &before,
                PropertyValue::BitString { unused_bits: 5, data } if data.len() == 1
            ),
            "{label}: Acked_Transitions must be a three-bit BitString, got {before:?}"
        );
        assert!(
            !object.is_writable_property(PropertyIdentifier::ACKED_TRANSITIONS),
            "{label}: Acked_Transitions must not be advertised writable"
        );

        let error = object
            .write_property(
                PropertyIdentifier::ACKED_TRANSITIONS,
                None,
                PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![0],
                },
                None,
            )
            .expect_err("a network Acked_Transitions write must be denied");
        assert!(
            matches!(
                error,
                Error::Protocol { class, code }
                    if class == ErrorClass::PROPERTY.to_raw() as u32
                        && code == ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32
            ),
            "{label}: expected PROPERTY / WRITE_ACCESS_DENIED, got {error:?}"
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
                .unwrap(),
            before,
            "{label}: denied write must not change Acked_Transitions"
        );

        object_types.push((label, object_type));
        database.add(object).unwrap();
    }

    let pics =
        PicsGenerator::new(&database, &ServerConfig::default(), &PicsConfig::default()).generate();
    for (label, object_type) in object_types {
        let property =
            pics.supported_object_types
                .iter()
                .find(|support| support.object_type == object_type)
                .and_then(|support| {
                    support.supported_properties.iter().find(|property| {
                        property.property_id == PropertyIdentifier::ACKED_TRANSITIONS
                    })
                })
                .unwrap_or_else(|| panic!("{label}: PICS must include Acked_Transitions"));
        assert!(property.access.readable, "{label}: PICS must mark readable");
        assert!(
            !property.access.writable,
            "{label}: PICS must not mark writable"
        );
    }
}
