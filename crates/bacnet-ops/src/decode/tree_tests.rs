use bacnet_transport::mstp::codec::FrameType;

use super::{Field, Framing, Range, decode_tree};
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
    let frame = bip(&npdu(&unconfirmed(&[0x3E, 0x11, 0x3F]), false), false);
    let fields = decode_tree(Framing::Bip, &frame);
    assert_eq!(
        find(&fields, "Boolean: true"),
        frame.len() - 2..frame.len() - 1
    );
    assert_eq!(find(&fields, "[3] opening"), frame.len() - 3..frame.len());
    // False Booleans likewise, here a ReadProperty value of two: the
    // first is the one octet before the second.
    let frame = bip(&npdu(&rp_ack_with(7, vec![0x10, 0x10]), false), false);
    let fields = decode_tree(Framing::Bip, &frame);
    assert_eq!(
        find(&fields, "Boolean: false"),
        frame.len() - 3..frame.len() - 2
    );
    assert_eq!(
        find(&fields, "Value: [false, false]"),
        frame.len() - 4..frame.len()
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
    // The parameters, named: an object identifier [0] and a property [1].
    let object = find(&fields, "Object: analog-input 1");
    assert_eq!(frame[object.start], 0x0C);
    assert_eq!(object.len(), 5);
    assert_eq!(find(&fields, "Property: present-value"), 15..17);
    assert_eq!(frame.len(), 17);
}

#[test]
fn an_ack_value_and_an_error_decode_to_their_tags() {
    let frame = bip(&npdu(&rp_ack(7), false), false);
    let fields = decode_tree(Framing::Bip, &frame);
    let value = find(&fields, "Value: 72.5");
    assert_eq!(
        &frame[value.clone()],
        &[0x3E, 0x44, 0x42, 0x91, 0x00, 0x00, 0x3F]
    );

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

/// A character string in the parameters marks its field as device text;
/// the frame's own fields aren't. A bare NPDU and a bare APDU decode with
/// ranges in their own octets.
#[test]
fn a_character_string_is_marked_and_bare_layers_decode() {
    // An application character string, UTF-8: "Fan".
    let params = [0x74, 0x00, b'F', b'a', b'n'];
    let apdu = unconfirmed(&params);
    let data = npdu(&apdu, false);
    let fields = decode_tree(Framing::Bip, &bip(&data, false));
    let mut marked = Vec::new();
    let mut stack: Vec<&Field> = fields.iter().collect();
    while let Some(field) = stack.pop() {
        if field.device_text {
            marked.push(field.label.clone());
        }
        stack.extend(field.children.iter());
    }
    assert_eq!(marked, ["Character string: \"Fan\""]);

    let bare = super::decode_npdu_tree(&data);
    assert_eq!(find(&bare, "NPDU"), 0..2);
    let at = data.len() - params.len();
    assert_eq!(find(&bare, "Character string"), at..data.len());
    let apdu_octets = &data[2..];
    let tree = super::decode_apdu_tree(apdu_octets);
    assert_eq!(tree.range, 0..apdu_octets.len());
    let summary = crate::decode::frame::summarize_npdu(&data);
    assert_eq!(summary.pdu.as_deref(), Some("Unconfirmed-Request"));
}

/// A function octet no table assigns reads `unassigned`; a code the 2020
/// standard removed reads `withdrawn`; an assigned one keeps its name.
#[test]
fn an_unassigned_or_withdrawn_function_is_labelled_so() {
    let cases: [(Framing, &[u8], &str); 6] = [
        (
            Framing::Bip,
            &[0x81, 0x0C, 0x00, 0x04],
            "Function: withdrawn (0x0C)",
        ),
        (
            Framing::Bip,
            &[0x81, 0x7F, 0x00, 0x04],
            "Function: unassigned (0x7F)",
        ),
        (
            Framing::Bip6,
            &[0x82, 0x0B, 0x00, 0x07, 0, 1, 2],
            "Function: withdrawn (0x0B)",
        ),
        (
            Framing::Bip6,
            &[0x82, 0x7F, 0x00, 0x04],
            "Function: unassigned (0x7F)",
        ),
        (
            Framing::Sc,
            &[0x7F, 0x00, 0x00, 0x01],
            "Function: unassigned (0x7F)",
        ),
        (
            Framing::Sc,
            &[0x0C, 0x00, 0x00, 0x01],
            "Function: proprietary-message (0x0C)",
        ),
    ];
    for (framing, frame, label) in cases {
        let fields = decode_tree(framing, frame);
        let labels: Vec<String> = flat(&fields).into_iter().map(|(l, _)| l).collect();
        assert!(
            labels.iter().any(|l| l == label),
            "{framing:?}: {labels:#?}"
        );
    }
}

#[test]
fn hidden_passwords_do_not_change_mstp_tree_labels_or_spans() {
    // A confirmed DCC request with Enable, followed by a one-character password.
    let make = |frame_type, letter| {
        let data = [1, 4, 0, 5, 1, 17, 0x19, 0, 0x2a, 0, letter];
        mstp(frame_type, 7, 3, &data)
    };
    for (frame_type, width, label) in [
        (FrameType::BACNET_DATA_EXPECTING_REPLY, 2, "Data CRC: valid"),
        (
            FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
            5,
            "Encoded CRC-32K: valid",
        ),
    ] {
        let one = make(frame_type, b'A');
        let two = make(frame_type, b'B');
        let end = one.len();
        let crc = end - width..end;
        assert_ne!(one[crc.clone()], two[crc.clone()]);
        let fields = decode_tree(Framing::Mstp, &one);
        assert_eq!(fields, decode_tree(Framing::Mstp, &two));
        let hidden = find(&fields, "Password: (hidden)");
        assert_eq!(find(&fields, label), crc);
        if width == 5 {
            assert_eq!(hidden, 8..end - width);
        }
        // A second frame or unrelated bytes beyond the first frame's CRC
        // cannot extend the field ranges or appear in any label.
        let joined = [one.as_slice(), two.as_slice()].concat();
        assert_eq!(fields, decode_tree(Framing::Mstp, &joined));
        let mut corrupt = one.clone();
        corrupt[end - 1] ^= 1;
        let broken = flat(&decode_tree(Framing::Mstp, &corrupt));
        assert!(
            broken
                .iter()
                .any(|(label, _)| label.contains("data CRC mismatch"))
        );
        assert!(!broken.iter().any(|(label, _)| label.ends_with(": valid")));
    }
}

/// The labels and ranges directly under the MS/TP header node.
fn mstp_header(fields: &[Field]) -> Vec<(String, Range<usize>)> {
    fields[0]
        .children
        .iter()
        .map(|field| (field.label.clone(), field.range.clone()))
        .collect()
}

/// An MS/TP frame whose header is good and whose octets are all there keeps
/// its Data and CRC fields when its data check fails: the CRC field says
/// `mismatch`, never the CRC's value, and nothing is decoded from the data.
/// A truncated frame has neither field, and a second frame after the first
/// never widens a span.
#[test]
fn an_mstp_data_check_shows_its_status_and_span_never_its_value() {
    let data = npdu(&i_am(5), false);
    for (frame_type, width, crc) in [
        (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 2, "Data CRC"),
        (
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            5,
            "Encoded CRC-32K",
        ),
    ] {
        let good = mstp(frame_type, 255, 5, &data);
        let end = good.len();
        let data_label = format!("Data: {} octets", end - width - 8);
        let fields = decode_tree(Framing::Mstp, &good);
        assert_eq!(fields[0].range, 0..end);
        let header = mstp_header(&fields);
        assert_eq!(header[6], (data_label.clone(), 8..end - width));
        assert_eq!(header[7], (format!("{crc}: valid"), end - width..end));
        assert_eq!(fields.len(), 3, "{:#?}", flat(&fields));

        let mut bad = good.clone();
        bad[end - 2] ^= 0x40;
        let fields = decode_tree(Framing::Mstp, &bad);
        let header = mstp_header(&fields);
        assert_eq!(header[5].0, format!("Header CRC: {:02X}", bad[7]));
        assert_eq!(header[6], (data_label.clone(), 8..end - width));
        assert_eq!(header[7], (format!("{crc}: mismatch"), end - width..end));
        assert_eq!(header[8].0, "malformed: data CRC mismatch");
        assert_eq!(header.len(), 9, "{header:#?}");
        assert_eq!(fields.len(), 1, "nothing decoded: {:#?}", flat(&fields));
        // The tree is the same whatever CRC octets arrived.
        let mut other = bad.clone();
        other[end - 1] ^= 0x01;
        assert_eq!(decode_tree(Framing::Mstp, &other), fields);

        // A second frame after a bad first one: every span stops at the
        // first frame's end.
        let joined = [bad.as_slice(), good.as_slice()].concat();
        let after = decode_tree(Framing::Mstp, &joined);
        assert_eq!(after, fields);
        assert!(flat(&after).iter().all(|(_, range)| range.end <= end));

        // Truncated by one octet: header leaves and the fault, no Data.
        let fields = decode_tree(Framing::Mstp, &good[..end - 1]);
        let header = mstp_header(&fields);
        assert_eq!(header.len(), 7, "{header:#?}");
        assert!(header[6].0.starts_with("malformed: truncated"));
        assert!(!header.iter().any(|(label, _)| label.starts_with("Data")));
    }
}

/// A frame with no data (a Token: Length zero, so no data and no data CRC
/// follow the header) has its six header leaves and nothing else, and every
/// span stays inside its eight octets.
#[test]
fn an_mstp_frame_without_data_has_header_leaves_only() {
    let token = mstp(FrameType::TOKEN, 2, 1, &[]);
    assert_eq!(token.len(), 8);
    let fields = decode_tree(Framing::Mstp, &token);
    assert_eq!(fields.len(), 1, "{:#?}", flat(&fields));
    assert_eq!(fields[0].range, 0..8);
    let header = mstp_header(&fields);
    assert_eq!(header.len(), 6, "{header:#?}");
    assert!(!header.iter().any(|(label, _)| label.starts_with("Data")));
    assert!(flat(&fields).iter().all(|(_, range)| range.end <= 8));
}
