//! A written Description in a slot (#2364): schema version 3 adds it after
//! the reporting settings, a slot without one keeps version 2's octets, and
//! damage to it is refused.

use bacnet_types::constructed::BACnetLogRecord;

use super::codec::{self, HEADER_LEN, SCHEMA_VERSION, SCHEMA_VERSION_DESCRIBED, TRAILER_LEN};
use super::reporting_codec_tests::{decoded, event, every_setting, reseal, trend};
use super::*;
use crate::snapshot::SnapshotErrorKind;

fn described(text: &str) -> LogReportingSettings {
    LogReportingSettings {
        description: Some(text.into()),
        ..every_setting()
    }
}

fn version(slot: &[u8]) -> u16 {
    u16::from_be_bytes([slot[8], slot[9]])
}

/// The slot's payload: past the header, before the checksum.
fn payload(slot: &[u8]) -> &[u8] {
    &slot[HEADER_LEN..slot.len() - TRAILER_LEN]
}

/// A slot with a Description is version 3: version 2's payload with the
/// Description's length and octets after it. Without one it stays version
/// 2, and both round-trip.
#[test]
fn a_description_follows_the_reporting_settings_in_version_3() {
    let plain = codec::encode(&trend(every_setting())).unwrap();
    assert_eq!(version(&plain), SCHEMA_VERSION);
    let snapshot = trend(described("Lobby"));
    let slot = codec::encode(&snapshot).unwrap();
    assert_eq!(version(&slot), SCHEMA_VERSION_DESCRIBED);
    assert_eq!(slot[..8], plain[..8]);
    assert_eq!(slot[10..22], plain[10..22]);
    let mut expected = payload(&plain).to_vec();
    expected.extend_from_slice(&[0, 0, 0, 5]);
    expected.extend_from_slice(b"Lobby");
    assert_eq!(payload(&slot), expected);
    assert_eq!(
        decoded(&slot, snapshot.object_identifier).unwrap(),
        snapshot
    );

    let snapshot = event(described(""));
    let slot = codec::encode(&snapshot).unwrap();
    assert_eq!(version(&slot), SCHEMA_VERSION_DESCRIBED);
    assert_eq!(
        decoded(&slot, snapshot.object_identifier).unwrap(),
        snapshot
    );
}

/// A Description alone, with no other setting written, is kept too.
#[test]
fn a_description_alone_round_trips() {
    let settings = LogReportingSettings {
        description: Some("Lobby".into()),
        ..LogReportingSettings::default()
    };
    let snapshot = event(settings);
    let slot = codec::encode(&snapshot).unwrap();
    assert_eq!(
        decoded(&slot, snapshot.object_identifier).unwrap(),
        snapshot
    );
}

/// 16 KiB is kept; one octet more is refused on encode and as damage in a
/// slot, as are octets that aren't UTF-8 and a slot cut inside it. A
/// version past 3 is a version this build doesn't read.
#[test]
fn a_damaged_description_is_refused() {
    let longest = "d".repeat(16 * 1024);
    let snapshot = trend(described(&longest));
    let slot = codec::encode(&snapshot).unwrap();
    assert_eq!(
        decoded(&slot, snapshot.object_identifier).unwrap(),
        snapshot
    );
    let error = codec::encode(&trend(described(&format!("{longest}d")))).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);

    let object = snapshot.object_identifier;
    let slot = codec::encode(&trend(described("Lobby"))).unwrap();
    let text_at = slot.len() - TRAILER_LEN - 5;
    let mut past_cap = slot.clone();
    past_cap[text_at - 4..text_at].copy_from_slice(&(16 * 1024 + 1_u32).to_be_bytes());
    reseal(&mut past_cap);
    assert!(decoded::<BACnetLogRecord>(&past_cap, object).is_err());

    let mut not_utf8 = slot.clone();
    not_utf8[text_at] = 0xff;
    reseal(&mut not_utf8);
    assert!(decoded::<BACnetLogRecord>(&not_utf8, object).is_err());

    for cut in 1..=9 {
        let mut short = slot.clone();
        short.drain(slot.len() - TRAILER_LEN - cut..slot.len() - TRAILER_LEN);
        reseal(&mut short);
        assert!(
            decoded::<BACnetLogRecord>(&short, object).is_err(),
            "cut {cut}"
        );
    }

    // Version 2 holds no Description, so one after the settings is damage.
    let mut as_version_2 = slot.clone();
    as_version_2[8..10].copy_from_slice(&SCHEMA_VERSION.to_be_bytes());
    reseal(&mut as_version_2);
    assert!(decoded::<BACnetLogRecord>(&as_version_2, object).is_err());

    let mut later = slot;
    later[8..10].copy_from_slice(&4_u16.to_be_bytes());
    reseal(&mut later);
    let error = codec::decode::<BACnetLogRecord>(&later, object);
    assert_eq!(
        error.unwrap_err().kind(),
        SnapshotErrorKind::UnknownVersion(4)
    );
}
