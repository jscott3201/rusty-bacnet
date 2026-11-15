//! `tag_tree_decode`: the generic walk of tagged octets (#2618).
//!
//! - `TagReader` yields items in wire order, each one's octets the input's
//!   from its offset, end to end with the one before it; contents a suffix
//!   of the octets; depths within `MAX_TAG_TREE_DEPTH`, a closing tag at
//!   its opening tag's depth. It stops at its first fault and yields
//!   nothing after it.
//! - `decode_tag_tree` agrees with the reader: it accepts what the reader
//!   accepts and refuses with the reader's own fault, unless it first
//!   reaches an element past `MAX_TAG_TREE_NODES`, before any fault the
//!   reader meets.
//! - An accepted tree covers the input exactly, and encodes back to it
//!   through the crate's tag encoders (each header again, each primitive's
//!   contents, each constructed element's members between its tags); a
//!   constructed node's contents are its members' octets end to end.
//! - A refusal's `valid_up_to` is at or before its offset, and the input
//!   up to it reads as a tree that encodes back to it.
//! - An application node's octets, read with `decode_application_value`,
//!   give a `PropertyValue` that encodes to the same tag number.

use bacnet_encoding::primitives::{decode_application_value, encode_property_value};
use bacnet_encoding::tags::tree::{
    MAX_TAG_TREE_DEPTH, MAX_TAG_TREE_NODES, TagBody, TagNode, TagReader, TagTreeError,
    TagTreeFault, decode_tag_tree,
};
use bacnet_encoding::tags::{decode_tag, encode_closing_tag, encode_opening_tag, encode_tag};
use bytes::BytesMut;

/// Runs every check on `input`.
pub fn check(input: &[u8]) {
    let read = reader(input);
    match decode_tag_tree(input) {
        Ok(tree) => {
            assert!(read.is_ok(), "the tree accepted what the reader refused");
            assert!(count(&tree) <= MAX_TAG_TREE_NODES);
            assert_eq!(encoded(&tree), input, "the tree doesn't encode back");
            members(&tree);
        }
        Err(error) => {
            assert!(error.valid_up_to <= error.offset && error.offset <= input.len());
            match read {
                // The bound stops the tree at an element the reader reads.
                Err((_, stop)) if error.fault == TagTreeFault::TooManyNodes => {
                    assert!(stop > error.offset, "the tree counted past a fault");
                }
                Ok(()) => assert_eq!(error.fault, TagTreeFault::TooManyNodes),
                Err((first, _)) => assert_eq!(first, error, "the tree and the reader disagree"),
            }
            let prefix = &input[..error.valid_up_to];
            let tree = decode_tag_tree(prefix).expect("what read before a fault reads again");
            assert_eq!(encoded(&tree), prefix);
        }
    }
}

/// The reader's walk, checked item by item; its first fault and where the
/// reader stood when it found it.
fn reader(input: &[u8]) -> Result<(), (TagTreeError, usize)> {
    let mut reader = TagReader::new(input);
    let mut at = 0;
    let mut depth = 0usize;
    while let Some(item) = reader.next() {
        let item = match item {
            Ok(item) => item,
            Err(error) => {
                let stop = reader.offset();
                assert_eq!(reader.next(), None, "the reader went on after a fault");
                return Err((error, stop));
            }
        };
        assert_eq!(
            item.offset, at,
            "an item doesn't start where the last ended"
        );
        assert_eq!(item.octets, &input[at..item.end()]);
        assert!(item.octets.ends_with(item.contents));
        assert!(item.depth <= MAX_TAG_TREE_DEPTH);
        if item.tag.is_opening {
            assert_eq!(item.depth, depth);
            depth += 1;
        } else if item.tag.is_closing {
            depth -= 1;
            assert_eq!(item.depth, depth);
        } else {
            assert_eq!(item.depth, depth);
        }
        at = item.end();
    }
    assert_eq!((at, depth), (input.len(), 0), "the reader stopped early");
    Ok(())
}

/// How many elements `nodes` hold, members included.
fn count(nodes: &[TagNode<'_>]) -> usize {
    nodes.iter().map(|node| 1 + count(node.children())).sum()
}

/// Each constructed node's contents are its members' octets; each
/// application value converts and encodes under its own tag number.
fn members(nodes: &[TagNode<'_>]) {
    for node in nodes {
        match &node.body {
            TagBody::Constructed(inner) => {
                let joined: Vec<u8> = inner.iter().flat_map(|n| n.octets.to_vec()).collect();
                assert_eq!(node.contents(), joined.as_slice());
                members(inner);
            }
            TagBody::Application(_) => {
                if let Ok((owned, _)) = decode_application_value(node.octets, 0) {
                    let mut buf = BytesMut::new();
                    encode_property_value(&mut buf, &owned).expect("a decoded value encodes");
                    let (tag, _) = decode_tag(&buf, 0).expect("an encoded header");
                    assert_eq!(tag.number, node.number);
                }
            }
            _ => {}
        }
    }
}

/// The octets of `nodes` from their parts.
fn encoded(nodes: &[TagNode<'_>]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    for node in nodes {
        if node.is_constructed() {
            encode_opening_tag(&mut buf, node.number);
            buf.extend_from_slice(&encoded(node.children()));
            encode_closing_tag(&mut buf, node.number);
        } else {
            // An application BOOLEAN's value is its header's length field.
            let (tag, _) = decode_tag(node.octets, 0).expect("a node's header");
            encode_tag(&mut buf, node.number, node.class, tag.length);
            buf.extend_from_slice(node.contents());
        }
    }
    buf.to_vec()
}
