//! Elevator, Escalator, and Lift objects per ASHRAE 135-2020 Clause 12.
//!
//! - ElevatorGroupObject (type 57): manages a group of lifts
//! - EscalatorObject (type 58): represents an escalator
//! - LiftObject (type 59): represents a single lift/elevator car

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

// ===========================================================================
// ElevatorGroupObject (type 57)
// ===========================================================================

/// BACnet Elevator Group object — manages a group of lifts.
pub struct ElevatorGroupObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Group identifier (Unsigned).
    group_id: u64,
    /// List of lift ObjectIdentifiers in this group.
    group_members: Vec<ObjectIdentifier>,
    /// Group mode (LiftGroupMode enumerated value).
    group_mode: u32,
    /// Number of landing calls (stored as count).
    landing_calls: u64,
    /// Landing call control (Enumerated).
    landing_call_control: u32,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl ElevatorGroupObject {
    /// Create a new Elevator Group object with default values.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ELEVATOR_GROUP, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            group_id: 0,
            group_members: Vec::new(),
            group_mode: 0, // Unknown
            landing_calls: 0,
            landing_call_control: 0,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }

    /// Add a lift member to this elevator group.
    pub fn add_member(&mut self, oid: ObjectIdentifier) {
        self.group_members.push(oid);
    }
}

impl BACnetObject for ElevatorGroupObject {
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
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::ELEVATOR_GROUP.to_raw(),
            )),
            p if p == PropertyIdentifier::GROUP_ID => Ok(PropertyValue::Unsigned(self.group_id)),
            p if p == PropertyIdentifier::GROUP_MEMBERS => {
                let items: Vec<PropertyValue> = self
                    .group_members
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .collect();
                Ok(PropertyValue::List(items))
            }
            p if p == PropertyIdentifier::GROUP_MODE => {
                Ok(PropertyValue::Enumerated(self.group_mode))
            }
            p if p == PropertyIdentifier::LANDING_CALLS => {
                Ok(PropertyValue::Unsigned(self.landing_calls))
            }
            p if p == PropertyIdentifier::LANDING_CALL_CONTROL => {
                Ok(PropertyValue::Enumerated(self.landing_call_control))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
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
        match property {
            p if p == PropertyIdentifier::GROUP_ID => {
                if let PropertyValue::Unsigned(v) = value {
                    self.group_id = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::GROUP_MODE => {
                if let PropertyValue::Enumerated(v) = value {
                    self.group_mode = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::LANDING_CALL_CONTROL => {
                if let PropertyValue::Enumerated(v) = value {
                    self.landing_call_control = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(common::write_access_denied_error()),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::GROUP_ID,
            PropertyIdentifier::GROUP_MEMBERS,
            PropertyIdentifier::GROUP_MODE,
            PropertyIdentifier::LANDING_CALLS,
            PropertyIdentifier::LANDING_CALL_CONTROL,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ===========================================================================
// EscalatorObject (type 58)
// ===========================================================================

/// BACnet Escalator object — represents an escalator.
pub struct EscalatorObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Escalator mode (Enumerated: 0=unknown, 1=stop, 2=up, 3=down, 4=inspection).
    escalator_mode: u32,
    /// List of fault signal codes (Unsigned).
    fault_signals: Vec<u64>,
    /// Energy meter reading (Real).
    energy_meter: f32,
    /// Energy meter reference (stored as raw bytes).
    energy_meter_ref: Vec<u8>,
    /// Power mode (Boolean).
    power_mode: bool,
    /// Operation direction (Enumerated: 0=unknown, 1=up, 2=down, 3=stopped).
    operation_direction: u32,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl EscalatorObject {
    /// Create a new Escalator object with default values.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ESCALATOR, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            escalator_mode: 0, // unknown
            fault_signals: Vec::new(),
            energy_meter: 0.0,
            energy_meter_ref: Vec::new(),
            power_mode: false,
            operation_direction: 0, // unknown
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }
}

impl BACnetObject for EscalatorObject {
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
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ESCALATOR.to_raw()))
            }
            p if p == PropertyIdentifier::ESCALATOR_MODE => {
                Ok(PropertyValue::Enumerated(self.escalator_mode))
            }
            p if p == PropertyIdentifier::FAULT_SIGNALS => {
                let items: Vec<PropertyValue> = self
                    .fault_signals
                    .iter()
                    .map(|v| PropertyValue::Unsigned(*v))
                    .collect();
                Ok(PropertyValue::List(items))
            }
            p if p == PropertyIdentifier::ENERGY_METER => {
                Ok(PropertyValue::Real(self.energy_meter))
            }
            p if p == PropertyIdentifier::ENERGY_METER_REF => {
                Ok(PropertyValue::OctetString(self.energy_meter_ref.clone()))
            }
            p if p == PropertyIdentifier::POWER_MODE => Ok(PropertyValue::Boolean(self.power_mode)),
            p if p == PropertyIdentifier::OPERATION_DIRECTION => {
                Ok(PropertyValue::Enumerated(self.operation_direction))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
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
        match property {
            p if p == PropertyIdentifier::ESCALATOR_MODE => {
                if let PropertyValue::Enumerated(v) = value {
                    if v > 4 {
                        return Err(common::value_out_of_range_error());
                    }
                    self.escalator_mode = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::OPERATION_DIRECTION => {
                if let PropertyValue::Enumerated(v) = value {
                    if v > 3 {
                        return Err(common::value_out_of_range_error());
                    }
                    self.operation_direction = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(common::write_access_denied_error()),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::ESCALATOR_MODE,
            PropertyIdentifier::FAULT_SIGNALS,
            PropertyIdentifier::ENERGY_METER,
            PropertyIdentifier::ENERGY_METER_REF,
            PropertyIdentifier::POWER_MODE,
            PropertyIdentifier::OPERATION_DIRECTION,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ===========================================================================
// LiftObject (type 59)
// ===========================================================================

/// BACnet Lift object — represents a single lift/elevator car.
pub struct LiftObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Tracking value (Unsigned — current floor).
    tracking_value: u64,
    /// Car position (Unsigned).
    car_position: u64,
    /// Car moving direction (Enumerated: 0=unknown, 1=stopped, 2=up, 3=down).
    car_moving_direction: u32,
    /// Car door status (List of Unsigned).
    car_door_status: Vec<u64>,
    /// Car load as a percentage (Unsigned).
    car_load: u64,
    /// Number of landing doors (stored as count).
    landing_doors: u64,
    /// Floor text labels (List of String).
    floor_text: Vec<String>,
    /// Energy meter reading (Real).
    energy_meter: f32,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl LiftObject {
    /// Create a new Lift object with the given number of floors.
    ///
    /// Floor text is initialized to "Floor 1", "Floor 2", etc.
    pub fn new(instance: u32, name: impl Into<String>, num_floors: usize) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::LIFT, instance)?;
        let floor_text = (1..=num_floors).map(|i| format!("Floor {i}")).collect();
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            tracking_value: 1,
            car_position: 1,
            car_moving_direction: 1, // stopped
            car_door_status: Vec::new(),
            car_load: 0,
            landing_doors: num_floors as u64,
            floor_text,
            energy_meter: 0.0,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
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
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::LIFT.to_raw()))
            }
            p if p == PropertyIdentifier::TRACKING_VALUE => {
                Ok(PropertyValue::Unsigned(self.tracking_value))
            }
            p if p == PropertyIdentifier::CAR_POSITION => {
                Ok(PropertyValue::Unsigned(self.car_position))
            }
            p if p == PropertyIdentifier::CAR_MOVING_DIRECTION => {
                Ok(PropertyValue::Enumerated(self.car_moving_direction))
            }
            p if p == PropertyIdentifier::CAR_DOOR_STATUS => {
                let items: Vec<PropertyValue> = self
                    .car_door_status
                    .iter()
                    .map(|v| PropertyValue::Unsigned(*v))
                    .collect();
                Ok(PropertyValue::List(items))
            }
            p if p == PropertyIdentifier::CAR_LOAD => Ok(PropertyValue::Unsigned(self.car_load)),
            p if p == PropertyIdentifier::LANDING_DOOR_STATUS => {
                Ok(PropertyValue::Unsigned(self.landing_doors))
            }
            p if p == PropertyIdentifier::FLOOR_TEXT => {
                let items: Vec<PropertyValue> = self
                    .floor_text
                    .iter()
                    .map(|s| PropertyValue::CharacterString(s.clone()))
                    .collect();
                Ok(PropertyValue::List(items))
            }
            p if p == PropertyIdentifier::ENERGY_METER => {
                Ok(PropertyValue::Real(self.energy_meter))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
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
        match property {
            p if p == PropertyIdentifier::TRACKING_VALUE => {
                if let PropertyValue::Unsigned(v) = value {
                    self.tracking_value = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::CAR_POSITION => {
                if let PropertyValue::Unsigned(v) = value {
                    self.car_position = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::CAR_MOVING_DIRECTION => {
                if let PropertyValue::Enumerated(v) = value {
                    if v > 3 {
                        return Err(common::value_out_of_range_error());
                    }
                    self.car_moving_direction = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::CAR_LOAD => {
                if let PropertyValue::Unsigned(v) = value {
                    if v > 100 {
                        return Err(common::value_out_of_range_error());
                    }
                    self.car_load = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(common::write_access_denied_error()),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::TRACKING_VALUE,
            PropertyIdentifier::CAR_POSITION,
            PropertyIdentifier::CAR_MOVING_DIRECTION,
            PropertyIdentifier::CAR_DOOR_STATUS,
            PropertyIdentifier::CAR_LOAD,
            PropertyIdentifier::LANDING_DOOR_STATUS,
            PropertyIdentifier::FLOOR_TEXT,
            PropertyIdentifier::ENERGY_METER,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- ElevatorGroupObject ---

    #[test]
    fn elevator_group_create_and_read_defaults() {
        let eg = ElevatorGroupObject::new(1, "EG-1").unwrap();
        assert_eq!(eg.object_name(), "EG-1");
        assert_eq!(
            eg.read_property(PropertyIdentifier::GROUP_ID, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            eg.read_property(PropertyIdentifier::GROUP_MEMBERS, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
        assert_eq!(
            eg.read_property(PropertyIdentifier::GROUP_MODE, None)
                .unwrap(),
            PropertyValue::Enumerated(0) // Unknown
        );
    }

    #[test]
    fn elevator_group_object_type() {
        let eg = ElevatorGroupObject::new(1, "EG-1").unwrap();
        assert_eq!(
            eg.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::ELEVATOR_GROUP.to_raw())
        );
    }

    #[test]
    fn elevator_group_add_members() {
        let mut eg = ElevatorGroupObject::new(1, "EG-1").unwrap();
        let lift1 = ObjectIdentifier::new(ObjectType::LIFT, 1).unwrap();
        let lift2 = ObjectIdentifier::new(ObjectType::LIFT, 2).unwrap();
        eg.add_member(lift1);
        eg.add_member(lift2);
        assert_eq!(
            eg.read_property(PropertyIdentifier::GROUP_MEMBERS, None)
                .unwrap(),
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(lift1),
                PropertyValue::ObjectIdentifier(lift2),
            ])
        );
    }

    #[test]
    fn elevator_group_read_landing_calls() {
        let eg = ElevatorGroupObject::new(1, "EG-1").unwrap();
        assert_eq!(
            eg.read_property(PropertyIdentifier::LANDING_CALLS, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
    }

    #[test]
    fn elevator_group_property_list() {
        let eg = ElevatorGroupObject::new(1, "EG-1").unwrap();
        let list = eg.property_list();
        assert!(list.contains(&PropertyIdentifier::GROUP_ID));
        assert!(list.contains(&PropertyIdentifier::GROUP_MEMBERS));
        assert!(list.contains(&PropertyIdentifier::GROUP_MODE));
        assert!(list.contains(&PropertyIdentifier::LANDING_CALLS));
        assert!(list.contains(&PropertyIdentifier::LANDING_CALL_CONTROL));
        assert!(list.contains(&PropertyIdentifier::STATUS_FLAGS));
    }

    // --- EscalatorObject ---

    #[test]
    fn escalator_create_and_read_defaults() {
        let esc = EscalatorObject::new(1, "ESC-1").unwrap();
        assert_eq!(esc.object_name(), "ESC-1");
        assert_eq!(
            esc.read_property(PropertyIdentifier::ESCALATOR_MODE, None)
                .unwrap(),
            PropertyValue::Enumerated(0) // unknown
        );
        assert_eq!(
            esc.read_property(PropertyIdentifier::ENERGY_METER, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
        assert_eq!(
            esc.read_property(PropertyIdentifier::POWER_MODE, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
    }

    #[test]
    fn escalator_object_type() {
        let esc = EscalatorObject::new(1, "ESC-1").unwrap();
        assert_eq!(
            esc.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::ESCALATOR.to_raw())
        );
    }

    #[test]
    fn escalator_read_operation_direction() {
        let esc = EscalatorObject::new(1, "ESC-1").unwrap();
        assert_eq!(
            esc.read_property(PropertyIdentifier::OPERATION_DIRECTION, None)
                .unwrap(),
            PropertyValue::Enumerated(0) // unknown
        );
    }

    #[test]
    fn escalator_read_fault_signals() {
        let esc = EscalatorObject::new(1, "ESC-1").unwrap();
        assert_eq!(
            esc.read_property(PropertyIdentifier::FAULT_SIGNALS, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
    }

    #[test]
    fn escalator_property_list() {
        let esc = EscalatorObject::new(1, "ESC-1").unwrap();
        let list = esc.property_list();
        assert!(list.contains(&PropertyIdentifier::ESCALATOR_MODE));
        assert!(list.contains(&PropertyIdentifier::FAULT_SIGNALS));
        assert!(list.contains(&PropertyIdentifier::ENERGY_METER));
        assert!(list.contains(&PropertyIdentifier::ENERGY_METER_REF));
        assert!(list.contains(&PropertyIdentifier::POWER_MODE));
        assert!(list.contains(&PropertyIdentifier::OPERATION_DIRECTION));
        assert!(list.contains(&PropertyIdentifier::STATUS_FLAGS));
    }

    // --- LiftObject ---

    #[test]
    fn lift_create_and_read_defaults() {
        let lift = LiftObject::new(1, "LIFT-1", 10).unwrap();
        assert_eq!(lift.object_name(), "LIFT-1");
        assert_eq!(
            lift.read_property(PropertyIdentifier::TRACKING_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(1)
        );
        assert_eq!(
            lift.read_property(PropertyIdentifier::CAR_POSITION, None)
                .unwrap(),
            PropertyValue::Unsigned(1)
        );
        assert_eq!(
            lift.read_property(PropertyIdentifier::CAR_MOVING_DIRECTION, None)
                .unwrap(),
            PropertyValue::Enumerated(1) // stopped
        );
    }

    #[test]
    fn lift_object_type() {
        let lift = LiftObject::new(1, "LIFT-1", 5).unwrap();
        assert_eq!(
            lift.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::LIFT.to_raw())
        );
    }

    #[test]
    fn lift_floor_text() {
        let lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
        assert_eq!(
            lift.read_property(PropertyIdentifier::FLOOR_TEXT, None)
                .unwrap(),
            PropertyValue::List(vec![
                PropertyValue::CharacterString("Floor 1".into()),
                PropertyValue::CharacterString("Floor 2".into()),
                PropertyValue::CharacterString("Floor 3".into()),
            ])
        );
    }

    #[test]
    fn lift_read_car_load() {
        let lift = LiftObject::new(1, "LIFT-1", 5).unwrap();
        assert_eq!(
            lift.read_property(PropertyIdentifier::CAR_LOAD, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
    }

    #[test]
    fn lift_write_tracking_value() {
        let mut lift = LiftObject::new(1, "LIFT-1", 10).unwrap();
        lift.write_property(
            PropertyIdentifier::TRACKING_VALUE,
            None,
            PropertyValue::Unsigned(5),
            None,
        )
        .unwrap();
        assert_eq!(
            lift.read_property(PropertyIdentifier::TRACKING_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(5)
        );
    }

    #[test]
    fn lift_write_car_load_out_of_range() {
        let mut lift = LiftObject::new(1, "LIFT-1", 5).unwrap();
        let result = lift.write_property(
            PropertyIdentifier::CAR_LOAD,
            None,
            PropertyValue::Unsigned(101),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn lift_read_landing_doors() {
        let lift = LiftObject::new(1, "LIFT-1", 8).unwrap();
        assert_eq!(
            lift.read_property(PropertyIdentifier::LANDING_DOOR_STATUS, None)
                .unwrap(),
            PropertyValue::Unsigned(8)
        );
    }

    #[test]
    fn lift_read_energy_meter() {
        let lift = LiftObject::new(1, "LIFT-1", 5).unwrap();
        assert_eq!(
            lift.read_property(PropertyIdentifier::ENERGY_METER, None)
                .unwrap(),
            PropertyValue::Real(0.0)
        );
    }

    #[test]
    fn lift_property_list() {
        let lift = LiftObject::new(1, "LIFT-1", 5).unwrap();
        let list = lift.property_list();
        assert!(list.contains(&PropertyIdentifier::TRACKING_VALUE));
        assert!(list.contains(&PropertyIdentifier::CAR_POSITION));
        assert!(list.contains(&PropertyIdentifier::CAR_MOVING_DIRECTION));
        assert!(list.contains(&PropertyIdentifier::CAR_DOOR_STATUS));
        assert!(list.contains(&PropertyIdentifier::CAR_LOAD));
        assert!(list.contains(&PropertyIdentifier::LANDING_DOOR_STATUS));
        assert!(list.contains(&PropertyIdentifier::FLOOR_TEXT));
        assert!(list.contains(&PropertyIdentifier::ENERGY_METER));
        assert!(list.contains(&PropertyIdentifier::STATUS_FLAGS));
    }
}
