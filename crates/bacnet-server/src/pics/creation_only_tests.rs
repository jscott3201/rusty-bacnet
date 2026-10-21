//! The PICS records, for each createable type, the properties a CreateObject
//! initial value sets although WriteProperty can't change them (#1429).

use super::*;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use bacnet_types::enums::EngineeringUnits;
use PropertyIdentifier as P;

fn pics() -> Pics {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        AnalogInputObject::new(1, "ai", EngineeringUnits::NO_UNITS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogOutputObject::new(1, "ao", EngineeringUnits::NO_UNITS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogValueObject::new(1, "av", EngineeringUnits::NO_UNITS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "bv").unwrap()))
        .unwrap();
    db.add(Box::new(MultiStateInputObject::new(1, "msi", 2).unwrap()))
        .unwrap();
    db.add(Box::new(MultiStateOutputObject::new(1, "mso", 2).unwrap()))
        .unwrap();
    db.add(Box::new(MultiStateValueObject::new(1, "msv", 2).unwrap()))
        .unwrap();
    generate_pics(&db, &ServerConfig::default(), &PicsConfig::default())
}

#[test]
fn each_createable_type_lists_what_only_creation_sets() {
    let pics = pics();
    let set = |object_type| {
        pics.supported_object_types
            .iter()
            .find(|support| support.object_type == object_type)
            .unwrap()
            .creation_only_properties
            .clone()
    };
    // Since #1443 a whole State_Text write sets Number_Of_States too, so the
    // multi-state types list nothing here: the count's row carries a note
    // instead (below).
    for (object_type, expected) in [
        (ObjectType::ANALOG_INPUT, vec![P::UNITS]),
        (ObjectType::ANALOG_OUTPUT, vec![P::UNITS]),
        // Not createable, so nothing is set at creation.
        (ObjectType::ANALOG_VALUE, vec![]),
        (ObjectType::BINARY_VALUE, vec![]),
        (ObjectType::MULTI_STATE_INPUT, vec![]),
        (ObjectType::MULTI_STATE_OUTPUT, vec![]),
        (ObjectType::MULTI_STATE_VALUE, vec![]),
    ] {
        assert_eq!(set(object_type), expected, "{object_type:?}");
    }

    // The rows stay read-only: the set is a separate line, not a writable flag.
    let ai = pics
        .supported_object_types
        .iter()
        .find(|support| support.object_type == ObjectType::ANALOG_INPUT)
        .unwrap();
    let units = ai
        .supported_properties
        .iter()
        .find(|row| row.property_id == P::UNITS)
        .unwrap();
    assert!(!units.access.writable);

    let text = pics.generate_text();
    assert!(text.contains(
        "  Object Type: ANALOG_INPUT (createable=true, deleteable=true)\n  \
         Whole value set only by CreateObject: UNITS\n"
    ));
    // The count's row is read-only and says how it changes, in both forms.
    let note = "R (resized through STATE_TEXT: a whole write or its size at index 0)";
    assert!(text.contains(&format!("    {:<40} {note}\n", "NUMBER_OF_STATES")));
    let markdown = pics.generate_markdown();
    assert!(markdown.contains(&format!("| NUMBER_OF_STATES | {note} |\n")));
    assert_eq!(
        text.matches(note).count(),
        3,
        "one row per multi-state type"
    );
    let msv = pics
        .supported_object_types
        .iter()
        .find(|support| support.object_type == ObjectType::MULTI_STATE_VALUE)
        .unwrap();
    let count = msv
        .supported_properties
        .iter()
        .find(|row| row.property_id == P::NUMBER_OF_STATES)
        .unwrap();
    assert!(!count.access.writable);
    assert_eq!(count.written_through, Some(P::STATE_TEXT));
    // A type with nothing to list keeps its old shape.
    assert!(
        markdown.contains("### MULTI_STATE_VALUE\n\n- Createable: true\n- Deleteable: true\n\n|")
    );
    assert!(markdown.contains("### BINARY_VALUE\n\n- Createable: true\n- Deleteable: true\n\n|"));
    assert_eq!(
        text.matches("Whole value set only by CreateObject").count(),
        2
    );
}
