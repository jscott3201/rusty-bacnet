use super::*;
use bacnet_types::enums::{LifeSafetyMode, LifeSafetyState, ObjectType};

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
