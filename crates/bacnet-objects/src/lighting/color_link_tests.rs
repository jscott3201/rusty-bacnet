//! The colour links of the two lighting output types (#1527,
//! Addendum 135-2020ca part 4): the three rows, their writes, and
//! `ObjectDatabase::lighting_color` following the reference in use, with the
//! override switching sources without disturbing a colour fade.

use std::sync::{Arc, Mutex};

use super::*;
use crate::color::{ColorObject, ColorTemperatureObject};
use crate::database::ObjectDatabase;
use crate::property_metadata::PropertyPresenceCondition;
use bacnet_types::constructed::{BACnetColorCommand, BACnetXyColor};
use bacnet_types::enums::{ColorOperation, ErrorClass, ErrorCode};

const REFERENCE: PropertyIdentifier = PropertyIdentifier::COLOR_REFERENCE;
const OVERRIDE: PropertyIdentifier = PropertyIdentifier::COLOR_OVERRIDE;
const OVERRIDE_REFERENCE: PropertyIdentifier = PropertyIdentifier::OVERRIDE_COLOR_REFERENCE;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn color(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::COLOR, instance)
}

fn temperature(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::COLOR_TEMPERATURE, instance)
}

/// COLOR 1, overridable to COLOR_TEMPERATURE 2.
fn overridable() -> ColorLink {
    ColorLink {
        reference: color(1),
        color_override: Some(ColorOverride {
            active: false,
            reference: temperature(2),
        }),
    }
}

fn assert_error(result: Result<impl std::fmt::Debug, Error>, expected: ErrorCode) {
    let error = result.unwrap_err();
    assert!(
        matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == expected.to_raw() as u32),
        "expected {expected:?}, got {error:?}"
    );
}

/// Both lighting output types, each linked with `link`.
fn outputs(link: Option<ColorLink>) -> [Box<dyn BACnetObject>; 2] {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    lo.set_color_link(link).unwrap();
    let mut blo = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    blo.set_color_link(link).unwrap();
    [Box::new(lo), Box::new(blo)]
}

#[test]
fn the_rows_follow_the_link_in_table_order_and_are_required() {
    for object in outputs(None) {
        for p in [REFERENCE, OVERRIDE, OVERRIDE_REFERENCE] {
            assert!(!object.property_list().contains(&p));
            assert_error(object.read_property(p, None), ErrorCode::UNKNOWN_PROPERTY);
        }
    }
    for (link, rows) in [
        (ColorLink::new(color(1)), &[REFERENCE][..]),
        (
            overridable(),
            &[REFERENCE, OVERRIDE, OVERRIDE_REFERENCE][..],
        ),
    ] {
        for object in outputs(Some(link)) {
            let metadata = object.property_metadata();
            let (last, rest) = metadata.split_last().unwrap();
            assert_eq!(last.property_identifier, PropertyIdentifier::PROPERTY_LIST);
            let added = &rest[rest.len() - rows.len()..];
            for (row, &p) in added.iter().zip(rows) {
                assert_eq!(row.property_identifier, p);
                assert_eq!(
                    row.presence_condition,
                    Some(PropertyPresenceCondition::LightingColor)
                );
                assert!(object.required_properties().contains(&p));
                assert!(object.is_writable_property(p));
            }
            if rows.len() == 1 {
                assert_error(
                    object.read_property(OVERRIDE, None),
                    ErrorCode::UNKNOWN_PROPERTY,
                );
            }
        }
    }
    // The colour rows go ahead of the trims.
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    lo.set_high_end_trim(Some(90.0)).unwrap();
    lo.set_color_link(Some(ColorLink::new(color(1)))).unwrap();
    let list = lo.property_list();
    let at = |p| list.iter().position(|&q| q == p).unwrap();
    assert!(at(REFERENCE) < at(PropertyIdentifier::HIGH_END_TRIM));
}

#[test]
fn the_rows_read_and_take_writes_that_name_a_colour_object() {
    for mut object in outputs(Some(overridable())) {
        assert_eq!(
            object.read_property(REFERENCE, None).unwrap(),
            PropertyValue::ObjectIdentifier(color(1))
        );
        assert_eq!(
            object.read_property(OVERRIDE, None).unwrap(),
            PropertyValue::Boolean(false)
        );
        assert_eq!(
            object.read_property(OVERRIDE_REFERENCE, None).unwrap(),
            PropertyValue::ObjectIdentifier(temperature(2))
        );
        for (p, value) in [
            (REFERENCE, PropertyValue::ObjectIdentifier(temperature(7))),
            // Instance 4194303 names no companion.
            (
                OVERRIDE_REFERENCE,
                PropertyValue::ObjectIdentifier(color(ObjectIdentifier::WILDCARD_INSTANCE)),
            ),
            (OVERRIDE, PropertyValue::Boolean(true)),
        ] {
            object.write_property(p, None, value.clone(), None).unwrap();
            assert_eq!(object.read_property(p, None).unwrap(), value);
        }
        // Only a colour object will do.
        for p in [REFERENCE, OVERRIDE_REFERENCE] {
            assert_error(
                object.write_property(
                    p,
                    None,
                    PropertyValue::ObjectIdentifier(oid(ObjectType::ANALOG_VALUE, 1)),
                    None,
                ),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
        }
        for (p, value) in [
            (REFERENCE, PropertyValue::Unsigned(1)),
            (OVERRIDE, PropertyValue::Enumerated(1)),
            (OVERRIDE_REFERENCE, PropertyValue::Null),
        ] {
            assert_error(
                object.write_property(p, None, value, None),
                ErrorCode::INVALID_DATA_TYPE,
            );
        }
        assert_eq!(
            object.read_property(REFERENCE, None).unwrap(),
            PropertyValue::ObjectIdentifier(temperature(7))
        );
    }
}

#[test]
fn set_color_link_refuses_a_reference_to_anything_but_a_colour_object() {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    lo.set_color_link(Some(ColorLink::new(color(1)))).unwrap();
    let bad = ColorLink {
        color_override: Some(ColorOverride {
            active: true,
            reference: oid(ObjectType::LIGHTING_OUTPUT, 2),
        }),
        ..ColorLink::new(color(3))
    };
    assert_error(lo.set_color_link(Some(bad)), ErrorCode::VALUE_OUT_OF_RANGE);
    assert_eq!(lo.color_link(), Some(&ColorLink::new(color(1))));
    let mut blo = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    assert_error(
        blo.set_color_link(Some(ColorLink::new(oid(ObjectType::DEVICE, 1)))),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(blo.color_link(), None);
    lo.set_color_link(None).unwrap();
    assert_error(
        lo.read_property(REFERENCE, None),
        ErrorCode::UNKNOWN_PROPERTY,
    );
}

#[test]
fn the_active_reference_is_the_override_while_it_is_on() {
    let mut link = overridable();
    assert_eq!(link.active_reference(), Some(color(1)));
    link.color_override.as_mut().unwrap().active = true;
    assert_eq!(link.active_reference(), Some(temperature(2)));
    link.color_override.as_mut().unwrap().reference =
        temperature(ObjectIdentifier::WILDCARD_INSTANCE);
    assert_eq!(link.active_reference(), None);
    assert_eq!(
        ColorLink::new(color(ObjectIdentifier::WILDCARD_INSTANCE)).active_reference(),
        None
    );
}

/// A database on a hand-set clock holding LO 1 and BLO 1, both linked to
/// COLOR 1 and overridable to COLOR_TEMPERATURE 2, with both colour objects.
fn linked_database() -> (ObjectDatabase, Arc<Mutex<Duration>>) {
    let mut db = ObjectDatabase::new();
    let now = Arc::new(Mutex::new(Duration::ZERO));
    let source = Arc::clone(&now);
    db.set_monotonic_clock_internal(Some(Arc::new(move || *source.lock().unwrap())));
    for object in outputs(Some(overridable())) {
        db.add(object).unwrap();
    }
    let mut xy = ColorObject::new(1, "CLR-1").unwrap();
    xy.set_present_value(BACnetXyColor::new(0.2, 0.2)).unwrap();
    db.add(Box::new(xy)).unwrap();
    let mut ct = ColorTemperatureObject::new(2, "CT-2").unwrap();
    ct.set_present_value(2_700).unwrap();
    db.add(Box::new(ct)).unwrap();
    (db, now)
}

#[test]
fn lighting_color_follows_the_reference_in_use_and_the_override_leaves_a_fade_running() {
    let (mut db, now) = linked_database();
    let outputs = [
        oid(ObjectType::LIGHTING_OUTPUT, 1),
        oid(ObjectType::BINARY_LIGHTING_OUTPUT, 1),
    ];
    let xy = |x, y| LightingColor {
        source: color(1),
        color: OutputColor::Xy(BACnetXyColor::new(x, y)),
    };
    for output in outputs {
        assert_eq!(db.lighting_color(&output), Some(xy(0.2, 0.2)));
    }
    // COLOR 1 fades to (0.6, 0.4) over 4 s.
    let mut fade = BACnetColorCommand::new(ColorOperation::FADE_TO_COLOR);
    fade.target_color = Some(BACnetXyColor::new(0.6, 0.4));
    fade.fade_time = Some(4_000);
    let mut command = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_color_command(&mut command, &fade);
    db.get_mut(&color(1))
        .unwrap()
        .write_property(
            PropertyIdentifier::COLOR_COMMAND,
            None,
            PropertyValue::ApplicationData(command.to_vec()),
            None,
        )
        .unwrap();
    *now.lock().unwrap() = Duration::from_secs(1);
    for output in outputs {
        assert_eq!(db.lighting_color(&output), Some(xy(0.3, 0.25)));
        // The override takes the colour from COLOR_TEMPERATURE 2 ...
        db.get_mut(&output)
            .unwrap()
            .write_property(OVERRIDE, None, PropertyValue::Boolean(true), None)
            .unwrap();
        assert_eq!(
            db.lighting_color(&output),
            Some(LightingColor {
                source: temperature(2),
                color: OutputColor::Kelvin(2_700),
            })
        );
    }
    // ... while the fade on COLOR 1 runs on, so ending the override finds it
    // where it has got to.
    *now.lock().unwrap() = Duration::from_secs(3);
    for output in outputs {
        db.get_mut(&output)
            .unwrap()
            .write_property(OVERRIDE, None, PropertyValue::Boolean(false), None)
            .unwrap();
        assert_eq!(db.lighting_color(&output), Some(xy(0.5, 0.35)));
    }
}

#[test]
fn lighting_color_is_none_with_no_companion_to_follow() {
    let (mut db, _) = linked_database();
    let lo = oid(ObjectType::LIGHTING_OUTPUT, 1);
    // No link at all, or no such object.
    let unlinked = LightingOutputObject::new(2, "LO-2").unwrap();
    db.add(Box::new(unlinked)).unwrap();
    assert_eq!(
        db.lighting_color(&oid(ObjectType::LIGHTING_OUTPUT, 2)),
        None
    );
    assert_eq!(
        db.lighting_color(&oid(ObjectType::LIGHTING_OUTPUT, 9)),
        None
    );
    // A reference to an object this database doesn't hold is served but not
    // followed, and instance 4194303 names none.
    for reference in [color(5), color(ObjectIdentifier::WILDCARD_INSTANCE)] {
        db.get_mut(&lo)
            .unwrap()
            .write_property(
                REFERENCE,
                None,
                PropertyValue::ObjectIdentifier(reference),
                None,
            )
            .unwrap();
        assert_eq!(db.lighting_color(&lo), None);
    }
}

#[test]
fn instance_4194303_names_no_companion_whatever_its_type() {
    // The instance alone means none (Clauses 12.54.X and 12.54.Z), so any
    // type is taken with it.
    let none = |object_type| oid(object_type, ObjectIdentifier::WILDCARD_INSTANCE);
    for object_type in [ObjectType::ANALOG_VALUE, ObjectType::DEVICE] {
        let link = ColorLink {
            reference: none(object_type),
            color_override: Some(ColorOverride {
                active: true,
                reference: none(object_type),
            }),
        };
        assert_eq!(link.active_reference(), None);
        for mut object in outputs(Some(link)) {
            for p in [REFERENCE, OVERRIDE_REFERENCE] {
                let value = PropertyValue::ObjectIdentifier(none(object_type));
                object.write_property(p, None, value.clone(), None).unwrap();
                assert_eq!(object.read_property(p, None).unwrap(), value);
            }
        }
    }
    let (mut db, _) = linked_database();
    let lo = oid(ObjectType::LIGHTING_OUTPUT, 1);
    db.get_mut(&lo)
        .unwrap()
        .write_property(
            REFERENCE,
            None,
            PropertyValue::ObjectIdentifier(none(ObjectType::DEVICE)),
            None,
        )
        .unwrap();
    assert_eq!(db.lighting_color(&lo), None);
}
