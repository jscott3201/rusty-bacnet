//! The State_Text row of the written-configuration format's kind 6
//! (#2414): a count, then each label as its length and its UTF-8. Its
//! octets, its caps, that a count or a length is held to its cap and to
//! the file before anything is allocated or read, and that a row no write
//! leaves is refused on encode and as damage on decode. Which types keep
//! it, and that an older build refuses it as an unknown kind, are in
//! `parameters_tests` with the other rows.

use bacnet_types::enums::ObjectType;

use super::parameters_tests::{STATE_TEXT, encode, oid, snapshot, with_tail};
use super::{SnapshotErrorKind, write_config};
use crate::multistate::{MAX_KEPT_STATE_TEXT_OCTETS, MAX_NUMBER_OF_STATES};
use crate::write_config::AlgorithmWriteConfig;

const KINDS: [ObjectType; 3] = [
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
];

fn holding(texts: Vec<String>) -> AlgorithmWriteConfig {
    AlgorithmWriteConfig {
        state_text: Some(texts),
        ..Default::default()
    }
}

/// The tail of a file whose State_Text row states `count` and then holds
/// `octets`.
fn tail(count: u32, octets: &[u8]) -> Vec<u8> {
    let mut tail = STATE_TEXT.to_be_bytes().to_vec();
    tail.extend_from_slice(&count.to_be_bytes());
    tail.extend_from_slice(octets);
    tail
}

/// One label's octets: its length, then `text`.
fn label(text: &[u8]) -> Vec<u8> {
    let mut octets = (text.len() as u32).to_be_bytes().to_vec();
    octets.extend_from_slice(text);
    octets
}

fn decoded(kind: ObjectType, tail: &[u8]) -> Result<AlgorithmWriteConfig, SnapshotErrorKind> {
    match write_config::decode(oid(kind), &with_tail(kind, tail)) {
        Ok(crate::write_config::WriteConfigSnapshot::Reporting(config)) => Ok(config.algorithm),
        Ok(other) => panic!("{other:?}"),
        Err(error) => {
            assert_eq!(
                error.is_damage(),
                error.kind() != SnapshotErrorKind::OutOfRange
            );
            Err(error.kind())
        }
    }
}

/// The largest State_Text an object keeps, every label at its cap, saves
/// within the format's 1 MiB and reads back; so do one empty label, and
/// labels past ASCII.
#[test]
fn the_largest_state_text_fits_the_file_and_reads_back() {
    let full: Vec<String> = (0..MAX_NUMBER_OF_STATES)
        .map(|state| format!("{:03}", state % 1000).repeat(MAX_KEPT_STATE_TEXT_OCTETS / 3 + 1))
        .map(|text| text[..MAX_KEPT_STATE_TEXT_OCTETS].to_string())
        .collect();
    for kind in KINDS {
        for texts in [
            full.clone(),
            vec![String::new()],
            vec!["ä".into(), "€".into()],
        ] {
            let octets = encode(kind, holding(texts.clone())).unwrap();
            assert!(octets.len() <= write_config::MAX_LEN);
            let back = write_config::decode(oid(kind), &octets).unwrap();
            assert_eq!(back, snapshot(holding(texts)), "{kind}");
        }
    }
}

/// The row's octets, spelled out: the count, then each label's length and
/// UTF-8.
#[test]
fn a_row_is_its_count_then_each_labels_length_and_octets() {
    let texts = vec!["Off".to_string(), "Ein".to_string()];
    for kind in KINDS {
        let plain = encode(kind, AlgorithmWriteConfig::default()).unwrap();
        let octets = encode(kind, holding(texts.clone())).unwrap();
        let mut expected = tail(2, &label(b"Off"));
        expected.extend_from_slice(&label(b"Ein"));
        assert_eq!(octets[plain.len()..], expected, "{kind}");
    }
}

/// A State_Text no write leaves storage holding (no label, more than 1,024
/// labels, a label past 512 octets) is refused as it is encoded, and as
/// damage in a file: a count past the cap before a label is read, a label
/// length past the cap before its octets are.
#[test]
fn a_state_text_past_its_caps_is_refused() {
    for kind in KINDS {
        let refused = [
            vec![],
            vec!["x".to_string(); MAX_NUMBER_OF_STATES as usize + 1],
            vec!["x".repeat(MAX_KEPT_STATE_TEXT_OCTETS + 1)],
        ];
        for texts in refused {
            let error = encode(kind, holding(texts)).unwrap_err();
            assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange, "{kind}");
        }
        let one = label(b"x");
        assert_eq!(
            decoded(kind, &tail(0, &[])),
            Err(SnapshotErrorKind::Damaged)
        );
        let past = MAX_NUMBER_OF_STATES + 1;
        assert_eq!(
            decoded(kind, &tail(past, &one)),
            Err(SnapshotErrorKind::Damaged)
        );
        assert_eq!(
            decoded(kind, &tail(u32::MAX, &one)),
            Err(SnapshotErrorKind::Damaged)
        );
        let long = vec![b'x'; MAX_KEPT_STATE_TEXT_OCTETS + 1];
        let file = with_tail(kind, &tail(1, &label(&long)));
        let error = write_config::decode(oid(kind), &file).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::Damaged, "{kind}");
        // Refused at the length, before the octets are read.
        assert!(
            error.to_string().contains("longer than its cap"),
            "{kind}: {error}"
        );
        let at_cap = vec![b'x'; MAX_KEPT_STATE_TEXT_OCTETS];
        assert!(decoded(kind, &tail(1, &label(&at_cap))).is_ok(), "{kind}");
    }
}

/// A count the file's octets can't hold, at four octets a label at least,
/// is refused as cut short before anything is allocated for it; a label
/// whose length goes past the file, a row cut inside a label, and octets
/// after the last label are refused too, none as a shorter State_Text.
#[test]
fn a_row_the_file_doesnt_hold_is_refused() {
    for kind in KINDS {
        let one = label(b"Off");
        let file = with_tail(kind, &tail(MAX_NUMBER_OF_STATES, &one));
        let error = write_config::decode(oid(kind), &file).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::Truncated, "{kind}");
        // Refused at the count, before a label is read or room made.
        assert!(error.to_string().contains("count needs"), "{kind}: {error}");
        let mut past_the_file = 400u32.to_be_bytes().to_vec();
        past_the_file.extend_from_slice(b"Off");
        let error = decoded(kind, &tail(1, &past_the_file));
        assert_eq!(error, Err(SnapshotErrorKind::Truncated), "{kind}");
        let mut two = label(b"Off");
        two.extend_from_slice(&label(b"Ein"));
        for cut in 0..two.len() {
            assert!(
                decoded(kind, &tail(2, &two[..cut])).is_err(),
                "{kind} {cut}"
            );
        }
        let mut longer = one.clone();
        longer.push(b'!');
        let error = decoded(kind, &tail(1, &longer));
        assert_eq!(error, Err(SnapshotErrorKind::Damaged), "{kind}");
        assert!(decoded(kind, &tail(2, &two)).is_ok());
    }
}

/// A label must be UTF-8, the text a write leaves: octets that aren't are
/// damage, and so is a label cut inside a character.
#[test]
fn a_label_that_isnt_utf_8_is_damage() {
    for kind in KINDS {
        for octets in [
            &[0xFF][..],
            &[b'a', 0xC3],
            &[0xED, 0xA0, 0x80],
            &[0xC0, 0x80],
        ] {
            let error = decoded(kind, &tail(1, &label(octets)));
            assert_eq!(error, Err(SnapshotErrorKind::Damaged), "{kind} {octets:?}");
        }
    }
}
