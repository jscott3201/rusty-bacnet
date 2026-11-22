//! The Audit Log keeps a written Description in its own snapshot (#2407):
//! schema version 4, written only while it holds one. The shared checks run
//! on it through [`cases`]; the ones here are the format's own: the slot's
//! version, and a log whose storage predates the version.

use bacnet_types::enums::ObjectType;

use super::*;
use crate::audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot};
use crate::snapshot::audit_log;

impl AuditLogPersistence for Memory<AuditLogSnapshot> {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.saved())
    }
    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        self.store(snapshot)
    }
}

kept_object!(
    AuditLogObject,
    |log| Some(log),
    |log, text| log.set_description(text),
    |log| Some(log.clear_written_description())
);

/// The state a log already saved, with logging off so Buffer_Size takes a
/// write, and `description` as its Description: a log opened on it saves
/// nothing until a write.
fn saved_state(description: Option<String>) -> AuditLogSnapshot {
    AuditLogSnapshot {
        object_identifier: oid(ObjectType::AUDIT_LOG),
        generation: 1,
        capacity: 8,
        log_enable: false,
        total_record_count: 0,
        records: Vec::new(),
        completed_receipts: Vec::new(),
        description,
    }
}

fn open(storage: Arc<Memory<AuditLogSnapshot>>) -> Result<Box<dyn KeptObject>, Error> {
    Ok(Box::new(AuditLogObject::new(1, "AL", 8, storage)?))
}

/// The slot's tag and its schema version: the tag stays `RBALOG01`, so the
/// version is what a Description changes.
fn tag(saved: &AuditLogSnapshot) -> [u8; 8] {
    tag_of(audit_log::encode(saved).map(|octets| octets[2..].to_vec()))
}

/// The Audit Log, on storage holding `saved` as its Description.
pub(super) fn cases(saved: Option<&str>) -> Vec<Box<dyn Kept>> {
    vec![Box::new(Case {
        storage: Memory::holding(Some(saved_state(saved.map(String::from)))),
        open,
        description: |saved: &AuditLogSnapshot| saved.description.as_ref(),
        other: (P::BUFFER_SIZE, PropertyValue::Unsigned(4)),
        tag: Some(tag),
    })]
}

/// A slot holding a written Description is schema version 4; the same log
/// without one writes version 3, octet for octet what it wrote before.
#[test]
fn only_a_described_slot_takes_version_4() {
    let plain = saved_state(None);
    let octets = audit_log::encode(&plain).unwrap();
    assert_eq!(octets[8..10], audit_log::VERSION.to_be_bytes());
    assert_eq!(audit_log::VERSION, 3);
    let described = saved_state(Some("lobby".into()));
    let described_octets = audit_log::encode(&described).unwrap();
    assert_eq!(
        described_octets[8..10],
        audit_log::VERSION_WITH_DESCRIPTION.to_be_bytes()
    );
    assert_eq!(audit_log::VERSION_WITH_DESCRIPTION, 4);
    let object = oid(ObjectType::AUDIT_LOG);
    assert_eq!(
        audit_log::decode(object, &described_octets).unwrap(),
        described
    );
    assert_eq!(audit_log::decode(object, &octets).unwrap(), plain);
}

/// Storage a release before #2407 saved has no Description: the log opens
/// on it, serves the configured text, and keeps writing version 3 until a
/// peer writes one.
#[test]
fn a_log_saved_before_the_description_was_kept_opens() {
    let storage = Memory::holding(Some(saved_state(None)));
    let mut log = AuditLogObject::new(1, "AL", 8, storage.clone()).unwrap();
    log.set_description("configured");
    assert_eq!(description(&log), text("configured"));
    write(&mut log, P::BUFFER_SIZE, PropertyValue::Unsigned(4)).unwrap();
    assert_eq!(storage.saved().unwrap().description, None);
    write(&mut log, P::DESCRIPTION, text("lobby")).unwrap();
    assert_eq!(log.written_description(), Some("lobby"));
    let saved = storage.saved().unwrap();
    assert_eq!(saved.description.as_deref(), Some("lobby"));
    assert_eq!(saved.capacity, 4);
}

/// A slot of version 4 that holds no Description, or one past the cap, is
/// refused: no save writes either.
#[test]
fn a_version_4_slot_must_hold_a_description_within_its_cap() {
    let object = oid(ObjectType::AUDIT_LOG);
    // Version 3's octets under version 4's number, sealed again.
    let mut octets = audit_log::encode(&saved_state(None)).unwrap();
    octets[8..10].copy_from_slice(&4u16.to_be_bytes());
    reseal(&mut octets);
    let error = audit_log::decode(object, &octets).unwrap_err();
    assert!(error.is_damage(), "{error}");

    let long = "d".repeat(16 * 1024 + 1);
    assert!(audit_log::encode(&saved_state(Some(long.clone()))).is_err());
    let mut octets = audit_log::encode(&saved_state(Some("d".repeat(16 * 1024)))).unwrap();
    // Grow the stored length past the cap, with the text to match.
    let at = octets.len() - 4 - 16 * 1024 - 4;
    octets.truncate(octets.len() - 4);
    octets[at..at + 4].copy_from_slice(&(16 * 1024 + 1u32).to_be_bytes());
    octets.push(b'd');
    let payload = (octets.len() - 26) as u32;
    octets[22..26].copy_from_slice(&payload.to_be_bytes());
    octets.extend_from_slice(&[0; 4]);
    reseal(&mut octets);
    let error = audit_log::decode(object, &octets).unwrap_err();
    assert!(error.is_damage(), "{error}");
}

/// Write a slot's CRC-32 again for what it holds.
fn reseal(octets: &mut [u8]) {
    let end = octets.len() - 4;
    let checksum = crate::durable::crc32(&octets[..end]);
    octets[end..].copy_from_slice(&checksum.to_be_bytes());
}
