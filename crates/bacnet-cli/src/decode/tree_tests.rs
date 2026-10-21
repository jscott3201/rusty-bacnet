use bacnet_transport::mstp::codec::FrameType;

use super::{decode_tree, Field, Framing, Range};
use crate::decode::testing::*;

/// Every field's label, depth first, with its range.
fn flat(fields: &[Field]) -> Vec<(String, Range<usize>)> {
    let mut out = Vec::new();
    for field in fields {
        out.push((field.label.clone(), field.range.clone()));
        out.extend(flat(&field.children));
    }
    out
}

fn find(fields: &[Field], prefix: &str) -> Range<usize> {
    flat(fields)
        .into_iter()
        .find(|(label, _)| label.starts_with(prefix))
        .unwrap_or_else(|| panic!("no field {prefix}: {:#?}", flat(fields)))
        .1
}

/// An application Boolean keeps its value in the tag header: it is one
/// octet, and the closing tag after it stays the opening node's.
#[test]
fn a_boolean_is_one_octet_and_leaves_the_closing_tag_alone() {
    let frame = bip(&npdu(&rp_ack_with(7, vec![0x11]), false), false);
    assert_eq!(&frame[frame.len() - 3..], &[0x3E, 0x11, 0x3F]);
    let fields = decode_tree(Framing::Bip, &frame);
    assert_eq!(
        find(&fields, "Boolean: true"),
        frame.len() - 2..frame.len() - 1
    );
    assert_eq!(find(&fields, "[3] opening"), frame.len() - 3..frame.len());
    // A false Boolean likewise.
    let frame = bip(&npdu(&rp_ack_with(7, vec![0x10]), false), false);
    let fields = decode_tree(Framing::Bip, &frame);
    assert_eq!(
        find(&fields, "Boolean: false"),
        frame.len() - 2..frame.len() - 1
    );
}

/// Tag numbers from 15 up take a second header octet, closing tags too.
#[test]
fn a_two_octet_closing_tag_ends_its_node() {
    let params = [0xFE, 0x15, 0x09, 0x01, 0xFF, 0x15, 0x19, 0x02];
    let frame = bip(&npdu(&unconfirmed(&params), false), false);
    let at = frame.len() - params.len();
    let fields = decode_tree(Framing::Bip, &frame);
    assert_eq!(find(&fields, "[21] opening"), at..at + 6);
    assert_eq!(find(&fields, "[0] 01 (1)"), at + 2..at + 4);
    assert_eq!(find(&fields, "[1] 02 (2)"), at + 6..at + 8);
}

#[test]
fn a_bip_read_property_decodes_layer_by_layer_with_its_octets() {
    let frame = bip(&npdu(&rp_request(7), true), false);
    let fields = decode_tree(Framing::Bip, &frame);
    let labels: Vec<&str> = fields.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "BVLL: original-unicast-npdu",
            "NPDU",
            "APDU: Confirmed-Request"
        ]
    );
    assert_eq!(find(&fields, "Function: original-unicast-npdu"), 1..2);
    assert_eq!(find(&fields, "Length: "), 2..4);
    // BVLL 4 octets, NPDU 2: the APDU starts at 6.
    assert_eq!(find(&fields, "Control: 0x04"), 5..6);
    assert_eq!(find(&fields, "Invoke ID: 7"), 8..9);
    assert_eq!(find(&fields, "Service: read-property"), 9..10);
    assert_eq!(&frame[8], &7);
    // The parameters: an object identifier [0] and a property [1].
    let object = find(&fields, "[0] 00000001");
    assert_eq!(frame[object.start], 0x0C);
    assert_eq!(object.len(), 5);
    assert_eq!(find(&fields, "[1] 55 (85)"), 15..17);
    assert_eq!(frame.len(), 17);
}

#[test]
fn an_ack_value_and_an_error_decode_to_their_tags() {
    let frame = bip(&npdu(&rp_ack(7), false), false);
    let fields = decode_tree(Framing::Bip, &frame);
    let value = find(&fields, "Real: 72.5");
    assert_eq!(&frame[value.clone()], &[0x44, 0x42, 0x91, 0x00, 0x00]);
    let opening = find(&fields, "[3] opening");
    assert_eq!(frame[opening.start], 0x3E);
    assert_eq!(frame[opening.end - 1], 0x3F);

    let service = bacnet_types::enums::ConfirmedServiceChoice::READ_PROPERTY;
    let frame = bip(&npdu(&error(9, service), false), false);
    let fields = decode_tree(Framing::Bip, &frame);
    assert_eq!(
        find(&fields, "Error: property:unknown-property").end,
        frame.len()
    );
    find(&fields, "Enumerated: 2");
}

#[test]
fn an_mstp_frame_shows_its_header_and_crcs() {
    let data = npdu(&i_am(5), false);
    let frame = mstp(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 255, 5, &data);
    let fields = decode_tree(Framing::Mstp, &frame);
    assert_eq!(find(&fields, "Preamble"), 0..2);
    assert_eq!(find(&fields, "Destination: 255"), 3..4);
    assert_eq!(find(&fields, "Source: 5"), 4..5);
    assert_eq!(find(&fields, "Header CRC"), 7..8);
    assert_eq!(find(&fields, "Data CRC"), frame.len() - 2..frame.len());
    assert_eq!(find(&fields, "NPDU"), 8..10);
    find(&fields, "Object identifier: device 5");
}

#[test]
fn sc_bip6_and_bad_frames_decode_as_far_as_they_go() {
    let data = npdu(&rp_request(3), true);
    let frame = sc_npdu(&data, None);
    let fields = decode_tree(Framing::Sc, &frame);
    assert_eq!(find(&fields, "Message ID: 4660"), 2..4);
    assert_eq!(find(&fields, "Originating VMAC"), 4..10);
    assert_eq!(find(&fields, "NPDU"), 10..12);

    let frame = bip6_broadcast(&data);
    let fields = decode_tree(Framing::Bip6, &frame);
    assert_eq!(find(&fields, "Source VMAC: bip6:0x000102"), 4..7);
    assert_eq!(find(&fields, "NPDU"), 7..9);

    let fields = decode_tree(Framing::Bip, &[0x81, 0x0A, 0x00, 0x09, 0x01]);
    find(&fields, "malformed");
}
