//! Optional output profiles preserve command ownership and saved Tags authority.

use super::test_support::*;
use super::*;
use crate::analog::AnalogOutputObject;
use crate::binary::BinaryOutputObject;
use crate::multistate::MultiStateOutputObject;
use crate::property_metadata::{PropertyConformance, PropertyWriteCapability};
use crate::traits::BACnetObject;
use bacnet_types::enums::ObjectType;
use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

const KINDS: [ObjectType; 3] = [
    ObjectType::ANALOG_OUTPUT,
    ObjectType::BINARY_OUTPUT,
    ObjectType::MULTI_STATE_OUTPUT,
];
const ROWS: [P; 3] = [P::TAGS, P::PROFILE_LOCATION, P::PROFILE_NAME];

trait ProfileOutput: BACnetObject {
    fn provision(&mut self, profile: ObjectProfile) -> Result<(), Error>;
    fn wait(&self);
}
macro_rules! profile_output {
    ($ty:ty) => {
        impl ProfileOutput for $ty {
            fn provision(&mut self, profile: ObjectProfile) -> Result<(), Error> {
                self.set_profile(profile)
            }
            fn wait(&self) {
                self.wait_for_tag_saves();
            }
        }
    };
}
profile_output!(AnalogOutputObject);
profile_output!(BinaryOutputObject);
profile_output!(MultiStateOutputObject);

fn object(kind: ObjectType, store: Option<Arc<Memory>>) -> Result<Box<dyn ProfileOutput>, Error> {
    macro_rules! build {
        ($ty:ty $(, $extra:expr)?) => {
            Ok(Box::new(match store {
                None => <$ty>::new(1, "output" $(, $extra)?)?,
                Some(store) => <$ty>::with_tags_persistence(1, "output", $($extra,)? store)?,
            }))
        };
    }
    match kind {
        ObjectType::ANALOG_OUTPUT => build!(AnalogOutputObject, 95),
        ObjectType::BINARY_OUTPUT => build!(BinaryOutputObject),
        ObjectType::MULTI_STATE_OUTPUT => build!(MultiStateOutputObject, 3),
        _ => unreachable!(),
    }
}

fn optional(mask: u8) -> ObjectProfile {
    ObjectProfile {
        tags: (mask & 1 != 0).then(|| tags("configured")),
        profile_location: (mask & 2 != 0).then(|| "https://example.com/profile.xdd".into()),
        profile_name: (mask & 4 != 0).then(|| "555-output".into()),
    }
}

#[test]
fn all_outputs_eight_masks_and_oos_states_preserve_existing_metadata() {
    for kind in KINDS {
        for mask in 0..8 {
            for oos in [false, true] {
                let mut object = object(kind, None).unwrap();
                assert!(matches!(object.property_metadata(), Cow::Borrowed(_)));
                object
                    .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(oos), None)
                    .unwrap();
                let before = object.property_metadata().into_owned();
                object.provision(optional(mask)).unwrap();
                let metadata = object.property_metadata().into_owned();
                assert_eq!(
                    metadata
                        .iter()
                        .filter(|row| !ROWS.contains(&row.property_identifier))
                        .copied()
                        .collect::<Vec<_>>(),
                    before
                );
                let expected: Vec<_> = ROWS
                    .into_iter()
                    .enumerate()
                    .filter_map(|(bit, p)| (mask & (1 << bit) != 0).then_some(p))
                    .collect();
                assert_eq!(
                    metadata
                        .iter()
                        .filter(|row| ROWS.contains(&row.property_identifier))
                        .map(|row| row.property_identifier)
                        .collect::<Vec<_>>(),
                    expected
                );
                assert_eq!(
                    object
                        .property_list()
                        .iter()
                        .filter(|p| ROWS.contains(p))
                        .copied()
                        .collect::<Vec<_>>(),
                    expected
                );
                for (bit, property) in ROWS.into_iter().enumerate() {
                    if mask & (1 << bit) == 0 {
                        assert_code(
                            object.read_property(property, None),
                            ErrorCode::UNKNOWN_PROPERTY,
                        );
                        assert_code(
                            object.write_property(property, None, PropertyValue::Null, None),
                            ErrorCode::UNKNOWN_PROPERTY,
                        );
                        continue;
                    }
                    assert!(object.read_property(property, None).is_ok());
                    let row = metadata
                        .iter()
                        .find(|row| row.property_identifier == property)
                        .unwrap();
                    assert_eq!(row.conformance, PropertyConformance::Optional);
                    assert_eq!(row.presence_condition, None);
                    assert_eq!(
                        row.write_capability,
                        if property == P::TAGS {
                            PropertyWriteCapability::Always
                        } else {
                            PropertyWriteCapability::ReadOnly
                        }
                    );
                    if property != P::TAGS {
                        assert_code(
                            object.write_property(
                                property,
                                None,
                                PropertyValue::CharacterString("555-other".into()),
                                None,
                            ),
                            ErrorCode::WRITE_ACCESS_DENIED,
                        );
                        assert_code(
                            object.read_property(property, Some(0)),
                            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
                        );
                    }
                }
                object
                    .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(!oos), None)
                    .unwrap();
                assert_eq!(object.property_metadata().as_ref(), metadata);
                assert!(!object.advance_monotonic_time_internal(Duration::from_secs(20)));
                assert!(object.cov_snapshot_internal().is_none());
                assert!(object.read_property(P::VALUE_SOURCE, None).is_ok());
                assert!(object.durable_writes_internal().is_none());
            }
        }
    }
}

#[test]
fn output_saved_overrides_need_provisioning_and_invalid_replacement_preserves_state() {
    for kind in KINDS {
        for saved in [
            None,
            Some(TagsSnapshot::default()),
            Some(snapshot("saved")),
            Some(TagsSnapshot { tags: Some(vec![]) }),
        ] {
            let store = Arc::new(Memory::default());
            *store.saved.lock().unwrap() = saved.clone();
            let mut object = object(kind, Some(store.clone())).unwrap();
            assert_code(
                object.read_property(P::TAGS, None),
                ErrorCode::UNKNOWN_PROPERTY,
            );
            assert!(!object.property_list().contains(&P::TAGS));
            assert!(object.durable_writes_internal().is_some());
            object.provision(optional(7)).unwrap();
            let count = saved
                .as_ref()
                .and_then(|s| s.tags.as_ref())
                .map_or(1, Vec::len);
            assert_eq!(
                object.read_property(P::TAGS, Some(0)).unwrap(),
                PropertyValue::Unsigned(count as u64)
            );
            if count != 0 {
                assert_eq!(
                    object.read_property(P::TAGS, Some(1)).unwrap(),
                    framed(&tags(if saved.as_ref().is_some_and(|s| s.tags.is_some()) {
                        "saved"
                    } else {
                        "configured"
                    }))
                );
            }
            let served = ROWS.map(|p| object.read_property(p, None).unwrap());
            for invalid in [
                profile("bad;name"),
                ObjectProfile {
                    profile_name: Some("missing-vendor".into()),
                    ..optional(7)
                },
                ObjectProfile {
                    profile_location: Some("file:///local".into()),
                    ..optional(7)
                },
            ] {
                assert!(object.provision(invalid).is_err());
                assert_eq!(ROWS.map(|p| object.read_property(p, None).unwrap()), served);
            }
            object.provision(ObjectProfile::default()).unwrap();
            assert_code(
                object.read_property(P::TAGS, None),
                ErrorCode::UNKNOWN_PROPERTY,
            );
            object.provision(optional(7)).unwrap();
            assert_eq!(ROWS.map(|p| object.read_property(p, None).unwrap()), served);
            assert_eq!(store.attempts(), 0);
        }
        let store = Arc::new(Memory::default());
        *store.saved.lock().unwrap() = Some(snapshot("bad;name"));
        assert!(object(kind, Some(store.clone())).is_err());
        assert_eq!(store.attempts(), 0);
    }
}

#[test]
fn output_invalid_and_oversized_tags_make_no_save_attempts() {
    let oversized = framed(&tags(&"x".repeat(MAX_TAGS_SNAPSHOT_BYTES - 1)));
    for kind in KINDS {
        let store = Arc::new(Memory::default());
        *store.saved.lock().unwrap() = Some(snapshot("saved"));
        let mut object = object(kind, Some(store.clone())).unwrap();
        object.provision(optional(7)).unwrap();
        let before = object.read_property(P::TAGS, None).unwrap();
        for (index, value, code) in [
            (
                None,
                framed(&tags("bad;name")),
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
            (
                Some(0),
                PropertyValue::Unsigned(1025),
                ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
            ),
            (
                Some(2),
                framed(&tags("outside")),
                ErrorCode::INVALID_ARRAY_INDEX,
            ),
            (
                None,
                oversized.clone(),
                ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
            ),
        ] {
            assert_code(object.write_property(P::TAGS, index, value, None), code);
            assert_eq!(object.read_property(P::TAGS, None).unwrap(), before);
            assert_eq!(store.saved(), Some(snapshot("saved")));
            assert_eq!(store.attempts(), 0);
        }
    }
}

#[test]
fn output_monotonic_hooks_correct_expired_stages_without_output_changes() {
    for kind in KINDS {
        let store = Arc::new(Memory::default());
        *store.saved.lock().unwrap() = Some(snapshot("saved"));
        let mut object = object(kind, Some(store.clone())).unwrap();
        object.provision(optional(7)).unwrap();
        object
            .write_property_from(
                P::PRESENT_VALUE,
                None,
                command_tests::values(kind)[0].clone(),
                Some(8),
                &crate::command_source::test_origin(),
            )
            .unwrap();
        let command_before = command_tests::state(object.as_ref());
        let wait = staged(
            object
                .durable_writes_internal()
                .unwrap()
                .stage_writes(&[whole("never-served")]),
        );
        wait.block();
        assert_eq!(store.saved(), Some(snapshot("never-served")));
        assert!(!object.advance_monotonic_time_internal(Duration::ZERO));
        assert!(!object.advance_monotonic_time_internal(Duration::from_secs(11)));
        object.wait();
        assert_eq!(store.saved(), Some(snapshot("saved")));
        assert_eq!(command_tests::state(object.as_ref()), command_before);
        assert_eq!(store.attempts(), 2);
        assert_eq!(
            object.read_property(P::TAGS, Some(1)).unwrap(),
            framed(&tags("saved"))
        );
    }
}

mod command_tests;
