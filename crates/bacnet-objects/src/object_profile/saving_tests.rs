use super::test_support::*;
use super::*;
use crate::durable::{DurableWrites, StageStep};
use crate::traits::BACnetObject;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

#[test]
fn saved_empty_is_distinct_from_no_write_and_provisioning_stays_independent() {
    for saved in [
        None,
        Some(TagsSnapshot::default()),
        Some(snapshot("saved")),
        Some(TagsSnapshot { tags: Some(vec![]) }),
    ] {
        let store = Arc::new(Memory::default());
        *store.saved.lock().unwrap() = saved.clone();
        let mut state = ProfileState::persistent(oid(), store.clone()).unwrap();
        assert_code(
            state.read(P::TAGS, None).unwrap(),
            ErrorCode::UNKNOWN_PROPERTY,
        );
        assert!(state.metadata().next().is_none());
        assert_code(
            apply(&mut state, &whole("ignored")),
            ErrorCode::UNKNOWN_PROPERTY,
        );
        state
            .provision(ObjectProfile {
                profile_name: Some("555-name".into()),
                ..ObjectProfile::default()
            })
            .unwrap();
        assert_eq!(
            state
                .metadata()
                .map(|row| row.property_identifier)
                .collect::<Vec<_>>(),
            vec![P::PROFILE_NAME]
        );
        state.provision(profile("configured")).unwrap();
        let expected = saved
            .and_then(|s| s.tags)
            .unwrap_or_else(|| tags("configured"));
        assert_eq!(state.profile().tags, Some(expected.clone()));
        state.provision(ObjectProfile::default()).unwrap();
        assert!(state.profile().tags.is_none());
        state.provision(profile("reconfigured")).unwrap();
        let expected = store
            .saved()
            .and_then(|s| s.tags)
            .unwrap_or_else(|| tags("reconfigured"));
        assert_eq!(state.profile().tags, Some(expected));
        assert_eq!(store.attempts(), 0);
    }
}

#[test]
fn failed_save_and_invalid_writes_leave_served_state_and_backend_unchanged() {
    let store = Arc::new(Memory::default());
    let mut state = state(&store);
    apply(&mut state, &whole("old")).unwrap();
    store.fail_save.store(true, Ordering::SeqCst);
    assert_code(
        apply(&mut state, &whole("new")),
        ErrorCode::OPERATIONAL_PROBLEM,
    );
    assert_eq!(state.profile().tags, Some(tags("old")));
    assert_eq!(store.saved(), Some(snapshot("old")));
    let before = store.attempts();
    for invalid in [
        whole("invalid;name"),
        write(Some(2), framed(&tags("new"))),
        write(Some(0), PropertyValue::Unsigned(MAX_TAGS as u64 + 1)),
        whole(&"x".repeat(MAX_TAGS_SNAPSHOT_BYTES - 1)),
    ] {
        assert!(apply(&mut state, &invalid).is_err());
        assert_eq!(store.attempts(), before);
    }
}

#[test]
fn load_errors_and_invalid_custom_snapshots_fail_before_a_writer_runs() {
    let store = Arc::new(Memory::default());
    store.fail_load.store(true, Ordering::SeqCst);
    assert!(ProfileState::persistent(oid(), store.clone()).is_err());
    store.fail_load.store(false, Ordering::SeqCst);
    for invalid in [
        snapshot("invalid;name"),
        TagsSnapshot {
            tags: Some(vec![BACnetNameValue::semantic(""); MAX_TAGS + 1]),
        },
        snapshot(&"x".repeat(MAX_TAGS_SNAPSHOT_BYTES)),
        TagsSnapshot {
            tags: Some(vec![BACnetNameValue::valued(
                "array",
                PropertyValue::List(vec![]),
            )]),
        },
        TagsSnapshot {
            tags: Some(vec![BACnetNameValue::valued(
                "raw",
                PropertyValue::ApplicationData(vec![0xa4, 126, 10, 6, 2, 0xb4, 12, 30, 0, 0]),
            )]),
        },
    ] {
        *store.saved.lock().unwrap() = Some(invalid);
        assert!(ProfileState::persistent(oid(), store.clone()).is_err());
    }
    assert_eq!(store.attempts(), 0);
}

#[test]
fn combined_datetime_is_refused_before_direct_or_staged_save() {
    let store = Arc::new(Memory::default());
    let mut state = state(&store);
    let pair = vec![0x0a, 0, b'a', 0xa4, 126, 10, 6, 2, 0xb4, 12, 30, 0, 0];
    for index in [None, Some(1)] {
        let invalid = write(index, PropertyValue::ApplicationData(pair.clone()));
        assert!(matches!(
            state.stage_writes(std::slice::from_ref(&invalid)),
            StageStep::Skip
        ));
        assert_code(
            apply(&mut state, &invalid),
            ErrorCode::INVALID_DATA_ENCODING,
        );
        state.wait_for_saves();
        assert_eq!(state.profile().tags, Some(tags("configured")));
        assert_eq!(store.saved(), None);
        assert_eq!(store.attempts(), 0);
    }
}

#[test]
fn ordered_array_steps_save_once_and_each_claim_serves_its_prefix() {
    let store = Arc::new(Memory::default());
    let mut state = state(&store);
    let writes = [
        whole("first"),
        write(Some(0), PropertyValue::Unsigned(2)),
        write(Some(2), framed(&tags("second"))),
        write(Some(1), framed(&tags("last"))),
    ];
    let wait = staged(state.stage_writes(&writes));
    wait.block();
    let expected = vec![
        BACnetNameValue::semantic("last"),
        BACnetNameValue::semantic("second"),
    ];
    assert_eq!(
        store.saved(),
        Some(TagsSnapshot {
            tags: Some(expected.clone())
        })
    );
    assert_eq!(state.profile().tags, Some(tags("configured")));
    for (i, write) in writes.iter().enumerate() {
        apply(&mut state, write).unwrap();
        if i == 0 {
            assert_eq!(state.profile().tags, Some(tags("first")));
        }
    }
    state.release_staged_write(&wait);
    state.wait_for_saves();
    assert_eq!(store.attempts(), 1);
    assert_eq!(state.profile().tags, Some(expected));
}

#[test]
fn invalid_middle_stops_staging_but_a_null_noop_does_not() {
    for middle in [
        write(Some(9), framed(&tags("bad"))),
        write(None, PropertyValue::Null),
    ] {
        let store = Arc::new(Memory::default());
        let mut state = state(&store);
        let null = middle.value == PropertyValue::Null;
        let writes = [whole("prefix"), middle, whole("suffix")];
        let wait = staged(state.stage_writes(&writes));
        wait.block();
        apply(&mut state, &writes[0]).unwrap();
        assert!(apply(&mut state, &writes[1]).is_err());
        if null {
            apply(&mut state, &writes[2]).unwrap();
        }
        state.release_staged_write(&wait);
        state.wait_for_saves();
        assert_eq!(
            store.saved(),
            Some(snapshot(if null { "suffix" } else { "prefix" }))
        );
        assert_eq!(store.attempts(), 1);
    }
}

#[test]
fn refused_or_unrelated_write_does_not_steal_another_stage() {
    let store = Arc::new(Memory::default());
    let mut state = state(&store);
    let (started, go) = store.hold();
    let wanted = whole("wanted");
    let wait = staged(state.stage_writes(std::slice::from_ref(&wanted)));
    started.recv_timeout(WAIT).unwrap();
    assert!(apply(&mut state, &whole("bad;name")).is_err());
    assert!(state
        .write(
            P::DESCRIPTION,
            None,
            &PropertyValue::CharacterString("name".into())
        )
        .is_none());
    assert!(state.provision(profile("bad;name")).is_err());
    assert!(state.has_staged_write());
    assert!(matches!(
        state.stage_writes(&[whole("other")]),
        StageStep::Busy(_)
    ));
    drop(go);
    wait.block();
    apply(&mut state, &wanted).unwrap();
    assert_eq!(store.attempts(), 1);
}

#[test]
fn partial_claim_release_corrects_to_successful_prefix() {
    let store = Arc::new(Memory::default());
    let mut state = state(&store);
    let writes = [whole("prefix"), whole("suffix")];
    let wait = staged(state.stage_writes(&writes));
    wait.block();
    apply(&mut state, &writes[0]).unwrap();
    state.release_staged_write(&wait);
    state.wait_for_saves();
    assert_eq!(store.saved(), Some(snapshot("prefix")));
    assert_eq!(store.attempts(), 2);
}

#[test]
fn dropped_forgotten_expired_and_reprovisioned_stages_correct_saved_authority() {
    for action in 0..4 {
        let store = Arc::new(Memory::default());
        let mut state = state(&store);
        let wait = staged(state.stage_writes(&[whole("never-served")]));
        wait.block();
        match action {
            0 => {
                drop(state);
            }
            1 => {
                state.settle_forgotten_writes().unwrap().block();
            }
            2 => {
                state.expire(Duration::ZERO);
                state.expire(Duration::from_secs(11));
                state.wait_for_saves();
            }
            _ => {
                state.provision(ObjectProfile::default()).unwrap();
                state.wait_for_saves();
                assert!(state.profile().tags.is_none());
            }
        }
        assert_eq!(store.saved(), Some(TagsSnapshot::default()));
        assert_eq!(store.attempts(), 2);
    }
}

#[test]
fn failed_staged_save_never_publishes_or_needs_a_correction() {
    let store = Arc::new(Memory::default());
    let mut state = state(&store);
    store.fail_save.store(true, Ordering::SeqCst);
    let write = whole("refused");
    let wait = staged(state.stage_writes(std::slice::from_ref(&write)));
    wait.block();
    assert_code(apply(&mut state, &write), ErrorCode::OPERATIONAL_PROBLEM);
    state.release_staged_write(&wait);
    state.settle_forgotten_writes().unwrap().block();
    assert_eq!(store.attempts(), 1);
    assert_eq!(state.profile().tags, Some(tags("configured")));
    assert_eq!(store.saved(), None);
}

fn detached_clones<T: BACnetObject + Clone>(mut object: T, store: &Arc<Memory>) {
    let old = object.read_property(P::TAGS, None).unwrap();
    let (started, go) = store.hold();
    let desired = framed(&tags("new"));
    let wait = staged(object.durable_writes_internal().unwrap().stage_write(
        P::TAGS,
        None,
        &desired,
    ));
    started.recv_timeout(WAIT).unwrap();
    let mut cloned = object.clone();
    assert_eq!(cloned.read_property(P::TAGS, None).unwrap(), old);
    assert!(cloned.durable_writes_internal().is_none());
    cloned
        .write_property(P::TAGS, None, framed(&tags("clone-only")), None)
        .unwrap();
    drop(cloned);
    let mut cov = object.cov_snapshot_internal().unwrap();
    assert_eq!(cov.read_property(P::TAGS, None).unwrap(), old);
    assert!(cov.durable_writes_internal().is_none());
    drop(cov);
    assert_eq!(store.attempts(), 1);
    assert!(object.durable_writes_internal().unwrap().has_staged_write());
    assert_eq!(object.read_property(P::TAGS, None).unwrap(), old);
    drop(go);
    wait.block();
    object.write_property(P::TAGS, None, desired, None).unwrap();
    assert_eq!(store.saved(), Some(snapshot("new")));
    assert_eq!(store.attempts(), 1);
}

#[test]
fn all_four_application_clones_and_cov_snapshots_detach_a_held_writer() {
    macro_rules! check {
        ($ty:ty) => {{
            let store = Arc::new(Memory::default());
            let mut object = <$ty>::with_tags_persistence(1, "profile", store.clone()).unwrap();
            object.set_profile(profile("configured")).unwrap();
            detached_clones(object, &store);
        }};
    }
    check!(crate::color::ColorObject);
    check!(crate::color::ColorTemperatureObject);
    check!(crate::lighting::LightingOutputObject);
    check!(crate::lighting::BinaryLightingOutputObject);
}

#[test]
fn provisioning_supersedes_a_stage_without_publishing_it_or_detaching_storage() {
    let store = Arc::new(Memory::default());
    let mut state = state(&store);
    let (started, go) = store.hold();
    let wanted = whole("wanted");
    let wait = staged(state.stage_writes(std::slice::from_ref(&wanted)));
    started.recv_timeout(WAIT).unwrap();
    state.provision(profile("changed-config")).unwrap();
    assert_eq!(state.profile().tags, Some(tags("changed-config")));
    assert!(!state.has_staged_write());
    assert!(state.capability().is_some());
    drop(go);
    wait.block();
    state.wait_for_saves();
    assert_eq!(store.saved(), Some(TagsSnapshot::default()));
    // The old request cannot claim its old stage; this is a fresh saved write.
    apply(&mut state, &wanted).unwrap();
    assert_eq!(store.attempts(), 3);
    assert_eq!(store.saved(), Some(snapshot("wanted")));
    let mut detached = state.clone();
    detached.provision(profile("clone-config")).unwrap();
    assert_eq!(detached.profile().tags, Some(tags("clone-config")));
    assert_eq!(state.profile().tags, Some(tags("wanted")));
    assert_eq!(store.attempts(), 3);
}

#[test]
fn an_unstaged_valid_write_supersedes_the_old_stage_in_writer_order() {
    let store = Arc::new(Memory::default());
    let mut state = state(&store);
    let wait = staged(state.stage_writes(&[whole("never-served")]));
    wait.block();
    apply(&mut state, &whole("replacement")).unwrap();
    state.release_staged_write(&wait);
    state.wait_for_saves();
    assert_eq!(store.saved(), Some(snapshot("replacement")));
    assert_eq!(state.profile().tags, Some(tags("replacement")));
    assert_eq!(store.attempts(), 3);
}
