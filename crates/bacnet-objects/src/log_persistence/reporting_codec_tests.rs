//! The written reporting settings in a slot (#1952): schema version 2 keeps
//! them after the records, a version 1 slot still loads with none, and
//! every cut, unknown bit or value no write could leave is damage.

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogRecord, BACnetObjectPropertyReference,
};
use bacnet_types::enums::{LoggingType, NotifyType, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::codec::{self, HEADER_LEN, SCHEMA_VERSION_1, TRAILER_LEN};
use super::tests::{event_record, reference, trend_record};
use super::*;
use crate::clock::UNSPECIFIED_DATETIME;
use crate::durable::crc32;

fn oid(kind: ObjectType) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, 5).unwrap()
}

/// Every setting every log keeps, as writes set them.
pub(super) fn every_setting() -> LogReportingSettings {
    LogReportingSettings {
        notification_class: Some(7),
        event_enable: Some(EventTransitionBits::TO_FAULT),
        notify_type: Some(NotifyType::ACK_NOTIFICATION),
        event_detection_enable: Some(false),
        event_message_texts_config: Some(["off".into(), String::new(), "back".into()]),
        event_algorithm_inhibit_ref: Some(BACnetObjectPropertyReference::new(
            oid(ObjectType::BINARY_VALUE),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )),
        event_algorithm_inhibit: Some(true),
        client_cov_increment: None,
        cov_resubscription_interval: None,
        description: None,
    }
}

/// A Trend Log holding two records, with `reporting`.
pub(super) fn trend(reporting: LogReportingSettings) -> LogSnapshot<BACnetLogRecord> {
    LogSnapshot {
        object_identifier: oid(ObjectType::TREND_LOG),
        generation: 3,
        buffer_size: 4,
        log_enable: true,
        stop_when_full: false,
        total_record_count: 2,
        records: vec![trend_record(1), trend_record(2)],
        last_notify_record: 1,
        records_since_notification_from: 1,
        notification_threshold: 2,
        start_time: UNSPECIFIED_DATETIME,
        stop_time: UNSPECIFIED_DATETIME,
        acquisition: Some(LogAcquisitionSettings {
            logging_type: LoggingType::POLLED,
            log_interval: 100,
            align_intervals: false,
            interval_offset: 0,
        }),
        log_device_object_property: vec![reference(1)],
        reporting,
    }
}

/// Every setting a Trend Log keeps: a NULL increment, or a REAL one.
fn trend_settings(increment: Option<f32>) -> LogReportingSettings {
    LogReportingSettings {
        client_cov_increment: Some(increment),
        cov_resubscription_interval: Some(600),
        ..every_setting()
    }
}

/// An Event Log holding one record, with `reporting`.
pub(super) fn event(reporting: LogReportingSettings) -> LogSnapshot<BACnetEventLogRecord> {
    LogSnapshot {
        object_identifier: oid(ObjectType::EVENT_LOG),
        generation: 2,
        buffer_size: 4,
        log_enable: true,
        stop_when_full: false,
        total_record_count: 1,
        records: vec![event_record(1)],
        last_notify_record: 0,
        records_since_notification_from: 0,
        notification_threshold: 0,
        start_time: UNSPECIFIED_DATETIME,
        stop_time: UNSPECIFIED_DATETIME,
        acquisition: None,
        log_device_object_property: Vec::new(),
        reporting,
    }
}

/// Rewrite the declared payload length and the checksum after an edit, so
/// only the edit is wrong.
pub(super) fn reseal(data: &mut [u8]) {
    let payload = (data.len() - HEADER_LEN - TRAILER_LEN) as u32;
    data[22..26].copy_from_slice(&payload.to_be_bytes());
    let at = data.len() - TRAILER_LEN;
    let checksum = crc32(&data[..at]);
    data[at..].copy_from_slice(&checksum.to_be_bytes());
}

/// Where the reporting settings start in `snapshot`'s slot, and the slot.
fn slot_and_block<R: codec::SnapshotRecord>(snapshot: &LogSnapshot<R>) -> (Vec<u8>, usize) {
    let mut bare = snapshot.clone();
    bare.reporting = LogReportingSettings::default();
    // With nothing written the settings are the two-octet mask alone.
    let start = codec::encode(&bare).unwrap().len() - TRAILER_LEN - 2;
    (codec::encode(snapshot).unwrap(), start)
}

pub(super) fn decoded<R: codec::SnapshotRecord + core::fmt::Debug>(
    data: &[u8],
    expected: ObjectIdentifier,
) -> Result<LogSnapshot<R>, Error> {
    match codec::decode::<R>(data, expected) {
        Ok(snapshot) => Ok(snapshot),
        Err(error) if error.is_damage() => Err(error.into()),
        Err(error) => panic!("a sound slot refused: {error:?}"),
    }
}

#[test]
fn every_setting_round_trips_on_each_log() {
    for increment in [None, Some(0.25)] {
        let snapshot = trend(trend_settings(increment));
        let data = codec::encode(&snapshot).unwrap();
        assert_eq!(
            decoded(&data, snapshot.object_identifier).unwrap(),
            snapshot
        );
    }
    let snapshot = event(every_setting());
    let data = codec::encode(&snapshot).unwrap();
    assert_eq!(
        decoded(&data, snapshot.object_identifier).unwrap(),
        snapshot
    );
}

/// The Trend Log's settings take 52 octets after the records: the mask,
/// then each setting in the order the codec documents.
#[test]
fn the_settings_follow_the_records_in_the_documented_layout() {
    let (data, start) = slot_and_block(&trend(trend_settings(Some(0.5))));
    let block = &data[start..data.len() - TRAILER_LEN];
    assert_eq!(block.len(), 52);
    assert_eq!(&block[..2], &0x01FFu16.to_be_bytes());
    assert_eq!(&block[2..6], &7u32.to_be_bytes());
    assert_eq!(block[6], EventTransitionBits::TO_FAULT.bits());
    assert_eq!(&block[7..11], &2u32.to_be_bytes());
    assert_eq!(block[11], 0);
    assert_eq!(&block[12..19], b"\0\0\0\x03off");
    assert_eq!(&block[43..48], &[1, 0x3F, 0, 0, 0]);
    assert_eq!(&block[48..], &600u32.to_be_bytes());
}

/// A slot of the first schema, which ends with the records, loads with no
/// setting written.
#[test]
fn a_version_1_slot_loads_with_nothing_written() {
    let snapshot = trend(LogReportingSettings::default());
    let (mut data, start) = slot_and_block(&snapshot);
    data.drain(start..start + 2);
    data[8..10].copy_from_slice(&SCHEMA_VERSION_1.to_be_bytes());
    reseal(&mut data);
    assert_eq!(
        decoded(&data, snapshot.object_identifier).unwrap(),
        snapshot
    );
    // The version names the layout: settings after the records of a
    // version 1 slot are trailing octets.
    let (mut data, _) = slot_and_block(&trend(trend_settings(None)));
    data[8..10].copy_from_slice(&SCHEMA_VERSION_1.to_be_bytes());
    reseal(&mut data);
    let error = decoded::<BACnetLogRecord>(&data, oid(ObjectType::TREND_LOG)).unwrap_err();
    assert!(
        format!("{error:?}").contains("trailing octets"),
        "{error:?}"
    );
}

/// A slot cut anywhere in the settings, its length and checksum made to
/// match, is damage: every field's end is checked.
#[test]
fn every_cut_of_the_settings_is_damage() {
    let (data, start) = slot_and_block(&trend(trend_settings(Some(0.5))));
    for cut in start..data.len() - TRAILER_LEN {
        let mut cut_slot = data[..cut].to_vec();
        cut_slot.extend_from_slice(&[0; TRAILER_LEN]);
        reseal(&mut cut_slot);
        assert!(
            decoded::<BACnetLogRecord>(&cut_slot, oid(ObjectType::TREND_LOG)).is_err(),
            "a slot cut at {cut} loaded"
        );
    }
}

/// One octet of the Trend Log's settings changed to `value`, at `offset`
/// into them.
fn patched(offset: usize, value: &[u8]) -> Vec<u8> {
    let (mut data, start) = slot_and_block(&trend(trend_settings(Some(0.5))));
    data[start + offset..start + offset + value.len()].copy_from_slice(value);
    reseal(&mut data);
    data
}

#[test]
fn a_value_no_write_could_leave_is_damage() {
    let cases: [(&str, usize, &[u8]); 9] = [
        ("an unknown mask bit", 0, &[0x03, 0xFF]),
        ("an unknown Event_Enable bit", 6, &[0x08]),
        ("an unknown Notify_Type", 7, &[0, 0, 0, 3]),
        ("a non-Boolean Event_Detection_Enable", 11, &[2]),
        ("a string that isn't UTF-8", 16, &[0xFF]),
        ("an over-long string", 12, &[0, 1, 0, 0]),
        ("an unknown increment choice", 43, &[2]),
        (
            "a negative increment",
            44,
            &(-1.0f32).to_bits().to_be_bytes(),
        ),
        ("a zero resubscription interval", 48, &[0, 0, 0, 0]),
    ];
    for (what, offset, value) in cases {
        let data = patched(offset, value);
        assert!(
            decoded::<BACnetLogRecord>(&data, oid(ObjectType::TREND_LOG)).is_err(),
            "{what} loaded"
        );
    }
}

/// The COV settings belong to a Trend Log: an Event Log slot holding one is
/// damage, and an Event Log snapshot holding one never encodes.
#[test]
fn cov_settings_off_a_trend_log_are_refused() {
    let (mut data, start) = slot_and_block(&event(LogReportingSettings::default()));
    data[start..start + 2].copy_from_slice(&0x0100u16.to_be_bytes());
    data.splice(
        data.len() - TRAILER_LEN..data.len() - TRAILER_LEN,
        600u32.to_be_bytes(),
    );
    reseal(&mut data);
    let error = decoded::<BACnetEventLogRecord>(&data, oid(ObjectType::EVENT_LOG)).unwrap_err();
    assert!(format!("{error:?}").contains("COV settings"), "{error:?}");
    let mut written = every_setting();
    written.cov_resubscription_interval = Some(600);
    assert!(codec::encode(&event(written)).is_err());
}

/// What a write would refuse never reaches a slot either.
#[test]
fn a_snapshot_holding_a_setting_no_write_leaves_never_encodes() {
    let unset = BACnetObjectPropertyReference::new(
        ObjectIdentifier::new(ObjectType::BINARY_VALUE, 4_194_303).unwrap(),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    );
    let cases = [
        LogReportingSettings {
            notify_type: Some(NotifyType::from_raw(9)),
            ..every_setting()
        },
        LogReportingSettings {
            event_enable: Some(EventTransitionBits::from_bits_retain(0x10)),
            ..every_setting()
        },
        LogReportingSettings {
            event_algorithm_inhibit_ref: Some(unset),
            ..every_setting()
        },
        LogReportingSettings {
            client_cov_increment: Some(Some(f32::NAN)),
            ..trend_settings(None)
        },
        LogReportingSettings {
            cov_resubscription_interval: Some(0),
            ..trend_settings(None)
        },
        LogReportingSettings {
            event_message_texts_config: Some([
                "x".repeat(16 * 1024 + 1),
                String::new(),
                String::new(),
            ]),
            ..every_setting()
        },
    ];
    for reporting in cases {
        assert!(
            codec::encode(&trend(reporting.clone())).is_err(),
            "{reporting:?} encoded"
        );
    }
}

/// A version 1 slot pair on disk reopens a Trend Log with nothing written,
/// so its setters apply, and the next commit writes version 2.
// std: the file backend.
#[cfg(feature = "std")]
#[test]
fn a_trend_log_reopens_from_version_1_files() {
    use crate::traits::BACnetObject;
    use crate::trend::TrendLogObject;
    use bacnet_types::primitives::PropertyValue;
    use std::sync::Arc;

    let base = super::tests::TempBase::new("v1-reporting");
    let store = FileLogPersistence::new(base.path()).unwrap();
    let mut snapshot = trend(LogReportingSettings::default());
    snapshot.object_identifier = ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap();
    store.commit(&snapshot).unwrap();
    // Generation 3 lands in slot 1; make it a version 1 slot.
    let slot = store.slot_paths()[1].clone();
    let mut data = std::fs::read(&slot).unwrap();
    let at = data.len() - TRAILER_LEN - 2;
    data.drain(at..at + 2);
    data[8..10].copy_from_slice(&SCHEMA_VERSION_1.to_be_bytes());
    reseal(&mut data);
    std::fs::write(&slot, &data).unwrap();

    let mut log = TrendLogObject::with_persistence(1, "TL", 4, Arc::new(store.clone())).unwrap();
    assert!(log.reopened());
    assert_eq!(log.written_reporting(), LogReportingSettings::default());
    log.set_notification_class(4);
    assert_eq!(
        log.read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
            .unwrap(),
        PropertyValue::Unsigned(4)
    );
    log.write_property(
        PropertyIdentifier::NOTIFY_TYPE,
        None,
        PropertyValue::Enumerated(NotifyType::ALARM.to_raw()),
        None,
    )
    .unwrap();
    drop(log);
    let written = std::fs::read(&store.slot_paths()[0]).unwrap();
    assert_eq!(&written[8..10], &codec::SCHEMA_VERSION.to_be_bytes());
    let loaded = LogPersistence::<BACnetLogRecord>::load(&store, snapshot.object_identifier)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.reporting.notify_type, Some(NotifyType::ALARM));
    assert_eq!(loaded.reporting.notification_class, None);
}
