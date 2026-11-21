//! The tag walk's fallbacks (#2074): a password-bearing request's segments
//! hidden, and every octet covered when the walk stops early.

use super::super::decode_apdu_tree;
use super::tests::snapshot;

/// `text` as octets, spaces ignored.
fn octets(text: &str) -> Vec<u8> {
    let digits: String = text.split_whitespace().collect();
    (0..digits.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&digits[at..at + 2], 16).unwrap())
        .collect()
}

/// "hunter2" as a UTF-8 character string's content, as hex.
const SECRET: &str = "00 68756E74657232";

/// A segmented request: the first segment's parameters, which no decoder
/// reads whole, are one hidden leaf, and so is a later one's.
#[test]
fn segmented_password_requests_hide_the_password() {
    let cases = [
        // ReinitializeDevice, first segment (sequence 0, window 4).
        (
            format!("0E 05 01 00 04 14 0900 1D08 {SECRET}"),
            "\
reinitialize-device parameters @6..18
  Hidden: may hold a password @6..18",
        ),
        // DeviceCommunicationControl, first segment.
        (
            format!("0E 05 01 00 04 11 0900 1900 2D08 {SECRET}"),
            "\
device-communication-control parameters @6..20
  Hidden: may hold a password @6..20",
        ),
        // A later segment.
        (
            format!("0E 05 01 01 04 11 2D08 {SECRET}"),
            "\
device-communication-control parameters @6..16
  Segment data: 10 octets, hidden (may hold a password) @6..16",
        ),
    ];
    for (apdu, expected) in cases {
        assert_eq!(snapshot(&octets(&apdu)).trim_end(), expected, "{apdu}");
    }
}

/// A stray closing tag stops the walk with a leaf saying so, and a PDU
/// without parameters may carry octets after its header: either way every
/// octet is covered.
#[test]
fn octets_after_the_walk_stops_are_one_leaf() {
    let text = snapshot(&octets("10 03 0901 1C02000005 08 1F"));
    assert!(
        text.contains("\n  Tag walk stopped: closing tag [1] with no opening tag @10..11\n"),
        "{text}"
    );
    let reject = decode_apdu_tree(&octets("60 01 02 AABB"));
    let last = reject.children.last().unwrap();
    assert_eq!(
        (last.label.as_str(), last.range.clone()),
        ("Trailing octets: 2", 3..5)
    );
}
