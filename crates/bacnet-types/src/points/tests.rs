//! The point-list constructors (#2445): `new` leaves every optional
//! property out, and each `with_*` sets exactly its own.
use super::*;
use crate::enums::ObjectType;

fn analog_input() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap()
}

#[test]
fn new_leaves_every_optional_property_out() {
    let analog = Analog::new(EngineeringUnits::PERCENT, Access::ReadOnly);
    assert_eq!(
        (
            analog.cov_increment,
            analog.min_pres_value,
            analog.max_pres_value
        ),
        (None, None, None)
    );
    let binary = Binary::new(Access::Writable);
    assert_eq!((binary.active_text, binary.inactive_text), (None, None));
    let multi_state = MultiState::new(3, Access::ReadOnly);
    assert!(multi_state.state_texts.is_empty());
    let device = DeviceInfo::new(7);
    assert_eq!((device.name, device.description), (None, None));
    let point = Point::new(analog_input(), "zone-temp", analog);
    assert_eq!(point.description, None);
}

#[test]
fn each_with_sets_its_own_field() {
    let analog = Analog::new(
        EngineeringUnits::DEGREES_CELSIUS,
        Access::Commandable {
            relinquish_default: Some(21.0),
        },
    )
    .with_cov_increment(0.5)
    .with_min_pres_value(-40.0)
    .with_max_pres_value(120.0);
    assert_eq!(analog.units, EngineeringUnits::DEGREES_CELSIUS);
    assert_eq!(analog.cov_increment, Some(0.5));
    assert_eq!(analog.min_pres_value, Some(-40.0));
    assert_eq!(analog.max_pres_value, Some(120.0));
    assert_eq!(
        analog.access,
        Access::Commandable {
            relinquish_default: Some(21.0)
        }
    );

    let binary = Binary::new(Access::ReadOnly)
        .with_active_text("On")
        .with_inactive_text("Off");
    assert_eq!(binary.active_text.as_deref(), Some("On"));
    assert_eq!(binary.inactive_text.as_deref(), Some("Off"));

    let texts: Vec<String> = vec!["Off".into(), "Low".into(), "High".into()];
    let multi_state = MultiState::new(3, Access::Writable).with_state_texts(texts.clone());
    assert_eq!(multi_state.number_of_states, 3);
    assert_eq!(multi_state.state_texts, texts);
    // Borrowed labels are taken too.
    let borrowed = MultiState::new(3, Access::Writable).with_state_texts(["Off", "Low", "High"]);
    assert_eq!(borrowed, multi_state);

    let device = DeviceInfo::new(7)
        .with_name("AHU-1")
        .with_description("Air handler");
    assert_eq!(device.name.as_deref(), Some("AHU-1"));
    assert_eq!(device.description.as_deref(), Some("Air handler"));

    let point = Point::new(analog_input(), "zone-temp", analog.clone())
        .with_description("Zone temperature");
    assert_eq!(point.object, analog_input());
    assert_eq!(point.name, "zone-temp");
    assert_eq!(point.description.as_deref(), Some("Zone temperature"));
    assert_eq!(point.kind, PointKind::Analog(analog));
}

#[test]
fn each_family_converts_into_its_kind() {
    let binary = Binary::new(Access::ReadOnly);
    assert_eq!(PointKind::from(binary.clone()), PointKind::Binary(binary));
    let multi_state = MultiState::new(2, Access::ReadOnly);
    assert_eq!(
        PointKind::from(multi_state.clone()),
        PointKind::MultiState(multi_state)
    );
}

#[test]
fn a_list_keeps_its_points_in_order() {
    let first = Point::new(
        ObjectIdentifier::new(ObjectType::BINARY_INPUT, 0).unwrap(),
        "a",
        Binary::new(Access::ReadOnly),
    );
    let second = Point::new(
        ObjectIdentifier::new(ObjectType::BINARY_VALUE, 0).unwrap(),
        "b",
        Binary::new(Access::Writable),
    );
    let list = PointList::new(DeviceInfo::new(1), vec![first.clone(), second.clone()]);
    assert_eq!(list.points, [first, second]);
}
