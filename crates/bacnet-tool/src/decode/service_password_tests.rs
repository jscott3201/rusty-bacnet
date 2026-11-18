//! A password-bearing request shows nothing of its password (#2074,
//! #2618): not its octets, nor any number read from them, such as a tag
//! number, a length, a fault's text, a nesting or a range.

use bacnet_encoding::tags::{TagClass, encode_closing_tag, encode_opening_tag, encode_tag};
use bytes::BytesMut;
use proptest::collection::vec;
use proptest::prelude::*;

use super::tests::snapshot;

/// `text` as octets, spaces ignored.
fn octets(text: &str) -> Vec<u8> {
    let digits: String = text.split_whitespace().collect();
    (0..digits.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&digits[at..at + 2], 16).unwrap())
        .collect()
}

/// A password that claims too few octets leaves the rest of it to read as
/// tags: a closing tag [6] ('o'), contents claiming 119 octets ("ew"), an
/// extended closing tag [65] ("\xFFA"), all after the password, and one
/// inside a constructed value. None of their numbers or texts show.
#[test]
fn octets_after_a_short_password_show_nothing() {
    let cases = [
        (
            "00050111 1900 2A0041 6F",
            "\
device-communication-control parameters @4..10
  [1] 00 (0) @4..6
  Password: (hidden) @6..9
  Hidden: may hold a password @9..10
  Not read as named fields: reason hidden (it may quote the password) @4..10",
        ),
        (
            "00050111 1900 2A0041 6577",
            "\
device-communication-control parameters @4..11
  [1] 00 (0) @4..6
  Password: (hidden) @6..9
  Hidden: may hold a password @9..11
  Not read as named fields: reason hidden (it may quote the password) @4..11",
        ),
        (
            "00050114 0900 1900 70613535776F7264",
            "\
reinitialize-device parameters @4..16
  [0] 00 (0) @4..6
  Password: (hidden) @6..8
  Hidden: may hold a password @8..16
  Not read as named fields: reason hidden (it may quote the password) @4..16",
        ),
        (
            "00050111 1900 2A0041 FF41",
            "\
device-communication-control parameters @4..11
  [1] 00 (0) @4..6
  Password: (hidden) @6..9
  Hidden: may hold a password @9..11
  Not read as named fields: reason hidden (it may quote the password) @4..11",
        ),
        (
            "00050111 1900 3E 2A0041 6F",
            "\
device-communication-control parameters @4..11
  [1] 00 (0) @4..6
  Hidden: may hold a password @6..11
  Not read as named fields: reason hidden (it may quote the password) @4..11",
        ),
    ];
    for (apdu, expected) in cases {
        let text = snapshot(&octets(apdu));
        assert_eq!(text.trim_end(), expected, "{apdu}");
        for leaked in [
            "[6]",
            "[65]",
            "130",
            "132",
            "closing tag",
            "contents",
            "stopped",
        ] {
            assert!(!text.contains(leaked), "{leaked} in {apdu}:\n{text}");
        }
    }
}

/// How a generated request breaks at or after its password.
#[derive(Debug, Clone, Copy)]
enum Break {
    /// The password claims more octets than follow.
    Short,
    /// It claims fewer: the rest of it reads as tags.
    Long,
    /// It sits inside a constructed [3], closed or not.
    Wrapped { closed: bool },
}

/// A DCC (or ReinitializeDevice) request whose password field claims
/// `claim` octets and is followed by `password`, broken as `how` says,
/// possibly sent as segment 0 of several. Returns it and where
/// the password's tag starts.
fn request(
    reinit: bool,
    segmented: bool,
    claim: u32,
    password: &[u8],
    how: Break,
) -> (Vec<u8>, usize) {
    let service = if reinit { 0x14 } else { 0x11 };
    let mut apdu = if segmented {
        vec![0x0E, 0x05, 0x01, 0x00, 0x04, service]
    } else {
        vec![0x00, 0x05, 0x01, service]
    };
    // The fields before the password: the state, or a duration of 60
    // minutes and enable.
    apdu.extend_from_slice(if reinit {
        &[0x09, 0x00]
    } else {
        &[0x09, 0x3C, 0x19, 0x00]
    });
    let at = apdu.len();
    let mut buf = BytesMut::new();
    let wrapped = matches!(how, Break::Wrapped { .. });
    if wrapped {
        encode_opening_tag(&mut buf, 3);
    }
    encode_tag(
        &mut buf,
        if reinit { 1 } else { 2 },
        TagClass::Context,
        claim,
    );
    buf.extend_from_slice(password);
    if let Break::Wrapped { closed: true } = how {
        encode_closing_tag(&mut buf, 3);
    }
    apdu.extend_from_slice(&buf);
    (apdu, at)
}

/// The labels of `text`'s fields (`label @start..end`) that start at or
/// after `at`.
fn labels_from(text: &str, at: usize) -> Vec<&str> {
    text.lines()
        .filter_map(|line| {
            let (label, range) = line.trim_start().rsplit_once(" @")?;
            let start: usize = range.split("..").next()?.parse().ok()?;
            (start >= at).then_some(label)
        })
        .collect()
}

fn breaks() -> impl Strategy<Value = Break> {
    prop_oneof![
        Just(Break::Short),
        Just(Break::Long),
        any::<bool>().prop_map(|closed| Break::Wrapped { closed }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// Two requests that differ only in their password's octets decode to
    /// the same tree, ranges and marks included: nothing in it is read from
    /// them. And from the password's tag on, no label holds a digit or two
    /// of the password's octets in hex.
    #[test]
    fn nothing_in_the_tree_is_read_from_the_password(
        reinit in any::<bool>(),
        segmented in any::<bool>(),
        how in breaks(),
        claim in 0u32..40,
        first in vec(any::<u8>(), 0..48),
        second in vec(any::<u8>(), 48),
    ) {
        // A short password needs octets claimed and missing; a long one,
        // octets past its claim; the decoder then refuses either way.
        let len = match how {
            Break::Short if claim == 0 => return Ok(()),
            Break::Short => first.len() % claim as usize,
            Break::Long => claim as usize + 1 + first.len() % 8,
            Break::Wrapped { .. } => claim as usize,
        };
        let mut one = first.clone();
        one.resize(len, 0xA5);
        let other = &second.repeat(2)[..len];
        let (apdu, at) = request(reinit, segmented, claim, &one, how);
        let text = snapshot(&apdu);
        let (twin, _) = request(reinit, segmented, claim, other, how);
        prop_assert_eq!(&text, &snapshot(&twin));
        for label in labels_from(&text, at) {
            prop_assert!(!label.chars().any(|c| c.is_ascii_digit()), "{label}\n{text}");
        }
        for pair in one.windows(2) {
            let hex = format!("{:02X}{:02X}", pair[0], pair[1]);
            prop_assert!(!text.contains(&hex), "{hex}\n{text}");
        }
    }
}
