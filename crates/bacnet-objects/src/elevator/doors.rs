//! The Lift's per-door arrays (Clause 12.59, Table 12-77): Car_Door_Status,
//! Landing_Door_Status, Assigned_Landing_Calls, Making_Car_Call,
//! Registered_Car_Call and Car_Door_Command.
//!
//! Clause 12.59 lines these arrays up by car door: element N of each one
//! describes the same door, so all of them have one element per car door. The
//! application sets the door count with `LiftObject::set_car_door_status`,
//! and the other arrays grow or shrink with it.
//!
//! Items (c) and (d) of the Lift's Out_Of_Service description open all six
//! to writes while Out_Of_Service is TRUE, so a test tool can simulate the
//! car. In service they stay read-only over the network. A write takes
//! the whole array or one element. The size is the door count, so it isn't
//! writable over the network: a write of index 0 is WRITE_ACCESS_DENIED and a
//! whole-array write of another size is VALUE_OUT_OF_RANGE, as for the
//! Staging arrays. That keeps every array the same size. A whole array
//! arrives as a `List` of elements, or, from the service decoder, as the bare
//! element when the car has one door; `door_values` checks each element. A
//! refused write changes nothing.

use crate::prelude::*;
use bacnet_types::constructed::{
    BACnetAssignedLandingCalls, BACnetLandingDoorStatus, BACnetLiftCarCallList,
};
use bacnet_types::enums::{DoorStatus, LiftCarDoorCommand, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::door_values as values;
use crate::common;

/// The per-door state of one lift car, every array one element per car door.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CarDoors {
    /// Car_Door_Status: the status of each car door.
    pub(super) status: Vec<DoorStatus>,
    /// Landing_Door_Status: the landing doors each car door pairs with.
    pub(super) landing: Vec<BACnetLandingDoorStatus>,
    /// Assigned_Landing_Calls: the landing calls assigned to each car door.
    pub(super) landing_calls: Vec<BACnetAssignedLandingCalls>,
    /// Making_Car_Call: the last car call written for each car door, 0 when
    /// none has been.
    pub(super) making_car_call: Vec<u8>,
    /// Registered_Car_Call: the floors with a car call registered for each
    /// car door.
    pub(super) registered_car_call: Vec<BACnetLiftCarCallList>,
    /// Car_Door_Command: the pending command for each car door.
    pub(super) command: Vec<LiftCarDoorCommand>,
}

impl CarDoors {
    /// One car door: status UNKNOWN, no landing doors, no assigned or
    /// registered calls, no car call made and no pending command.
    pub(super) fn new() -> Self {
        let mut doors = Self {
            status: Vec::new(),
            landing: Vec::new(),
            landing_calls: Vec::new(),
            making_car_call: Vec::new(),
            registered_car_call: Vec::new(),
            command: Vec::new(),
        };
        doors.resize(vec![DoorStatus::UNKNOWN]);
        doors
    }

    /// Store a new Car_Door_Status, whose size sets the door count. A status
    /// outside BACnetDoorStatus is refused with VALUE_OUT_OF_RANGE and nothing
    /// changes.
    pub(super) fn set_status(&mut self, status: Vec<DoorStatus>) -> Result<(), Error> {
        if !status.iter().all(values::door_status_in_range) {
            return Err(common::value_out_of_range_error());
        }
        self.resize(status);
        Ok(())
    }

    /// Store `status` and fit every other array to its size: a removed
    /// door's elements are dropped, and an added door starts empty, with a
    /// Making_Car_Call of 0 and a Car_Door_Command of NONE.
    fn resize(&mut self, status: Vec<DoorStatus>) {
        let count = status.len();
        self.landing.resize_with(count, Default::default);
        self.landing_calls.resize_with(count, Default::default);
        self.making_car_call.resize(count, 0);
        self.registered_car_call
            .resize_with(count, Default::default);
        self.command.resize(count, LiftCarDoorCommand::NONE);
        self.status = status;
    }

    /// Store a new Landing_Door_Status.
    pub(super) fn set_landing(&mut self, array: Vec<BACnetLandingDoorStatus>) -> Result<(), Error> {
        fit(
            &self.status,
            &mut self.landing,
            array,
            values::landing_doors_in_range,
        )
    }

    /// Store a new Assigned_Landing_Calls.
    pub(super) fn set_landing_calls(
        &mut self,
        array: Vec<BACnetAssignedLandingCalls>,
    ) -> Result<(), Error> {
        fit(
            &self.status,
            &mut self.landing_calls,
            array,
            values::landing_calls_in_range,
        )
    }

    /// Store a new Making_Car_Call.
    pub(super) fn set_making_car_call(&mut self, array: Vec<u8>) -> Result<(), Error> {
        fit(
            &self.status,
            &mut self.making_car_call,
            array,
            values::always_in_range,
        )
    }

    /// Store a new Registered_Car_Call.
    pub(super) fn set_registered_car_call(
        &mut self,
        array: Vec<BACnetLiftCarCallList>,
    ) -> Result<(), Error> {
        fit(
            &self.status,
            &mut self.registered_car_call,
            array,
            values::always_in_range,
        )
    }

    /// Store a new Car_Door_Command.
    pub(super) fn set_command(&mut self, array: Vec<LiftCarDoorCommand>) -> Result<(), Error> {
        fit(
            &self.status,
            &mut self.command,
            array,
            values::door_command_in_range,
        )
    }

    /// The value of `property` at `array_index` if it is a per-door array.
    pub(super) fn read(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        let elements: Vec<PropertyValue> = match property {
            PropertyIdentifier::CAR_DOOR_STATUS => self
                .status
                .iter()
                .map(|door| PropertyValue::Enumerated(door.to_raw()))
                .collect(),
            PropertyIdentifier::LANDING_DOOR_STATUS => self
                .landing
                .iter()
                .map(values::encode_landing_doors)
                .collect(),
            PropertyIdentifier::ASSIGNED_LANDING_CALLS => self
                .landing_calls
                .iter()
                .map(values::encode_landing_calls)
                .collect(),
            PropertyIdentifier::MAKING_CAR_CALL => self
                .making_car_call
                .iter()
                .map(|&floor| PropertyValue::Unsigned(floor.into()))
                .collect(),
            PropertyIdentifier::REGISTERED_CAR_CALL => self
                .registered_car_call
                .iter()
                .map(values::encode_car_calls)
                .collect(),
            PropertyIdentifier::CAR_DOOR_COMMAND => self
                .command
                .iter()
                .map(|command| PropertyValue::Enumerated(command.to_raw()))
                .collect(),
            _ => return None,
        };
        Some(common::read_array(elements, array_index))
    }

    /// Handle a WriteProperty on a per-door array; `None` for any other
    /// property. In service every one is refused with WRITE_ACCESS_DENIED.
    pub(super) fn write(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &PropertyValue,
        out_of_service: bool,
    ) -> Option<Result<(), Error>> {
        let write = Simulated {
            array_index,
            value,
            out_of_service,
        };
        Some(match property {
            PropertyIdentifier::CAR_DOOR_STATUS => {
                write.apply(&mut self.status, values::decode_car_door)
            }
            PropertyIdentifier::LANDING_DOOR_STATUS => {
                write.apply(&mut self.landing, values::decode_landing_doors)
            }
            PropertyIdentifier::ASSIGNED_LANDING_CALLS => {
                write.apply(&mut self.landing_calls, values::decode_landing_calls)
            }
            PropertyIdentifier::MAKING_CAR_CALL => {
                write.apply(&mut self.making_car_call, values::decode_car_call)
            }
            PropertyIdentifier::REGISTERED_CAR_CALL => {
                write.apply(&mut self.registered_car_call, values::decode_car_calls)
            }
            PropertyIdentifier::CAR_DOOR_COMMAND => {
                write.apply(&mut self.command, values::decode_door_command)
            }
            _ => return None,
        })
    }
}

/// Replace `slot` with `array`, which must have one element per car door (the
/// size of `status`), each accepted by `in_range`. Anything else is refused
/// with VALUE_OUT_OF_RANGE and `slot` is left unchanged.
fn fit<T>(
    status: &[DoorStatus],
    slot: &mut Vec<T>,
    array: Vec<T>,
    in_range: fn(&T) -> bool,
) -> Result<(), Error> {
    if array.len() != status.len() || !array.iter().all(in_range) {
        return Err(common::value_out_of_range_error());
    }
    *slot = array;
    Ok(())
}

/// One simulation write on a per-door array.
struct Simulated<'a> {
    array_index: Option<u32>,
    value: &'a PropertyValue,
    out_of_service: bool,
}

impl Simulated<'_> {
    /// Store the array this write leaves in `array`, each element decoded by
    /// `decode`, or refuse it and leave `array` unchanged.
    fn apply<T: Clone>(
        self,
        array: &mut Vec<T>,
        decode: fn(PropertyValue) -> Result<T, Error>,
    ) -> Result<(), Error> {
        if !self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        *array = written_array(array, self.array_index, self.value.clone(), decode)?;
        Ok(())
    }
}

/// The array a write of `value` at `array_index` leaves, starting from
/// `current`, with each element decoded by `decode`. The caller stores it.
fn written_array<T: Clone>(
    current: &[T],
    array_index: Option<u32>,
    value: PropertyValue,
    decode: fn(PropertyValue) -> Result<T, Error>,
) -> Result<Vec<T>, Error> {
    match array_index {
        None => {
            let elements = match value {
                PropertyValue::List(elements) => elements,
                element => vec![element],
            };
            let array = elements
                .into_iter()
                .map(decode)
                .collect::<Result<Vec<_>, _>>()?;
            if array.len() != current.len() {
                return Err(common::value_out_of_range_error());
            }
            Ok(array)
        }
        Some(0) => Err(common::write_access_denied_error()),
        Some(index) => {
            let mut array = current.to_vec();
            let element = usize::try_from(index - 1)
                .ok()
                .and_then(|slot| array.get_mut(slot))
                .ok_or_else(common::invalid_array_index_error)?;
            *element = decode(value)?;
            Ok(array)
        }
    }
}
