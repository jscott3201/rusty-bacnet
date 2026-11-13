//! The datatype the server learns for a member in another device stays with
//! that member until it is replaced or the server forgets it (#1342).
use super::tests::{device, encoded, member};
use super::*;
use PropertyIdentifier as P;

/// AV-`instance`'s Present_Value in Device 9.
fn remote(instance: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        device_identifier: Some(device(9)),
        ..member(instance)
    }
}

/// What each member of the next distribution carries as learned.
fn learned(channel: &mut ChannelObject) -> Vec<Option<MemberDatatype>> {
    channel
        .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(1.0), None)
        .unwrap();
    let run = channel.take_command_run().unwrap();
    // Ended, so the next write isn't refused BUSY.
    channel.complete_command_run(run.generation, Ok(()));
    let RunPlan::Channel(distribution) = run.plan else {
        panic!("a Channel run");
    };
    distribution
        .members
        .iter()
        .map(|member| member.learned)
        .collect()
}

fn learn(channel: &mut ChannelObject, slot: usize, instance: u32) {
    let datatype = MemberDatatype::Enumerated;
    channel.remember_member_datatype(slot, &remote(instance), Some(datatype));
}

#[test]
fn a_learned_datatype_stays_until_its_member_is_replaced() {
    let mut channel = ChannelObject::new(1, "CH-1", 7).unwrap();
    channel
        .set_members(vec![remote(1), remote(2), remote(3)])
        .unwrap();
    // Slot 1 holds AV-2, not AV-1, so nothing is kept for it.
    learn(&mut channel, 1, 1);
    learn(&mut channel, 0, 1);
    learn(&mut channel, 2, 3);
    let some = Some(MemberDatatype::Enumerated);
    assert_eq!(learned(&mut channel), [some, None, some]);
    assert_eq!(learned(&mut channel), [some, None, some]);

    // The server forgets one with `None`, again only for that reference.
    channel.remember_member_datatype(0, &remote(2), None);
    assert_eq!(learned(&mut channel), [some, None, some]);
    channel.remember_member_datatype(0, &remote(1), None);
    assert_eq!(learned(&mut channel), [None, None, some]);
    learn(&mut channel, 0, 1);

    // An element write forgets that member alone, even for the same
    // reference.
    let list = P::LIST_OF_OBJECT_PROPERTY_REFERENCES;
    let same = PropertyValue::ApplicationData(encoded(&[remote(3)]));
    channel.write_property(list, Some(3), same, None).unwrap();
    assert_eq!(learned(&mut channel), [some, None, None]);

    // A resize keeps what the members it keeps had learned.
    learn(&mut channel, 2, 3);
    channel
        .write_property(list, Some(0), PropertyValue::Unsigned(2), None)
        .unwrap();
    assert_eq!(learned(&mut channel), [some, None]);
    channel
        .write_property(
            P::EXECUTION_DELAY,
            Some(0),
            PropertyValue::Unsigned(3),
            None,
        )
        .unwrap();
    channel
        .write_property(
            list,
            Some(3),
            PropertyValue::ApplicationData(encoded(&[remote(3)])),
            None,
        )
        .unwrap();
    assert_eq!(learned(&mut channel), [some, None, None]);

    // A whole write, or `set_members`, forgets them all.
    let whole = PropertyValue::ApplicationData(encoded(&[remote(1), remote(2)]));
    channel.write_property(list, None, whole, None).unwrap();
    assert_eq!(learned(&mut channel), [None, None]);
    learn(&mut channel, 0, 1);
    channel.set_members(vec![remote(1)]).unwrap();
    assert_eq!(learned(&mut channel), [None]);
}

/// A staged write is worked out on the configuration served as it is made
/// (#2526), so a datatype learned between a request's stage and its write
/// stays, for every member the write doesn't replace.
#[cfg(feature = "durable-writes")]
#[test]
fn a_datatype_learned_while_a_write_is_staged_stays() {
    use crate::durable::{DurableWrites, PendingWrite, StageStep};
    use crate::write_config::WriteConfigPersistence;
    use crate::write_config::test_storage::MemoryConfig;
    let storage = MemoryConfig::new();
    let persistence =
        alloc::sync::Arc::clone(&storage) as alloc::sync::Arc<dyn WriteConfigPersistence>;
    let mut channel = ChannelObject::with_persistence(1, "CH-1", 7, persistence).unwrap();
    channel.set_members(vec![remote(1), remote(2)]).unwrap();
    let write = PendingWrite {
        property: P::EXECUTION_DELAY,
        array_index: Some(1),
        value: PropertyValue::Unsigned(5),
    };
    let StageStep::Staged(wait) = channel.stage_writes(core::slice::from_ref(&write)) else {
        panic!("the delay stages");
    };
    wait.block();
    let staged = storage.saved();
    // Learning isn't a counted change, so the write still takes its step:
    // nothing gives way, and the stage's save is the only one.
    learn(&mut channel, 0, 1);
    channel
        .write_property(write.property, write.array_index, write.value, None)
        .unwrap();
    assert!(!channel.has_staged_write(), "the write took its step");
    channel.release_staged_write(&wait);
    channel.wait_for_saves();
    assert_eq!(storage.saves(), 1);
    assert_eq!(storage.saved(), staged);
    assert_eq!(
        learned(&mut channel),
        [Some(MemberDatatype::Enumerated), None]
    );
}
