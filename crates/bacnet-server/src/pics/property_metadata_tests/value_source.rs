//! The PICS lists Value_Source on an object that tracks the source of a
//! noncommandable Present_Value (#1552), as required (the tables'
//! value-source footnote) and writable by its owner, and leaves it out of one
//! that doesn't.
use super::*;
use bacnet_objects::color::{ColorObject, ColorTemperatureObject};
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_objects::traits::BACnetObject;
use PropertyIdentifier as P;

/// One instance of each type that can track a single source, tracked as
/// `tracked` says.
fn objects(tracked: bool) -> Vec<Box<dyn BACnetObject>> {
    let mut color = ColorObject::new(1, "CLR-1").unwrap();
    color.set_value_source_tracking(tracked);
    let mut temperature = ColorTemperatureObject::new(1, "CT-1").unwrap();
    temperature.set_value_source_tracking(tracked);
    let mut av =
        AnalogValueObject::with_access(1, "AV-1", 62, PresentValueAccess::Writable).unwrap();
    av.set_value_source_tracking(tracked);
    let mut bv = BinaryValueObject::with_access(1, "BV-1", PresentValueAccess::ReadOnly).unwrap();
    bv.set_value_source_tracking(tracked);
    let mut msv =
        MultiStateValueObject::with_access(1, "MSV-1", 3, PresentValueAccess::Writable).unwrap();
    msv.set_value_source_tracking(tracked);
    vec![
        Box::new(color),
        Box::new(temperature),
        Box::new(av),
        Box::new(bv),
        Box::new(msv),
    ]
}

fn pics(tracked: bool) -> Pics {
    let mut db = ObjectDatabase::new();
    for object in objects(tracked) {
        db.add(object).unwrap();
    }
    generate_pics(&db, &ServerConfig::default(), &PicsConfig::default())
}

#[test]
fn pics_lists_a_tracked_single_value_source_as_required_and_writable() {
    let untracked = pics(false);
    let tracked = pics(true);
    assert_eq!(tracked.supported_object_types.len(), 5);
    for (before, after) in untracked
        .supported_object_types
        .iter()
        .zip(&tracked.supported_object_types)
    {
        let kind = after.object_type;
        assert!(
            before
                .supported_properties
                .iter()
                .all(|row| row.property_id != P::VALUE_SOURCE),
            "{kind:?}"
        );
        let row = property_support(&tracked, kind, P::VALUE_SOURCE);
        assert!(row.access.readable && row.access.writable, "{kind:?}");
        assert!(!row.access.optional, "{kind:?}: required once tracked");
        // Nothing else changes, and no source array or command time joins it.
        let others: Vec<_> = after
            .supported_properties
            .iter()
            .filter(|row| row.property_id != P::VALUE_SOURCE)
            .map(|row| (row.property_id, row.access))
            .collect();
        let previous: Vec<_> = before
            .supported_properties
            .iter()
            .map(|row| (row.property_id, row.access))
            .collect();
        assert_eq!(others, previous, "{kind:?}");
    }
    let name = P::VALUE_SOURCE.to_string();
    assert!(tracked.generate_text().contains(&name));
    assert!(!untracked.generate_text().contains(&name));
}
