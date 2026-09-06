use std::borrow::Cow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bacnet_objects::life_safety::{
    LifeSafetyPointObject, LifeSafetyPointResetCommit, LifeSafetyPointResetContext,
    LifeSafetyPointResetExecutor, LifeSafetyResetError, LifeSafetyZoneObject,
    LifeSafetyZoneResetCommit, LifeSafetyZoneResetContext, LifeSafetyZoneResetExecutor,
};
use bacnet_objects::traits::LifeSafetyOperationEffect;
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, LifeSafetyOperation, LifeSafetyState, ObjectType, PropertyIdentifier,
    SilencedState,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::*;

fn request(
    operation: LifeSafetyOperation,
    object_identifier: Option<ObjectIdentifier>,
) -> LifeSafetyOperationRequest {
    LifeSafetyOperationRequest {
        requesting_process_identifier: 7,
        requesting_source: "operator".into(),
        request: operation,
        object_identifier,
    }
}

fn read(db: &ObjectDatabase, oid: ObjectIdentifier, property: PropertyIdentifier) -> u32 {
    match db.get(&oid).unwrap().read_property(property, None).unwrap() {
        PropertyValue::Enumerated(value) => value,
        other => panic!("expected enumerated value, got {other:?}"),
    }
}

fn assert_protocol_error(error: Error, class: ErrorClass, code: ErrorCode) {
    assert!(matches!(
        error,
        Error::Protocol {
            class: actual_class,
            code: actual_code,
        } if actual_class == class.to_raw() as u32 && actual_code == code.to_raw() as u32
    ));
}

#[test]
fn targeted_reset_variants_apply_to_point_and_zone() {
    for operation in [
        LifeSafetyOperation::RESET,
        LifeSafetyOperation::RESET_ALARM,
        LifeSafetyOperation::RESET_FAULT,
    ] {
        let point_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
        let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
        point.set_present_value(LifeSafetyState::ALARM.to_raw());
        point.set_tracking_value(LifeSafetyState::FAULT.to_raw());
        point.set_silenced(SilencedState::ALL_SILENCED);
        point.set_operation_expected(operation);
        point.set_reset_executor(Arc::new(move |context| {
            assert_eq!(
                *context,
                LifeSafetyPointResetContext {
                    object_identifier: point_oid,
                    operation,
                    present_value: LifeSafetyState::ALARM,
                    tracking_value: LifeSafetyState::FAULT,
                    silenced: SilencedState::ALL_SILENCED,
                    operation_expected: operation,
                }
            );
            Ok(LifeSafetyPointResetCommit {
                present_value: Some(LifeSafetyState::QUIET),
                tracking_value: None,
                silenced: Some(SilencedState::UNSILENCED),
            })
        }));

        let zone_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 1).unwrap();
        let mut zone = LifeSafetyZoneObject::new(1, "zone").unwrap();
        zone.set_present_value(LifeSafetyState::FAULT_ALARM.to_raw());
        zone.set_silenced(SilencedState::VISIBLE_SILENCED);
        zone.set_operation_expected(operation);
        zone.set_reset_executor(Arc::new(move |context| {
            assert_eq!(
                *context,
                LifeSafetyZoneResetContext {
                    object_identifier: zone_oid,
                    operation,
                    present_value: LifeSafetyState::FAULT_ALARM,
                    silenced: SilencedState::VISIBLE_SILENCED,
                    operation_expected: operation,
                }
            );
            Ok(LifeSafetyZoneResetCommit {
                present_value: Some(LifeSafetyState::SUPERVISORY),
                silenced: None,
            })
        }));

        let mut db = ObjectDatabase::new();
        db.add(Box::new(point)).unwrap();
        db.add(Box::new(zone)).unwrap();
        assert_eq!(
            handle_life_safety_operation(&mut db, &request(operation, Some(point_oid))).unwrap(),
            vec![point_oid]
        );
        assert_eq!(
            handle_life_safety_operation(&mut db, &request(operation, Some(zone_oid))).unwrap(),
            vec![zone_oid]
        );
        assert_eq!(
            read(&db, point_oid, PropertyIdentifier::PRESENT_VALUE),
            LifeSafetyState::QUIET.to_raw()
        );
        assert_eq!(
            read(&db, point_oid, PropertyIdentifier::TRACKING_VALUE),
            LifeSafetyState::FAULT.to_raw(),
            "an omitted update remains unchanged"
        );
        assert_eq!(
            read(&db, zone_oid, PropertyIdentifier::SILENCED),
            SilencedState::VISIBLE_SILENCED.to_raw(),
            "reset never implicitly unsilences"
        );
        for oid in [point_oid, zone_oid] {
            assert_eq!(
                read(&db, oid, PropertyIdentifier::OPERATION_EXPECTED),
                LifeSafetyOperation::NONE.to_raw()
            );
        }
    }
}

struct ResetHookSpy {
    oid: ObjectIdentifier,
    calls: Arc<AtomicUsize>,
}

impl BACnetObject for ResetHookSpy {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "reset-spy"
    }

    fn read_property(
        &self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Err(Error::Encoding("unused reset-spy read".into()))
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Encoding("unused reset-spy write".into()))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }

    fn apply_life_safety_operation(
        &mut self,
        _operation: LifeSafetyOperation,
    ) -> Result<LifeSafetyOperationEffect, Error> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Ok(LifeSafetyOperationEffect::Applied)
    }
}

#[test]
fn targeted_reset_never_calls_non_life_safety_hook() {
    let calls = Arc::new(AtomicUsize::new(0));
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 9).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(ResetHookSpy {
        oid,
        calls: Arc::clone(&calls),
    }))
    .unwrap();

    let error =
        handle_life_safety_operation(&mut db, &request(LifeSafetyOperation::RESET, Some(oid)))
            .unwrap_err();

    assert_protocol_error(
        error,
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
    assert_eq!(calls.load(Ordering::Acquire), 0);
}

#[test]
fn targetless_reset_attempts_only_ordered_point_and_zone_candidates() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let make_point_executor = |oid: ObjectIdentifier,
                               result: Result<LifeSafetyPointResetCommit, LifeSafetyResetError>|
     -> LifeSafetyPointResetExecutor {
        let order = Arc::clone(&order);
        Arc::new(move |_: &LifeSafetyPointResetContext| {
            order.lock().unwrap().push(oid);
            result
        })
    };
    let make_zone_executor = |oid: ObjectIdentifier,
                              result: Result<LifeSafetyZoneResetCommit, LifeSafetyResetError>|
     -> LifeSafetyZoneResetExecutor {
        let order = Arc::clone(&order);
        Arc::new(move |_: &LifeSafetyZoneResetContext| {
            order.lock().unwrap().push(oid);
            result
        })
    };

    let point_success_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
    let mut point_success = LifeSafetyPointObject::new(1, "point-success").unwrap();
    point_success.set_present_value(LifeSafetyState::ALARM.to_raw());
    point_success.set_operation_expected(LifeSafetyOperation::RESET);
    point_success.set_reset_executor(make_point_executor(
        point_success_oid,
        Ok(LifeSafetyPointResetCommit {
            present_value: Some(LifeSafetyState::QUIET),
            ..Default::default()
        }),
    ));

    let point_no_executor_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 2).unwrap();
    let mut point_no_executor = LifeSafetyPointObject::new(2, "point-no-executor").unwrap();
    point_no_executor.set_operation_expected(LifeSafetyOperation::RESET);

    let point_rejected_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 3).unwrap();
    let mut point_rejected = LifeSafetyPointObject::new(3, "point-rejected").unwrap();
    point_rejected.set_operation_expected(LifeSafetyOperation::RESET);
    point_rejected.set_reset_executor(make_point_executor(
        point_rejected_oid,
        Err(LifeSafetyResetError::InvalidOperationInThisState),
    ));

    let point_panics_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 4).unwrap();
    let panic_order = Arc::clone(&order);
    let mut point_panics = LifeSafetyPointObject::new(4, "point-panics").unwrap();
    point_panics.set_operation_expected(LifeSafetyOperation::RESET);
    point_panics.set_reset_executor(Arc::new(move |_| {
        panic_order.lock().unwrap().push(point_panics_oid);
        panic!("executor bug")
    }));

    let zone_success_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 1).unwrap();
    let mut zone_success = LifeSafetyZoneObject::new(1, "zone-success").unwrap();
    zone_success.set_present_value(LifeSafetyState::FAULT.to_raw());
    zone_success.set_operation_expected(LifeSafetyOperation::RESET);
    zone_success.set_reset_executor(make_zone_executor(
        zone_success_oid,
        Ok(LifeSafetyZoneResetCommit {
            present_value: Some(LifeSafetyState::ACTIVE),
            ..Default::default()
        }),
    ));

    let zone_wrong_expected_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 2).unwrap();
    let mut zone_wrong_expected = LifeSafetyZoneObject::new(2, "zone-wrong-expected").unwrap();
    zone_wrong_expected.set_operation_expected(LifeSafetyOperation::RESET_ALARM);
    zone_wrong_expected.set_reset_executor(make_zone_executor(
        zone_wrong_expected_oid,
        Ok(LifeSafetyZoneResetCommit::default()),
    ));

    let zone_denied_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 3).unwrap();
    let mut zone_denied = LifeSafetyZoneObject::new(3, "zone-denied").unwrap();
    zone_denied.set_operation_expected(LifeSafetyOperation::RESET);
    zone_denied.set_reset_executor(make_zone_executor(
        zone_denied_oid,
        Err(LifeSafetyResetError::ServiceRequestDenied),
    ));

    let non_life_calls = Arc::new(AtomicUsize::new(0));
    let non_life_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let mut db = ObjectDatabase::new();
    for object in [
        Box::new(point_success) as Box<dyn BACnetObject>,
        Box::new(point_no_executor),
        Box::new(point_rejected),
        Box::new(point_panics),
        Box::new(zone_success),
        Box::new(zone_wrong_expected),
        Box::new(zone_denied),
        Box::new(ResetHookSpy {
            oid: non_life_oid,
            calls: Arc::clone(&non_life_calls),
        }),
    ] {
        db.add(object).unwrap();
    }

    let changed =
        handle_life_safety_operation(&mut db, &request(LifeSafetyOperation::RESET, None)).unwrap();

    assert_eq!(changed, vec![point_success_oid, zone_success_oid]);
    assert_eq!(
        *order.lock().unwrap(),
        vec![
            point_success_oid,
            point_rejected_oid,
            point_panics_oid,
            zone_success_oid,
            zone_denied_oid,
        ]
    );
    assert_eq!(non_life_calls.load(Ordering::Acquire), 0);
    assert_eq!(
        read(&db, point_success_oid, PropertyIdentifier::PRESENT_VALUE),
        LifeSafetyState::QUIET.to_raw()
    );
    assert_eq!(
        read(&db, zone_success_oid, PropertyIdentifier::PRESENT_VALUE),
        LifeSafetyState::ACTIVE.to_raw()
    );
    for oid in [
        point_no_executor_oid,
        point_rejected_oid,
        point_panics_oid,
        zone_denied_oid,
    ] {
        assert_eq!(
            read(&db, oid, PropertyIdentifier::OPERATION_EXPECTED),
            LifeSafetyOperation::RESET.to_raw()
        );
    }
    assert_eq!(
        read(
            &db,
            zone_wrong_expected_oid,
            PropertyIdentifier::OPERATION_EXPECTED,
        ),
        LifeSafetyOperation::RESET_ALARM.to_raw()
    );
}
