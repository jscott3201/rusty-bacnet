//! A setter given the value the Timer already serves counts no change, so a
//! staged request stays staged and is saved once; one given another value
//! makes the staged write give way (#2552). What a setter does besides
//! changing the value still happens, and counts when it changes what is
//! served or saved.

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::PropertyValue;

use super::saving_tests::{kept, make, pending, reference, staged, timer, values};
use super::{DEFAULT_MAX_PRES_VALUE, DEFAULT_MIN_PRES_VALUE, DEFAULT_TIMEOUT, TimerObject};
use crate::durable::{DurableWrites, PendingWrite};
use crate::traits::BACnetObject;
use crate::write_config::test_storage::MemoryConfig;

type Call = fn(&mut TimerObject);

/// Each setter with a call that gives what the Timer from `timer` serves
/// (its set of values(1.0), reference 1 and priority 9, and the default
/// Default_Timeout and range) and one that gives another.
const SETTERS: [(&str, Call, Call); 3] = [
    (
        "set_state_change",
        |t| {
            t.set_state_change(values(1.0), vec![reference(1)], 9)
                .unwrap()
        },
        |t| {
            t.set_state_change(values(2.0), vec![reference(1)], 9)
                .unwrap()
        },
    ),
    (
        "set_default_timeout",
        |t| t.set_default_timeout(DEFAULT_TIMEOUT).unwrap(),
        |t| t.set_default_timeout(DEFAULT_TIMEOUT + 1).unwrap(),
    ),
    (
        "set_pres_value_range",
        |t| {
            t.set_pres_value_range(DEFAULT_MIN_PRES_VALUE, DEFAULT_MAX_PRES_VALUE)
                .unwrap()
        },
        |t| {
            t.set_pres_value_range(DEFAULT_MIN_PRES_VALUE, DEFAULT_MAX_PRES_VALUE + 1)
                .unwrap()
        },
    ),
];

fn request() -> [PendingWrite; 2] {
    [
        pending(
            PropertyIdentifier::STATE_CHANGE_VALUES,
            Some(2),
            PropertyValue::Real(7.0),
        ),
        pending(
            PropertyIdentifier::DEFAULT_TIMEOUT,
            None,
            PropertyValue::Unsigned(5_000),
        ),
    ]
}

#[test]
fn an_unchanged_setter_during_a_stage_leaves_the_staged_write() {
    for (name, unchanged, _) in SETTERS {
        let storage = MemoryConfig::new();
        let mut timer = timer(&storage, 1.0);
        let writes = request();
        let wait = staged(&mut timer, &writes);
        let saved = storage.saved();
        unchanged(&mut timer);
        assert!(timer.has_staged_write(), "{name}");
        for write in &writes {
            make(&mut timer, write).unwrap();
        }
        assert!(!timer.has_staged_write(), "{name}");
        timer.release_staged_write(&wait);
        timer.wait_for_saves();
        assert_eq!(storage.saved(), saved, "{name}");
        assert_eq!(storage.saves(), 1, "{name}");
        assert_eq!(kept(&storage).1, Some(5_000), "{name}");
    }
}

#[test]
fn a_changing_setter_during_a_stage_makes_the_staged_write_give_way() {
    for (name, _, changed) in SETTERS {
        let storage = MemoryConfig::new();
        let mut timer = timer(&storage, 1.0);
        let wait = staged(&mut timer, &request());
        assert!(timer.has_staged_write(), "{name}");
        changed(&mut timer);
        assert!(!timer.has_staged_write(), "{name}");
        timer.release_staged_write(&wait);
        timer.wait_for_saves();
    }
}

/// The values are compared bit for bit: a NaN given again is the same set,
/// and the other zero is another.
#[test]
fn state_change_values_are_the_same_set_bit_for_bit() {
    let nan = f32::from_bits(0x7fc0_1234);
    let with = |first: f32| {
        let storage = MemoryConfig::new();
        let mut timer = timer(&storage, nan);
        let wait = staged(&mut timer, &request());
        timer
            .set_state_change(values(first), vec![reference(1)], 9)
            .unwrap();
        let held = timer.has_staged_write();
        timer.release_staged_write(&wait);
        timer.wait_for_saves();
        held
    };
    assert!(with(nan), "the same NaN");
    assert!(!with(f32::from_bits(0x7fc0_1235)), "another NaN");
    let storage = MemoryConfig::new();
    let mut zero = timer(&storage, 0.0);
    let wait = staged(&mut zero, &request());
    zero.set_state_change(values(-0.0), vec![reference(1)], 9)
        .unwrap();
    assert!(!zero.has_staged_write(), "the other zero");
    zero.release_staged_write(&wait);
    zero.wait_for_saves();
    // The served set is the one given.
    assert_eq!(
        zero.read_property(PropertyIdentifier::STATE_CHANGE_VALUES, Some(1))
            .unwrap(),
        PropertyValue::Real(-0.0)
    );
    let PropertyValue::Real(served) = zero
        .read_property(PropertyIdentifier::STATE_CHANGE_VALUES, Some(1))
        .unwrap()
    else {
        panic!("a REAL");
    };
    assert_eq!(served.to_bits(), (-0.0f32).to_bits());
}

/// An unchanged `set_state_change` still restarts the run: a transition's
/// writes queued before it are gone. The run isn't saved and no staged step
/// is worked out from it, so it counts no change.
#[test]
fn an_unchanged_set_state_change_still_restarts_the_run() {
    let storage = MemoryConfig::new();
    let mut timer = timer(&storage, 1.0);
    let writes = request();
    let wait = staged(&mut timer, &writes);
    timer
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(5_000),
            None,
        )
        .unwrap();
    assert!(timer.has_pending_writes());
    timer
        .set_state_change(values(1.0), vec![reference(1)], 9)
        .unwrap();
    assert!(!timer.has_pending_writes());
    assert!(timer.has_staged_write());
    timer.release_staged_write(&wait);
    timer.wait_for_saves();
}

/// A range given again decides a held Default_Timeout all the same: it was
/// past the range a new Timer takes, so it is dropped from what storage
/// holds, and that counts as a change though the range is the one served.
#[test]
fn an_unchanged_range_still_decides_a_held_default_timeout_and_counts() {
    use super::timeout_saving_tests as held;
    let storage = held::holding_long();
    let mut timer = held::timer(&storage, 30_000);
    let writes = request();
    let wait = staged(&mut timer, &writes[..1]);
    assert!(timer.has_staged_write());
    timer
        .set_pres_value_range(DEFAULT_MIN_PRES_VALUE, DEFAULT_MAX_PRES_VALUE)
        .unwrap();
    assert!(!timer.has_staged_write(), "the staged write gave way");
    assert_eq!(
        timer
            .read_property(PropertyIdentifier::DEFAULT_TIMEOUT, None)
            .unwrap(),
        PropertyValue::Unsigned(30_000)
    );
    timer.release_staged_write(&wait);
    timer.wait_for_saves();
    assert_eq!(held::saved(&storage).default_timeout, None);
}

/// Another priority alone, or other references alone, is another set.
#[test]
fn another_priority_or_other_references_make_the_staged_write_give_way() {
    let calls: [(&str, Call); 2] = [
        ("priority", |t| {
            t.set_state_change(values(1.0), vec![reference(1)], 10)
                .unwrap()
        }),
        ("references", |t| {
            t.set_state_change(values(1.0), vec![reference(2)], 9)
                .unwrap()
        }),
    ];
    for (name, call) in calls {
        let storage = MemoryConfig::new();
        let mut timer = timer(&storage, 1.0);
        let wait = staged(&mut timer, &request());
        call(&mut timer);
        assert!(!timer.has_staged_write(), "{name}");
        timer.release_staged_write(&wait);
        timer.wait_for_saves();
    }
}

/// Two sets whose values' encodings, joined, are the same octets (a
/// constructed value whose octets close and reopen the frame) are still two
/// sets.
#[test]
fn sets_whose_joined_encodings_agree_are_not_the_same_set() {
    use bacnet_types::constructed::BACnetTimerStateChangeValue as V;
    let a = vec![0x21, 0x01];
    let mut join = a.clone();
    join.extend_from_slice(&[0x1F, 0x1E]);
    join.extend_from_slice(&a);
    let held = [
        V::Constructed(join.clone()),
        V::NoValue,
        V::NoValue,
        V::NoValue,
        V::NoValue,
        V::Constructed(a.clone()),
        V::Constructed(a.clone()),
    ];
    let given = [
        V::Constructed(a.clone()),
        V::Constructed(a.clone()),
        V::NoValue,
        V::NoValue,
        V::NoValue,
        V::NoValue,
        V::Constructed(join),
    ];
    let storage = MemoryConfig::new();
    let persistence = alloc::sync::Arc::clone(&storage)
        as alloc::sync::Arc<dyn crate::write_config::WriteConfigPersistence>;
    let mut timer = TimerObject::with_persistence(1, "TMR", persistence).unwrap();
    timer.set_state_change(held, vec![reference(1)], 9).unwrap();
    let write = pending(
        PropertyIdentifier::STATE_CHANGE_VALUES,
        Some(3),
        PropertyValue::Real(7.0),
    );
    let wait = staged(&mut timer, core::slice::from_ref(&write));
    timer
        .set_state_change(given, vec![reference(1)], 9)
        .unwrap();
    assert!(!timer.has_staged_write());
    make(&mut timer, &write).unwrap();
    timer.release_staged_write(&wait);
    timer.wait_for_saves();
    let saved = kept(&storage).0.expect("a written set").values;
    for index in 1..=7 {
        let served = timer
            .read_property(PropertyIdentifier::STATE_CHANGE_VALUES, Some(index))
            .unwrap();
        let held = &saved[index as usize - 1];
        let mut octets = bytes::BytesMut::new();
        bacnet_encoding::constructed::encode_timer_state_change_value(&mut octets, held).unwrap();
        let held = match held {
            bacnet_types::constructed::BACnetTimerStateChangeValue::Primitive(v) => v.clone(),
            _ => PropertyValue::ApplicationData(octets.to_vec()),
        };
        assert_eq!(served, held, "element {index}");
    }
}

/// An unchanged `set_default_timeout` still remembers the value as the
/// application's own, which a later range decision falls back to.
#[test]
fn an_unchanged_default_timeout_is_still_remembered() {
    let storage = MemoryConfig::new();
    let mut timer = timer(&storage, 1.0);
    let unsigned = PropertyValue::Unsigned;
    let write = |timer: &mut TimerObject, value| {
        timer
            .write_property(
                PropertyIdentifier::DEFAULT_TIMEOUT,
                None,
                unsigned(value),
                None,
            )
            .unwrap()
    };
    write(&mut timer, 5_000);
    timer.clear_written_config().unwrap();
    timer.set_default_timeout(5_000).unwrap();
    write(&mut timer, 7_000);
    timer.set_pres_value_range(1, 6_000).unwrap();
    assert_eq!(
        timer
            .read_property(PropertyIdentifier::DEFAULT_TIMEOUT, None)
            .unwrap(),
        unsigned(5_000)
    );
}
