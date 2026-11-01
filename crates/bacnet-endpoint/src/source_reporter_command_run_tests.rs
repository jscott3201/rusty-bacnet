//! SourceReporter forwards the run hooks of a Command (#1150) or Channel
//! (#1151) object instead of inheriting the trait defaults, which would
//! leave a wrapped Command or Channel busy.
use super::*;
use bacnet_objects::channel::ChannelObject;
use bacnet_objects::command::{CommandObject, WriteFailure};
use bacnet_types::constructed::{
    BACnetActionCommand, BACnetActionList, BACnetDeviceObjectPropertyReference,
};
use bacnet_types::enums::{Reliability, WriteStatus};

#[test]
fn channel_run_hooks_survive_wrapping() {
    let mut channel = ChannelObject::new(1, "CH-1", 7).unwrap();
    channel
        .set_members(vec![BACnetDeviceObjectPropertyReference::new_local(
            oid(ObjectType::ANALOG_VALUE, 1),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )])
        .unwrap();
    let mut object: Box<dyn BACnetObject> = Box::new(channel);
    let owner = bacnet_objects::database::AuditOwnership::for_source(
        oid(ObjectType::DEVICE, 123),
        selected(),
    );
    source_reporter::install(&mut object, &owner).unwrap();

    object
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(1.0),
            Some(8),
        )
        .unwrap();
    let run = object
        .command_runner_mut()
        .unwrap()
        .take_command_run()
        .unwrap();
    let generation = object.command_runner().map(|r| r.command_generation());
    assert_eq!(generation, Some(run.generation));
    // The failure's kind reaches the Channel too (#1264).
    let failure = Err(WriteFailure::Communication);
    let runner = object.command_runner_mut().unwrap();
    assert!(runner.complete_command_run(run.generation, failure));
    assert_eq!(
        object
            .read_property(PropertyIdentifier::WRITE_STATUS, None)
            .unwrap(),
        PropertyValue::Enumerated(WriteStatus::FAILED.to_raw())
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw())
    );
}

#[test]
fn command_run_hooks_and_property_cov_admission_survive_wrapping() {
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    command
        .set_action(vec![BACnetActionList {
            commands: vec![BACnetActionCommand {
                device_identifier: None,
                object_identifier: oid(ObjectType::ANALOG_VALUE, 1),
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                property_value: PropertyValue::Real(1.0),
                priority: None,
                post_delay: None,
                quit_on_failure: false,
                write_successful: false,
            }],
        }])
        .unwrap();
    let mut object: Box<dyn BACnetObject> = Box::new(command);
    let owner = bacnet_objects::database::AuditOwnership::for_source(
        oid(ObjectType::DEVICE, 123),
        selected(),
    );
    source_reporter::install(&mut object, &owner).unwrap();
    assert!(object.supports_subscribe_cov_property());

    object
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(1),
            None,
        )
        .unwrap();
    let run = object
        .command_runner_mut()
        .unwrap()
        .take_command_run()
        .unwrap();
    let generation = object.command_runner().map(|r| r.command_generation());
    assert_eq!(generation, Some(run.generation));
    let runner = object.command_runner_mut().unwrap();
    assert!(runner.record_command_write(run.generation, 0, true));
    assert!(runner.complete_command_run(run.generation, Ok(())));
    for (property, expected) in [
        (PropertyIdentifier::IN_PROCESS, false),
        (PropertyIdentifier::ALL_WRITES_SUCCESSFUL, true),
    ] {
        assert_eq!(
            object.read_property(property, None).unwrap(),
            PropertyValue::Boolean(expected),
            "{property:?}"
        );
    }
}
