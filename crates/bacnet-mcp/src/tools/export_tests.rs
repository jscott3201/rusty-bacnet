//! The Turtle a result carries: whole when it fits, otherwise cut at an
//! object's end, every point whole, with a closing comment saying so.

use bacnet_ops::export::s223::testing::check_structure;
use bacnet_ops::export::s223::{Inventory, InventoryObject, TurtleOptions, to_turtle};
use bacnet_types::enums::{EngineeringUnits, ObjectType};
use bacnet_types::primitives::ObjectIdentifier;

use super::{MAX_TURTLE, turtle};

/// A site controller's points: `n` objects, mostly analog with units and
/// names and descriptions of ordinary length, some binary and multi-state.
fn controller(n: u32) -> Inventory {
    let kinds = [
        ObjectType::ANALOG_INPUT,
        ObjectType::ANALOG_INPUT,
        ObjectType::ANALOG_OUTPUT,
        ObjectType::ANALOG_VALUE,
        ObjectType::BINARY_INPUT,
        ObjectType::BINARY_OUTPUT,
        ObjectType::BINARY_VALUE,
        ObjectType::MULTI_STATE_VALUE,
    ];
    let objects = (1..=n)
        .map(|instance| {
            let object_type = kinds[instance as usize % kinds.len()];
            let mut object =
                InventoryObject::new(ObjectIdentifier::new(object_type, instance).unwrap());
            object.name = Some(format!("AHU-1 Supply Air Point {instance}"));
            object.description = Some("Air handler 1, mechanical room 2".into());
            object.units = Some(EngineeringUnits::DEGREES_FAHRENHEIT);
            object.priority_array = object_type != ObjectType::ANALOG_INPUT;
            if object_type == ObjectType::MULTI_STATE_VALUE {
                object.states = Some(4);
                object.state_texts = ["Off", "Auto", "Low", "High"].map(String::from).into();
            }
            object
        })
        .collect();
    Inventory {
        device: 1201,
        device_name: Some("AHU-1 Controller".into()),
        objects,
        stopped: None,
    }
}

/// An export that fits comes back whole: the core's Turtle exactly.
#[test]
fn a_small_export_is_whole() {
    let inventory = controller(5);
    let options = TurtleOptions::default();
    let written = turtle(&inventory, &options, MAX_TURTLE);
    assert!(!written.cut);
    assert_eq!(written.text, to_turtle(&inventory, &options));
    assert_eq!((written.points, written.all.exported), (5, 5));
}

/// A 500-object controller's Turtle is about 370 kB, far past what one
/// result carries: it is cut to its first 30 or so objects, each point
/// whole, and the text is still well formed, its header counting what it
/// holds.
#[test]
fn a_large_export_is_cut_at_an_objects_end() {
    let inventory = controller(500);
    let options = TurtleOptions::default();
    let whole = to_turtle(&inventory, &options);
    assert!(
        (250_000..400_000).contains(&whole.len()),
        "{} bytes",
        whole.len()
    );
    let written = turtle(&inventory, &options, MAX_TURTLE);
    assert!(written.cut);
    assert!(written.text.len() <= MAX_TURTLE, "{}", written.text.len());
    assert_eq!(written.all.exported, 500);
    assert!((20..100).contains(&written.points), "{}", written.points);
    check_structure(&written.text);
    // The first objects exactly as the export writes them, then the note.
    let part = Inventory {
        objects: inventory.objects[..written.points].to_vec(),
        ..inventory.clone()
    };
    let (body, note) = written.text.rsplit_once(".\n\n").unwrap();
    assert_eq!(format!("{body}.\n\n"), to_turtle(&part, &options));
    assert_eq!(
        note,
        format!(
            "# Cut to fit one MCP result of {MAX_TURTLE} bytes: the first {} of the 500 \
             objects read. bacnet export 223-experimental writes them all.\n",
            written.points
        )
    );
    assert!(
        written
            .text
            .contains(&format!("# {} objects exported, 0 skipped", written.points)),
        "the header counts what the text holds"
    );
    // The most that fit: one more object would not have.
    let more = Inventory {
        objects: inventory.objects[..written.points + 1].to_vec(),
        ..inventory.clone()
    };
    assert!(to_turtle(&more, &options).len() + super::NOTE_ROOM > MAX_TURTLE);
}

/// A first object too large alone (a multi-state point with a thousand
/// long state texts) leaves only the note, still a well-formed document.
#[test]
fn an_object_too_large_alone_leaves_the_note() {
    let mut inventory = controller(3);
    let huge = &mut inventory.objects[0];
    *huge = InventoryObject::new(ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap());
    huge.states = Some(1000);
    huge.state_texts = (1..=1000)
        .map(|n| format!("Stage {n} of the plant"))
        .collect();
    let written = turtle(&inventory, &TurtleOptions::default(), MAX_TURTLE);
    assert!(written.cut);
    assert_eq!(written.points, 0);
    assert!(check_structure(&written.text).is_empty());
    assert_eq!(
        written.text,
        format!(
            "# Cut to fit one MCP result of {MAX_TURTLE} bytes: the first object alone is too \
             large, so none of the 3 objects read. bacnet export 223-experimental writes them \
             all.\n"
        )
    );
}
