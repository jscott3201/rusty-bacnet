//! Device profile rows compose with clock, segmentation, services and Audit state.
use super::test_support::*;
use super::*;
use crate::clock::{ClockFrame, ClockReader};
use crate::device::{AuditRecipientChangeSink, AuditWriteSource, DeviceConfig, DeviceObject};
use crate::property_metadata::{PropertyConformance, PropertyWriteCapability};
use crate::traits::BACnetObject;
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::{ObjectType, Segmentation, ServiceSupported};
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const ROWS: [P; 3] = [P::TAGS, P::PROFILE_LOCATION, P::PROFILE_NAME];
fn optional(mask: u8) -> ObjectProfile {
    ObjectProfile {
        tags: (mask & 1 != 0).then(|| tags("configured")),
        profile_location: (mask & 2 != 0).then(|| "https://example.com/p.xdd".into()),
        profile_name: (mask & 4 != 0).then(|| "555-device".into()),
    }
}
struct Clock;
impl ClockReader for Clock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: Date {
                year: 126,
                month: 10,
                day: 8,
                day_of_week: 4,
            },
            local_time: Time {
                hour: 12,
                minute: 34,
                second: 56,
                hundredths: 0,
            },
            utc_offset: 300,
            daylight_savings_status: true,
        })
    }
}
struct Sink {
    active: AtomicBool,
    changes: AtomicUsize,
}
impl AuditRecipientChangeSink for Sink {
    fn change(
        &self,
        current: &mut BACnetRecipient,
        new: BACnetRecipient,
        _: Option<&AuditWriteSource>,
        _: Option<ClockFrame>,
    ) -> Result<(), Error> {
        self.changes.fetch_add(1, Ordering::SeqCst);
        *current = new;
        Ok(())
    }
    fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
}
fn configured(clock: bool, segmented: bool, cov: bool, audit: u8) -> (DeviceObject, Arc<Sink>) {
    let mut object = DeviceObject::new(DeviceConfig {
        instance: 123,
        name: "Profile Device".into(),
        vendor_id: 555,
        segmentation_supported: if segmented {
            Segmentation::BOTH
        } else {
            Segmentation::NONE
        },
        ..Default::default()
    })
    .unwrap();
    object.set_device_uuid([7; 16]);
    if clock {
        object.bind_clock_internal(Some(Arc::new(Clock)));
    }
    if !cov {
        object.set_services_supported(&[ServiceSupported::READ_PROPERTY]);
    }
    let sink = Arc::new(Sink {
        active: AtomicBool::new(audit != 3),
        changes: AtomicUsize::new(0),
    });
    if audit != 0 {
        object
            .provision_audit_recipient(BACnetRecipient::Device(
                ObjectIdentifier::new(ObjectType::DEVICE, 999).unwrap(),
            ))
            .unwrap();
    }
    if audit >= 2 {
        let erased: Arc<dyn AuditRecipientChangeSink> = sink.clone();
        object
            .device_authority_internal()
            .unwrap()
            .install_audit_recipient(&erased)
            .unwrap();
    }
    (object, sink)
}
fn unrelated(object: &DeviceObject) -> Vec<(P, PropertyValue)> {
    object
        .property_metadata()
        .iter()
        .map(|row| row.property_identifier)
        .filter(|p| !ROWS.contains(p) && *p != P::PROPERTY_LIST)
        .map(|p| (p, object.read_property(p, None).unwrap()))
        .collect()
}

#[test]
fn device_eight_masks_compose_with_clock_segmentation_cov_and_audit_metadata() {
    for clock in [false, true] {
        for segmented in [false, true] {
            for cov in [false, true] {
                for audit in 0..4 {
                    for mask in 0..8 {
                        let (mut object, sink) = configured(clock, segmented, cov, audit);
                        if cov && audit < 2 {
                            assert!(matches!(object.property_metadata(), Cow::Borrowed(_)));
                        }
                        let before = object.property_metadata().into_owned();
                        let values = unrelated(&object);
                        let list = object.property_list().into_owned();
                        object.set_profile(optional(mask)).unwrap();
                        let metadata = object.property_metadata();
                        assert_eq!(
                            metadata
                                .iter()
                                .filter(|r| !ROWS.contains(&r.property_identifier))
                                .copied()
                                .collect::<Vec<_>>(),
                            before
                        );
                        assert_eq!(unrelated(&object), values);
                        let expected: Vec<_> = ROWS
                            .into_iter()
                            .enumerate()
                            .filter(|(bit, _)| mask & (1 << bit) != 0)
                            .map(|(_, p)| p)
                            .collect();
                        assert_eq!(&object.property_list()[list.len()..], expected);
                        assert!(object.property_list().contains(&P::PROPERTY_LIST));
                        assert!(!object
                            .property_list()
                            .contains(&P::DEPLOYED_PROFILE_LOCATION));
                        for (bit, p) in ROWS.into_iter().enumerate() {
                            let present = mask & (1 << bit) != 0;
                            let row = metadata.iter().find(|r| r.property_identifier == p);
                            assert_eq!(row.is_some(), present);
                            if let Some(row) = row {
                                assert_eq!(row.conformance, PropertyConformance::Optional);
                                assert_eq!(row.presence_condition, None);
                                assert_eq!(
                                    row.write_capability,
                                    if p == P::TAGS {
                                        PropertyWriteCapability::Always
                                    } else {
                                        PropertyWriteCapability::ReadOnly
                                    }
                                );
                                assert!(object.read_property(p, None).is_ok());
                                if p != P::TAGS {
                                    assert_code(
                                        object.read_property(p, Some(1)),
                                        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
                                    );
                                }
                            } else {
                                assert_code(
                                    object.read_property(p, None),
                                    ErrorCode::UNKNOWN_PROPERTY,
                                );
                            }
                        }
                        drop(metadata);
                        for (bit, p) in ROWS.into_iter().enumerate() {
                            if mask & (1 << bit) == 0 {
                                assert_code(
                                    object.write_property(p, None, PropertyValue::Null, None),
                                    ErrorCode::UNKNOWN_PROPERTY,
                                );
                            } else if p != P::TAGS {
                                assert_code(
                                    object.write_property(
                                        p,
                                        None,
                                        PropertyValue::CharacterString("555-other".into()),
                                        None,
                                    ),
                                    ErrorCode::WRITE_ACCESS_DENIED,
                                );
                            }
                        }
                        if mask & 1 != 0 {
                            object
                                .write_property(
                                    P::TAGS,
                                    None,
                                    PropertyValue::ApplicationData(vec![]),
                                    None,
                                )
                                .unwrap();
                            assert_eq!(
                                object.read_property(P::TAGS, Some(0)).unwrap(),
                                PropertyValue::Unsigned(0)
                            );
                        }
                        assert_eq!(unrelated(&object), values);
                        assert_eq!(sink.changes.load(Ordering::SeqCst), 0);
                        assert!(!object.advance_monotonic_time_internal(Duration::from_secs(30)));
                        assert!(object.durable_writes_internal().is_none());
                        assert!(object.cov_snapshot_internal().is_none());
                    }
                }
            }
        }
    }
}

fn persistent(store: &Arc<Memory>) -> Result<DeviceObject, Error> {
    DeviceObject::with_tags_persistence(DeviceConfig::default(), store.clone())
}
#[test]
fn device_saved_overrides_need_provisioning_and_invalid_replacement_preserves_state() {
    for saved in [
        None,
        Some(TagsSnapshot::default()),
        Some(snapshot("saved")),
        Some(TagsSnapshot { tags: Some(vec![]) }),
    ] {
        let store = Arc::new(Memory::default());
        *store.saved.lock().unwrap() = saved.clone();
        let mut object = persistent(&store).unwrap();
        assert_code(
            object.read_property(P::TAGS, None),
            ErrorCode::UNKNOWN_PROPERTY,
        );
        assert!(!object.property_list().contains(&P::TAGS));
        object.set_profile(optional(7)).unwrap();
        let expected = saved
            .as_ref()
            .and_then(|s| s.tags.clone())
            .unwrap_or_else(|| tags("configured"));
        assert_eq!(
            object.read_property(P::TAGS, Some(0)).unwrap(),
            PropertyValue::Unsigned(expected.len() as u64)
        );
        for (i, tag) in expected.iter().enumerate() {
            assert_eq!(
                object.read_property(P::TAGS, Some(i as u32 + 1)).unwrap(),
                framed(std::slice::from_ref(tag))
            );
        }
        let served = ROWS.map(|p| object.read_property(p, None).unwrap());
        for invalid in [
            profile("bad;name"),
            ObjectProfile {
                profile_name: Some("no-vendor".into()),
                ..optional(7)
            },
            ObjectProfile {
                profile_location: Some("file:///local".into()),
                ..optional(7)
            },
        ] {
            assert!(object.set_profile(invalid).is_err());
            assert_eq!(ROWS.map(|p| object.read_property(p, None).unwrap()), served);
        }
        object.set_profile(ObjectProfile::default()).unwrap();
        assert_code(
            object.read_property(P::TAGS, None),
            ErrorCode::UNKNOWN_PROPERTY,
        );
        object.set_profile(optional(7)).unwrap();
        assert_eq!(ROWS.map(|p| object.read_property(p, None).unwrap()), served);
        assert_eq!(store.attempts(), 0);
    }
    let store = Arc::new(Memory::default());
    *store.saved.lock().unwrap() = Some(snapshot("bad;name"));
    assert!(persistent(&store).is_err());
    *store.saved.lock().unwrap() = None;
    store.fail_load.store(true, Ordering::SeqCst);
    assert!(persistent(&store).is_err());
    assert_eq!(store.attempts(), 0);
}

#[test]
fn device_invalid_writes_and_save_failure_preserve_served_and_stored_tags() {
    let store = Arc::new(Memory::default());
    *store.saved.lock().unwrap() = Some(snapshot("saved"));
    let mut object = persistent(&store).unwrap();
    object.set_profile(optional(7)).unwrap();
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
            framed(&tags(&"x".repeat(MAX_TAGS_SNAPSHOT_BYTES - 1))),
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        ),
        (None, PropertyValue::Real(1.0), ErrorCode::INVALID_DATA_TYPE),
    ] {
        assert_code(object.write_property(P::TAGS, index, value, None), code);
        assert_eq!(object.read_property(P::TAGS, None).unwrap(), before);
        assert_eq!(store.saved(), Some(snapshot("saved")));
        assert_eq!(store.attempts(), 0);
    }
    store.fail_save.store(true, Ordering::SeqCst);
    assert!(object
        .write_property(P::TAGS, None, framed(&tags("refused")), None)
        .is_err());
    object.wait_for_tag_saves();
    assert_eq!(store.attempts(), 1);
    assert_eq!(store.saved(), Some(snapshot("saved")));
    assert_eq!(object.read_property(P::TAGS, None).unwrap(), before);
}

#[test]
fn device_expired_stage_corrects_tags_and_accepts_the_next_write() {
    let store = Arc::new(Memory::default());
    *store.saved.lock().unwrap() = Some(snapshot("saved"));
    let mut object = persistent(&store).unwrap();
    object.set_profile(optional(7)).unwrap();
    let values = unrelated(&object);
    staged(
        object
            .durable_writes_internal()
            .unwrap()
            .stage_writes(&[whole("never-served")]),
    )
    .block();
    assert_eq!(store.saved(), Some(snapshot("never-served")));
    assert!(!object.advance_monotonic_time_internal(Duration::ZERO));
    assert!(!object.advance_monotonic_time_internal(Duration::from_secs(11)));
    object.wait_for_tag_saves();
    assert_eq!(store.saved(), Some(snapshot("saved")));
    assert_eq!(store.attempts(), 2);
    assert_eq!(
        object.read_property(P::TAGS, Some(1)).unwrap(),
        framed(&tags("saved"))
    );
    object
        .write_property(P::TAGS, None, framed(&tags("next")), None)
        .unwrap();
    object.wait_for_tag_saves();
    assert_eq!(store.attempts(), 3);
    assert_eq!(store.saved(), Some(snapshot("next")));
    assert_eq!(
        object.read_property(P::TAGS, Some(1)).unwrap(),
        framed(&tags("next"))
    );
    assert_eq!(unrelated(&object), values);
}

#[test]
fn device_persistence_constructor_keeps_validation_before_loading() {
    struct NoLoad;
    impl TagsPersistence for NoLoad {
        fn load(&self, _: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
            panic!("invalid config must fail before loading");
        }
        fn save(&self, _: ObjectIdentifier, _: &TagsSnapshot) -> Result<(), Error> {
            panic!("constructor must not save");
        }
    }
    for config in [
        DeviceConfig {
            instance: ObjectIdentifier::MAX_INSTANCE + 1,
            ..Default::default()
        },
        DeviceConfig {
            segmentation_supported: Segmentation::BOTH,
            apdu_segment_timeout: 0,
            ..Default::default()
        },
    ] {
        assert!(DeviceObject::with_tags_persistence(config, Arc::new(NoLoad)).is_err());
    }
}
