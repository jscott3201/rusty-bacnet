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
