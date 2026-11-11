//! Pages as the tool fills them, without a device: their size, their
//! comment lines, and their points against the whole export's.

use bacnet_ops::export::s223::testing::check_structure;
use bacnet_ops::export::s223::{Inventory, InventoryObject, TurtleOptions, to_turtle};
use bacnet_types::enums::{EngineeringUnits, ObjectType};
use bacnet_types::primitives::ObjectIdentifier;

use super::{LITERALS_COMMENT, MAX_TURTLE, PAGE, Page, page_note};

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

/// A Turtle document's points: what follows the last prefix.
pub(crate) fn points(text: &str) -> &str {
    let at = text.find("\n@prefix d: ").expect("the device's prefix");
    let end = text[at..].find("\n\n").expect("the blank line after it");
    &text[at + end + 2..]
}

/// `inventory`'s objects in pages as the tool fills them, each page's
/// Turtle as the tool writes it.
fn paged(inventory: &Inventory, options: &TurtleOptions) -> Vec<(usize, String)> {
    let mut pages = Vec::new();
    let mut at = 0;
    while at < inventory.objects.len() {
        let mut page = Page::new(inventory.device, inventory.device_name.clone(), options);
        while at < inventory.objects.len() && !page.full(PAGE) {
            page.push(inventory.objects[at].clone());
            at += 1;
        }
        let read = page.inventory.objects.len();
        let more = at < inventory.objects.len();
        let note = page_note(
            inventory.device,
            at - read,
            read,
            inventory.objects.len(),
            more,
        );
        pages.push((read, format!("{note}{LITERALS_COMMENT}{}", page.render().0)));
    }
    pages
}

/// A 500-object controller's model is about 370 kB: it comes in pages of
/// about 30 points, each a well-formed document under the text cap, and
/// their points, in order, are the whole export's.
#[test]
fn a_large_controller_comes_in_pages() {
    let inventory = controller(500);
    let options = TurtleOptions::default();
    let whole = to_turtle(&inventory, &options);
    assert!(
        (300_000..450_000).contains(&whole.len()),
        "{} bytes",
        whole.len()
    );
    let pages = paged(&inventory, &options);
    assert!((14..=20).contains(&pages.len()), "{} pages", pages.len());
    let mut joined = String::new();
    for (read, text) in &pages {
        assert!(text.len() <= MAX_TURTLE, "{} bytes", text.len());
        check_structure(text);
        assert!(text.contains(&format!("# {read} objects exported, 0 skipped")));
        joined.push_str(points(text));
    }
    assert_eq!(joined, points(&whole));
    assert_eq!(pages.iter().map(|(read, _)| read).sum::<usize>(), 500);
    let first = pages[0].0;
    assert!(
        (25..=35).contains(&first),
        "{first} points on the first page"
    );
    assert!(pages[0].1.starts_with(&format!(
        "# Page: objects 1 to {first} of the 500 of the selected types on device 1201 (in \
         Object_List order); more follow: call bacnet_export_223 again with next_cursor.\n# Its \
         string literals hold device text exactly"
    )));
    assert!(
        pages
            .last()
            .unwrap()
            .1
            .lines()
            .next()
            .unwrap()
            .ends_with("; the last page.")
    );
}

/// A page always takes its first object, however large: a multi-state
/// point with a thousand long state texts gets a page of its own, and the
/// next page goes on after it.
#[test]
fn an_object_larger_than_a_page_gets_one_of_its_own() {
    let mut inventory = controller(40);
    let huge = &mut inventory.objects[0];
    *huge = InventoryObject::new(ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap());
    huge.states = Some(1000);
    huge.state_texts = (1..=1000)
        .map(|n| format!("Stage {n} of the plant"))
        .collect();
    let options = TurtleOptions::default();
    let pages = paged(&inventory, &options);
    assert_eq!(pages[0].0, 1);
    assert!(pages[0].1.len() > MAX_TURTLE);
    assert!(pages[1].1.len() <= MAX_TURTLE);
    let joined: String = pages.iter().map(|(_, text)| points(text)).collect();
    assert_eq!(joined, points(&to_turtle(&inventory, &options)));
}

/// The policy's object page caps a page of small points.
#[test]
fn a_page_stops_at_its_cap() {
    let mut inventory = controller(60);
    for object in &mut inventory.objects {
        object.name = None;
        object.description = None;
    }
    let options = TurtleOptions::default();
    let mut page = Page::new(1201, None, &options);
    for object in inventory.objects.iter().take(5) {
        assert!(!page.full(5));
        page.push(object.clone());
    }
    assert!(page.full(5));
    assert!(!page.full(PAGE));
}
