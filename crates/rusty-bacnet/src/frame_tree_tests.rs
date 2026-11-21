use super::*;
use bacnet_ops::decode::frame::Framing;

/// A DeviceCommunicationControl request on B/IP whose nine-character
/// password ends in `last`.
fn dcc(last: u8) -> Vec<u8> {
    let mut frame = vec![
        0x81, 0x0A, 0x00, 0x18, 0x01, 0x04, 0x00, 0x05, 0x01, 0x11, 0x19, 0x00, 0x2D, 0x0A, 0x00,
    ];
    frame.extend_from_slice(b"secret12");
    frame.push(last);
    frame
}

fn module(py: Python<'_>) -> Bound<'_, PyModule> {
    let module = PyModule::new(py, "frames").unwrap();
    register(&module).unwrap();
    module
}

/// Trees from frames that differ only in a hidden password are equal and
/// hash alike, root by root and as whole tuples; another frame's are not.
#[test]
fn trees_compare_and_hash_by_value_all_the_way_down() {
    Python::initialize();
    Python::attach(|py| {
        let decode = module(py).getattr("decode_frame_tree").unwrap();
        let one = decode.call1((dcc(b'3'), "bip")).unwrap();
        let two = decode.call1((dcc(b'4'), "bip")).unwrap();
        assert!(one.eq(&two).unwrap());
        assert_eq!(one.hash().unwrap(), two.hash().unwrap());
        let apdu = one.get_item(2).unwrap();
        assert!(apdu.eq(two.get_item(2).unwrap()).unwrap());
        // Another invoke ID: the APDU roots share label and span, and
        // differ only in a child.
        let mut another = dcc(b'3');
        another[10] = 2;
        let other = decode.call1((another, "bip")).unwrap();
        let other_apdu = other.get_item(2).unwrap();
        assert_eq!(
            apdu.getattr("label").unwrap().to_string(),
            "APDU: Confirmed-Request"
        );
        assert!(
            apdu.getattr("span")
                .unwrap()
                .eq(other_apdu.getattr("span").unwrap())
                .unwrap()
        );
        assert!(!apdu.eq(&other_apdu).unwrap());
        assert!(!one.eq(&other).unwrap());
        assert!(!apdu.eq(one.get_item(0).unwrap()).unwrap());
        assert!(!apdu.eq("APDU").unwrap());
    });
}

/// The repr names the label, span and child count, and never shows octets.
#[test]
fn a_repr_is_short_and_shows_no_octets() {
    Python::initialize();
    Python::attach(|py| {
        let decode = module(py).getattr("decode_frame_tree").unwrap();
        let roots = decode.call1((dcc(b'3'), "bip")).unwrap();
        assert_eq!(
            roots.get_item(0).unwrap().repr().unwrap().to_string(),
            "<FieldNode 'BVLL: original-unicast-npdu' 0..4, 3 children>"
        );
        let text = roots.repr().unwrap().to_string();
        assert!(!text.contains("secret"), "{text}");
        // The password's span runs from its tag at 12 to the frame's end.
        let hidden = "<FieldNode 'Password: (hidden)' 12..24, 0 children>";
        let mut stack = vec![roots.get_item(2).unwrap()];
        let mut found = false;
        while let Some(node) = stack.pop() {
            found |= node.repr().unwrap() == hidden;
            for child in node.getattr("children").unwrap().try_iter().unwrap() {
                stack.push(child.unwrap());
            }
        }
        assert!(found, "no {hidden}");
    });
}

/// `copy` and `pickle` rebuild an equal tree, and a rebuilt span must not
/// end before it starts.
#[test]
fn a_tree_copies_and_pickles_to_an_equal_tree() {
    Python::initialize();
    Python::attach(|py| {
        let decode = module(py).getattr("decode_frame_tree").unwrap();
        let roots = decode.call1((dcc(b'3'), "bip")).unwrap();
        let copy = py.import("copy").unwrap();
        let copied = copy.call_method1("deepcopy", (&roots,)).unwrap();
        assert!(copied.eq(&roots).unwrap());
        let root = roots.get_item(2).unwrap();
        let (rebuild, arguments): (Bound<'_, PyAny>, Bound<'_, PyTuple>) =
            root.call_method0("__reduce__").unwrap().extract().unwrap();
        let again = rebuild.call1(&arguments).unwrap();
        assert!(again.eq(&root).unwrap());
        assert_eq!(again.hash().unwrap(), root.hash().unwrap());
        let error = rebuild.call1(("x", (2, 1), (), false)).unwrap_err();
        assert!(error.is_instance_of::<PyValueError>(py), "{error}");
    });
}

/// Input longer than the link's largest frame is refused; input of exactly
/// that length decodes.
#[test]
fn input_past_the_largest_frame_is_refused() {
    Python::initialize();
    Python::attach(|py| {
        let decode = module(py).getattr("decode_frame_tree").unwrap();
        for (link, framing) in [
            ("bip", Framing::Bip),
            ("ipv6", Framing::Bip6),
            ("sc", Framing::Sc),
            ("mstp", Framing::Mstp),
        ] {
            let limit = framing.max_frame_length();
            decode.call1((vec![0u8; limit], link)).unwrap();
            let error = decode.call1((vec![0u8; limit + 1], link)).unwrap_err();
            assert!(error.is_instance_of::<PyValueError>(py), "{link}: {error}");
            assert!(error.to_string().contains("longer than"), "{error}");
        }
        assert_eq!(Framing::Bip.max_frame_length(), 65_535);
        assert_eq!(Framing::Mstp.max_frame_length(), 2_053);
    });
}
