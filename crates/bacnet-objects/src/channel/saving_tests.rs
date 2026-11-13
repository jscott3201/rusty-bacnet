//! The configuration a peer writes to a Channel kept across a rebuild
//! (#2243).

use alloc::sync::Arc;

use bacnet_encoding::constructed::encode_device_object_property_reference;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

use super::ChannelObject;
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::durable::{DurableWrites, PendingWrite, StageStep};
use crate::traits::BACnetObject;
use crate::write_config::test_storage::MemoryConfig;
use crate::write_config::{
    ChannelMembers, ChannelWriteConfig, TimerWriteConfig, WriteConfigPersistence,
    WriteConfigSnapshot,
};

const MEMBERS: PropertyIdentifier = PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES;
const DELAYS: PropertyIdentifier = PropertyIdentifier::EXECUTION_DELAY;
const NUMBER: PropertyIdentifier = PropertyIdentifier::CHANNEL_NUMBER;
const GROUPS: PropertyIdentifier = PropertyIdentifier::CONTROL_GROUPS;
const INHIBIT: PropertyIdentifier = PropertyIdentifier::ALLOW_GROUP_DELAY_INHIBIT;

fn member(instance: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, instance).unwrap(),
        property_identifier: 85,
        property_array_index: None,
        device_identifier: None,
    }
}

fn member_value(reference: &BACnetDeviceObjectPropertyReference) -> PropertyValue {
    let mut octets = BytesMut::new();
    encode_device_object_property_reference(&mut octets, reference);
    PropertyValue::ApplicationData(octets.to_vec())
}

fn unsigned_list(values: &[u64]) -> PropertyValue {
    PropertyValue::List(values.iter().map(|v| PropertyValue::Unsigned(*v)).collect())
}

/// Channel 1 kept in `storage`, configured as an application would: one
/// member with a 10 ms delay, number 4, groups [2] and inhibit off.
fn channel(storage: &Arc<MemoryConfig>) -> ChannelObject {
    let persistence = Arc::clone(storage) as Arc<dyn WriteConfigPersistence>;
    let mut channel = ChannelObject::with_persistence(1, "CH", 4, persistence).unwrap();
    channel.set_members(vec![member(1)]).unwrap();
    channel.set_execution_delay(vec![10]).unwrap();
    channel.set_control_groups(vec![2]).unwrap();
    channel.set_allow_group_delay_inhibit(false);
    channel
}

fn read(channel: &ChannelObject, property: PropertyIdentifier) -> PropertyValue {
    channel.read_property(property, None).unwrap()
}

fn write(
    channel: &mut ChannelObject,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: PropertyValue,
) -> Result<(), Error> {
    channel.write_property(property, index, value, None)
}

/// A write to each part survives a rebuild and wins over the configured
/// value; the part nobody wrote takes the configured one.
#[test]
fn each_written_part_survives_a_rebuild() {
    for (property, index, value) in [
        (MEMBERS, Some(1), member_value(&member(9))),
        (DELAYS, Some(1), PropertyValue::Unsigned(500)),
        (MEMBERS, Some(0), PropertyValue::Unsigned(2)),
        (NUMBER, None, PropertyValue::Unsigned(77)),
        (GROUPS, None, unsigned_list(&[5, 6])),
        (INHIBIT, None, PropertyValue::Boolean(true)),
    ] {
        let storage = MemoryConfig::new();
        let served = {
            let mut channel = channel(&storage);
            write(&mut channel, property, index, value.clone()).unwrap();
            assert_eq!(storage.saves(), 1, "{property:?}");
            [MEMBERS, DELAYS, NUMBER, GROUPS, INHIBIT].map(|p| read(&channel, p))
        };
        let rebuilt = channel(&storage);
        let after = [MEMBERS, DELAYS, NUMBER, GROUPS, INHIBIT].map(|p| read(&rebuilt, p));
        assert_eq!(after, served, "{property:?} at {index:?}");
    }
}

/// Every part written: the saved snapshot holds them all, and the
/// configured values given at the next start are checked and passed over.
#[test]
fn a_fully_written_channel_keeps_its_configuration() {
    let storage = MemoryConfig::new();
    {
        let mut channel = channel(&storage);
        write(
            &mut channel,
            MEMBERS,
            None,
            PropertyValue::List(vec![member_value(&member(3)), member_value(&member(4))]),
        )
        .unwrap();
        write(&mut channel, DELAYS, None, unsigned_list(&[1, 2])).unwrap();
        write(&mut channel, NUMBER, None, PropertyValue::Unsigned(9)).unwrap();
        write(&mut channel, GROUPS, None, unsigned_list(&[7])).unwrap();
        write(&mut channel, INHIBIT, None, PropertyValue::Boolean(true)).unwrap();
    }
    assert_eq!(
        storage.saved(),
        Some(WriteConfigSnapshot::Channel(ChannelWriteConfig {
            members: Some(ChannelMembers {
                references: vec![member(3), member(4)],
                execution_delay: vec![1, 2],
            }),
            channel_number: Some(9),
            control_groups: Some(vec![7]),
            allow_group_delay_inhibit: Some(true),
            description: None,
        }))
    );
    let mut rebuilt = channel(&storage);
    assert_eq!(read(&rebuilt, NUMBER), PropertyValue::Unsigned(9));
    assert_eq!(read(&rebuilt, DELAYS), unsigned_list(&[1, 2]));
    // A configured value the setter would refuse is still refused.
    assert!(rebuilt.set_control_groups(vec![]).is_err());
}

#[test]
fn a_write_that_cannot_be_saved_is_refused_and_the_old_value_stays() {
    let storage = MemoryConfig::new();
    let mut channel = channel(&storage);
    storage.fail(true);
    let error = write(&mut channel, GROUPS, Some(1), PropertyValue::Unsigned(8)).unwrap_err();
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::DEVICE && code == ErrorCode::OPERATIONAL_PROBLEM),
        "{error:?}"
    );
    assert_eq!(read(&channel, GROUPS), unsigned_list(&[2]));
    assert!(!channel.saved().groups);
}

/// A WritePropertyMultiple's writes to one Channel stage one save of what
/// they leave together, and each takes its step (#1423).
#[cfg(feature = "durable-writes")]
#[test]
fn a_requests_writes_stage_one_save() {
    let storage = MemoryConfig::new();
    let mut channel = channel(&storage);
    let writes = [
        PendingWrite {
            property: MEMBERS,
            array_index: Some(0),
            value: PropertyValue::Unsigned(3),
        },
        PendingWrite {
            property: PropertyIdentifier::DESCRIPTION,
            array_index: None,
            value: PropertyValue::CharacterString("lights".into()),
        },
        PendingWrite {
            property: DELAYS,
            array_index: Some(3),
            value: PropertyValue::Unsigned(30),
        },
    ];
    let StageStep::Staged(wait) = channel.stage_writes(&writes) else {
        panic!("the writes didn't stage");
    };
    wait.block();
    assert_eq!(storage.saves(), 1);
    for write in writes {
        channel
            .write_property(write.property, write.array_index, write.value, None)
            .unwrap();
    }
    channel.release_staged_write(&wait);
    channel.wait_for_saves();
    assert_eq!(storage.saves(), 1);
    assert_eq!(read(&channel, DELAYS), unsigned_list(&[10, 0, 30]));
}

/// Storage holding what the Channel can't serve refuses the build.
#[test]
fn a_saved_configuration_the_channel_would_refuse_fails_the_build() {
    let build = |snapshot| {
        ChannelObject::with_persistence(
            1,
            "CH",
            4,
            MemoryConfig::holding(snapshot) as Arc<dyn WriteConfigPersistence>,
        )
    };
    let mut remote = member(1);
    remote.device_identifier = Some(ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap());
    for refused in [
        WriteConfigSnapshot::Timer(TimerWriteConfig::default()),
        WriteConfigSnapshot::Channel(ChannelWriteConfig {
            control_groups: Some(vec![]),
            ..ChannelWriteConfig::default()
        }),
        WriteConfigSnapshot::Channel(ChannelWriteConfig {
            members: Some(ChannelMembers {
                references: vec![member(1)],
                execution_delay: vec![],
            }),
            ..ChannelWriteConfig::default()
        }),
        // A device identifier that isn't a Device.
        WriteConfigSnapshot::Channel(ChannelWriteConfig {
            members: Some(ChannelMembers {
                references: vec![remote],
                execution_delay: vec![0],
            }),
            ..ChannelWriteConfig::default()
        }),
    ] {
        assert!(build(refused.clone()).is_err(), "{refused:?} built");
    }
    let restored = build(WriteConfigSnapshot::Channel(ChannelWriteConfig {
        channel_number: Some(300),
        ..ChannelWriteConfig::default()
    }))
    .unwrap();
    assert_eq!(read(&restored, NUMBER), PropertyValue::Unsigned(300));
}

/// Clearing the written parts lets the configured ones apply again, now and
/// after a rebuild.
#[test]
fn clearing_the_written_config_lets_the_configured_one_apply() {
    let storage = MemoryConfig::new();
    write(&mut channel(&storage), GROUPS, None, unsigned_list(&[5, 6])).unwrap();
    let mut rebuilt = channel(&storage);
    assert!(rebuilt.saved().groups);
    rebuilt.clear_written_config().unwrap();
    assert!(!rebuilt.saved().groups);
    assert_eq!(
        storage.saved(),
        Some(WriteConfigSnapshot::Channel(ChannelWriteConfig::default()))
    );
    assert_eq!(read(&rebuilt, GROUPS), unsigned_list(&[5, 6]));
    rebuilt.set_control_groups(vec![2]).unwrap();
    assert_eq!(read(&rebuilt, GROUPS), unsigned_list(&[2]));
    drop(rebuilt);
    assert_eq!(read(&channel(&storage), GROUPS), unsigned_list(&[2]));
}

#[cfg(feature = "durable-writes")]
fn pending(property: PropertyIdentifier, index: Option<u32>, value: PropertyValue) -> PendingWrite {
    PendingWrite {
        property,
        array_index: index,
        value,
    }
}

/// Stage `writes` on `channel` and wait for the save; the wait releases it.
#[cfg(feature = "durable-writes")]
fn staged(channel: &mut ChannelObject, writes: &[PendingWrite]) -> crate::durable::SaveWait {
    let StageStep::Staged(wait) = channel.stage_writes(writes) else {
        panic!("the writes didn't stage");
    };
    wait.block();
    wait
}

#[cfg(feature = "durable-writes")]
fn make(channel: &mut ChannelObject, write: &PendingWrite) -> Result<(), Error> {
    let PendingWrite {
        property,
        array_index,
        value,
    } = write.clone();
    channel.write_property(property, array_index, value, None)
}

/// A step keeps only its write, so each write is worked out again on the
/// configuration served as it is made (#2526): every claim, the later ones
/// included, serves its request's prefix and takes its step, and storage
/// ends where the stage saved it, with one save.
#[cfg(feature = "durable-writes")]
#[test]
fn each_claim_makes_its_write_on_the_configuration_served() {
    let storage = MemoryConfig::new();
    let mut channel = channel(&storage);
    let writes = [
        pending(MEMBERS, Some(0), PropertyValue::Unsigned(3)),
        pending(DELAYS, Some(3), PropertyValue::Unsigned(30)),
        pending(DELAYS, Some(1), PropertyValue::Unsigned(5)),
        pending(MEMBERS, Some(2), member_value(&member(7))),
        pending(DELAYS, Some(2), PropertyValue::Unsigned(20)),
    ];
    let served: [(u64, [u64; 3]); 5] = [
        (3, [10, 0, 0]),
        (3, [10, 0, 30]),
        (3, [5, 0, 30]),
        (3, [5, 0, 30]),
        (3, [5, 20, 30]),
    ];
    let wait = staged(&mut channel, &writes);
    let saved = storage.saved();
    for (at, (write, (size, delays))) in writes.iter().zip(served).enumerate() {
        make(&mut channel, write).unwrap();
        let members = channel.read_property(MEMBERS, Some(0)).unwrap();
        assert_eq!(members, PropertyValue::Unsigned(size), "write {at}");
        assert_eq!(read(&channel, DELAYS), unsigned_list(&delays), "write {at}");
        assert_eq!(
            channel.has_staged_write(),
            at + 1 < writes.len(),
            "write {at}"
        );
    }
    let second = channel.read_property(MEMBERS, Some(2)).unwrap();
    assert_eq!(second, member_value(&member(7)));
    channel.release_staged_write(&wait);
    channel.wait_for_saves();
    assert_eq!(storage.saved(), saved);
    assert_eq!(storage.saves(), 1);
}

/// A Channel dropped part way through a request puts storage back to what
/// it served then: the writes it made, not the configuration before the
/// request nor the one the stage saved (#1363).
#[cfg(feature = "durable-writes")]
#[test]
fn a_drop_part_way_puts_storage_back_to_the_writes_made() {
    let storage = MemoryConfig::new();
    let mut channel = channel(&storage);
    let delay = |value| pending(DELAYS, Some(1), PropertyValue::Unsigned(value));
    let writes = [delay(5), delay(6)];
    let delays = |storage: &MemoryConfig| match storage.saved() {
        Some(WriteConfigSnapshot::Channel(saved)) => saved.members.map(|m| m.execution_delay),
        other => panic!("not a Channel's: {other:?}"),
    };
    let _wait = staged(&mut channel, &writes);
    assert_eq!(delays(&storage), Some(vec![6]));
    make(&mut channel, &writes[0]).unwrap();
    drop(channel);
    assert_eq!(delays(&storage), Some(vec![5]));
    assert_eq!(storage.saves(), 2);
}

/// A later write of a request is checked against the configuration the
/// earlier ones leave, not the one served: a delay past the end of a member
/// list the request shortens stops the stage, and is refused as made.
#[cfg(feature = "durable-writes")]
#[test]
fn a_later_write_is_checked_against_the_configuration_the_earlier_ones_leave() {
    let storage = MemoryConfig::new();
    let mut channel = channel(&storage);
    channel
        .set_members(vec![member(1), member(2), member(3)])
        .unwrap();
    channel.set_execution_delay(vec![10, 20, 30]).unwrap();
    let shorter = PropertyValue::List(vec![member_value(&member(4)), member_value(&member(5))]);
    let writes = [
        pending(MEMBERS, None, shorter),
        pending(DELAYS, Some(3), PropertyValue::Unsigned(9)),
    ];
    let wait = staged(&mut channel, &writes);
    let two = Some(WriteConfigSnapshot::Channel(ChannelWriteConfig {
        members: Some(ChannelMembers {
            references: vec![member(4), member(5)],
            execution_delay: vec![10, 20],
        }),
        ..ChannelWriteConfig::default()
    }));
    assert_eq!(storage.saved(), two);
    make(&mut channel, &writes[0]).unwrap();
    let error = make(&mut channel, &writes[1]).unwrap_err();
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { code, .. })
            if code == ErrorCode::INVALID_ARRAY_INDEX),
        "{error:?}"
    );
    channel.release_staged_write(&wait);
    channel.wait_for_saves();
    assert_eq!(storage.saved(), two);
    assert_eq!(storage.saves(), 1);
}
