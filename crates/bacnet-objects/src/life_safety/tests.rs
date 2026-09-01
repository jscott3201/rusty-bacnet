use super::*;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, LifeSafetyMode, LifeSafetyOperation, LifeSafetyState, ObjectType,
    SilencedState,
};

use crate::database::ObjectDatabase;
use crate::traits::LifeSafetyOperationEffect;

fn read_enumerated(object: &dyn BACnetObject, property: PropertyIdentifier) -> u32 {
    match object.read_property(property, None).unwrap() {
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

// -----------------------------------------------------------------------
// LifeSafetyPointObject
// -----------------------------------------------------------------------

#[test]
fn point_object_type() {
    let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    assert_eq!(
        pt.object_identifier().object_type(),
        ObjectType::LIFE_SAFETY_POINT
    );
    assert_eq!(pt.object_identifier().instance_number(), 1);
    assert_eq!(pt.object_name(), "LSP-1");
}

#[test]
fn point_read_present_value_default() {
    let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let val = pt
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(LifeSafetyState::QUIET.to_raw())
    );
}

#[test]
fn point_set_and_read_present_value() {
    let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    pt.set_present_value(LifeSafetyState::ALARM.to_raw());
    let val = pt
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(LifeSafetyState::ALARM.to_raw())
    );
}

#[test]
fn point_present_value_write_denied() {
    let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let result = pt.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(2),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn point_read_mode_default() {
    let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let val = pt.read_property(PropertyIdentifier::MODE, None).unwrap();
    assert_eq!(val, PropertyValue::Enumerated(LifeSafetyMode::OFF.to_raw()));
}

#[test]
fn point_set_mode() {
    let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    pt.set_mode(LifeSafetyMode::ON.to_raw());
    let val = pt.read_property(PropertyIdentifier::MODE, None).unwrap();
    assert_eq!(val, PropertyValue::Enumerated(LifeSafetyMode::ON.to_raw()));
}

#[test]
fn point_write_mode() {
    let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    pt.write_property(
        PropertyIdentifier::MODE,
        None,
        PropertyValue::Enumerated(LifeSafetyMode::ARMED.to_raw()),
        None,
    )
    .unwrap();
    let val = pt.read_property(PropertyIdentifier::MODE, None).unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(LifeSafetyMode::ARMED.to_raw())
    );
}

#[test]
fn point_read_silenced_default() {
    let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let val = pt
        .read_property(PropertyIdentifier::SILENCED, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // UNSILENCED
}

#[test]
fn point_life_safety_operation_combines_silenced_components() {
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();

    point.set_operation_expected(LifeSafetyOperation::SILENCE_AUDIBLE);
    assert_eq!(
        point
            .apply_life_safety_operation(LifeSafetyOperation::SILENCE_AUDIBLE)
            .unwrap(),
        LifeSafetyOperationEffect::Applied
    );
    assert_eq!(
        read_enumerated(&point, PropertyIdentifier::SILENCED),
        SilencedState::AUDIBLE_SILENCED.to_raw()
    );
    assert_eq!(
        read_enumerated(&point, PropertyIdentifier::OPERATION_EXPECTED),
        LifeSafetyOperation::NONE.to_raw()
    );

    point.set_operation_expected(LifeSafetyOperation::SILENCE_VISUAL);
    assert_eq!(
        point
            .apply_life_safety_operation(LifeSafetyOperation::SILENCE_VISUAL)
            .unwrap(),
        LifeSafetyOperationEffect::Applied
    );
    assert_eq!(
        read_enumerated(&point, PropertyIdentifier::SILENCED),
        SilencedState::ALL_SILENCED.to_raw()
    );
}

#[test]
fn point_replayed_silence_without_response_cache_is_invalid_state() {
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    point.set_operation_expected(LifeSafetyOperation::SILENCE);
    assert_eq!(
        point
            .apply_life_safety_operation(LifeSafetyOperation::SILENCE)
            .unwrap(),
        LifeSafetyOperationEffect::Applied
    );

    let error = point
        .apply_life_safety_operation(LifeSafetyOperation::SILENCE)
        .unwrap_err();
    assert_protocol_error(
        error,
        ErrorClass::OBJECT,
        ErrorCode::INVALID_OPERATION_IN_THIS_STATE,
    );
    assert_eq!(
        read_enumerated(&point, PropertyIdentifier::SILENCED),
        SilencedState::ALL_SILENCED.to_raw()
    );
}

#[test]
fn point_same_state_honors_and_clears_operation_expected() {
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    point.set_silenced(SilencedState::VISIBLE_SILENCED);
    point.set_operation_expected(LifeSafetyOperation::UNSILENCE_VISUAL);

    let error = point
        .apply_life_safety_operation(LifeSafetyOperation::SILENCE_VISUAL)
        .unwrap_err();
    assert_protocol_error(
        error,
        ErrorClass::OBJECT,
        ErrorCode::INVALID_OPERATION_IN_THIS_STATE,
    );
    assert_eq!(
        read_enumerated(&point, PropertyIdentifier::OPERATION_EXPECTED),
        LifeSafetyOperation::UNSILENCE_VISUAL.to_raw()
    );

    point.set_operation_expected(LifeSafetyOperation::SILENCE_VISUAL);
    assert_eq!(
        point
            .apply_life_safety_operation(LifeSafetyOperation::SILENCE_VISUAL)
            .unwrap(),
        LifeSafetyOperationEffect::Applied
    );
    assert_eq!(
        read_enumerated(&point, PropertyIdentifier::OPERATION_EXPECTED),
        LifeSafetyOperation::NONE.to_raw()
    );
}

#[test]
fn point_life_safety_operation_covers_silence_and_unsilence_matrix() {
    let cases = [
        (
            SilencedState::UNSILENCED,
            LifeSafetyOperation::SILENCE,
            SilencedState::ALL_SILENCED,
        ),
        (
            SilencedState::UNSILENCED,
            LifeSafetyOperation::SILENCE_AUDIBLE,
            SilencedState::AUDIBLE_SILENCED,
        ),
        (
            SilencedState::UNSILENCED,
            LifeSafetyOperation::SILENCE_VISUAL,
            SilencedState::VISIBLE_SILENCED,
        ),
        (
            SilencedState::ALL_SILENCED,
            LifeSafetyOperation::UNSILENCE,
            SilencedState::UNSILENCED,
        ),
        (
            SilencedState::ALL_SILENCED,
            LifeSafetyOperation::UNSILENCE_AUDIBLE,
            SilencedState::VISIBLE_SILENCED,
        ),
        (
            SilencedState::ALL_SILENCED,
            LifeSafetyOperation::UNSILENCE_VISUAL,
            SilencedState::AUDIBLE_SILENCED,
        ),
    ];

    for (initial, operation, expected) in cases {
        let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        point.set_silenced(initial);
        point.set_operation_expected(operation);
        assert_eq!(
            point.apply_life_safety_operation(operation).unwrap(),
            LifeSafetyOperationEffect::Applied
        );
        assert_eq!(
            read_enumerated(&point, PropertyIdentifier::SILENCED),
            expected.to_raw(),
            "operation {} from state {}",
            operation.to_raw(),
            initial.to_raw()
        );
    }
}

#[test]
fn point_rejects_wrong_expected_silence_and_reset_without_mutation() {
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    point.set_operation_expected(LifeSafetyOperation::SILENCE_AUDIBLE);

    let error = point
        .apply_life_safety_operation(LifeSafetyOperation::SILENCE_VISUAL)
        .unwrap_err();
    assert_protocol_error(
        error,
        ErrorClass::OBJECT,
        ErrorCode::INVALID_OPERATION_IN_THIS_STATE,
    );
    assert_eq!(
        read_enumerated(&point, PropertyIdentifier::SILENCED),
        SilencedState::UNSILENCED.to_raw()
    );

    let error = point
        .apply_life_safety_operation(LifeSafetyOperation::RESET)
        .unwrap_err();
    assert_protocol_error(
        error,
        ErrorClass::OBJECT,
        ErrorCode::INVALID_OPERATION_IN_THIS_STATE,
    );
    assert_eq!(
        read_enumerated(&point, PropertyIdentifier::OPERATION_EXPECTED),
        LifeSafetyOperation::SILENCE_AUDIBLE.to_raw()
    );
}

#[test]
fn point_silenced_and_operation_expected_are_network_read_only() {
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    for property in [
        PropertyIdentifier::SILENCED,
        PropertyIdentifier::OPERATION_EXPECTED,
    ] {
        let error = point
            .write_property(property, None, PropertyValue::Enumerated(3), None)
            .unwrap_err();
        assert_protocol_error(error, ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
        assert!(!point.is_writable_property(property));
    }
}

#[test]
fn point_can_be_rearmed_through_the_local_trait_channel() {
    let oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(LifeSafetyPointObject::new(1, "LSP-1").unwrap()))
        .unwrap();

    let object = db.get_mut(&oid).unwrap();
    object
        .set_life_safety_operation_expected_internal(LifeSafetyOperation::SILENCE)
        .unwrap();
    assert_eq!(
        object
            .apply_life_safety_operation(LifeSafetyOperation::SILENCE)
            .unwrap(),
        LifeSafetyOperationEffect::Applied
    );
    object
        .set_life_safety_operation_expected_internal(LifeSafetyOperation::UNSILENCE)
        .unwrap();
    assert_eq!(
        object
            .apply_life_safety_operation(LifeSafetyOperation::UNSILENCE)
            .unwrap(),
        LifeSafetyOperationEffect::Applied
    );
    assert_eq!(
        read_enumerated(object.as_ref(), PropertyIdentifier::OPERATION_EXPECTED),
        LifeSafetyOperation::NONE.to_raw()
    );
    assert_eq!(
        read_enumerated(object.as_ref(), PropertyIdentifier::SILENCED),
        SilencedState::UNSILENCED.to_raw()
    );
}

#[test]
fn point_read_tracking_value() {
    let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    pt.set_tracking_value(LifeSafetyState::PRE_ALARM.to_raw());
    let val = pt
        .read_property(PropertyIdentifier::TRACKING_VALUE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(LifeSafetyState::PRE_ALARM.to_raw())
    );
}

#[test]
fn point_read_direct_reading() {
    let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    pt.set_direct_reading(42.5);
    let val = pt
        .read_property(PropertyIdentifier::DIRECT_READING, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Real(42.5));
}

#[test]
fn point_read_maintenance_required() {
    let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let val = pt
        .read_property(PropertyIdentifier::MAINTENANCE_REQUIRED, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(false));
}

#[test]
fn point_add_member_and_read() {
    let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let zone1 = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 1).unwrap();
    let zone2 = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 2).unwrap();
    pt.add_member(zone1);
    pt.add_member(zone2);

    let val = pt
        .read_property(PropertyIdentifier::MEMBER_OF, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(zone1),
            PropertyValue::ObjectIdentifier(zone2),
        ])
    );
}

#[test]
fn point_member_of_empty() {
    let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let val = pt
        .read_property(PropertyIdentifier::MEMBER_OF, None)
        .unwrap();
    assert_eq!(val, PropertyValue::List(vec![]));
}

#[test]
fn point_read_event_state_default() {
    let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let val = pt
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // NORMAL
}

#[test]
fn point_read_object_type() {
    let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let val = pt
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::LIFE_SAFETY_POINT.to_raw())
    );
}

#[test]
fn point_property_list() {
    let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let props = pt.property_list();
    assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(props.contains(&PropertyIdentifier::MODE));
    assert!(props.contains(&PropertyIdentifier::SILENCED));
    assert!(props.contains(&PropertyIdentifier::OPERATION_EXPECTED));
    assert!(props.contains(&PropertyIdentifier::TRACKING_VALUE));
    assert!(props.contains(&PropertyIdentifier::MEMBER_OF));
    assert!(props.contains(&PropertyIdentifier::DIRECT_READING));
    assert!(props.contains(&PropertyIdentifier::MAINTENANCE_REQUIRED));
    assert!(props.contains(&PropertyIdentifier::EVENT_STATE));
    assert!(props.contains(&PropertyIdentifier::STATUS_FLAGS));
    assert!(props.contains(&PropertyIdentifier::OUT_OF_SERVICE));
    assert!(props.contains(&PropertyIdentifier::RELIABILITY));
}

#[test]
fn point_write_mode_wrong_type() {
    let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let result = pt.write_property(
        PropertyIdentifier::MODE,
        None,
        PropertyValue::Real(1.0),
        None,
    );
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// LifeSafetyZoneObject
// -----------------------------------------------------------------------

#[test]
fn zone_object_type() {
    let z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    assert_eq!(
        z.object_identifier().object_type(),
        ObjectType::LIFE_SAFETY_ZONE
    );
    assert_eq!(z.object_identifier().instance_number(), 1);
    assert_eq!(z.object_name(), "LSZ-1");
}

#[test]
fn zone_read_present_value_default() {
    let z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    let val = z
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(LifeSafetyState::QUIET.to_raw())
    );
}

#[test]
fn zone_set_and_read_present_value() {
    let mut z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    z.set_present_value(LifeSafetyState::ALARM.to_raw());
    let val = z
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(LifeSafetyState::ALARM.to_raw())
    );
}

#[test]
fn zone_present_value_write_denied() {
    let mut z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    let result = z.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(2),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn zone_read_mode_default() {
    let z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    let val = z.read_property(PropertyIdentifier::MODE, None).unwrap();
    assert_eq!(val, PropertyValue::Enumerated(LifeSafetyMode::OFF.to_raw()));
}

#[test]
fn zone_set_mode() {
    let mut z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    z.set_mode(LifeSafetyMode::ARMED.to_raw());
    let val = z.read_property(PropertyIdentifier::MODE, None).unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(LifeSafetyMode::ARMED.to_raw())
    );
}

#[test]
fn zone_add_zone_member_and_read() {
    let mut z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    let pt1 = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
    let pt2 = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 2).unwrap();
    let pt3 = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 3).unwrap();
    z.add_zone_member(pt1);
    z.add_zone_member(pt2);
    z.add_zone_member(pt3);

    let val = z
        .read_property(PropertyIdentifier::ZONE_MEMBERS, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(pt1),
            PropertyValue::ObjectIdentifier(pt2),
            PropertyValue::ObjectIdentifier(pt3),
        ])
    );
}

#[test]
fn zone_members_empty() {
    let z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    let val = z
        .read_property(PropertyIdentifier::ZONE_MEMBERS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::List(vec![]));
}

#[test]
fn zone_read_event_state_default() {
    let z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    let val = z
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // NORMAL
}

#[test]
fn zone_life_safety_operation_unsilences_one_component() {
    let mut zone = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    zone.set_silenced(SilencedState::ALL_SILENCED);
    zone.set_operation_expected(LifeSafetyOperation::UNSILENCE_AUDIBLE);

    assert_eq!(
        zone.apply_life_safety_operation(LifeSafetyOperation::UNSILENCE_AUDIBLE)
            .unwrap(),
        LifeSafetyOperationEffect::Applied
    );
    assert_eq!(
        read_enumerated(&zone, PropertyIdentifier::SILENCED),
        SilencedState::VISIBLE_SILENCED.to_raw()
    );
    assert_eq!(
        read_enumerated(&zone, PropertyIdentifier::OPERATION_EXPECTED),
        LifeSafetyOperation::NONE.to_raw()
    );
}

#[test]
fn zone_same_state_rejects_a_different_expected_operation() {
    let mut zone = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    zone.set_silenced(SilencedState::ALL_SILENCED);
    zone.set_operation_expected(LifeSafetyOperation::UNSILENCE);

    let error = zone
        .apply_life_safety_operation(LifeSafetyOperation::SILENCE)
        .unwrap_err();
    assert_protocol_error(
        error,
        ErrorClass::OBJECT,
        ErrorCode::INVALID_OPERATION_IN_THIS_STATE,
    );
    assert_eq!(
        read_enumerated(&zone, PropertyIdentifier::OPERATION_EXPECTED),
        LifeSafetyOperation::UNSILENCE.to_raw()
    );
}

#[test]
fn zone_read_object_type() {
    let z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    let val = z
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::LIFE_SAFETY_ZONE.to_raw())
    );
}

#[test]
fn zone_property_list() {
    let z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    let props = z.property_list();
    assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(props.contains(&PropertyIdentifier::MODE));
    assert!(props.contains(&PropertyIdentifier::SILENCED));
    assert!(props.contains(&PropertyIdentifier::OPERATION_EXPECTED));
    assert!(props.contains(&PropertyIdentifier::ZONE_MEMBERS));
    assert!(props.contains(&PropertyIdentifier::EVENT_STATE));
    assert!(props.contains(&PropertyIdentifier::STATUS_FLAGS));
    assert!(props.contains(&PropertyIdentifier::OUT_OF_SERVICE));
    assert!(props.contains(&PropertyIdentifier::RELIABILITY));
}

#[test]
fn zone_write_mode() {
    let mut z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    z.write_property(
        PropertyIdentifier::MODE,
        None,
        PropertyValue::Enumerated(LifeSafetyMode::ON.to_raw()),
        None,
    )
    .unwrap();
    let val = z.read_property(PropertyIdentifier::MODE, None).unwrap();
    assert_eq!(val, PropertyValue::Enumerated(LifeSafetyMode::ON.to_raw()));
}

#[test]
fn zone_write_out_of_service() {
    let mut z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    z.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    let val = z
        .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(true));
}

#[test]
fn zone_write_unknown_property_denied() {
    let mut z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    let result = z.write_property(
        PropertyIdentifier::TRACKING_VALUE,
        None,
        PropertyValue::Enumerated(0),
        None,
    );
    assert!(result.is_err());
}
