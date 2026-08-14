//! Lossless WPM rollback for event-state side effects and fallback-backed values.

use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::event_enrollment::{
    AlertEnrollmentObject, EventEnrollmentEvalState, EventEnrollmentObject, EventEnrollmentPending,
};
use bacnet_objects::file::FileObject;
use bacnet_objects::traits::WritePropertyRollback;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::constructed::BACnetEventParameter;
use std::borrow::Cow;

struct FailingRollbackObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
}

impl FailingRollbackObject {
    fn new() -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::BINARY_VALUE, 99).unwrap(),
            name: "failing-rollback".into(),
            description: "before".into(),
        }
    }
}

impl BACnetObject for FailingRollbackObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            p if p == PropertyIdentifier::OBJECT_IDENTIFIER => {
                Ok(PropertyValue::ObjectIdentifier(self.oid))
            }
            p if p == PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            }
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::BINARY_VALUE.to_raw()))
            }
            p if p == PropertyIdentifier::DESCRIPTION => {
                Ok(PropertyValue::CharacterString(self.description.clone()))
            }
            _ => Err(Error::Encoding("test property is not readable".into())),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::DESCRIPTION {
            if let PropertyValue::CharacterString(value) = value {
                self.description = value;
                return Ok(());
            }
        }
        Err(Error::Encoding("test property is not writable".into()))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::DESCRIPTION,
        ])
    }

    fn capture_write_property_rollback(
        &mut self,
        property: PropertyIdentifier,
        _value: &PropertyValue,
    ) -> Option<WritePropertyRollback> {
        (property == PropertyIdentifier::DESCRIPTION).then(|| WritePropertyRollback::new(()))
    }

    fn restore_write_property_rollback(
        &mut self,
        _rollback: WritePropertyRollback,
    ) -> Result<(), Error> {
        Err(Error::Encoding("injected rollback failure".into()))
    }
}

fn failed_wpm(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: Vec<u8>,
) {
    let mut read_only_value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut read_only_value, 0);
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: property,
                    property_array_index: None,
                    value,
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
    assert!(handle_write_property_multiple(db, &request_bytes).is_err());
}

fn out_of_range_params(time_delay: u32) -> BACnetEventParameter {
    BACnetEventParameter::OutOfRange {
        time_delay,
        low_limit: 10.0,
        high_limit: 90.0,
        deadband: 1.0,
    }
}

#[test]
fn wpm_rollback_restores_event_enrollment_detection_state() {
    let mut db = ObjectDatabase::new();
    let mut enrollment = EventEnrollmentObject::new(1, "EE-1", 5).unwrap();
    enrollment.set_event_state(EventState::HIGH_LIMIT.to_raw());
    enrollment
        .set_acked_transitions_internal(0x01, false)
        .unwrap();
    let evaluation = EventEnrollmentEvalState {
        pending: Some(EventEnrollmentPending {
            state: EventState::NORMAL,
            remaining: 4,
            condition: 7,
            params_fingerprint: 11,
        }),
        cov_baseline: Some(PropertyValue::Real(42.5)),
        last_offnormal_value: Some(3),
    };
    enrollment
        .set_enrollment_eval_state_internal(evaluation.clone())
        .unwrap();
    let oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let mut disabled = BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut disabled, false);
    failed_wpm(
        &mut db,
        oid,
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        disabled.to_vec(),
    );

    let object = db.get(&oid).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
            .unwrap(),
        PropertyValue::Boolean(true)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b0110_0000],
        }
    );
    assert_eq!(object.enrollment_eval_state_internal(), Some(evaluation));
}

#[test]
fn wpm_rollback_restores_alert_enrollment_detection_state() {
    let mut db = ObjectDatabase::new();
    let mut enrollment = AlertEnrollmentObject::new(1, "AE-1").unwrap();
    enrollment
        .set_event_state_internal(EventState::OFFNORMAL)
        .unwrap();
    enrollment
        .set_acked_transitions_internal(0x01, false)
        .unwrap();
    let oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let mut disabled = BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut disabled, false);
    failed_wpm(
        &mut db,
        oid,
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        disabled.to_vec(),
    );

    let object = db.get(&oid).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
            .unwrap(),
        PropertyValue::Boolean(true)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw())
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b0110_0000],
        }
    );
}

#[test]
fn duplicate_name_failure_rolls_back_prior_event_state_write() {
    let mut db = ObjectDatabase::new();
    let mut enrollment = AlertEnrollmentObject::new(1, "AE-1").unwrap();
    enrollment
        .set_event_state_internal(EventState::OFFNORMAL)
        .unwrap();
    let enrollment_oid = enrollment.object_identifier();
    let renamed = BinaryValueObject::new(1, "BV-1").unwrap();
    let renamed_oid = renamed.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    db.add(Box::new(renamed)).unwrap();
    db.add(Box::new(BinaryValueObject::new(2, "taken").unwrap()))
        .unwrap();

    let mut disabled = BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut disabled, false);
    let mut duplicate_name = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut duplicate_name, "taken").unwrap();
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![
            WriteAccessSpecification {
                object_identifier: enrollment_oid,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::EVENT_DETECTION_ENABLE,
                    property_array_index: None,
                    value: disabled.to_vec(),
                    priority: None,
                }],
            },
            WriteAccessSpecification {
                object_identifier: renamed_oid,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                    value: duplicate_name.to_vec(),
                    priority: None,
                }],
            },
        ],
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);

    assert!(handle_write_property_multiple(&mut db, &request_bytes).is_err());
    let object = db.get(&enrollment_oid).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
            .unwrap(),
        PropertyValue::Boolean(true)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw())
    );
}

#[test]
fn wpm_reports_object_state_rollback_failure() {
    let mut db = ObjectDatabase::new();
    let object = FailingRollbackObject::new();
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();

    let mut changed = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut changed, "changed").unwrap();
    let mut read_only_value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut read_only_value, 0);
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::DESCRIPTION,
                    property_array_index: None,
                    value: changed.to_vec(),
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

    let (result, residual_oids) =
        handle_write_property_multiple_with_residuals(&mut db, &request_bytes);
    let error = result.unwrap_err();
    assert!(error.to_string().contains("rollback failed"));
    assert_eq!(residual_oids, vec![oid]);
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("before".into()),
        "the readable property snapshot is independent of the failed private-state token"
    );
}

#[test]
fn rollback_failure_reports_only_the_object_that_failed_restoration() {
    let mut db = ObjectDatabase::new();
    let mut enrollment = AlertEnrollmentObject::new(1, "AE-1").unwrap();
    enrollment
        .set_event_state_internal(EventState::OFFNORMAL)
        .unwrap();
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    let failing = FailingRollbackObject::new();
    let failing_oid = failing.object_identifier();
    db.add(Box::new(failing)).unwrap();

    let mut disabled = BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut disabled, false);
    let mut changed = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut changed, "changed").unwrap();
    let mut read_only = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut read_only, 0);
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![
            WriteAccessSpecification {
                object_identifier: enrollment_oid,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::EVENT_DETECTION_ENABLE,
                    property_array_index: None,
                    value: disabled.to_vec(),
                    priority: None,
                }],
            },
            WriteAccessSpecification {
                object_identifier: failing_oid,
                list_of_properties: vec![
                    BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::DESCRIPTION,
                        property_array_index: None,
                        value: changed.to_vec(),
                        priority: None,
                    },
                    BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::OBJECT_TYPE,
                        property_array_index: None,
                        value: read_only.to_vec(),
                        priority: None,
                    },
                ],
            },
        ],
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);

    let (result, residual_oids) =
        handle_write_property_multiple_with_residuals(&mut db, &request_bytes);

    assert!(result.unwrap_err().to_string().contains("rollback failed"));
    assert_eq!(residual_oids, vec![failing_oid]);
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw()),
        "a successfully restored object must not be re-evaluated as residual"
    );
}

#[test]
fn wpm_entering_out_of_service_rolls_back_without_masking_write_error() {
    let mut db = ObjectDatabase::new();
    let mut input = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    input
        .set_reliability_internal(bacnet_types::enums::Reliability::OVER_RANGE.to_raw())
        .unwrap();
    let oid = input.object_identifier();
    db.add(Box::new(input)).unwrap();

    let mut out_of_service = BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut out_of_service, true);
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

    let error = handle_write_property_multiple(&mut db, &request_bytes).unwrap_err();
    assert!(matches!(
        error,
        Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32
    ));
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
        PropertyValue::Enumerated(bacnet_types::enums::Reliability::OVER_RANGE.to_raw())
    );
}

#[test]
fn wpm_leaving_plain_out_of_service_object_ignores_redundant_reliability_replay() {
    let mut db = ObjectDatabase::new();
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    let oid = file.object_identifier();
    db.add(Box::new(file)).unwrap();

    let mut in_service = BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut in_service, false);
    let mut read_only_value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut read_only_value, 0);
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OUT_OF_SERVICE,
                    property_array_index: None,
                    value: in_service.to_vec(),
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

    let error = handle_write_property_multiple(&mut db, &request_bytes).unwrap_err();
    assert!(matches!(
        error,
        Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32
    ));
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .unwrap(),
        PropertyValue::Boolean(true)
    );
}

#[test]
fn wpm_rollback_restores_event_enrollment_time_delay_normal_fallback() {
    let mut db = ObjectDatabase::new();
    let mut enrollment = EventEnrollmentObject::new(1, "EE-1", 5).unwrap();
    enrollment.set_event_parameters(out_of_range_params(3));
    let oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let mut configured = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut configured, 8);
    failed_wpm(
        &mut db,
        oid,
        PropertyIdentifier::TIME_DELAY_NORMAL,
        configured.to_vec(),
    );

    let mut framed = BytesMut::new();
    bacnet_encoding::constructed::encode_event_parameter(&mut framed, &out_of_range_params(9));
    db.get_mut(&oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::EVENT_PARAMETERS,
            None,
            PropertyValue::ApplicationData(framed.to_vec()),
            None,
        )
        .unwrap();
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None)
            .unwrap(),
        PropertyValue::Unsigned(9),
        "rollback must restore the unconfigured fallback, not store the old effective value"
    );
}

#[test]
fn wpm_rollback_restores_intrinsic_pending_transition() {
    let mut db = ObjectDatabase::new();
    let mut input = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    input
        .write_property(
            PropertyIdentifier::HIGH_LIMIT,
            None,
            PropertyValue::Real(50.0),
            None,
        )
        .unwrap();
    input
        .write_property(
            PropertyIdentifier::LIMIT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![0b0100_0000],
            },
            None,
        )
        .unwrap();
    input
        .write_property(
            PropertyIdentifier::TIME_DELAY,
            None,
            PropertyValue::Unsigned(2),
            None,
        )
        .unwrap();
    input.set_present_value(75.0);
    assert_eq!(input.evaluate_intrinsic_reporting(), None);
    let oid = input.object_identifier();
    db.add(Box::new(input)).unwrap();

    let mut disabled = BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut disabled, false);
    failed_wpm(
        &mut db,
        oid,
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        disabled.to_vec(),
    );

    let object = db.get_mut(&oid).unwrap();
    assert_eq!(object.tick_intrinsic_reporting(), None);
    let transition = object
        .tick_intrinsic_reporting()
        .expect("the pre-WPM countdown must fire on its original second tick");
    assert_eq!(transition.change.to, EventState::HIGH_LIMIT);
}

#[test]
fn wpm_rollback_restores_intrinsic_time_delay_normal_fallback() {
    let mut db = ObjectDatabase::new();
    let mut input = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    input
        .write_property(
            PropertyIdentifier::TIME_DELAY,
            None,
            PropertyValue::Unsigned(3),
            None,
        )
        .unwrap();
    let oid = input.object_identifier();
    db.add(Box::new(input)).unwrap();

    let mut configured = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut configured, 8);
    failed_wpm(
        &mut db,
        oid,
        PropertyIdentifier::TIME_DELAY_NORMAL,
        configured.to_vec(),
    );

    db.get_mut(&oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::TIME_DELAY,
            None,
            PropertyValue::Unsigned(9),
            None,
        )
        .unwrap();
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None)
            .unwrap(),
        PropertyValue::Unsigned(9)
    );
}
