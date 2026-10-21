//! Table 13-1a-2's noncommandable row on the single-source objects (#1582):
//! a Value_Source subscriber on a tracked Color, Color Temperature or
//! noncommandable Analog Value hears a Present_Value change from the same
//! writer and a Value_Source correction, each with the row's companions.
use super::noncommandable_value_source::{fire, fixture, subscribe, write};
use super::value_source::decoded;
use super::*;
use bacnet_objects::{
    color::{ColorObject, ColorTemperatureObject},
    present_value_access::PresentValueAccess,
    traits::BACnetObject,
};

/// One tracked object, the Present_Value pair it moves between, and the
/// report's fields: Color and Color Temperature have no Status_Flags.
struct Case {
    object: Box<dyn BACnetObject>,
    first: PropertyValue,
    changed: PropertyValue,
    fields: &'static [PropertyIdentifier],
}

const NO_FLAGS: &[PropertyIdentifier] = &[
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::VALUE_SOURCE,
];
const WITH_FLAGS: &[PropertyIdentifier] = &[
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::STATUS_FLAGS,
    PropertyIdentifier::VALUE_SOURCE,
];

fn xy(x: f32, y: f32) -> PropertyValue {
    PropertyValue::List(vec![PropertyValue::Real(x), PropertyValue::Real(y)])
}

fn cases() -> Vec<Case> {
    let mut color = ColorObject::new(3, "color").unwrap();
    color.set_value_source_tracking(true);
    let mut cct = ColorTemperatureObject::new(3, "cct").unwrap();
    cct.set_value_source_tracking(true);
    let mut av = AnalogValueObject::with_access(
        3,
        "av",
        EngineeringUnits::NO_UNITS,
        PresentValueAccess::Writable,
    )
    .unwrap();
    av.set_value_source_tracking(true);
    vec![
        Case {
            object: Box::new(color),
            first: xy(0.3, 0.3),
            changed: xy(0.4, 0.35),
            fields: NO_FLAGS,
        },
        Case {
            object: Box::new(cct),
            first: PropertyValue::Unsigned(3000),
            // One kelvin: with no COV_Increment, any change reports.
            changed: PropertyValue::Unsigned(3001),
            fields: NO_FLAGS,
        },
        Case {
            object: Box::new(av),
            first: PropertyValue::Real(10.0),
            changed: PropertyValue::Real(12.0),
            fields: WITH_FLAGS,
        },
    ]
}

/// Exactly one notification went out, carrying `fields` in order, each as
/// the object reads now.
async fn assert_report(
    f: &Fixture,
    oid: ObjectIdentifier,
    kind: CovNotificationKind,
    fields: &[PropertyIdentifier],
) -> Vec<(PropertyIdentifier, Vec<u8>)> {
    let frames = std::mem::take(&mut *f.sent.lock().unwrap());
    assert_eq!(frames.len(), 1, "{oid:?} {kind:?}");
    let values = decoded(frames[0].clone(), kind);
    assert_eq!(values.iter().map(|v| v.0).collect::<Vec<_>>(), fields);
    let db = f.db.read().await;
    for (property, bytes) in &values {
        let mut expected = BytesMut::new();
        encode_property_value(
            &mut expected,
            &db.get(&oid)
                .unwrap()
                .read_property(*property, None)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(bytes, &expected, "{oid:?} {property:?}");
    }
    values
}

fn field(values: &[(PropertyIdentifier, Vec<u8>)], property: PropertyIdentifier) -> &[u8] {
    &values.iter().find(|(p, _)| *p == property).unwrap().1
}

#[tokio::test]
async fn value_source_cov_single_source_present_value_change_and_correction() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for case in cases() {
            let oid = case.object.object_identifier();
            let f = fixture(case.object, false).await;
            write(&f, oid, PropertyIdentifier::PRESENT_VALUE, case.first).await;
            let sub = subscribe(&f, oid, kind, false).await;
            f.fire(true, &[sub]).await;
            let initial = assert_report(&f, oid, kind, case.fields).await;
            fire(&f, oid).await;
            assert!(f.sent.lock().unwrap().is_empty(), "{oid:?} no change");

            // The same writer moves Present_Value: Value_Source stays put,
            // so only the object's own criterion can trigger this report.
            write(&f, oid, PropertyIdentifier::PRESENT_VALUE, case.changed).await;
            fire(&f, oid).await;
            let moved = assert_report(&f, oid, kind, case.fields).await;
            assert_eq!(
                field(&moved, PropertyIdentifier::VALUE_SOURCE),
                field(&initial, PropertyIdentifier::VALUE_SOURCE)
            );
            assert_ne!(
                field(&moved, PropertyIdentifier::PRESENT_VALUE),
                field(&initial, PropertyIdentifier::PRESENT_VALUE)
            );
            fire(&f, oid).await;
            assert!(f.sent.lock().unwrap().is_empty(), "{oid:?} repeat");

            // A correction of Value_Source alone reports, with Present_Value.
            let corrected = PropertyValue::ApplicationData(vec![0x08]);
            write(&f, oid, PropertyIdentifier::VALUE_SOURCE, corrected.clone()).await;
            fire(&f, oid).await;
            let fixed = assert_report(&f, oid, kind, case.fields).await;
            assert_eq!(field(&fixed, PropertyIdentifier::VALUE_SOURCE), [0x08]);
            assert_eq!(
                field(&fixed, PropertyIdentifier::PRESENT_VALUE),
                field(&moved, PropertyIdentifier::PRESENT_VALUE)
            );
            write(&f, oid, PropertyIdentifier::VALUE_SOURCE, corrected).await;
            fire(&f, oid).await;
            assert!(f.sent.lock().unwrap().is_empty(), "{oid:?} same correction");
            f.finish(false).await;
        }
    }
}
