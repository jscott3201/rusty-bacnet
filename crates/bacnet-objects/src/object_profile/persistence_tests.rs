use super::test_support::*;
use super::*;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

struct FileCase {
    directory: PathBuf,
    store: FileTagsPersistence,
}

impl FileCase {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "rusty-tags-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let store = FileTagsPersistence::new(directory.join("tags.bin")).unwrap();
        Self { directory, store }
    }
}

impl Drop for FileCase {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

#[test]
fn file_snapshot_presence_identity_and_independent_bytes() {
    let case = FileCase::new();
    assert_eq!(case.store.load(oid()).unwrap(), None);
    for saved in [
        TagsSnapshot::default(),
        TagsSnapshot { tags: Some(vec![]) },
        snapshot("floor"),
    ] {
        case.store.save(oid(), &saved).unwrap();
        assert_eq!(case.store.load(oid()).unwrap(), Some(saved.clone()));
        let bytes = std::fs::read(case.store.path()).unwrap();
        assert_eq!(&bytes[..8], b"RBNTAG01");
        assert_eq!(&bytes[8..12], &[0x0f, 0xc0, 0x00, 0x01]); // Color 63, instance 1.
        let expected = match &saved.tags {
            None => vec![0],
            Some(tags) if tags.is_empty() => vec![1],
            Some(_) => vec![1, 0x0d, 6, 0, b'f', b'l', b'o', b'o', b'r'],
        };
        assert_eq!(bytes[12..], expected);
    }
    let other = ObjectIdentifier::new(ObjectType::COLOR_TEMPERATURE, 1).unwrap();
    assert!(case.store.load(other).is_err());
}

#[test]
fn semantic_primitive_and_datetime_tags_round_trip_without_reinterpretation() {
    let case = FileCase::new();
    let saved = TagsSnapshot {
        tags: Some(vec![
            BACnetNameValue::semantic("exhaust"),
            BACnetNameValue::valued("floor", TagValue::Primitive(PropertyValue::Unsigned(3))),
            BACnetNameValue::valued(
                "when",
                TagValue::DateTime {
                    date: Date {
                        year: 126,
                        month: 10,
                        day: 8,
                        day_of_week: 4,
                    },
                    time: Time {
                        hour: 12,
                        minute: 34,
                        second: 56,
                        hundredths: 7,
                    },
                },
            ),
        ]),
    };
    case.store.save(oid(), &saved).unwrap();
    assert_eq!(case.store.load(oid()).unwrap(), Some(saved));
}

#[test]
fn malformed_version_presence_trailing_tags_and_over_count_are_refused() {
    let case = FileCase::new();
    case.store.save(oid(), &TagsSnapshot::default()).unwrap();
    let header = std::fs::read(case.store.path()).unwrap()[..12].to_vec();
    let mut bad_version = header.clone();
    bad_version[7] = b'2';
    bad_version.push(0);
    std::fs::write(case.store.path(), bad_version).unwrap();
    assert!(case.store.load(oid()).is_err());
    for body in [
        vec![],
        vec![2],
        vec![0, 0],
        vec![1, 0],
        vec![1, 0x0d],
        vec![1, 0x0a, 0, b';'],
        [vec![1], [0x09, 0].repeat(MAX_TAGS + 1)].concat(),
    ] {
        std::fs::write(case.store.path(), [&header[..], &body].concat()).unwrap();
        assert!(case.store.load(oid()).is_err(), "accepted {body:?}");
    }
}

#[test]
fn exact_persisted_size_cap_succeeds_and_over_cap_never_replaces_old_file() {
    let case = FileCase::new();
    // A long UTF-8 name uses a 6-byte context header and one charset octet.
    let name_bytes = MAX_TAGS_SNAPSHOT_BYTES - 12 - 1 - 7;
    let exact = snapshot(&"x".repeat(name_bytes));
    let encoded = persistence::encode_snapshot(&exact).unwrap();
    assert_eq!(encoded.len() + 12, MAX_TAGS_SNAPSHOT_BYTES);
    case.store.save(oid(), &exact).unwrap();
    assert_eq!(
        std::fs::metadata(case.store.path()).unwrap().len(),
        MAX_TAGS_SNAPSHOT_BYTES as u64
    );
    assert_eq!(case.store.load(oid()).unwrap(), Some(exact));
    assert_code(
        case.store
            .save(oid(), &snapshot(&"x".repeat(name_bytes + 1))),
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
    );
    assert_eq!(
        std::fs::metadata(case.store.path()).unwrap().len(),
        MAX_TAGS_SNAPSHOT_BYTES as u64
    );
    std::fs::write(case.store.path(), vec![0; MAX_TAGS_SNAPSHOT_BYTES + 1]).unwrap();
    assert!(case.store.load(oid()).is_err());
}

#[test]
fn refused_file_save_leaves_previous_snapshot_and_memory_only_has_no_byte_cap() {
    let case = FileCase::new();
    case.store.save(oid(), &snapshot("old")).unwrap();
    // A directory at the backend's temporary path makes file creation fail.
    let temporary = case.store.path().with_file_name("tags.bin.tmp");
    std::fs::create_dir(&temporary).unwrap();
    assert!(case.store.save(oid(), &snapshot("new")).is_err());
    assert_eq!(case.store.load(oid()).unwrap(), Some(snapshot("old")));
    let mut memory = ProfileState::default();
    memory.provision(profile("configured")).unwrap();
    // Stay within the existing per-tag decoder sanity limit while the
    // complete representation exceeds the new persisted-file bound.
    apply(
        &mut memory,
        &whole(&"x".repeat(MAX_TAGS_SNAPSHOT_BYTES - 1)),
    )
    .unwrap();
    assert!(memory.capability().is_none());
    assert!(FileTagsPersistence::new("").is_err());
}
