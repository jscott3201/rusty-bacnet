use super::*;
use bacnet_objects::audit::ObjectAuditPolicy;
use bacnet_objects::color::{ColorObject, ColorTemperatureObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::AuditOperationFlags;
use bacnet_types::enums::AuditLevel;
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn expected_rows(kind: ObjectType) -> Vec<PropertyRow> {
    // Independent (identifier, optional, writable) rows in declaration order; PICS sorts by property ID.
    match kind {
        ObjectType::COLOR => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::OBJECT_TYPE, false, false),
            (P::PRESENT_VALUE, false, true),
            (P::TRACKING_VALUE, false, false),
            (P::COLOR_COMMAND, false, true),
            (P::IN_PROGRESS, false, false),
            (P::DEFAULT_COLOR, false, true),
            (P::DESCRIPTION, true, true),
            (P::DEFAULT_FADE_TIME, false, true),
            (P::TRANSITION, true, true),
            (P::PROPERTY_LIST, false, false),
        ],
        _ => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::OBJECT_TYPE, false, false),
            (P::PRESENT_VALUE, false, true),
            (P::TRACKING_VALUE, false, false),
            (P::COLOR_COMMAND, false, true),
            (P::IN_PROGRESS, false, false),
            (P::DEFAULT_COLOR_TEMPERATURE, false, true),
            (P::DESCRIPTION, true, true),
            (P::DEFAULT_FADE_TIME, false, true),
            (P::DEFAULT_RAMP_RATE, false, true),
            (P::DEFAULT_STEP_INCREMENT, false, true),
            (P::MIN_PRES_VALUE, true, false),
            (P::MAX_PRES_VALUE, true, false),
            (P::TRANSITION, true, true),
            (P::PROPERTY_LIST, false, false),
        ],
    }
}

#[test]
fn pics_color_property_metadata_is_exact() {
    let fresh: [FreshObject; 2] = [
        || {
            (
                Box::new(ColorObject::new(7, "CLR-7").unwrap()),
                ObjectType::COLOR,
            )
        },
        || {
            (
                Box::new(ColorTemperatureObject::new(7, "CT-7").unwrap()),
                ObjectType::COLOR_TEMPERATURE,
            )
        },
    ];
    for make in fresh {
        let expected = expected_rows(make().1);
        for configured in [false, true] {
            let (mut object, kind) = make();
            if configured {
                object
                    .write_property(
                        P::DESCRIPTION,
                        None,
                        PropertyValue::CharacterString("long color label".repeat(100)),
                        None,
                    )
                    .unwrap();
            }
            check_pics(object, kind, &expected, configured);
        }
    }
}

/// An audit policy with both of the colour objects' audit rows (#1525).
fn audit_policy() -> ObjectAuditPolicy {
    ObjectAuditPolicy {
        level: Some(AuditLevel::AUDIT_ALL),
        operations: Some(AuditOperationFlags::empty()),
        ..ObjectAuditPolicy::default()
    }
}

#[test]
fn pics_color_lists_provisioned_audit_rows_as_optional_and_writable() {
    let mut color = ColorObject::new(7, "CLR-7").unwrap();
    color.set_audit_policy(audit_policy());
    let mut temperature = ColorTemperatureObject::new(7, "CT-7").unwrap();
    temperature.set_audit_policy(audit_policy());
    let objects: [(Box<dyn BACnetObject>, ObjectType); 2] = [
        (Box::new(color), ObjectType::COLOR),
        (Box::new(temperature), ObjectType::COLOR_TEMPERATURE),
    ];
    for (object, kind) in objects {
        let mut expected = expected_rows(kind);
        expected.extend([
            (P::AUDIT_LEVEL, true, true),
            (P::AUDITABLE_OPERATIONS, true, true),
        ]);
        check_pics(object, kind, &expected, true);
    }
}

fn check_pics(
    object: Box<dyn BACnetObject>,
    kind: ObjectType,
    expected: &[PropertyRow],
    configured: bool,
) {
    let required = object.required_properties();
    let mut db = ObjectDatabase::new();
    db.add(object).unwrap();
    let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
    assert_eq!(pics.supported_object_types.len(), 1);
    let support = &pics.supported_object_types[0];
    assert_eq!(support.object_type, kind);
    assert!(!support.createable);
    assert!(support.deleteable);
    let rows: Vec<_> = support
        .supported_properties
        .iter()
        .map(|row| {
            assert!(row.access.readable);
            (row.property_id, row.access.optional, row.access.writable)
        })
        .collect();
    assert_eq!(
        rows,
        sorted_rows(expected),
        "{kind:?}, configured={configured}"
    );
    assert_eq!(
        rows.iter()
            .filter_map(|&(p, optional, _)| (!optional).then_some(p))
            .collect::<Vec<_>>(),
        sorted_required(required.as_ref())
    );
}
