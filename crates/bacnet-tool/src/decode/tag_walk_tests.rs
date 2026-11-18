//! The tag walk on the shared reader (#2618): each framing fault is a leaf
//! where the walk stopped, after everything read before it, and the
//! reader's bounds are the walk's.

use bacnet_encoding::tags::tree::{MAX_TAG_TREE_DEPTH, MAX_TAG_TREE_NODES};

use super::super::{Field, decode_apdu_tree};

#[path = "tag_walk_property_tests.rs"]
mod property;

/// `text` as octets, spaces ignored.
pub(super) fn octets(text: &str) -> Vec<u8> {
    let digits: String = text.split_whitespace().collect();
    (0..digits.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&digits[at..at + 2], 16).unwrap())
        .collect()
}

/// An Unconfirmed-PrivateTransfer carrying `params`: a service with no
/// named fields, so its parameters always take the tag walk.
pub(super) fn unnamed(params: &[u8]) -> Vec<u8> {
    let mut apdu = vec![0x10, 0x04];
    apdu.extend_from_slice(params);
    apdu
}

fn render(field: &Field, depth: usize, out: &mut String) {
    out.push_str(&format!(
        "{}{} @{}..{}\n",
        "  ".repeat(depth),
        field.label,
        field.range.start,
        field.range.end
    ));
    for child in &field.children {
        render(child, depth + 1, out);
    }
}

/// The parameters node of `apdu`, one field a line.
fn walked(apdu: &[u8]) -> String {
    let tree = decode_apdu_tree(apdu);
    let params = tree.children.last().expect("a parameters node");
    let mut out = String::new();
    render(params, 0, &mut out);
    out.trim_end().to_string()
}

#[track_caller]
fn assert_walk(params: &str, expected: &str) {
    assert_eq!(walked(&unnamed(&octets(params))), expected.trim());
}

/// A closing tag must close the innermost open opening tag of its number
/// (Clause 20.2.1.3.2). One that doesn't stops the walk there: the
/// elements before it stay, the open ones run to the end, and the fault's
/// leaf covers the octets from the closing tag on.
#[test]
fn a_closing_tag_of_another_number_is_a_fault_where_it_sits() {
    assert_walk(
        "0901 1E 2E 2101 3F 1F 0902",
        "
unconfirmed-private-transfer parameters @2..12
  [0] 01 (1) @2..4
  [1] opening @4..12
    [2] opening @5..12
      Unsigned: 1 @6..8
      Tag walk stopped: closing tag [3] inside opening tag [2] @8..12",
    );
    // At the top level, a closing tag closes nothing.
    assert_walk(
        "0901 1F 0902",
        "
unconfirmed-private-transfer parameters @2..7
  [0] 01 (1) @2..4
  Tag walk stopped: closing tag [1] with no opening tag @4..7",
    );
}

/// Octets that end inside a constructed element: what was read stays, and
/// the leaf, which covers no octet, since the walk read them all, sits at
/// the end of the element the walk ended in.
#[test]
fn an_unclosed_opening_tag_is_a_fault_after_its_members() {
    assert_walk(
        "0901 1E 2E 2101 2F 2102",
        "
unconfirmed-private-transfer parameters @2..11
  [0] 01 (1) @2..4
  [1] opening @4..11
    [2] opening @5..9
      Unsigned: 1 @6..8
    Unsigned: 2 @9..11
    Tag walk stopped: opening tag [1] is never closed @11..11",
    );
}

/// A primitive whose length runs past the end, context or application
/// tagged, stops the walk at its header, saying how many octets its
/// contents claim and how many remain: numbers that hold in any frame.
#[test]
fn contents_past_the_end_are_a_fault_at_their_header() {
    assert_walk(
        "0901 1C 0000",
        "
unconfirmed-private-transfer parameters @2..7
  [0] 01 (1) @2..4
  Tag walk stopped: contents claim 4 octets, 2 remain @4..7",
    );
    assert_walk(
        "3E 44 4291",
        "
unconfirmed-private-transfer parameters @2..6
  [3] opening @2..6
    Tag walk stopped: contents claim 4 octets, 2 remain @3..6",
    );
}

/// The same NPDU, cut short inside a value, in a B/IP frame and in an
/// extended MS/TP frame, whose COBS-encoded data puts the NPDU's octets
/// nowhere in the frame: the fault reads the same in both.
#[test]
fn a_fault_reads_the_same_in_any_frame() {
    use bacnet_ops::decode::frame::Framing;
    use bacnet_ops::decode::testing::{bip, mstp};
    use bacnet_transport::mstp::codec::FrameType;

    // A ReadProperty-ACK whose REAL value lost two octets, after an octet
    // string long enough to need an extended frame.
    let mut npdu = octets("0100 3001 0C 0C00000001 19 55 3E 65FE 0200");
    npdu.extend([0xAA; 512]);
    npdu.extend_from_slice(&[0x44, 0x42, 0x91]);
    let stop = |fields: &[Field]| {
        let mut labels = Vec::new();
        let mut stack: Vec<&Field> = fields.iter().collect();
        while let Some(field) = stack.pop() {
            if field.label.starts_with("Tag walk stopped") {
                labels.push(field.label.clone());
            }
            stack.extend(&field.children);
        }
        labels
    };
    let on_bip = stop(&super::super::decode_tree(Framing::Bip, &bip(&npdu, false)));
    let frame = mstp(
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
        1,
        2,
        &npdu,
    );
    let on_mstp = stop(&super::super::decode_tree(Framing::Mstp, &frame));
    assert_eq!(
        on_bip,
        ["Tag walk stopped: contents claim 4 octets, 2 remain"]
    );
    assert_eq!(on_mstp, on_bip);
}

/// A header that doesn't parse (here a length not in its shortest form)
/// is a fault at that header.
#[test]
fn a_malformed_header_is_a_fault_at_it() {
    assert_walk(
        "0901 0D 03 AABBCC",
        "
unconfirmed-private-transfer parameters @2..9
  [0] 01 (1) @2..4
  Tag walk stopped: malformed tag header: non-canonical extended tag length 3 @4..9",
    );
}

/// `depth` opening tags [1] around one primitive, closed again.
fn nested(depth: usize) -> Vec<u8> {
    let mut params = vec![0x1E; depth];
    params.extend_from_slice(&[0x09, 0x07]);
    params.extend(std::iter::repeat_n(0x1F, depth));
    params
}

/// The innermost field of a chain of single-member nodes.
fn innermost(field: &Field) -> (&Field, usize) {
    let mut field = field;
    let mut depth = 0;
    while let [only] = field.children.as_slice() {
        field = only;
        depth += 1;
    }
    (field, depth)
}

/// Inside a parameter, opening tags nest as deep as the reader's bound and
/// no deeper: the one past it is a fault, inside the elements open there.
#[test]
fn opening_tags_nest_to_the_readers_bound_inside_a_parameter() {
    let deepest = MAX_TAG_TREE_DEPTH + 1;
    let tree = decode_apdu_tree(&unnamed(&nested(deepest)));
    let params = tree.children.last().unwrap();
    let (leaf, depth) = innermost(params);
    assert_eq!((leaf.label.as_str(), depth), ("[0] 07 (7)", deepest + 1));

    let apdu = unnamed(&nested(deepest + 1));
    let tree = decode_apdu_tree(&apdu);
    let params = tree.children.last().unwrap();
    let (leaf, depth) = innermost(params);
    assert_eq!(
        leaf.label,
        format!("Tag walk stopped: opening tags nest deeper than {MAX_TAG_TREE_DEPTH}")
    );
    assert_eq!(depth, deepest + 1);
    // From the opening tag past the bound to the end.
    let at = 2 + deepest;
    assert_eq!(leaf.range, at..apdu.len());
}

/// A ReadPropertyMultiple-ACK whose value nests 31 deep inside its [4],
/// as deep as its decoder takes a value: inside the result list [1], 33
/// opening tags in all. Its fields are named, and the value's tags read
/// to the innermost.
#[test]
fn a_value_as_deep_as_its_decoder_takes_keeps_its_named_fields() {
    let mut apdu = octets("30 01 0E 0C00000001 1E 29 55 4E");
    apdu.extend(std::iter::repeat_n(0x0E, 31));
    apdu.extend_from_slice(&[0x21, 0x01]);
    apdu.extend(std::iter::repeat_n(0x0F, 31));
    apdu.extend_from_slice(&[0x4F, 0x1F]);
    let text = walked(&apdu);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        &lines[..4],
        [
            "read-property-multiple parameters @3..78",
            "  Results for analog-input 1 @3..78",
            "    Object: analog-input 1 @3..8",
            "    Properties @8..78",
        ]
    );
    assert!(
        lines[4].starts_with("      present-value: [0] ([0] ("),
        "{text}"
    );
    assert!(lines[6].starts_with("        Value: [0] ([0] ("), "{text}");
    assert_eq!(lines.last().unwrap().trim(), "Unsigned: 1 @43..45");
    assert!(
        !text.contains("stopped") && !text.contains("Not read"),
        "{text}"
    );
}

/// An EVENT_PARAMETERS value in its pre-framing form, which the stack
/// takes from older devices (#2376), isn't tags: in a ReadProperty-ACK,
/// whose decoder delimits it, the fields stay named, the value's tags
/// stopping at its first octets. Where something follows the value (a
/// WriteProperty's priority), the walk can't tell where the value ends, so
/// the parameters are walked tag by tag, saying why.
#[test]
fn a_legacy_value_its_decoder_delimits_keeps_its_named_fields() {
    let apdu = octets("30 01 0C 0C02400001 19 53 3E FEFF 0102 FFFF 3F");
    let text = walked(&apdu);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        [lines[0], lines[1], lines[2]],
        [
            "read-property parameters @3..18",
            "  Object: event-enrollment 1 @3..8",
            "  Property: event-parameters @8..10",
        ]
    );
    assert!(
        lines[3].starts_with("  Value: ") && lines[3].ends_with(" @10..18"),
        "{text}"
    );
    assert_eq!(
        lines[4..],
        ["    Tag walk stopped: malformed tag header: invalid extended tag number 255 @11..17"]
    );

    let apdu = octets("00 05 01 0F 0C02400001 19 53 3E FEFF 0102 FFFF 3F 4908");
    assert_eq!(
        walked(&apdu),
        "
write-property parameters @4..21
  [0] 02400001 (37748737) @4..9
  [1] 53 (83) @9..11
  [3] opening @11..21
    Tag walk stopped: malformed tag header: invalid extended tag number 255 @12..21
  Not read as named fields: tag walk stopped: malformed tag header: invalid extended tag number 255 @4..21"
            .trim()
    );
}

/// The walk holds at most the tree's bound of elements: the one past it
/// is a fault, and the elements before it stay.
#[test]
fn the_walk_holds_at_most_the_trees_bound_of_elements() {
    let params = [0x00].repeat(MAX_TAG_TREE_NODES + 1);
    let apdu = unnamed(&params);
    let tree = decode_apdu_tree(&apdu);
    let fields = &tree.children.last().unwrap().children;
    assert_eq!(fields.len(), MAX_TAG_TREE_NODES + 1);
    assert_eq!(fields[0].label, "Null: null");
    let last = fields.last().unwrap();
    assert_eq!(
        last.label,
        format!("Tag walk stopped: more than {MAX_TAG_TREE_NODES} tagged elements")
    );
    assert_eq!(last.range, apdu.len() - 1..apdu.len());
    // At the bound itself, every element reads.
    let apdu = unnamed(&params[..MAX_TAG_TREE_NODES]);
    let tree = decode_apdu_tree(&apdu);
    let fields = &tree.children.last().unwrap().children;
    assert_eq!(fields.len(), MAX_TAG_TREE_NODES);
    assert!(fields.iter().all(|field| field.label == "Null: null"));
}

/// A ReadProperty-ACK whose value's tags don't frame: the named fields
/// give way to the walk, which keeps what it read and marks where it
/// stopped, beside the decoder's own refusal.
#[test]
fn a_value_whose_tags_dont_frame_falls_back_to_the_walk() {
    // [0] analog-input 1, [1] present-value, [3] ( [1] ( 1, then [2] ).
    let apdu = octets("30 01 0C 0C00000001 19 55 3E 1E 2101 2F 3F");
    assert_eq!(
        walked(&apdu),
        "
read-property parameters @3..16
  [0] 00000001 (1) @3..8
  [1] 55 (85) @8..10
  [3] opening @10..16
    [1] opening @11..16
      Unsigned: 1 @12..14
      Tag walk stopped: closing tag [2] inside opening tag [1] @14..16
  Not read as named fields: decoding error at offset 11: closing tag 2 does not match opening tag 1 @3..16"
            .trim()
    );
}
