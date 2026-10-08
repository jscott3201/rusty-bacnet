//! Profile provisioning and storage remain independent of Value PV ownership.

use super::test_support::*;
use super::*;
use crate::analog::AnalogValueObject;
use crate::binary::BinaryValueObject;
use crate::multistate::MultiStateValueObject;
use crate::present_value_access::PresentValueAccess as Access;
use crate::property_metadata::{PropertyConformance, PropertyWriteCapability};
use crate::traits::BACnetObject;
use bacnet_types::enums::ObjectType;
use std::sync::Arc;
use std::time::Duration;

const KINDS: [ObjectType; 3] = [
    ObjectType::ANALOG_VALUE,
    ObjectType::BINARY_VALUE,
    ObjectType::MULTI_STATE_VALUE,
];
const MODES: [Access; 3] = [Access::Commandable, Access::Writable, Access::ReadOnly];
const ROWS: [P; 3] = [P::TAGS, P::PROFILE_LOCATION, P::PROFILE_NAME];

trait ProfileValue: BACnetObject {
    fn provision(&mut self, profile: ObjectProfile) -> Result<(), Error>;
    fn track(&mut self, enabled: bool);
    fn wait(&self);
}
macro_rules! profile_value {
    ($ty:ty) => {
        impl ProfileValue for $ty {
            fn provision(&mut self, profile: ObjectProfile) -> Result<(), Error> {
                self.set_profile(profile)
            }
            fn track(&mut self, enabled: bool) {
                self.set_value_source_tracking(enabled);
            }
            fn wait(&self) {
                self.wait_for_tag_saves();
            }
        }
    };
}
profile_value!(AnalogValueObject);
profile_value!(BinaryValueObject);
profile_value!(MultiStateValueObject);

fn object(
    kind: ObjectType,
    access: Access,
    store: Option<Arc<Memory>>,
) -> Result<Box<dyn ProfileValue>, Error> {
    macro_rules! build {
        ($ty:ty $(, $extra:expr)?) => {
            Ok(Box::new(match store {
                None => <$ty>::with_access(1, "value", $($extra,)? access)?,
                Some(store) => <$ty>::with_tags_persistence(1, "value", $($extra,)? access, store)?,
            }))
        };
    }
    match kind {
        ObjectType::ANALOG_VALUE => build!(AnalogValueObject, 95),
        ObjectType::BINARY_VALUE => build!(BinaryValueObject),
        ObjectType::MULTI_STATE_VALUE => build!(MultiStateValueObject, 3),
        _ => unreachable!(),
    }
}

fn optional(mask: u8) -> ObjectProfile {
    ObjectProfile {
        tags: (mask & 1 != 0).then(|| tags("configured")),
        profile_location: (mask & 2 != 0).then(|| "https://example.com/profile.xdd".into()),
        profile_name: (mask & 4 != 0).then(|| "555-value".into()),
    }
}

#[test]
fn all_nine_modes_and_eight_masks_preserve_unrelated_metadata_and_source_ownership() {
    for kind in KINDS {
        for access in MODES {
            for mask in 0..8 {
                for tracked in [false, true] {
                    let mut object = object(kind, access, None).unwrap();
                    object.track(tracked);
                    let before = object.property_metadata().into_owned();
                    let pv = object.read_property(P::PRESENT_VALUE, None).unwrap();
                    object.provision(optional(mask)).unwrap();
                    let metadata = object.property_metadata().into_owned();
                    assert_eq!(
                        metadata
                            .iter()
                            .filter(|row| !ROWS.contains(&row.property_identifier))
                            .copied()
                            .collect::<Vec<_>>(),
                        before,
                        "{kind:?} {access:?} mask {mask} tracking {tracked}"
                    );
                    assert_eq!(object.read_property(P::PRESENT_VALUE, None).unwrap(), pv);
                    assert_eq!(
                        object.property_list().contains(&P::PRIORITY_ARRAY),
                        access == Access::Commandable
                    );
                    assert_eq!(
                        object.property_list().contains(&P::VALUE_SOURCE),
                        access == Access::Commandable || tracked
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
                        let result = object.read_property(property, None);
                        if mask & (1 << bit) == 0 {
                            assert_code(result, ErrorCode::UNKNOWN_PROPERTY);
                            assert_code(
                                object.write_property(property, None, PropertyValue::Null, None),
                                ErrorCode::UNKNOWN_PROPERTY,
                            );
                            continue;
                        }
                        assert!(result.is_ok());
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
                    // A source-mode reconfiguration cannot add or remove profile rows.
                    object.track(!tracked);
                    assert_eq!(
                        object
                            .property_list()
                            .iter()
                            .filter(|p| ROWS.contains(p))
                            .copied()
                            .collect::<Vec<_>>(),
                        expected
                    );
                    assert!(!object.advance_monotonic_time_internal(Duration::from_secs(20)));
                    assert!(object.cov_snapshot_internal().is_none());
                }
            }
        }
    }
}

#[test]
fn invalid_provision_preserves_profile_and_memory_only_writes_remain_local() {
    for kind in KINDS {
        for access in MODES {
            let mut object = object(kind, access, None).unwrap();
            object.provision(optional(7)).unwrap();
            for invalid in [
                profile("bad;name"),
                ObjectProfile {
                    profile_location: Some("file:///local".into()),
                    ..optional(7)
                },
                ObjectProfile {
                    profile_name: Some("missing-vendor".into()),
                    ..optional(7)
                },
            ] {
                let before = ROWS.map(|p| object.read_property(p, None).unwrap());
                assert!(object.provision(invalid).is_err());
                assert_eq!(ROWS.map(|p| object.read_property(p, None).unwrap()), before);
            }
            assert!(object.durable_writes_internal().is_none());
            object
                .write_property(P::TAGS, Some(0), PropertyValue::Unsigned(0), None)
                .unwrap();
            assert_eq!(
                object.read_property(P::TAGS, None).unwrap(),
                PropertyValue::List(vec![])
            );
            assert_eq!(
                object.read_property(P::TAGS, Some(0)).unwrap(),
                PropertyValue::Unsigned(0)
            );
            object.wait();
        }
    }
}

#[test]
fn saved_overrides_require_provisioning_in_every_value_mode() {
    for kind in KINDS {
        for access in MODES {
            for saved in [
                None,
                Some(TagsSnapshot::default()),
                Some(snapshot("saved")),
                Some(TagsSnapshot { tags: Some(vec![]) }),
            ] {
                let store = Arc::new(Memory::default());
                *store.saved.lock().unwrap() = saved.clone();
                let mut object = object(kind, access, Some(store.clone())).unwrap();
                assert_code(
                    object.read_property(P::TAGS, None),
                    ErrorCode::UNKNOWN_PROPERTY,
                );
                assert!(!object.property_list().contains(&P::TAGS));
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
                let served = object.read_property(P::TAGS, None).unwrap();
                object.provision(ObjectProfile::default()).unwrap();
                assert_code(
                    object.read_property(P::TAGS, None),
                    ErrorCode::UNKNOWN_PROPERTY,
                );
                object.provision(optional(7)).unwrap();
                assert_eq!(object.read_property(P::TAGS, None).unwrap(), served);
                assert_eq!(
                    object.read_property(P::PROFILE_NAME, None).unwrap(),
                    PropertyValue::CharacterString("555-value".into())
                );
                assert_eq!(store.attempts(), 0);
            }
            let store = Arc::new(Memory::default());
            *store.saved.lock().unwrap() = Some(snapshot("bad;name"));
            assert!(object(kind, access, Some(store.clone())).is_err());
            assert_eq!(store.attempts(), 0);
        }
    }
}

#[test]
fn invalid_and_oversized_tags_never_attempt_a_save_in_any_value_mode() {
    let oversized = framed(&tags(&"x".repeat(MAX_TAGS_SNAPSHOT_BYTES - 1)));
    for kind in KINDS {
        for access in MODES {
            let store = Arc::new(Memory::default());
            *store.saved.lock().unwrap() = Some(snapshot("saved"));
            let mut object = object(kind, access, Some(store.clone())).unwrap();
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
}

#[test]
fn each_value_monotonic_hook_expires_unclaimed_stages_without_output_change() {
    for kind in KINDS {
        for access in MODES {
            let store = Arc::new(Memory::default());
            *store.saved.lock().unwrap() = Some(snapshot("saved"));
            let mut object = object(kind, access, Some(store.clone())).unwrap();
            object.provision(optional(7)).unwrap();
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
            assert_eq!(store.attempts(), 2);
            assert_eq!(
                object.read_property(P::TAGS, Some(1)).unwrap(),
                framed(&tags("saved"))
            );
        }
    }
}
