//! Optional input profiles do not acquire Present_Value simulation ownership.

use super::test_support::*;
use super::*;
use crate::analog::AnalogInputObject;
use crate::binary::BinaryInputObject;
use crate::multistate::MultiStateInputObject;
use crate::property_metadata::{PropertyConformance, PropertyWriteCapability};
use crate::traits::BACnetObject;
use bacnet_types::enums::ObjectType;
use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

const KINDS: [ObjectType; 3] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::BINARY_INPUT,
    ObjectType::MULTI_STATE_INPUT,
];
const ROWS: [P; 3] = [P::TAGS, P::PROFILE_LOCATION, P::PROFILE_NAME];

trait ProfileInput: BACnetObject {
    fn provision(&mut self, profile: ObjectProfile) -> Result<(), Error>;
    fn wait(&self);
}
macro_rules! profile_input {
    ($ty:ty) => {
        impl ProfileInput for $ty {
            fn provision(&mut self, profile: ObjectProfile) -> Result<(), Error> {
                self.set_profile(profile)
            }
            fn wait(&self) {
                self.wait_for_tag_saves();
            }
        }
    };
}
profile_input!(AnalogInputObject);
profile_input!(BinaryInputObject);
profile_input!(MultiStateInputObject);

fn object(kind: ObjectType, store: Option<Arc<Memory>>) -> Result<Box<dyn ProfileInput>, Error> {
    macro_rules! build {
        ($ty:ty $(, $extra:expr)?) => {
            Ok(Box::new(match store {
                None => <$ty>::new(1, "input" $(, $extra)?)?,
                Some(store) => <$ty>::with_tags_persistence(1, "input", $($extra,)? store)?,
            }))
        };
    }
    match kind {
        ObjectType::ANALOG_INPUT => build!(AnalogInputObject, 95),
        ObjectType::BINARY_INPUT => build!(BinaryInputObject),
        ObjectType::MULTI_STATE_INPUT => build!(MultiStateInputObject, 3),
        _ => unreachable!(),
    }
}

fn optional(mask: u8) -> ObjectProfile {
    ObjectProfile {
        tags: (mask & 1 != 0).then(|| tags("configured")),
        profile_location: (mask & 2 != 0).then(|| "https://example.com/profile.xdd".into()),
        profile_name: (mask & 4 != 0).then(|| "555-input".into()),
    }
}

#[test]
fn all_inputs_eight_masks_and_oos_states_preserve_existing_metadata() {
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
                assert_code(
                    object.read_property(P::VALUE_SOURCE, None),
                    ErrorCode::UNKNOWN_PROPERTY,
                );
            }
        }
    }
}

fn pv_values(kind: ObjectType) -> [PropertyValue; 2] {
    match kind {
        ObjectType::ANALOG_INPUT => [PropertyValue::Real(2.5), PropertyValue::Real(3.5)],
        ObjectType::BINARY_INPUT => [PropertyValue::Enumerated(1), PropertyValue::Enumerated(0)],
        ObjectType::MULTI_STATE_INPUT => [PropertyValue::Unsigned(2), PropertyValue::Unsigned(3)],
        _ => unreachable!(),
    }
}

#[test]
fn tags_in_both_oos_states_preserve_network_and_internal_pv_ownership() {
    for kind in KINDS {
        for persistent in [false, true] {
            let store = Arc::new(Memory::default());
            let mut object = object(kind, persistent.then(|| store.clone())).unwrap();
            object.provision(optional(7)).unwrap();
            let [local, network] = pv_values(kind);
            let origin = crate::command_source::test_origin();
            object.set_present_value_internal(local.clone()).unwrap();
            assert_code(
                object.write_property_from(P::PRESENT_VALUE, None, network.clone(), None, &origin),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            for oos in [false, true] {
                object
                    .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(oos), None)
                    .unwrap();
                if oos {
                    object
                        .write_property_from(P::PRESENT_VALUE, None, network.clone(), None, &origin)
                        .unwrap();
                    assert_code(
                        object.set_present_value_internal(local.clone()),
                        ErrorCode::WRITE_ACCESS_DENIED,
                    );
                    assert_code(
                        object.write_property_from(
                            P::PRESENT_VALUE,
                            None,
                            PropertyValue::CharacterString("wrong".into()),
                            None,
                            &origin,
                        ),
                        ErrorCode::INVALID_DATA_TYPE,
                    );
                } else {
                    assert_code(
                        object.set_present_value_internal(PropertyValue::CharacterString(
                            "wrong".into(),
                        )),
                        ErrorCode::INVALID_DATA_TYPE,
                    );
                }
                let preserved = [
                    P::PRESENT_VALUE,
                    P::STATUS_FLAGS,
                    P::RELIABILITY,
                    P::EVENT_STATE,
                ]
                .map(|p| object.read_property(p, None).unwrap());
                object
                    .write_property_from(P::TAGS, None, framed(&tags("changed")), None, &origin)
                    .unwrap();
                assert_eq!(
                    [
                        P::PRESENT_VALUE,
                        P::STATUS_FLAGS,
                        P::RELIABILITY,
                        P::EVENT_STATE
                    ]
                    .map(|p| object.read_property(p, None).unwrap()),
                    preserved
                );
                assert_eq!(
                    object.read_property(P::TAGS, Some(1)).unwrap(),
                    framed(&tags("changed"))
                );
                assert_eq!(
                    object.read_property(P::PRESENT_VALUE, None).unwrap(),
                    if oos { network.clone() } else { local.clone() }
                );
            }
            object
                .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(false), None)
                .unwrap();
            object.set_present_value_internal(local.clone()).unwrap();
            assert_eq!(object.read_property(P::PRESENT_VALUE, None).unwrap(), local);
            assert_eq!(store.attempts(), if persistent { 2 } else { 0 });
            assert_eq!(object.durable_writes_internal().is_some(), persistent);
        }
    }
}

fn preserves(mut object: Box<dyn ProfileInput>, properties: &[P]) {
    let before: Vec<_> = properties
        .iter()
        .map(|p| object.read_property(*p, None).unwrap())
        .collect();
    object.provision(optional(7)).unwrap();
    object
        .write_property(P::TAGS, Some(0), PropertyValue::Unsigned(0), None)
        .unwrap();
    assert_eq!(
        properties
            .iter()
            .map(|p| object.read_property(*p, None).unwrap())
            .collect::<Vec<_>>(),
        before
    );
    assert_eq!(
        object.read_property(P::TAGS, None).unwrap(),
        PropertyValue::List(vec![])
    );
}

#[test]
fn profiles_preserve_input_engineering_polarity_and_state_configuration() {
    let mut analog = AnalogInputObject::new(1, "AI", 95).unwrap();
    analog.set_min_pres_value(1.0);
    analog.set_max_pres_value(99.0);
    analog.configure_fault_out_of_range(10.0, 90.0).unwrap();
    analog.set_present_value(42.0);
    preserves(
        Box::new(analog),
        &[
            P::PRESENT_VALUE,
            P::UNITS,
            P::MIN_PRES_VALUE,
            P::MAX_PRES_VALUE,
            P::FAULT_HIGH_LIMIT,
            P::FAULT_LOW_LIMIT,
        ],
    );
    let mut binary = BinaryInputObject::new(1, "BI").unwrap();
    binary.set_present_value(1);
    preserves(
        Box::new(binary),
        &[
            P::PRESENT_VALUE,
            P::POLARITY,
            P::ACTIVE_TEXT,
            P::INACTIVE_TEXT,
            P::ALARM_VALUE,
        ],
    );
    let mut multi = MultiStateInputObject::new(1, "MSI", 3).unwrap();
    multi.set_number_of_states(5).unwrap();
    multi
        .write_property(
            P::STATE_TEXT,
            Some(1),
            PropertyValue::CharacterString("Idle".into()),
            None,
        )
        .unwrap();
    preserves(
        Box::new(multi),
        &[
            P::PRESENT_VALUE,
            P::NUMBER_OF_STATES,
            P::STATE_TEXT,
            P::ALARM_VALUES,
            P::RELIABILITY,
        ],
    );
}

#[test]
fn input_saved_overrides_need_provisioning_and_invalid_replacement_preserves_state() {
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
fn input_invalid_and_oversized_tags_make_no_save_attempts() {
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
fn input_monotonic_hooks_correct_expired_stages_without_output_changes() {
    for kind in KINDS {
        let store = Arc::new(Memory::default());
        *store.saved.lock().unwrap() = Some(snapshot("saved"));
        let mut object = object(kind, Some(store.clone())).unwrap();
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
