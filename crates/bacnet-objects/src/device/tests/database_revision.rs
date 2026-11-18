use super::*;
use crate::database::ObjectDatabase;

/// A Device on its own reads Database_Revision 0 and lists only itself:
/// the database keeps both for the local Device (#2669), so adding objects
/// beside it changes neither of the Device's own answers.
#[test]
fn a_device_on_its_own_knows_only_itself() {
    let mut db = ObjectDatabase::new();
    let device = make_device();
    let oid = device.object_identifier();
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(
        crate::analog::AnalogInputObject::new(
            1,
            "AI-1",
            bacnet_types::enums::EngineeringUnits::NO_UNITS,
        )
        .unwrap(),
    ))
    .unwrap();
    let device = db.get(&oid).unwrap();
    assert_eq!(
        device
            .read_property(PropertyIdentifier::DATABASE_REVISION, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
    assert_eq!(
        device
            .read_property(PropertyIdentifier::OBJECT_LIST, None)
            .unwrap(),
        PropertyValue::List(vec![PropertyValue::ObjectIdentifier(oid)])
    );
    assert_eq!(db.database_revision(), 2);
    assert_eq!(db.object_list().count(), 2);
}
