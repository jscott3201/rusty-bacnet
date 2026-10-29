//! A Channel's Reliability and the FAULT flag follow how its distributions
//! end, and a client simulates them while it is out of service (Clauses
//! 12.53.8 to 12.53.10, #1264).
use super::tests::{assert_property_error, configured, device, member, read};
use super::*;
use PropertyIdentifier as P;
use bacnet_types::enums::ErrorCode;

fn reliability(channel: &ChannelObject) -> Reliability {
    match read(channel, P::RELIABILITY) {
        PropertyValue::Enumerated(raw) => Reliability::from_raw(raw),
        other => panic!("Reliability read {other:?}"),
    }
}

/// Whether Status_Flags reads FAULT.
fn fault(channel: &ChannelObject) -> bool {
    match read(channel, P::STATUS_FLAGS) {
        PropertyValue::BitString { data, .. } => data[0] & 0x40 != 0,
        other => panic!("Status_Flags read {other:?}"),
    }
}

/// Write `value` to Present_Value and end the run it queues with `outcome`.
fn distribute(channel: &mut ChannelObject, outcome: Result<(), WriteFailure>) {
    channel
        .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(1.0), None)
        .unwrap();
    let run = channel.take_command_run_internal().unwrap();
    assert!(channel.complete_command_run_internal(run.generation, outcome));
}

fn set_out_of_service(channel: &mut ChannelObject, value: bool) {
    channel
        .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(value), None)
        .unwrap();
}

fn simulate(channel: &mut ChannelObject, value: Reliability) -> Result<(), Error> {
    channel.write_property(
        P::RELIABILITY,
        None,
        PropertyValue::Enumerated(value.to_raw()),
        None,
    )
}

#[test]
fn channel_reliability_names_the_failure_and_clears_on_the_next_success() {
    let mut channel = configured();
    for (failure, expected) in [
        (
            WriteFailure::Configuration,
            Reliability::CONFIGURATION_ERROR,
        ),
        (WriteFailure::Process, Reliability::PROCESS_ERROR),
        (
            WriteFailure::Communication,
            Reliability::COMMUNICATION_FAILURE,
        ),
    ] {
        distribute(&mut channel, Err(failure));
        assert_eq!(reliability(&channel), expected);
        assert!(fault(&channel));

        // The verdict stands while the next distribution runs, and a
        // SUCCESSFUL one clears it.
        channel
            .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(2.0), None)
            .unwrap();
        let run = channel.take_command_run_internal().unwrap();
        assert_eq!(reliability(&channel), expected);
        assert!(channel.complete_command_run_internal(run.generation, Ok(())));
        assert_eq!(reliability(&channel), Reliability::NO_FAULT_DETECTED);
        assert!(!fault(&channel));
    }

    // A stale completion changes neither Write_Status nor Reliability.
    channel
        .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(3.0), None)
        .unwrap();
    let run = channel.take_command_run_internal().unwrap();
    assert!(
        !channel.complete_command_run_internal(
            run.generation.wrapping_add(1),
            Err(WriteFailure::Process)
        )
    );
    assert_eq!(reliability(&channel), Reliability::NO_FAULT_DETECTED);
}

#[test]
fn channel_write_with_nothing_to_distribute_clears_reliability() {
    let mut channel = configured();
    distribute(&mut channel, Err(WriteFailure::Communication));
    channel.set_members(Vec::new()).unwrap();
    channel
        .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(1.0), None)
        .unwrap();
    assert_eq!(reliability(&channel), Reliability::NO_FAULT_DETECTED);
    assert!(!fault(&channel));
}

#[test]
fn channel_reliability_is_simulated_out_of_service_and_restored_in_service() {
    let mut channel = configured();
    // In service it isn't writable.
    assert_property_error(
        simulate(&mut channel, Reliability::PROCESS_ERROR),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    distribute(&mut channel, Err(WriteFailure::Configuration));

    // A distribution started in service ends out of service: its verdict is
    // kept aside, and Reliability holds what it read on going out.
    channel
        .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(2.0), None)
        .unwrap();
    let run = channel.take_command_run_internal().unwrap();
    set_out_of_service(&mut channel, true);
    assert_eq!(reliability(&channel), Reliability::CONFIGURATION_ERROR);
    assert!(channel.complete_command_run_internal(run.generation, Ok(())));
    assert_eq!(reliability(&channel), Reliability::CONFIGURATION_ERROR);

    simulate(&mut channel, Reliability::NO_FAULT_DETECTED).unwrap();
    assert!(!fault(&channel));
    simulate(&mut channel, Reliability::COMMUNICATION_FAILURE).unwrap();
    assert!(fault(&channel));
    assert_property_error(
        simulate(&mut channel, Reliability::from_raw(26)),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_property_error(
        channel.write_property(P::RELIABILITY, None, PropertyValue::Unsigned(0), None),
        ErrorCode::INVALID_DATA_TYPE,
    );
    // Writing TRUE again keeps the simulation.
    set_out_of_service(&mut channel, true);
    assert_eq!(reliability(&channel), Reliability::COMMUNICATION_FAILURE);

    // Back in service, the last distribution's verdict shows.
    set_out_of_service(&mut channel, false);
    assert_eq!(reliability(&channel), Reliability::NO_FAULT_DETECTED);
    assert!(!fault(&channel));
}

#[test]
fn channel_takes_members_in_other_devices() {
    let mut channel = configured();
    let remote = BACnetDeviceObjectPropertyReference {
        device_identifier: Some(device(9)),
        ..member(4)
    };
    channel
        .set_members(vec![member(1), remote.clone()])
        .unwrap();
    channel
        .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(1.0), None)
        .unwrap();
    let run = channel.take_command_run_internal().unwrap();
    let RunPlan::Channel(distribution) = &run.plan else {
        panic!("a Channel queues a distribution");
    };
    assert_eq!(distribution.members[1].reference, remote);
}
