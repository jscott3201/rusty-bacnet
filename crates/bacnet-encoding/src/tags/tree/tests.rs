//! The tag reader and the tag tree on hand-built octets, every fault and
//! bound, and properties over generated trees and arbitrary octets.

use super::*;

/// The items of `data`, or the first fault.
fn items(data: &[u8]) -> Result<Vec<TagItem<'_>>, TagTreeError> {
    TagReader::new(data).collect()
}

/// The fault `data` stops at.
fn fault(data: &[u8]) -> TagTreeError {
    items(data).expect_err("the octets are malformed")
}

#[test]
fn items_come_in_wire_order_with_their_depth_and_octets() {
    // [0] 2, [1] ( REAL 1.5, [2] ( TRUE ) ), NULL.
    let data = [
        0x09, 0x02, 0x1E, 0x44, 0x3F, 0xC0, 0x00, 0x00, 0x2E, 0x11, 0x2F, 0x1F, 0x00,
    ];
    let read = items(&data).unwrap();
    let shape: Vec<_> = read
        .iter()
        .map(|item| (item.offset, item.depth, item.octets.len(), item.contents))
        .collect();
    assert_eq!(
        shape,
        [
            (0, 0, 2, &[0x02][..]),
            (2, 0, 1, &[][..]),
            (3, 1, 5, &[0x3F, 0xC0, 0x00, 0x00][..]),
            (8, 1, 1, &[][..]),
            (9, 2, 1, &[][..]),
            (10, 1, 1, &[][..]),
            (11, 0, 1, &[][..]),
            (12, 0, 1, &[][..]),
        ]
    );
    assert!(read[1].tag.is_opening && read[6].tag.is_closing);
    assert!(read[0].is_primitive() && !read[3].is_primitive());
    assert_eq!(read[2].end(), 8);
    // Each item's octets are the input's, in order, end to end.
    let joined: Vec<u8> = read.iter().flat_map(|item| item.octets.to_vec()).collect();
    assert_eq!(joined, data);
    // Typing is on request, for application tags only.
    assert_eq!(read[0].application_value(), None);
    assert_eq!(
        read[2].application_value(),
        Some(Ok(ApplicationValueRef::Real(1.5)))
    );
    assert_eq!(
        read[4].application_value(),
        Some(Ok(ApplicationValueRef::Boolean(true)))
    );
    assert_eq!(items(&[]).unwrap(), []);
}

#[test]
fn extended_tag_numbers_and_lengths_read() {
    // Clause 20.2.1.2: [254] opens and closes in two octets; Clause
    // 20.2.1.3.1: lengths of 5 to 253, 254 to 65535 and 65536 up take one,
    // three and five octets after the initial one.
    let mut data = vec![0xFE, 254, 0xF9, 20, 0x07, 0xFF, 254];
    data.extend_from_slice(&[0x0D, 5, 1, 2, 3, 4, 5]);
    data.extend_from_slice(&[0x0D, 254, 0x01, 0x2C]);
    data.extend(core::iter::repeat_n(0xAA, 300));
    data.extend_from_slice(&[0x6D, 255, 0x00, 0x01, 0x11, 0x70]);
    data.extend(core::iter::repeat_n(0x55, 70_000));
    let read = items(&data).unwrap();
    let lengths: Vec<_> = read
        .iter()
        .map(|item| (item.tag.number, item.contents.len()))
        .collect();
    assert_eq!(
        lengths,
        [(254, 0), (20, 1), (254, 0), (0, 5), (0, 300), (6, 70_000)]
    );
    assert_eq!(read[1].octets, [0xF9, 20, 0x07]);
    assert_eq!(read.last().unwrap().end(), data.len());
}

#[test]
fn every_fault_says_where_and_what_read_before_it() {
    // A header cut short: an extended tag number with no number octet.
    let error = fault(&[0x21, 0x01, 0xF9]);
    assert!(matches!(error.fault, TagTreeFault::Header(refused)
        if refused.kind == DecodingKind::Missing && refused.offset == 3));
    assert_eq!((error.offset, error.valid_up_to), (2, 2));
    // A reserved application form (L/V/T 6) and a non-canonical length.
    assert!(matches!(fault(&[0x26]).fault, TagTreeFault::Header(_)));
    assert!(matches!(
        fault(&[0x0D, 0x03, 1, 2, 3]).fault,
        TagTreeFault::Header(_)
    ));
    // Contents past the end.
    let error = fault(&[0x21, 0x01, 0x1E, 0x44, 0x3F]);
    assert_eq!(error.fault, TagTreeFault::Truncated { need: 8, have: 5 });
    assert_eq!((error.offset, error.valid_up_to), (3, 2));
    // An element never closed: the innermost open one.
    let error = fault(&[0x09, 0x01, 0x1E, 0x2E, 0x21, 0x01, 0x2F]);
    assert_eq!(error.fault, TagTreeFault::Unclosed { number: 1 });
    assert_eq!((error.offset, error.valid_up_to), (2, 2));
    let error = fault(&[0x1E, 0x2E]);
    assert_eq!(error.fault, TagTreeFault::Unclosed { number: 2 });
    assert_eq!((error.offset, error.valid_up_to), (1, 0));
    // A closing tag with nothing open, and one of another number.
    let error = fault(&[0x21, 0x01, 0x1F]);
    let none = TagTreeFault::UnmatchedClosing {
        number: 1,
        open: None,
    };
    assert_eq!((error.fault, error.offset, error.valid_up_to), (none, 2, 2));
    let error = fault(&[0x0E, 0x21, 0x01, 0x2F]);
    let other = TagTreeFault::UnmatchedClosing {
        number: 2,
        open: Some(0),
    };
    assert_eq!(
        (error.fault, error.offset, error.valid_up_to),
        (other, 3, 0)
    );
    // The reader stops at the first fault and stays stopped.
    let mut reader = TagReader::new(&[0x1F, 0x21, 0x01]);
    assert!(matches!(reader.next(), Some(Err(_))));
    assert_eq!(reader.next(), None);
    assert_eq!(reader.next(), None);
}

/// `depth` opening tags of numbers 0 to 14 in turn, then their closing
/// tags.
fn nested(depth: usize) -> Vec<u8> {
    let numbers: Vec<u8> = (0..depth).map(|level| (level % 15) as u8).collect();
    let mut data: Vec<u8> = numbers.iter().map(|n| (n << 4) | 0x0E).collect();
    data.push(0x00);
    data.extend(numbers.iter().rev().map(|n| (n << 4) | 0x0F));
    data
}

#[test]
fn opening_tags_nest_up_to_the_bound() {
    let data = nested(MAX_TAG_TREE_DEPTH);
    let deepest = items(&data).unwrap();
    assert_eq!(
        deepest.iter().map(|item| item.depth).max(),
        Some(MAX_TAG_TREE_DEPTH)
    );
    let error = fault(&nested(MAX_TAG_TREE_DEPTH + 1));
    assert_eq!(error.fault, TagTreeFault::TooDeep);
    assert_eq!((error.offset, error.valid_up_to), (MAX_TAG_TREE_DEPTH, 0));
}

#[test]
fn faults_convert_to_the_crate_error_and_display() {
    let kind = |data: &[u8]| match Error::from(fault(data)) {
        Error::Decoding(refused) => Some((refused.kind, refused.offset)),
        _ => None,
    };
    assert_eq!(kind(&[0x1E]), Some((DecodingKind::Missing, 0)));
    assert_eq!(
        kind(&[0x21, 0x01, 0x1F]),
        Some((DecodingKind::InvalidTag, 2))
    );
    assert_eq!(kind(&[0x0E, 0x1F]), Some((DecodingKind::InvalidTag, 1)));
    assert_eq!(
        kind(&nested(MAX_TAG_TREE_DEPTH + 1)),
        Some((DecodingKind::InvalidTag, MAX_TAG_TREE_DEPTH))
    );
    assert_eq!(kind(&[0xF9]), Some((DecodingKind::Missing, 1)));
    assert!(matches!(
        Error::from(fault(&[0x44, 0x3F])),
        Error::BufferTooShort { need: 5, have: 2 }
    ));
    assert_eq!(
        fault(&[0x0E, 0x2F]).to_string(),
        "tagged octets at offset 1: closing tag [2] inside opening tag [0]"
    );
    assert_eq!(
        fault(&[0x3E]).to_string(),
        "tagged octets at offset 0: opening tag [3] is never closed"
    );
}

#[cfg(feature = "alloc")]
mod tree {
    use super::*;
    use crate::primitives::{decode_application_value, encode_property_value};
    use crate::tags::{encode_closing_tag, encode_opening_tag, encode_tag};
    use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
    use bytes::BytesMut;
    use proptest::prelude::*;

    /// The octets of `nodes` from their parts: each header encoded again,
    /// each primitive's contents and each constructed element's members.
    fn encoded(nodes: &[TagNode<'_>]) -> Vec<u8> {
        let mut buf = BytesMut::new();
        for node in nodes {
            if node.is_constructed() {
                encode_opening_tag(&mut buf, node.number);
                buf.extend_from_slice(&encoded(node.children()));
                encode_closing_tag(&mut buf, node.number);
            } else {
                // An application BOOLEAN's value is its header's length.
                let (tag, _) = crate::tags::decode_tag(node.octets, 0).unwrap();
                encode_tag(&mut buf, node.number, node.class, tag.length);
                buf.extend_from_slice(node.contents());
            }
        }
        buf.to_vec()
    }

    #[test]
    fn a_tree_types_application_values_and_keeps_context_octets() {
        // [5] ( [0] 0x0102, Unsigned 72, [1] ( "hi", object ), [2] ), REAL.
        let mut data = vec![0x5E, 0x0A, 0x01, 0x02, 0x21, 72, 0x1E];
        data.extend_from_slice(&[0x73, 0x00, b'h', b'i', 0xC4, 0x00, 0x00, 0x00, 0x07]);
        data.extend_from_slice(&[0x1F, 0x28, 0x5F, 0x44, 0x42, 0x91, 0x00, 0x00]);
        let tree = decode_tag_tree(&data).unwrap();
        assert_eq!(tree.len(), 2);
        let outer = &tree[0];
        assert_eq!(
            (outer.class, outer.number, outer.offset),
            (TagClass::Context, 5, 0)
        );
        assert_eq!(outer.octets, &data[..19]);
        assert_eq!(outer.contents(), &data[1..18]);
        let members = outer.children();
        assert_eq!(members[0].body, TagBody::Context(&[0x01, 0x02]));
        assert_eq!(members[0].contents(), [0x01, 0x02]);
        assert_eq!(
            members[1].body,
            TagBody::Application(ApplicationValueRef::Unsigned(72))
        );
        assert_eq!(
            (members[1].class, members[1].number),
            (TagClass::Application, 2)
        );
        assert_eq!(members[1].contents(), [72]);
        let inner = members[2].children();
        let ApplicationValueRef::CharacterString(text) = (match inner[0].body {
            TagBody::Application(value) => value,
            ref other => panic!("expected a string, got {other:?}"),
        }) else {
            panic!("expected a string");
        };
        assert_eq!(text.as_str(), Some("hi"));
        assert_eq!(
            inner[1].body,
            TagBody::Application(ApplicationValueRef::ObjectIdentifier(
                ObjectIdentifier::decode(&[0, 0, 0, 7]).unwrap()
            ))
        );
        // An empty context primitive.
        assert_eq!(members[3].body, TagBody::Context(&[]));
        assert!(members[3].children().is_empty() && !members[3].is_constructed());
        assert_eq!(
            tree[1].body,
            TagBody::Application(ApplicationValueRef::Real(72.5))
        );
        assert_eq!(encoded(&tree), data);
        assert_eq!(decode_tag_tree(&[]).unwrap(), []);
    }

    #[test]
    fn well_framed_application_values_that_dont_decode_stay_octets() {
        // A REAL of three octets, a reserved application tag (13), a
        // BOOLEAN whose L/V/T is 2, an ENUMERATED past 32 bits, then NULL.
        let data = [
            0x43, 1, 2, 3, 0xD1, 9, 0x12, 0x95, 0x05, 1, 0, 0, 0, 0, 0x00,
        ];
        let tree = decode_tag_tree(&data).unwrap();
        let invalid: Vec<_> = tree
            .iter()
            .map(|node| match node.body {
                TagBody::Invalid { contents, error } => Some((contents, error.offset)),
                _ => None,
            })
            .collect();
        assert_eq!(
            invalid,
            [
                Some((&[1u8, 2, 3][..], 0)),
                Some((&[9][..], 4)),
                Some((&[][..], 6)),
                Some((&[1, 0, 0, 0, 0][..], 9)),
                None,
            ]
        );
        assert_eq!(
            tree[4].body,
            TagBody::Application(ApplicationValueRef::Null)
        );
        assert_eq!(encoded(&tree), data);
    }

    #[test]
    fn a_tree_holds_at_most_its_bound_of_elements() {
        // MAX_TAG_TREE_NODES NULLs read; one more is past the bound.
        let mut data = vec![0x00; MAX_TAG_TREE_NODES];
        assert_eq!(decode_tag_tree(&data).unwrap().len(), MAX_TAG_TREE_NODES);
        data.push(0x00);
        let error = decode_tag_tree(&data).unwrap_err();
        assert_eq!(error.fault, TagTreeFault::TooManyNodes);
        assert_eq!(
            (error.offset, error.valid_up_to),
            (MAX_TAG_TREE_NODES, MAX_TAG_TREE_NODES)
        );
        assert!(matches!(
            Error::from(error),
            Error::Decoding(refused) if refused.kind == DecodingKind::Overflow
        ));
        // A constructed element counts once, its members each once: the
        // bound falls inside it, and what read before it is the prefix.
        let mut data = vec![0x00, 0x0E];
        data.extend(core::iter::repeat_n(0x00, MAX_TAG_TREE_NODES - 1));
        data.push(0x0F);
        let error = decode_tag_tree(&data).unwrap_err();
        assert_eq!((error.offset, error.valid_up_to), (MAX_TAG_TREE_NODES, 1));
        assert_eq!(
            decode_tag_tree(&data[..error.valid_up_to]).unwrap().len(),
            1
        );
        // The reader alone doesn't count.
        assert!(items(&data).is_ok());
        // The bound comes first inside an element that never closes.
        let mut data = vec![0x1E];
        data.extend(core::iter::repeat_n(0x00, MAX_TAG_TREE_NODES));
        let error = decode_tag_tree(&data).unwrap_err();
        assert_eq!(error.fault, TagTreeFault::TooManyNodes);
        assert_eq!(fault(&data).fault, TagTreeFault::Unclosed { number: 1 });
        check_any(&data);
    }

    #[test]
    fn a_tree_nests_as_deep_as_the_reader() {
        let data = nested(MAX_TAG_TREE_DEPTH);
        let deepest = decode_tag_tree(&data).unwrap();
        let mut node = &deepest[0];
        let mut depth = 1;
        while let [inner] = node.children() {
            node = inner;
            depth += 1;
        }
        // The innermost node is the NULL inside the last opening tag.
        assert_eq!(depth, MAX_TAG_TREE_DEPTH + 1);
        assert_eq!(
            decode_tag_tree(&nested(MAX_TAG_TREE_DEPTH + 1))
                .unwrap_err()
                .fault,
            TagTreeFault::TooDeep
        );
    }

    /// A generated element: an application value, a context primitive, or
    /// a constructed element of them.
    #[derive(Debug, Clone)]
    enum Gen {
        Application(PropertyValue),
        Context(u8, Vec<u8>),
        Constructed(u8, Vec<Gen>),
    }

    fn application() -> impl Strategy<Value = PropertyValue> {
        prop_oneof![
            Just(PropertyValue::Null),
            any::<bool>().prop_map(PropertyValue::Boolean),
            any::<u64>().prop_map(PropertyValue::Unsigned),
            any::<i32>().prop_map(PropertyValue::Signed),
            any::<f32>().prop_map(PropertyValue::Real),
            any::<u32>().prop_map(PropertyValue::Enumerated),
            proptest::collection::vec(any::<u8>(), 0..300).prop_map(PropertyValue::OctetString),
            "[a-z ]{0,20}".prop_map(PropertyValue::CharacterString),
        ]
    }

    fn element() -> impl Strategy<Value = Gen> {
        let leaf = prop_oneof![
            application().prop_map(Gen::Application),
            (any::<u8>(), proptest::collection::vec(any::<u8>(), 0..20))
                .prop_map(|(number, contents)| Gen::Context(number.min(254), contents)),
        ];
        leaf.prop_recursive(6, 64, 8, |inner| {
            (any::<u8>(), proptest::collection::vec(inner, 0..8))
                .prop_map(|(number, members)| Gen::Constructed(number.min(254), members))
        })
    }

    fn encode_gen(elements: &[Gen], buf: &mut BytesMut) {
        for element in elements {
            match element {
                Gen::Application(value) => encode_property_value(buf, value).unwrap(),
                Gen::Context(number, contents) => {
                    encode_tag(buf, *number, TagClass::Context, contents.len() as u32);
                    buf.extend_from_slice(contents);
                }
                Gen::Constructed(number, members) => {
                    encode_opening_tag(buf, *number);
                    encode_gen(members, buf);
                    encode_closing_tag(buf, *number);
                }
            }
        }
    }

    /// Whether `node` is what `element` encoded.
    fn matches_gen(node: &TagNode<'_>, element: &Gen) -> bool {
        match (element, &node.body) {
            (Gen::Application(value), TagBody::Application(_)) => {
                // A NaN equals nothing: compare the encodings.
                let (read, _) = decode_application_value(node.octets, 0).unwrap();
                let mut again = BytesMut::new();
                encode_property_value(&mut again, &read).unwrap();
                let mut first = BytesMut::new();
                encode_property_value(&mut first, value).unwrap();
                first == again
            }
            (Gen::Context(number, contents), TagBody::Context(read)) => {
                node.number == *number && read == contents
            }
            (Gen::Constructed(number, members), TagBody::Constructed(read)) => {
                node.number == *number
                    && read.len() == members.len()
                    && read.iter().zip(members).all(|(n, m)| matches_gen(n, m))
            }
            _ => false,
        }
    }

    /// The invariants of whatever the tree and the reader make of `data`.
    fn check_any(data: &[u8]) {
        let reader = items(data);
        match decode_tag_tree(data) {
            Ok(tree) => {
                assert!(reader.is_ok());
                assert_eq!(encoded(&tree), data);
                let joined: Vec<u8> = tree.iter().flat_map(|n| n.octets.to_vec()).collect();
                assert_eq!(joined, data);
            }
            Err(error) => {
                assert!(error.valid_up_to <= error.offset && error.offset <= data.len());
                let prefix = decode_tag_tree(&data[..error.valid_up_to]).unwrap();
                assert_eq!(encoded(&prefix), &data[..error.valid_up_to]);
                match reader {
                    // The bound stops the tree before any fault the reader
                    // meets later.
                    Err(first) if error.fault == TagTreeFault::TooManyNodes => {
                        assert!(first.offset >= error.valid_up_to);
                    }
                    Ok(_) => assert_eq!(error.fault, TagTreeFault::TooManyNodes),
                    Err(first) => assert_eq!(first, error),
                }
            }
        }
    }

    proptest! {
        #[test]
        fn generated_trees_read_back_and_encode_again(
            elements in proptest::collection::vec(element(), 0..6)
        ) {
            let mut buf = BytesMut::new();
            encode_gen(&elements, &mut buf);
            let tree = decode_tag_tree(&buf).unwrap();
            prop_assert_eq!(tree.len(), elements.len());
            for (node, element) in tree.iter().zip(&elements) {
                prop_assert!(matches_gen(node, element), "{:?} vs {:?}", node, element);
            }
            prop_assert_eq!(encoded(&tree), buf.to_vec());
            check_any(&buf);
        }

        #[test]
        fn arbitrary_octets_never_panic_and_keep_the_invariants(
            data in proptest::collection::vec(any::<u8>(), 0..64)
        ) {
            check_any(&data);
        }

        #[test]
        fn cut_short_trees_say_what_read(
            elements in proptest::collection::vec(element(), 1..5),
            cut in any::<prop::sample::Index>(),
        ) {
            let mut buf = BytesMut::new();
            encode_gen(&elements, &mut buf);
            let end = cut.index(buf.len() + 1);
            check_any(&buf[..end]);
        }
    }
}
