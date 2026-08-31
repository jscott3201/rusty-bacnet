//! WPM rollback coverage for object-owned reliability state.

use super::*;
use bacnet_objects::binary::BinaryInputObject;
use bacnet_objects::traits::ReliabilityEvaluation;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::enums::Reliability;
use bytes::BytesMut;

#[test]
fn wpm_out_of_service_rollback_preserves_range_fault_ownership() {
    let mut db = ObjectDatabase::new();
    let mut input = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    input.configure_fault_out_of_range(10.0, 20.0).unwrap();
    input.set_present_value(21.0);
    assert_eq!(
        input.evaluate_reliability_internal().unwrap(),
        ReliabilityEvaluation::Changed {
            old_reliability: Reliability::NO_FAULT_DETECTED.to_raw(),
            new_reliability: Reliability::OVER_RANGE.to_raw(),
        }
    );
    input.set_present_value(15.0);
    let oid = input.object_identifier();
    db.add(Box::new(input)).unwrap();

    let mut out_of_service = BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut out_of_service, true);
    let mut simulated_reliability = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(
        &mut simulated_reliability,
        Reliability::NO_SENSOR.to_raw(),
    );
    let mut read_only_value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut read_only_value, 0);
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OUT_OF_SERVICE,
                    property_array_index: None,
                    value: out_of_service.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::RELIABILITY,
                    property_array_index: None,
                    value: simulated_reliability.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_TYPE,
                    property_array_index: None,
                    value: read_only_value.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);
    assert!(handle_write_property_multiple(&mut db, &request_bytes).is_err());

    let object = db.get(&oid).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw())
    );
    assert_eq!(
        db.get_mut(&oid)
            .unwrap()
            .evaluate_reliability_internal()
            .unwrap(),
        ReliabilityEvaluation::Changed {
            old_reliability: Reliability::OVER_RANGE.to_raw(),
            new_reliability: Reliability::NO_FAULT_DETECTED.to_raw(),
        },
        "successful rollback must preserve the private owner needed for recovery"
    );
}

#[test]
fn wpm_rollback_restores_inhibit_oos_override_saved_reliability_and_range_owner() {
    let mut db = ObjectDatabase::new();
    let mut input = AnalogInputObject::new(2, "AI-inhibit", 62).unwrap();
    input.configure_fault_out_of_range(10.0, 20.0).unwrap();
    input.set_present_value(21.0);
    input.evaluate_reliability_internal().unwrap();
    input.set_present_value(15.0);
    input
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    input
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
            None,
        )
        .unwrap();
    let oid = input.object_identifier();
    db.add(Box::new(input)).unwrap();

    let mut inhibit = BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut inhibit, true);
    let mut changed_reliability = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(
        &mut changed_reliability,
        Reliability::OVER_RANGE.to_raw(),
    );
    let mut leave_oos = BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut leave_oos, false);
    let mut read_only_value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut read_only_value, 0);
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
                    property_array_index: None,
                    value: inhibit.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::RELIABILITY,
                    property_array_index: None,
                    value: changed_reliability.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OUT_OF_SERVICE,
                    property_array_index: None,
                    value: leave_oos.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_TYPE,
                    property_array_index: None,
                    value: read_only_value.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);
    assert!(handle_write_property_multiple(&mut db, &request_bytes).is_err());

    let object = db.get_mut(&oid).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .unwrap(),
        PropertyValue::Boolean(true)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw())
    );

    object
        .write_property(
            PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
        "rollback must restore the accepted-client-write marker"
    );
    object
        .write_property(
            PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw()),
        "rollback must restore the evaluated Reliability saved on OOS entry"
    );
    assert_eq!(
        object.evaluate_reliability_internal().unwrap(),
        ReliabilityEvaluation::Changed {
            old_reliability: Reliability::OVER_RANGE.to_raw(),
            new_reliability: Reliability::NO_FAULT_DETECTED.to_raw(),
        },
        "rollback must restore analog range-fault ownership"
    );
}

#[test]
fn wpm_rollback_restores_non_range_inhibit_reliability_and_future_oos_state() {
    let mut db = ObjectDatabase::new();
    let mut input = BinaryInputObject::new(3, "BI-inhibit").unwrap();
    input
        .set_reliability_internal(Reliability::NO_SENSOR.to_raw())
        .unwrap();
    let oid = input.object_identifier();
    db.add(Box::new(input)).unwrap();

    let mut inhibit = BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut inhibit, true);
    let mut read_only_value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut read_only_value, 0);
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
                    property_array_index: None,
                    value: inhibit.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_TYPE,
                    property_array_index: None,
                    value: read_only_value.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);
    assert!(handle_write_property_multiple(&mut db, &request_bytes).is_err());

    let object = db.get_mut(&oid).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
        "rollback must restore Reliability that TRUE normalized"
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );

    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    object
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
            None,
        )
        .unwrap();
    object
        .write_property(
            PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
        "an accepted same-value OOS write must establish override ownership"
    );
    object
        .write_property(
            PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
        "the OOS exit must restore the Reliability saved on entry"
    );

    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    object
        .write_property(
            PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
        "a new OOS period must not retain prior client ownership"
    );
    object
        .write_property(
            PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
        "the next OOS cycle must independently save and restore Reliability"
    );
}
