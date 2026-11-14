//! The tag walk's fallbacks (#2074): passwords hidden at any depth and in
//! segments, and every octet covered when the walk stops early.

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

/// The snapshot, checked to hold no trace of the secret.
#[track_caller]
fn hidden(apdu: &str) -> String {
    let text = snapshot(&octets(apdu));
    for leaked in ["hunter2", "68756E74", "756E74"] {
        assert!(!text.contains(leaked), "{leaked} in\n{text}");
    }
    text.trim_end().to_string()
}

/// A segmented request: the first segment is walked with the password
/// hidden, a later one is a single opaque leaf.
#[test]
fn segmented_password_requests_hide_the_password() {
    // ReinitializeDevice, first segment (sequence 0, window 4).
    let reinit = format!("0E 05 01 00 04 14 0900 1D08 {SECRET}");
    assert_eq!(
        hidden(&reinit),
        "\
reinitialize-device parameters @6..18
  [0] 00 (0) @6..8
  Password: (hidden) @8..18"
    );
    // DeviceCommunicationControl, first segment.
    let dcc = format!("0E 05 01 00 04 11 0900 1900 2D08 {SECRET}");
    assert_eq!(
        hidden(&dcc),
        "\
device-communication-control parameters @6..20
  [0] 00 (0) @6..8
  [1] 00 (0) @8..10
  Password: (hidden) @10..20"
    );
    // A later segment: its tag boundaries can't be trusted.
    let later = format!("0E 05 01 01 04 11 2D08 {SECRET}");
    assert_eq!(
        hidden(&later),
        "\
device-communication-control parameters @6..16
  Segment data: 10 octets, hidden (may hold a password) @6..16"
    );
}

/// Parameters the decoder refuses hide a character string at any depth,
/// application or context tagged, and ReinitializeDevice's tag 1.
#[test]
fn refused_password_requests_hide_strings_at_any_depth() {
    // DCC with the password inside a constructed [2].
    assert_eq!(
        hidden(&format!("00 05 01 11 1900 2E 7508 {SECRET} 2F")),
        "\
device-communication-control parameters @4..18
  [1] 00 (0) @4..6
  [2] opening @6..18
    Hidden: may hold a password @7..17
  Not read as named fields: decoding error at offset 2: DCC: 12 trailing byte(s) @4..18"
    );
    // ... and inside [3].
    let text = hidden(&format!("00 05 01 11 1900 3E 7508 {SECRET} 3F"));
    assert!(
        text.contains("  [3] opening @6..18\n    Hidden: may hold a password @7..17"),
        "{text}"
    );
    // An application character string at the top level.
    let text = hidden(&format!("00 05 01 11 1900 7508 {SECRET}"));
    assert!(
        text.contains("  Hidden: may hold a password @6..16"),
        "{text}"
    );
    // ReinitializeDevice with a password past 20 characters.
    let long = "00 5A71372D68756E746572322D5A71372D68756E746572322D";
    let text = snapshot(&octets(&format!("00 05 01 14 0900 1D19 {long}")));
    assert!(!text.contains("5A71372D"), "{text}");
    assert!(text.contains("  Password: (hidden) @6..33"), "{text}");
    assert!(text.contains("exceeds 20 characters"), "{text}");
}

/// A password whose length claims too little leaves the rest of it to read
/// as more tags: everything from the password tag on is hidden, so the
/// rest can't pass as a duration or a state.
#[test]
fn everything_from_the_password_on_is_hidden() {
    assert_eq!(
        hidden("00 05 01 11 1900 2A0068 0C756E7465"),
        "\
device-communication-control parameters @4..14
  [1] 00 (0) @4..6
  Password: (hidden) @6..9
  Hidden: may hold a password @9..14
  Not read as named fields: decoding error at offset 5: DCC: 5 trailing byte(s) @4..14"
    );
    let text = hidden("00 05 01 14 0900 1A0068 0B756E74");
    assert!(
        text.contains("  Password: (hidden) @6..9\n  Hidden: may hold a password @9..13"),
        "{text}"
    );
}

/// Only the fields before the password, at the top level and at their
/// sizes, stay readable: a short [0] inside a constructed value, a time
/// duration of five octets, and every field after it are hidden.
#[test]
fn only_expected_top_level_fields_stay() {
    let text = hidden("00 05 01 11 1900 3E0907 3F");
    assert!(text.contains("  [1] 00 (0) @4..6\n"), "{text}");
    assert!(
        text.contains("  [3] opening @6..10\n    Hidden: may hold a password @7..9"),
        "{text}"
    );
    let text = hidden("00 05 01 11 0D05 0102030405 1900");
    assert!(
        text.contains(
            "  Hidden: may hold a password @4..11\n  Hidden: may hold a password @11..13"
        ),
        "{text}"
    );
}

/// A stray closing tag ends the walk early, and a PDU without parameters
/// may carry octets after its header: either way every octet is covered.
#[test]
fn octets_after_the_walk_stops_are_one_leaf() {
    let text = snapshot(&octets("10 03 0901 1C02000005 08 1F"));
    assert!(text.contains("\n  Trailing octets: 1 @10..11\n"), "{text}");
    let reject = decode_apdu_tree(&octets("60 01 02 AABB"));
    let last = reject.children.last().unwrap();
    assert_eq!(
        (last.label.as_str(), last.range.clone()),
        ("Trailing octets: 2", 3..5)
    );
}
