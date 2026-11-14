//! A setter given the value the Channel already serves changes nothing, so
//! a staged request stays staged and is saved once; one given another value
//! makes the staged write give way (#2552).

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::PropertyValue;

use super::ChannelObject;
use super::learned_tests::{learn, learned, remote};
use super::saving_tests::{channel, make, member, pending, staged};
use crate::durable::DurableWrites;
use crate::write_config::test_storage::MemoryConfig;

/// One call of a setter on the Channel `channel` serves: the value it holds,
/// or another.
type Call = fn(&mut ChannelObject);

/// Each setter with a call that gives the served value and one that gives
/// another (the served ones: one member with a 10 ms delay, groups [2],
/// Allow_Group_Delay_Inhibit off).
const SETTERS: [(&str, Call, Call); 4] = [
    (
        "set_members",
        |c| c.set_members(vec![member(1)]).unwrap(),
        |c| c.set_members(vec![member(2)]).unwrap(),
    ),
    (
        "set_execution_delay",
        |c| c.set_execution_delay(vec![10]).unwrap(),
        |c| c.set_execution_delay(vec![11]).unwrap(),
    ),
    (
        "set_control_groups",
        |c| c.set_control_groups(vec![2]).unwrap(),
        |c| c.set_control_groups(vec![3]).unwrap(),
    ),
    (
        "set_allow_group_delay_inhibit",
        |c| c.set_allow_group_delay_inhibit(false),
        |c| c.set_allow_group_delay_inhibit(true),
    ),
];

/// A request that stages two writes, which the step each takes is worked out
/// on and which another save must not repeat.
fn request() -> [crate::durable::PendingWrite; 2] {
    [
        pending(
            PropertyIdentifier::EXECUTION_DELAY,
            Some(1),
            PropertyValue::Unsigned(5),
        ),
        pending(
            PropertyIdentifier::ALLOW_GROUP_DELAY_INHIBIT,
            None,
            PropertyValue::Boolean(true),
        ),
    ]
}

/// Called mid-request with the value served, a setter counts no change: both
/// writes take their steps and the stage's save is the only one.
#[test]
fn an_unchanged_setter_during_a_stage_leaves_the_staged_write() {
    for (name, unchanged, _) in SETTERS {
        let storage = MemoryConfig::new();
        let mut channel = channel(&storage);
        let writes = request();
        let wait = staged(&mut channel, &writes);
        let saved = storage.saved();
        unchanged(&mut channel);
        assert!(channel.has_staged_write(), "{name}");
        for write in &writes {
            make(&mut channel, write).unwrap();
        }
        assert!(!channel.has_staged_write(), "{name}");
        channel.release_staged_write(&wait);
        channel.wait_for_saves();
        assert_eq!(storage.saved(), saved, "{name}");
        assert_eq!(storage.saves(), 1, "{name}");
    }
}

/// Given another value, each makes the staged write give way.
#[test]
fn a_changing_setter_during_a_stage_makes_the_staged_write_give_way() {
    for (name, _, changed) in SETTERS {
        let storage = MemoryConfig::new();
        let mut channel = channel(&storage);
        let wait = staged(&mut channel, &request());
        assert!(channel.has_staged_write(), "{name}");
        changed(&mut channel);
        assert!(!channel.has_staged_write(), "{name}");
        channel.release_staged_write(&wait);
        channel.wait_for_saves();
    }
}

/// `set_members` forgets what was learned for the members even when the list
/// is the one served. That is no counted change: storage doesn't keep it, and
/// a staged write takes what is learned from what is served as it is made.
#[test]
fn an_unchanged_set_members_still_forgets_what_was_learned() {
    let storage = MemoryConfig::new();
    let mut channel = channel(&storage);
    channel.set_members(vec![remote(1), remote(2)]).unwrap();
    channel.set_execution_delay(vec![10, 20]).unwrap();
    learn(&mut channel, 0, 1);
    let write = pending(
        PropertyIdentifier::EXECUTION_DELAY,
        Some(1),
        PropertyValue::Unsigned(5),
    );
    let wait = staged(&mut channel, core::slice::from_ref(&write));
    channel.set_members(vec![remote(1), remote(2)]).unwrap();
    assert!(channel.has_staged_write());
    make(&mut channel, &write).unwrap();
    channel.release_staged_write(&wait);
    channel.wait_for_saves();
    assert_eq!(storage.saves(), 1);
    assert_eq!(learned(&mut channel), [None, None]);
}
