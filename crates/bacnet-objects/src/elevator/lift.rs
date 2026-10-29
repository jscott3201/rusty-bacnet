//! The Lift object (type 59, Clause 12.59).

use crate::prelude::*;
use alloc::borrow::Cow;
use bacnet_types::constructed::{
    BACnetAssignedLandingCalls, BACnetDeviceObjectReference, BACnetLandingDoorStatus,
    BACnetLiftCarCallList,
};
use bacnet_types::enums::{
    DoorStatus, EngineeringUnits, LiftCarDirection, LiftCarDoorCommand, LiftCarDriveStatus,
    LiftCarMode, LiftFault, ObjectType, PropertyIdentifier, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use super::car_state::{self, CarState};
use super::doors::CarDoors;
use super::energy_meter::{EnergyMeter, energy_meter_accessors};
use super::membership::{GroupMembership, group_membership_accessors};
use super::{checked, decode_fault_signals, direction_in_range, metadata};
use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

/// BACnet Lift object — represents a single lift/elevator car.
///
/// The object serves the rows of its table (Clause 12.59, Table 12-77) that
/// it implements, each with its table datatype. The per-door arrays
/// (Car_Door_Status, Landing_Door_Status, Assigned_Landing_Calls,
/// Making_Car_Call, Registered_Car_Call and Car_Door_Command) hold one
/// element per car door and always have the same size, as Clause 12.59
/// requires. They, and Car_Assigned_Direction, Car_Door_Zone, Car_Mode,
/// Next_Stopping_Floor and Car_Drive_Status, are set by the application in
/// service and take WriteProperty only while Out_Of_Service is TRUE.
pub struct LiftObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Elevator_Group, Group_ID and Installation_ID.
    membership: GroupMembership,
    /// Floor_Text: one label per universal floor number, floor 1 first.
    floor_text: Vec<String>,
    /// Car_Position: the universal floor number of the car (Unsigned8).
    car_position: u8,
    /// Car moving direction (BACnetLiftCarDirection, Clause 21); proprietary
    /// extensions (Clause 23.1) are preserved as raw values.
    pub(super) car_moving_direction: LiftCarDirection,
    /// The per-door arrays, one element per car door.
    doors: CarDoors,
    /// Car_Assigned_Direction, Car_Door_Zone, Car_Mode, Next_Stopping_Floor
    /// and Car_Drive_Status.
    car: CarState,
    /// Car_Load (REAL), in `car_load_units`.
    car_load: f32,
    /// Car_Load_Units, present because Car_Load is.
    car_load_units: EngineeringUnits,
    /// Passenger alarm state (Boolean).
    passenger_alarm: bool,
    /// Energy_Meter and Energy_Meter_Ref.
    energy_meter: EnergyMeter,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: Reliability,
    /// Fault signal set (BACnetLiftFault, Clause 21).
    fault_signals: Vec<LiftFault>,
}

impl LiftObject {
    /// Create a new Lift object with the given number of floors.
    ///
    /// Floor text is initialized to "Floor 1", "Floor 2", etc. The car stands
    /// at floor 1 with one door, whose status is UNKNOWN and which reports no
    /// landing doors, calls or door command, and Car_Load is 0.0 percent.
    /// Car_Assigned_Direction, Car_Mode and Car_Drive_Status start UNKNOWN,
    /// Car_Door_Zone FALSE and Next_Stopping_Floor 1.
    pub fn new(instance: u32, name: impl Into<String>, num_floors: usize) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::LIFT, instance)?;
        let floor_text = (1..=num_floors).map(|i| format!("Floor {i}")).collect();
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            membership: GroupMembership::new()?,
            floor_text,
            car_position: 1,
            car_moving_direction: LiftCarDirection::STOPPED,
            doors: CarDoors::new(),
            car: CarState::new(),
            car_load: 0.0,
            car_load_units: EngineeringUnits::PERCENT,
            passenger_alarm: false,
            energy_meter: EnergyMeter::new()?,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
            fault_signals: Vec::new(),
        })
    }

    group_membership_accessors!("lift");
    energy_meter_accessors!("lift");

    /// The status of each car door, served as Car_Door_Status.
    pub fn car_door_status(&self) -> &[DoorStatus] {
        &self.doors.status
    }

    /// Set the status of each car door, one element per door.
    ///
    /// Its size is the number of car doors, so a list of a different size
    /// changes that number, and the other per-door arrays follow: a removed
    /// door's elements are dropped, and an added door starts with no landing
    /// doors, no assigned or registered calls, a Making_Car_Call of 0 and a
    /// Car_Door_Command of NONE. A WriteProperty, accepted only while
    /// Out_Of_Service is TRUE, can't change the size, so this is the only way
    /// to. A status outside BACnetDoorStatus (a reserved value, or one above
    /// 65535) is refused with VALUE_OUT_OF_RANGE and nothing changes.
    pub fn set_car_door_status(&mut self, status: Vec<DoorStatus>) -> Result<(), Error> {
        self.doors.set_status(status)
    }

    /// The landing doors of each car door, served as Landing_Door_Status.
    pub fn landing_door_status(&self) -> &[BACnetLandingDoorStatus] {
        &self.doors.landing
    }

    /// Set the landing doors of each car door, one element per car door.
    ///
    /// A list whose size differs from Car_Door_Status, or a landing door
    /// status outside BACnetDoorStatus, is refused with VALUE_OUT_OF_RANGE
    /// and the property is left unchanged. A WriteProperty, accepted only
    /// while Out_Of_Service is TRUE, follows the same rules.
    pub fn set_landing_door_status(
        &mut self,
        status: Vec<BACnetLandingDoorStatus>,
    ) -> Result<(), Error> {
        self.doors.set_landing(status)
    }

    /// The landing calls assigned to each car door, served as
    /// Assigned_Landing_Calls.
    pub fn assigned_landing_calls(&self) -> &[BACnetAssignedLandingCalls] {
        &self.doors.landing_calls
    }

    /// Set the landing calls assigned to each car door, one element per car
    /// door.
    ///
    /// A list whose size differs from Car_Door_Status, or a call whose
    /// direction is outside BACnetLiftCarDirection, is refused with
    /// VALUE_OUT_OF_RANGE and the property is left unchanged. A
    /// WriteProperty, accepted only while Out_Of_Service is TRUE, follows the
    /// same rules.
    pub fn set_assigned_landing_calls(
        &mut self,
        calls: Vec<BACnetAssignedLandingCalls>,
    ) -> Result<(), Error> {
        self.doors.set_landing_calls(calls)
    }

    /// The last car call made for each car door, served as Making_Car_Call:
    /// a universal floor number, or 0 when no call has been made.
    pub fn making_car_call(&self) -> &[u8] {
        &self.doors.making_car_call
    }

    /// Set the last car call made for each car door, one element per car
    /// door.
    ///
    /// A list whose size differs from Car_Door_Status is refused with
    /// VALUE_OUT_OF_RANGE and the property is left unchanged. A
    /// WriteProperty, accepted only while Out_Of_Service is TRUE, follows the
    /// same rule.
    pub fn set_making_car_call(&mut self, floors: Vec<u8>) -> Result<(), Error> {
        self.doors.set_making_car_call(floors)
    }

    /// The floors with a car call registered for each car door, served as
    /// Registered_Car_Call.
    pub fn registered_car_call(&self) -> &[BACnetLiftCarCallList] {
        &self.doors.registered_car_call
    }

    /// Set the floors with a car call registered for each car door, one
    /// element per car door.
    ///
    /// A list whose size differs from Car_Door_Status is refused with
    /// VALUE_OUT_OF_RANGE and the property is left unchanged. A
    /// WriteProperty, accepted only while Out_Of_Service is TRUE, follows the
    /// same rule.
    pub fn set_registered_car_call(
        &mut self,
        calls: Vec<BACnetLiftCarCallList>,
    ) -> Result<(), Error> {
        self.doors.set_registered_car_call(calls)
    }

    /// The pending command of each car door, served as Car_Door_Command.
    pub fn car_door_command(&self) -> &[LiftCarDoorCommand] {
        &self.doors.command
    }

    /// Set the pending command of each car door, one element per car door,
    /// for instance back to NONE once a command has run.
    ///
    /// A list whose size differs from Car_Door_Status, or a command other
    /// than NONE, OPEN or CLOSE, is refused with VALUE_OUT_OF_RANGE and the
    /// property is left unchanged. A WriteProperty, accepted only while
    /// Out_Of_Service is TRUE, follows the same rules.
    pub fn set_car_door_command(&mut self, commands: Vec<LiftCarDoorCommand>) -> Result<(), Error> {
        self.doors.set_command(commands)
    }

    /// The direction the car is assigned to move, served as
    /// Car_Assigned_Direction.
    pub fn car_assigned_direction(&self) -> LiftCarDirection {
        self.car.assigned_direction
    }

    /// Set the direction the car is assigned to move.
    ///
    /// A direction outside BACnetLiftCarDirection is refused with
    /// VALUE_OUT_OF_RANGE and the property is left unchanged.
    pub fn set_car_assigned_direction(&mut self, direction: LiftCarDirection) -> Result<(), Error> {
        self.car.assigned_direction = checked(direction, direction_in_range)?;
        Ok(())
    }

    /// Whether the car sits in its door zone, served as Car_Door_Zone.
    pub fn car_door_zone(&self) -> bool {
        self.car.door_zone
    }

    /// Set whether the car sits in its door zone, close enough to a landing
    /// that its doors may begin to open.
    pub fn set_car_door_zone(&mut self, in_zone: bool) {
        self.car.door_zone = in_zone;
    }

    /// The car's operating mode, served as Car_Mode.
    pub fn car_mode(&self) -> LiftCarMode {
        self.car.mode
    }

    /// Set the car's operating mode.
    ///
    /// A mode outside BACnetLiftCarMode is refused with VALUE_OUT_OF_RANGE
    /// and the property is left unchanged.
    pub fn set_car_mode(&mut self, mode: LiftCarMode) -> Result<(), Error> {
        self.car.mode = checked(mode, car_state::mode_in_range)?;
        Ok(())
    }

    /// The floor of the car's next stop (universal numbering), served as
    /// Next_Stopping_Floor.
    pub fn next_stopping_floor(&self) -> u8 {
        self.car.next_stopping_floor
    }

    /// Set the floor of the car's next stop, or, while it stands still, the
    /// floor it is at.
    pub fn set_next_stopping_floor(&mut self, floor: u8) {
        self.car.next_stopping_floor = floor;
    }

    /// The state of the car's motor drive, served as Car_Drive_Status.
    pub fn car_drive_status(&self) -> LiftCarDriveStatus {
        self.car.drive_status
    }

    /// Set the state of the car's motor drive.
    ///
    /// A status outside BACnetLiftCarDriveStatus is refused with
    /// VALUE_OUT_OF_RANGE and the property is left unchanged.
    pub fn set_car_drive_status(&mut self, status: LiftCarDriveStatus) -> Result<(), Error> {
        self.car.drive_status = checked(status, car_state::drive_status_in_range)?;
        Ok(())
    }

    /// The units of Car_Load, served as Car_Load_Units.
    pub fn car_load_units(&self) -> EngineeringUnits {
        self.car_load_units
    }

    /// Set the units Car_Load is measured in. A new Lift uses PERCENT.
    ///
    /// Car_Load_Units is read-only over the network. A value above 65535,
    /// outside BACnetEngineeringUnits, is refused with VALUE_OUT_OF_RANGE and
    /// the property is left unchanged.
    pub fn set_car_load_units(&mut self, units: EngineeringUnits) -> Result<(), Error> {
        if units.to_raw() > 65_535 {
            return Err(common::value_out_of_range_error());
        }
        self.car_load_units = units;
        Ok(())
    }
}

impl BACnetObject for LiftObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        if let Some(value) = self.membership.read(property) {
            return Ok(value);
        }
        if let Some(value) = self.energy_meter.read(property) {
            return Ok(value);
        }
        if let Some(value) = self.car.read(property) {
            return Ok(value);
        }
        if let Some(result) = self.doors.read(property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::LIFT.to_raw()))
            }
            p if p == PropertyIdentifier::FLOOR_TEXT => common::read_array(
                self.floor_text
                    .iter()
                    .cloned()
                    .map(PropertyValue::CharacterString)
                    .collect(),
                array_index,
            ),
            p if p == PropertyIdentifier::CAR_POSITION => {
                Ok(PropertyValue::Unsigned(self.car_position.into()))
            }
            p if p == PropertyIdentifier::CAR_MOVING_DIRECTION => Ok(PropertyValue::Enumerated(
                self.car_moving_direction.to_raw(),
            )),
            p if p == PropertyIdentifier::CAR_LOAD => Ok(PropertyValue::Real(self.car_load)),
            p if p == PropertyIdentifier::CAR_LOAD_UNITS => {
                Ok(PropertyValue::Enumerated(self.car_load_units.to_raw()))
            }
            p if p == PropertyIdentifier::PASSENGER_ALARM => {
                Ok(PropertyValue::Boolean(self.passenger_alarm))
            }
            p if p == PropertyIdentifier::FAULT_SIGNALS => Ok(PropertyValue::List(
                self.fault_signals
                    .iter()
                    .map(|fault| PropertyValue::Enumerated(fault.to_raw()))
                    .collect(),
            )),
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if let Some(result) = self.energy_meter.write(property, &value) {
            return result;
        }
        // Simulation writes (items (c) and (d) of the Out_Of_Service
        // description), refused in service. None can change the car door
        // count, so the per-door arrays keep the same size.
        if let Some(result) = self.car.write(property, &value, self.out_of_service) {
            return result;
        }
        if let Some(result) = self
            .doors
            .write(property, array_index, &value, self.out_of_service)
        {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::CAR_POSITION => {
                if let PropertyValue::Unsigned(v) = value {
                    // Car_Position is an Unsigned8 (Table 12-77).
                    self.car_position =
                        u8::try_from(v).map_err(|_| common::value_out_of_range_error())?;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::CAR_MOVING_DIRECTION => {
                if let PropertyValue::Enumerated(v) = value {
                    self.car_moving_direction =
                        checked(LiftCarDirection::from_raw(v), direction_in_range)?;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::CAR_LOAD => {
                if let PropertyValue::Real(v) = value {
                    common::reject_non_finite(v)?;
                    self.car_load = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::PASSENGER_ALARM => {
                if let PropertyValue::Boolean(v) = value {
                    self.passenger_alarm = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::FAULT_SIGNALS => {
                self.fault_signals =
                    decode_fault_signals(value, LiftFault::ALL_NAMED, LiftFault::from_raw)?;
                Ok(())
            }
            _ => Err(crate::common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
                array_index,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_lift_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }
}
