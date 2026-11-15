use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use super::*;
use crate::types::decode_read_value;

/// [0] 2, [1] ( REAL 1.5, [2] ( TRUE ) ), Unsigned 7: a vendor's value.
const FIELDS: [u8; 14] = [
    0x09, 0x02, 0x1E, 0x44, 0x3F, 0xC0, 0, 0, 0x2E, 0x11, 0x2F, 0x1F, 0x21, 7,
];

/// The value a read of [`FIELDS`] from a vendor property gives, in Python.
fn read(py: Python<'_>) -> Bound<'_, PyPropertyValue> {
    let value = decode_read_value(
        ObjectType::from_raw(200),
        PropertyIdentifier::from_raw(1000),
        None,
        &FIELDS,
    )
    .unwrap();
    Bound::new(py, value).unwrap()
}

fn attr<'py, T: FromPyObjectOwned<'py>>(object: &Bound<'py, PyAny>, name: &str) -> T {
    object
        .getattr(name)
        .unwrap()
        .extract()
        .map_err(Into::<PyErr>::into)
        .unwrap()
}

#[test]
fn a_value_with_context_tags_reads_as_tag_nodes() {
    Python::initialize();
    Python::attach(|py| {
        let value = read(py).into_any();
        assert_eq!(attr::<String>(&value, "tag"), "tagged");
        assert_eq!(attr::<Vec<u8>>(&value, "octets"), FIELDS);
        assert_eq!(
            value.repr().unwrap().to_string(),
            "PropertyValue.tagged(<14 bytes>)"
        );
        let nodes: Vec<Bound<'_, PyAny>> = attr(&value, "value");
        assert_eq!(nodes.len(), 3);
        // [0] 2: a context primitive's contents.
        assert_eq!(attr::<String>(&nodes[0], "tag_class"), "context");
        assert_eq!(attr::<u8>(&nodes[0], "tag_number"), 0);
        assert!(!attr::<bool>(&nodes[0], "constructed"));
        assert_eq!(attr::<Vec<u8>>(&nodes[0], "value"), [0x02]);
        assert_eq!(attr::<Vec<u8>>(&nodes[0], "octets"), [0x09, 0x02]);
        assert_eq!(
            nodes[0].repr().unwrap().to_string(),
            "<TagNode context 0, 1 octet>"
        );
        // [1] ( ... ): its members, each with its exact octets.
        assert!(attr::<bool>(&nodes[1], "constructed"));
        assert_eq!(attr::<Vec<u8>>(&nodes[1], "octets"), &FIELDS[2..12]);
        assert_eq!(
            nodes[1].repr().unwrap().to_string(),
            "<TagNode context 1 constructed, 2 members>"
        );
        let members: Vec<Bound<'_, PyAny>> = attr(&nodes[1], "value");
        let real = members[0].getattr("value").unwrap();
        assert_eq!(attr::<String>(&real, "tag"), "real");
        assert_eq!(attr::<f64>(&real, "value"), 1.5);
        assert_eq!(attr::<String>(&members[0], "tag_class"), "application");
        assert_eq!(attr::<u8>(&members[0], "tag_number"), 4);
        assert_eq!(
            members[0].repr().unwrap().to_string(),
            "<TagNode application 4 real 1.5>"
        );
        let inner: Vec<Bound<'_, PyAny>> = attr(&members[1], "value");
        assert!(attr::<bool>(&inner[0].getattr("value").unwrap(), "value"));
        // Unsigned 7 at the top level.
        assert_eq!(attr::<u64>(&nodes[2].getattr("value").unwrap(), "value"), 7);
    });
}

#[test]
fn tagged_values_and_nodes_compare_hash_and_rebuild_from_octets() {
    Python::initialize();
    Python::attach(|py| {
        let value = read(py);
        let again = read(py);
        assert!(value.eq(&again).unwrap());
        assert_eq!(value.hash().unwrap(), again.hash().unwrap());
        // Not equal to the same octets as application data.
        let blob = Bound::new(
            py,
            PyPropertyValue::from_rust(PropertyValue::ApplicationData(FIELDS.to_vec())),
        )
        .unwrap();
        assert!(!value.eq(&blob).unwrap());
        // copy.copy and pickle go through __reduce__.
        let copy = py.import("copy").unwrap();
        let copied = copy.call_method1("deepcopy", (&value,)).unwrap();
        assert!(copied.eq(&value).unwrap());
        assert_eq!(attr::<String>(&copied, "tag"), "tagged");
        // What pickle stores: the constructor and its arguments (the module
        // isn't importable from a Rust test, so they're called here).
        let reduced = value.call_method0("__reduce__").unwrap();
        let (constructor, arguments): (Bound<'_, PyAny>, Bound<'_, PyTuple>) =
            reduced.extract().unwrap();
        assert!(constructor.call1(arguments).unwrap().eq(&value).unwrap());
        let nodes: Vec<Bound<'_, PyAny>> = attr(value.as_any(), "value");
        for node in &nodes {
            let copied = copy.call_method1("deepcopy", (node,)).unwrap();
            assert!(copied.eq(node).unwrap());
            assert_eq!(copied.hash().unwrap(), node.hash().unwrap());
        }
        assert!(!nodes[0].eq(&nodes[2]).unwrap());
        // Octets that aren't a tree, or not one node, refuse to rebuild.
        let class = py.get_type::<PyPropertyValue>();
        for octets in [&[0x1E][..], &[0x09, 0x01, 0x75, 0x05, 3, 0, 0, 0, 0x41]] {
            let error = class
                .call_method1("tagged", (octets.to_vec(),))
                .unwrap_err();
            assert!(error.is_instance_of::<PyValueError>(py));
        }
        let node_class = py.get_type::<PyTagNode>();
        for octets in [&[0x09, 0x01, 0x09, 0x02][..], &[], &[0x1E]] {
            let error = node_class
                .call_method1("_from_octets", (PyBytes::new(py, octets),))
                .unwrap_err();
            assert!(error.is_instance_of::<PyValueError>(py));
        }
    });
}

#[test]
fn a_list_of_tagged_values_keeps_them_tagged() {
    use crate::types::property_value::Form;
    Python::initialize();
    Python::attach(|py| {
        let class = py.get_type::<PyPropertyValue>();
        let list = |items: Vec<PyPropertyValue>| {
            let listed = class.call_method1("list", (items,)).unwrap();
            let form = listed.extract::<PyPropertyValue>().unwrap().form;
            (listed, form)
        };
        let tagged = PyPropertyValue::tree(FIELDS.to_vec());
        let one = PyPropertyValue::from_rust(PropertyValue::Unsigned(1));
        let plain = PyPropertyValue::from_rust(PropertyValue::ApplicationData(vec![0x09, 0x01]));
        let (listed, form) = list(vec![tagged.clone()]);
        assert_eq!(form, Form::Tagged);
        assert_eq!(list(vec![tagged.clone(), one]).1, Form::Tagged);
        // Beside plain application data nothing says which item is which.
        assert_eq!(list(vec![tagged, plain]).1, Form::Plain);
        let items: Vec<Bound<'_, PyAny>> = attr(&listed, "value");
        let nodes: Vec<Bound<'_, PyAny>> = items[0].extract().unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(attr::<u8>(&nodes[1], "tag_number"), 1);
    });
}
