//! The application's Present_Value and Tracking_Value route through the
//! boxed object (#1123): `set_present_value_internal` and
//! `set_tracking_value_internal` on a Point or Zone held as
//! `Box<dyn BACnetObject>`.

use std::sync::{Arc, Mutex};

use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, LifeSafetyOperation, LifeSafetyState,
    PropertyIdentifier as P, SilencedState,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::traits::BACnetObject;

use super::*;

fn boxed_point() -> Box<dyn BACnetObject> {
    Box::new(LifeSafetyPointObject::new(1, "LSP-1").unwrap())
}

fn boxed_zone() -> Box<dyn BACnetObject> {
    Box::new(LifeSafetyZoneObject::new(1, "LSZ-1").unwrap())
}

fn read(object: &dyn BACnetObject, property: P) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn state(state: LifeSafetyState) -> PropertyValue {
    PropertyValue::Enumerated(state.to_raw())
}

fn set_out_of_service(object: &mut dyn BACnetObject, out_of_service: bool) {
    object
        .write_property(
            P::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(out_of_service),
            None,
        )
        .unwrap();
}

fn assert_error(result: Result<(), Error>, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class: c, code: e })
            if c == class && e == code),
        "expected {class:?} / {code:?}, got {result:?}"
    );
}

/// The properties a Present_Value or Tracking_Value update must not move.
fn untouched(object: &dyn BACnetObject) -> [PropertyValue; 3] {
    [
        read(object, P::SILENCED),
        read(object, P::OPERATION_EXPECTED),
        read(object, P::STATUS_FLAGS),
    ]
}

fn assert_values_take_effect(object: &mut dyn BACnetObject) {
    object
        .set_life_safety_operation_expected_internal(LifeSafetyOperation::RESET)
        .unwrap();
    let before = untouched(object);

    object
        .set_present_value_internal(state(LifeSafetyState::ALARM))
        .unwrap();
    assert_eq!(
        read(object, P::PRESENT_VALUE),
        state(LifeSafetyState::ALARM)
    );
    assert_eq!(
        read(object, P::TRACKING_VALUE),
        state(LifeSafetyState::QUIET),
        "Present_Value doesn't move Tracking_Value"
    );

    // A latching application keeps Present_Value on ALARM while the live
    // state returns to QUIET through Tracking_Value.
    object
        .set_tracking_value_internal(state(LifeSafetyState::PRE_ALARM))
        .unwrap();
    assert_eq!(
        read(object, P::TRACKING_VALUE),
        state(LifeSafetyState::PRE_ALARM)
    );
    object
        .set_tracking_value_internal(state(LifeSafetyState::QUIET))
        .unwrap();
    assert_eq!(
        read(object, P::PRESENT_VALUE),
        state(LifeSafetyState::ALARM)
    );
    assert_eq!(untouched(object), before, "Silenced, Operation_Expected");

    // The proprietary range reads back verbatim.
    for raw in [256, 65_535] {
        object
            .set_present_value_internal(PropertyValue::Enumerated(raw))
            .unwrap();
        object
            .set_tracking_value_internal(PropertyValue::Enumerated(raw))
            .unwrap();
        assert_eq!(
            read(object, P::PRESENT_VALUE),
            PropertyValue::Enumerated(raw)
        );
        assert_eq!(
            read(object, P::TRACKING_VALUE),
            PropertyValue::Enumerated(raw)
        );
    }
}

#[test]
fn point_and_zone_take_application_present_and_tracking_values_through_the_box() {
    assert_values_take_effect(boxed_point().as_mut());
    assert_values_take_effect(boxed_zone().as_mut());
}

fn assert_bad_values_are_refused(object: &mut dyn BACnetObject) {
    object
        .set_present_value_internal(state(LifeSafetyState::FAULT))
        .unwrap();
    object
        .set_tracking_value_internal(state(LifeSafetyState::FAULT))
        .unwrap();
    // 35..=255 are reserved for ASHRAE; past 65535 is outside the datatype.
    let refused = [
        (PropertyValue::Enumerated(35), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            PropertyValue::Enumerated(255),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            PropertyValue::Enumerated(65_536),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (PropertyValue::Unsigned(2), ErrorCode::INVALID_DATA_TYPE),
        (PropertyValue::Null, ErrorCode::INVALID_DATA_TYPE),
    ];
    for (value, code) in refused {
        assert_error(
            object.set_present_value_internal(value.clone()),
            ErrorClass::PROPERTY,
            code,
        );
        assert_error(
            object.set_tracking_value_internal(value),
            ErrorClass::PROPERTY,
            code,
        );
    }
    assert_eq!(
        read(object, P::PRESENT_VALUE),
        state(LifeSafetyState::FAULT)
    );
    assert_eq!(
        read(object, P::TRACKING_VALUE),
        state(LifeSafetyState::FAULT)
    );
}

#[test]
fn application_values_outside_the_state_range_or_datatype_are_refused_unchanged() {
    assert_bad_values_are_refused(boxed_point().as_mut());
    assert_bad_values_are_refused(boxed_zone().as_mut());
}

#[test]
fn other_objects_refuse_the_tracking_value_hook() {
    let mut input: Box<dyn BACnetObject> = Box::new(
        crate::analog::AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS)
            .unwrap(),
    );
    assert_error(
        input.set_tracking_value_internal(state(LifeSafetyState::ALARM)),
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
}

fn assert_out_of_service_rule(object: &mut dyn BACnetObject) {
    set_out_of_service(object, true);
    // The application's Tracking_Value is set aside while out of service.
    object
        .set_tracking_value_internal(state(LifeSafetyState::ALARM))
        .unwrap();
    assert_eq!(
        read(object, P::TRACKING_VALUE),
        state(LifeSafetyState::QUIET)
    );
    // A client simulation keeps being served over a later application value.
    object
        .write_property(
            P::TRACKING_VALUE,
            None,
            state(LifeSafetyState::TAMPER),
            None,
        )
        .unwrap();
    object
        .set_tracking_value_internal(state(LifeSafetyState::PRE_ALARM))
        .unwrap();
    assert_eq!(
        read(object, P::TRACKING_VALUE),
        state(LifeSafetyState::TAMPER)
    );
    // Present_Value isn't decoupled, so it is served at once.
    object
        .set_present_value_internal(state(LifeSafetyState::ALARM))
        .unwrap();
    assert_eq!(
        read(object, P::PRESENT_VALUE),
        state(LifeSafetyState::ALARM)
    );

    set_out_of_service(object, false);
    assert_eq!(
        read(object, P::TRACKING_VALUE),
        state(LifeSafetyState::PRE_ALARM),
        "the latest application value takes over on the return to service"
    );
    assert_eq!(
        read(object, P::PRESENT_VALUE),
        state(LifeSafetyState::ALARM)
    );
}

#[test]
fn out_of_service_sets_the_application_tracking_value_aside_but_serves_present_value() {
    assert_out_of_service_rule(boxed_point().as_mut());
    assert_out_of_service_rule(boxed_zone().as_mut());
}

#[test]
fn reset_executor_sees_the_values_the_route_left_and_commits_after_them() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let point_seen = Arc::clone(&seen);
    let point = LifeSafetyPointObject::new(1, "LSP-1")
        .unwrap()
        .with_reset_executor(Arc::new(move |context| {
            point_seen
                .lock()
                .unwrap()
                .push((context.present_value, context.tracking_value));
            Ok(LifeSafetyPointResetCommit {
                present_value: Some(LifeSafetyState::QUIET),
                ..LifeSafetyPointResetCommit::default()
            })
        }));
    let zone_seen = Arc::clone(&seen);
    let zone = LifeSafetyZoneObject::new(1, "LSZ-1")
        .unwrap()
        .with_reset_executor(Arc::new(move |context| {
            zone_seen
                .lock()
                .unwrap()
                .push((context.present_value, context.tracking_value));
            Ok(LifeSafetyZoneResetCommit {
                present_value: Some(LifeSafetyState::QUIET),
                ..LifeSafetyZoneResetCommit::default()
            })
        }));
    for mut object in [
        Box::new(point) as Box<dyn BACnetObject>,
        Box::new(zone) as Box<dyn BACnetObject>,
    ] {
        object
            .set_present_value_internal(state(LifeSafetyState::ALARM))
            .unwrap();
        object
            .set_tracking_value_internal(state(LifeSafetyState::QUIET))
            .unwrap();
        object
            .set_life_safety_operation_expected_internal(LifeSafetyOperation::RESET)
            .unwrap();
        let outcome = object
            .apply_life_safety_operation(LifeSafetyOperation::RESET)
            .unwrap();
        assert_eq!(
            outcome.changed_properties,
            vec![P::PRESENT_VALUE, P::OPERATION_EXPECTED]
        );
        assert_eq!(
            read(object.as_ref(), P::PRESENT_VALUE),
            state(LifeSafetyState::QUIET)
        );
        assert_eq!(
            read(object.as_ref(), P::SILENCED),
            PropertyValue::Enumerated(SilencedState::UNSILENCED.to_raw())
        );
    }
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(LifeSafetyState::ALARM, LifeSafetyState::QUIET); 2]
    );
}
