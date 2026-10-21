//! Table 13-1a-2's commandable row on the two lighting outputs (#1866): a
//! Value_Source subscriber on a Lighting Output or Binary Lighting Output
//! hears a Present_Value change and a Value_Source correction, each with
//! Status_Flags, Last_Command_Time and Current_Command_Priority beside them.
use super::noncommandable_value_source::{fire, fixture, subscribe, write};
use super::value_source::decoded;
use super::*;
use bacnet_objects::lighting::{BinaryLightingOutputObject, LightingOutputObject};
use bacnet_objects::traits::BACnetObject;

const FIELDS: &[PropertyIdentifier] = &[
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::STATUS_FLAGS,
    PropertyIdentifier::VALUE_SOURCE,
    PropertyIdentifier::LAST_COMMAND_TIME,
    PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
];

/// Each output with the Present_Value pair it moves between.
fn cases() -> Vec<(Box<dyn BACnetObject>, PropertyValue, PropertyValue)> {
    vec![
        (
            Box::new(LightingOutputObject::new(3, "lo").unwrap()),
            PropertyValue::Real(10.0),
            // COV_Increment starts at 0.0, so any change reports.
            PropertyValue::Real(12.0),
        ),
        (
            Box::new(BinaryLightingOutputObject::new(3, "blo").unwrap()),
            PropertyValue::Enumerated(1),
            PropertyValue::Enumerated(0),
        ),
    ]
}

/// Exactly one notification went out, carrying [`FIELDS`] in order, each as
/// the object reads now.
async fn assert_report(
    f: &Fixture,
    oid: ObjectIdentifier,
    kind: CovNotificationKind,
) -> Vec<(PropertyIdentifier, Vec<u8>)> {
    let frames = std::mem::take(&mut *f.sent.lock().unwrap());
    assert_eq!(frames.len(), 1, "{oid:?} {kind:?}");
    let values = decoded(frames[0].clone(), kind);
    assert_eq!(values.iter().map(|v| v.0).collect::<Vec<_>>(), FIELDS);
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
async fn value_source_cov_lighting_present_value_change_and_correction() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for (object, first, changed) in cases() {
            let oid = object.object_identifier();
            let f = fixture(object, false).await;
            write(&f, oid, PropertyIdentifier::PRESENT_VALUE, first).await;
            let sub = subscribe(&f, oid, kind, false).await;
            f.fire(true, &[sub]).await;
            let initial = assert_report(&f, oid, kind).await;
            fire(&f, oid).await;
            assert!(f.sent.lock().unwrap().is_empty(), "{oid:?} no change");

            // The same writer moves Present_Value: Value_Source stays put,
            // and Last_Command_Time moves on with the value.
            write(&f, oid, PropertyIdentifier::PRESENT_VALUE, changed).await;
            fire(&f, oid).await;
            let moved = assert_report(&f, oid, kind).await;
            assert_eq!(
                field(&moved, PropertyIdentifier::VALUE_SOURCE),
                field(&initial, PropertyIdentifier::VALUE_SOURCE)
            );
            assert_ne!(
                field(&moved, PropertyIdentifier::PRESENT_VALUE),
                field(&initial, PropertyIdentifier::PRESENT_VALUE)
            );
            assert_ne!(
                field(&moved, PropertyIdentifier::LAST_COMMAND_TIME),
                field(&initial, PropertyIdentifier::LAST_COMMAND_TIME)
            );
            fire(&f, oid).await;
            assert!(f.sent.lock().unwrap().is_empty(), "{oid:?} repeat");

            // The slot's writer corrects Value_Source: that alone reports.
            let corrected = PropertyValue::ApplicationData(vec![0x08]);
            write(&f, oid, PropertyIdentifier::VALUE_SOURCE, corrected.clone()).await;
            fire(&f, oid).await;
            let fixed = assert_report(&f, oid, kind).await;
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
