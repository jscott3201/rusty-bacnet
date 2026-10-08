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

// Context-0 name "a", then a Date and a Time. Errata 2024-04-29 item 37
// permits either primitive alone, but not their pair as one NameValue.
const DATE_TIME_TAG: &[u8] = &[0x0a, 0, b'a', 0xa4, 126, 10, 6, 2, 0xb4, 12, 30, 0, 0];

fn assert_pair_write_refused(index: Option<u32>) {
    let case = FileCase::new();
    case.store.save(oid(), &snapshot("old")).unwrap();
    let original = std::fs::read(case.store.path()).unwrap();
    let mut state =
        ProfileState::persistent(oid(), std::sync::Arc::new(case.store.clone())).unwrap();
    state.provision(profile("configured")).unwrap();
    // A valid first tag must not publish when a later tag is invalid.
    let bytes = if index.is_none() {
        [&[0x0a, 0, b'b'][..], DATE_TIME_TAG].concat()
    } else {
        DATE_TIME_TAG.to_vec()
    };
    assert_code(
        apply(
            &mut state,
            &write(index, PropertyValue::ApplicationData(bytes)),
        ),
        ErrorCode::INVALID_DATA_ENCODING,
    );
    state.wait_for_saves();
    assert_eq!(state.profile().tags, Some(tags("old")));
    assert_eq!(std::fs::read(case.store.path()).unwrap(), original);
    assert_eq!(case.store.load(oid()).unwrap(), Some(snapshot("old")));
}

#[test]
fn combined_datetime_whole_write_is_refused_without_replacing_file_or_served_tags() {
    assert_pair_write_refused(None);
}

#[test]
fn combined_datetime_indexed_write_is_refused_without_replacing_file_or_served_tags() {
    assert_pair_write_refused(Some(1));
}

#[test]
fn combined_datetime_file_load_is_refused_without_truncating_file() {
    let case = FileCase::new();
    let original = [
        b"RBNTAG01".as_slice(),
        &[0x0f, 0xc0, 0, 1, 1],
        DATE_TIME_TAG,
    ]
    .concat();
    std::fs::write(case.store.path(), &original).unwrap();
    assert!(case.store.load(oid()).is_err());
    assert_eq!(std::fs::read(case.store.path()).unwrap(), original);
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
fn semantic_null_date_and_time_tags_round_trip_without_reinterpretation() {
    let case = FileCase::new();
    let saved = TagsSnapshot {
        tags: Some(vec![
            BACnetNameValue::semantic("a"),
            BACnetNameValue::valued("a", PropertyValue::Null),
            BACnetNameValue::valued(
                "a",
                PropertyValue::Date(Date {
                    year: 126,
                    month: 10,
                    day: 6,
                    day_of_week: 2,
                }),
            ),
            BACnetNameValue::valued(
                "a",
                PropertyValue::Time(Time {
                    hour: 12,
                    minute: 30,
                    second: 0,
                    hundredths: 0,
                }),
            ),
        ]),
    };
    case.store.save(oid(), &saved).unwrap();
    // Independently authored body; absent value and explicit NULL differ.
    let expected = [
        1, 0x0a, 0, b'a', 0x0a, 0, b'a', 0, 0x0a, 0, b'a', 0xa4, 126, 10, 6, 2, 0x0a, 0, b'a',
        0xb4, 12, 30, 0, 0,
    ];
    assert_eq!(std::fs::read(case.store.path()).unwrap()[12..], expected);
    assert_eq!(case.store.load(oid()).unwrap(), Some(saved.clone()));
    let mut state =
        ProfileState::persistent(oid(), std::sync::Arc::new(case.store.clone())).unwrap();
    state.provision(profile("configured")).unwrap();
    apply(
        &mut state,
        &write(None, PropertyValue::ApplicationData(expected[1..].to_vec())),
    )
    .unwrap();
    assert_eq!(state.profile().tags, saved.tags);
    // Indexed primitive Date and Time writes preserve their distinct types.
    for (index, bytes) in [(3, &expected[8..16]), (4, &expected[16..])] {
        apply(
            &mut state,
            &write(Some(index), PropertyValue::ApplicationData(bytes.to_vec())),
        )
        .unwrap();
    }
    assert_eq!(case.store.load(oid()).unwrap(), Some(saved));
}

#[test]
fn nonprimitive_file_saves_leave_existing_bytes_intact() {
    let case = FileCase::new();
    case.store.save(oid(), &snapshot("old")).unwrap();
    let original = std::fs::read(case.store.path()).unwrap();
    for value in [
        PropertyValue::List(vec![]),
        PropertyValue::ApplicationData(DATE_TIME_TAG.to_vec()),
    ] {
        let invalid = TagsSnapshot {
            tags: Some(vec![BACnetNameValue::valued("a", value)]),
        };
        assert_code(
            case.store.save(oid(), &invalid),
            ErrorCode::INVALID_DATA_TYPE,
        );
        assert_eq!(std::fs::read(case.store.path()).unwrap(), original);
    }
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
