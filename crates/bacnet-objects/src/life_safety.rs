//! Life Safety Point (type 21) and Life Safety Zone (type 22) objects
//! per ASHRAE 135-2020 Clauses 12.15 and 12.16.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

// ---------------------------------------------------------------------------
// LifeSafetyPointObject (type 21)
// ---------------------------------------------------------------------------

/// BACnet Life Safety Point object.
///
/// Represents a single life-safety sensor or detector (e.g. smoke detector,
/// pull station). Present_Value is an enumerated LifeSafetyState, set by the
/// application via [`set_present_value`](Self::set_present_value).
pub struct LifeSafetyPointObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Present value — LifeSafetyState enumeration (read-only via protocol).
    present_value: u32,
    /// Operating mode — LifeSafetyMode enumeration.
    mode: u32,
    /// Silenced state — SilencedState enumeration.
    silenced: u32,
    /// Expected operation — LifeSafetyOperation enumeration.
    operation_expected: u32,
    /// Tracking value — LifeSafetyState enumeration.
    tracking_value: u32,
    /// Zones this point belongs to.
    member_of: Vec<ObjectIdentifier>,
    /// Raw sensor reading.
    direct_reading: f32,
    /// Whether maintenance is required.
    maintenance_required: bool,
    /// Event state (0 = NORMAL).
    event_state: u32,
    status_flags: StatusFlags,
    out_of_service: bool,
    /// Reliability (0 = NO_FAULT_DETECTED).
    reliability: u32,
}

impl LifeSafetyPointObject {
    /// Create a new Life Safety Point object.
    ///
    /// Defaults: present_value = QUIET (0), mode = OFF (0), silenced = UNSILENCED (0),
    /// operation_expected = NONE (0), tracking_value = QUIET (0).
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,      // QUIET
            mode: 0,               // OFF
            silenced: 0,           // UNSILENCED
            operation_expected: 0, // NONE
            tracking_value: 0,     // QUIET
            member_of: Vec::new(),
            direct_reading: 0.0,
            maintenance_required: false,
            event_state: 0, // NORMAL
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }

    /// Set the present value (LifeSafetyState enumeration).
    pub fn set_present_value(&mut self, state: u32) {
        self.present_value = state;
    }

    /// Set the operating mode (LifeSafetyMode enumeration).
    pub fn set_mode(&mut self, mode: u32) {
        self.mode = mode;
    }

    /// Set the tracking value (LifeSafetyState enumeration).
    pub fn set_tracking_value(&mut self, state: u32) {
        self.tracking_value = state;
    }

    /// Set the direct reading (raw sensor value).
    pub fn set_direct_reading(&mut self, value: f32) {
        self.direct_reading = value;
    }

    /// Set the description.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Add a zone membership (ObjectIdentifier of a LifeSafetyZone).
    pub fn add_member(&mut self, zone_oid: ObjectIdentifier) {
        self.member_of.push(zone_oid);
    }
}

impl BACnetObject for LifeSafetyPointObject {
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
                ObjectType::LIFE_SAFETY_POINT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::MODE => Ok(PropertyValue::Enumerated(self.mode)),
            p if p == PropertyIdentifier::SILENCED => Ok(PropertyValue::Enumerated(self.silenced)),
            p if p == PropertyIdentifier::OPERATION_EXPECTED => {
                Ok(PropertyValue::Enumerated(self.operation_expected))
            }
            p if p == PropertyIdentifier::TRACKING_VALUE => {
                Ok(PropertyValue::Enumerated(self.tracking_value))
            }
            p if p == PropertyIdentifier::MEMBER_OF => Ok(PropertyValue::List(
                self.member_of
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .collect(),
            )),
            p if p == PropertyIdentifier::DIRECT_READING => {
                Ok(PropertyValue::Real(self.direct_reading))
            }
            p if p == PropertyIdentifier::MAINTENANCE_REQUIRED => {
                Ok(PropertyValue::Boolean(self.maintenance_required))
            }
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state))
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
        // Present value is read-only via protocol
        if property == PropertyIdentifier::PRESENT_VALUE {
            return Err(common::write_access_denied_error());
        }
        if property == PropertyIdentifier::MODE {
            if let PropertyValue::Enumerated(v) = value {
                self.mode = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::SILENCED {
            if let PropertyValue::Enumerated(v) = value {
                self.silenced = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::OPERATION_EXPECTED {
            if let PropertyValue::Enumerated(v) = value {
                self.operation_expected = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::DIRECT_READING {
            if let PropertyValue::Real(v) = value {
                common::reject_non_finite(v)?;
                self.direct_reading = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::MAINTENANCE_REQUIRED {
            if let PropertyValue::Boolean(v) = value {
                self.maintenance_required = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::MODE,
            PropertyIdentifier::SILENCED,
            PropertyIdentifier::OPERATION_EXPECTED,
            PropertyIdentifier::TRACKING_VALUE,
            PropertyIdentifier::MEMBER_OF,
            PropertyIdentifier::DIRECT_READING,
            PropertyIdentifier::MAINTENANCE_REQUIRED,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }

    fn supports_cov(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// LifeSafetyZoneObject (type 22)
// ---------------------------------------------------------------------------

/// BACnet Life Safety Zone object.
///
/// Aggregates one or more Life Safety Point objects into a zone.
/// Present_Value is an enumerated LifeSafetyState, set by the application
/// (typically the worst-case state among zone members).
pub struct LifeSafetyZoneObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Present value — LifeSafetyState enumeration (read-only via protocol).
    present_value: u32,
    /// Operating mode — LifeSafetyMode enumeration.
    mode: u32,
    /// Silenced state — SilencedState enumeration.
    silenced: u32,
    /// Expected operation — LifeSafetyOperation enumeration.
    operation_expected: u32,
    /// Points belonging to this zone.
    zone_members: Vec<ObjectIdentifier>,
    /// Event state (0 = NORMAL).
    event_state: u32,
    status_flags: StatusFlags,
    out_of_service: bool,
    /// Reliability (0 = NO_FAULT_DETECTED).
    reliability: u32,
}

impl LifeSafetyZoneObject {
    /// Create a new Life Safety Zone object.
    ///
    /// Defaults: present_value = QUIET (0), mode = OFF (0), silenced = UNSILENCED (0),
    /// operation_expected = NONE (0).
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,      // QUIET
            mode: 0,               // OFF
            silenced: 0,           // UNSILENCED
            operation_expected: 0, // NONE
            zone_members: Vec::new(),
            event_state: 0, // NORMAL
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }

    /// Set the present value (LifeSafetyState enumeration).
    pub fn set_present_value(&mut self, state: u32) {
        self.present_value = state;
    }

    /// Set the operating mode (LifeSafetyMode enumeration).
    pub fn set_mode(&mut self, mode: u32) {
        self.mode = mode;
    }

    /// Set the description.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Add a point to this zone (ObjectIdentifier of a LifeSafetyPoint).
    pub fn add_zone_member(&mut self, point_oid: ObjectIdentifier) {
        self.zone_members.push(point_oid);
    }
}

impl BACnetObject for LifeSafetyZoneObject {
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
                ObjectType::LIFE_SAFETY_ZONE.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::MODE => Ok(PropertyValue::Enumerated(self.mode)),
            p if p == PropertyIdentifier::SILENCED => Ok(PropertyValue::Enumerated(self.silenced)),
            p if p == PropertyIdentifier::OPERATION_EXPECTED => {
                Ok(PropertyValue::Enumerated(self.operation_expected))
            }
            p if p == PropertyIdentifier::ZONE_MEMBERS => Ok(PropertyValue::List(
                self.zone_members
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .collect(),
            )),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state))
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
        // Present value is read-only via protocol
        if property == PropertyIdentifier::PRESENT_VALUE {
            return Err(common::write_access_denied_error());
        }
        if property == PropertyIdentifier::MODE {
            if let PropertyValue::Enumerated(v) = value {
                self.mode = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::SILENCED {
            if let PropertyValue::Enumerated(v) = value {
                self.silenced = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::OPERATION_EXPECTED {
            if let PropertyValue::Enumerated(v) = value {
                self.operation_expected = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::MODE,
            PropertyIdentifier::SILENCED,
            PropertyIdentifier::OPERATION_EXPECTED,
            PropertyIdentifier::ZONE_MEMBERS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }

    fn supports_cov(&self) -> bool {
        true
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::{LifeSafetyMode, LifeSafetyState, ObjectType};

    // -----------------------------------------------------------------------
    // LifeSafetyPointObject
    // -----------------------------------------------------------------------

    #[test]
    fn point_object_type() {
        let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        assert_eq!(
            pt.object_identifier().object_type(),
            ObjectType::LIFE_SAFETY_POINT
        );
        assert_eq!(pt.object_identifier().instance_number(), 1);
        assert_eq!(pt.object_name(), "LSP-1");
    }

    #[test]
    fn point_read_present_value_default() {
        let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        let val = pt
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(LifeSafetyState::QUIET.to_raw())
        );
    }

    #[test]
    fn point_set_and_read_present_value() {
        let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        pt.set_present_value(LifeSafetyState::ALARM.to_raw());
        let val = pt
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(LifeSafetyState::ALARM.to_raw())
        );
    }

    #[test]
    fn point_present_value_write_denied() {
        let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        let result = pt.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(2),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn point_read_mode_default() {
        let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        let val = pt.read_property(PropertyIdentifier::MODE, None).unwrap();
        assert_eq!(val, PropertyValue::Enumerated(LifeSafetyMode::OFF.to_raw()));
    }

    #[test]
    fn point_set_mode() {
        let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        pt.set_mode(LifeSafetyMode::ON.to_raw());
        let val = pt.read_property(PropertyIdentifier::MODE, None).unwrap();
        assert_eq!(val, PropertyValue::Enumerated(LifeSafetyMode::ON.to_raw()));
    }

    #[test]
    fn point_write_mode() {
        let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        pt.write_property(
            PropertyIdentifier::MODE,
            None,
            PropertyValue::Enumerated(LifeSafetyMode::ARMED.to_raw()),
            None,
        )
        .unwrap();
        let val = pt.read_property(PropertyIdentifier::MODE, None).unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(LifeSafetyMode::ARMED.to_raw())
        );
    }

    #[test]
    fn point_read_silenced_default() {
        let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        let val = pt
            .read_property(PropertyIdentifier::SILENCED, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // UNSILENCED
    }

    #[test]
    fn point_read_tracking_value() {
        let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        pt.set_tracking_value(LifeSafetyState::PRE_ALARM.to_raw());
        let val = pt
            .read_property(PropertyIdentifier::TRACKING_VALUE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(LifeSafetyState::PRE_ALARM.to_raw())
        );
    }

    #[test]
    fn point_read_direct_reading() {
        let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        pt.set_direct_reading(42.5);
        let val = pt
            .read_property(PropertyIdentifier::DIRECT_READING, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(42.5));
    }

    #[test]
    fn point_read_maintenance_required() {
        let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        let val = pt
            .read_property(PropertyIdentifier::MAINTENANCE_REQUIRED, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(false));
    }

    #[test]
    fn point_add_member_and_read() {
        let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        let zone1 = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 1).unwrap();
        let zone2 = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 2).unwrap();
        pt.add_member(zone1);
        pt.add_member(zone2);

        let val = pt
            .read_property(PropertyIdentifier::MEMBER_OF, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(zone1),
                PropertyValue::ObjectIdentifier(zone2),
            ])
        );
    }

    #[test]
    fn point_member_of_empty() {
        let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        let val = pt
            .read_property(PropertyIdentifier::MEMBER_OF, None)
            .unwrap();
        assert_eq!(val, PropertyValue::List(vec![]));
    }

    #[test]
    fn point_read_event_state_default() {
        let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        let val = pt
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // NORMAL
    }

    #[test]
    fn point_read_object_type() {
        let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        let val = pt
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(ObjectType::LIFE_SAFETY_POINT.to_raw())
        );
    }

    #[test]
    fn point_property_list() {
        let pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        let props = pt.property_list();
        assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(props.contains(&PropertyIdentifier::MODE));
        assert!(props.contains(&PropertyIdentifier::SILENCED));
        assert!(props.contains(&PropertyIdentifier::OPERATION_EXPECTED));
        assert!(props.contains(&PropertyIdentifier::TRACKING_VALUE));
        assert!(props.contains(&PropertyIdentifier::MEMBER_OF));
        assert!(props.contains(&PropertyIdentifier::DIRECT_READING));
        assert!(props.contains(&PropertyIdentifier::MAINTENANCE_REQUIRED));
        assert!(props.contains(&PropertyIdentifier::EVENT_STATE));
        assert!(props.contains(&PropertyIdentifier::STATUS_FLAGS));
        assert!(props.contains(&PropertyIdentifier::OUT_OF_SERVICE));
        assert!(props.contains(&PropertyIdentifier::RELIABILITY));
    }

    #[test]
    fn point_write_mode_wrong_type() {
        let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        let result = pt.write_property(
            PropertyIdentifier::MODE,
            None,
            PropertyValue::Real(1.0),
            None,
        );
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // LifeSafetyZoneObject
    // -----------------------------------------------------------------------

    #[test]
    fn zone_object_type() {
        let z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        assert_eq!(
            z.object_identifier().object_type(),
            ObjectType::LIFE_SAFETY_ZONE
        );
        assert_eq!(z.object_identifier().instance_number(), 1);
        assert_eq!(z.object_name(), "LSZ-1");
    }

    #[test]
    fn zone_read_present_value_default() {
        let z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        let val = z
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(LifeSafetyState::QUIET.to_raw())
        );
    }

    #[test]
    fn zone_set_and_read_present_value() {
        let mut z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        z.set_present_value(LifeSafetyState::ALARM.to_raw());
        let val = z
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(LifeSafetyState::ALARM.to_raw())
        );
    }

    #[test]
    fn zone_present_value_write_denied() {
        let mut z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        let result = z.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(2),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn zone_read_mode_default() {
        let z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        let val = z.read_property(PropertyIdentifier::MODE, None).unwrap();
        assert_eq!(val, PropertyValue::Enumerated(LifeSafetyMode::OFF.to_raw()));
    }

    #[test]
    fn zone_set_mode() {
        let mut z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        z.set_mode(LifeSafetyMode::ARMED.to_raw());
        let val = z.read_property(PropertyIdentifier::MODE, None).unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(LifeSafetyMode::ARMED.to_raw())
        );
    }

    #[test]
    fn zone_add_zone_member_and_read() {
        let mut z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        let pt1 = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
        let pt2 = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 2).unwrap();
        let pt3 = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 3).unwrap();
        z.add_zone_member(pt1);
        z.add_zone_member(pt2);
        z.add_zone_member(pt3);

        let val = z
            .read_property(PropertyIdentifier::ZONE_MEMBERS, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(pt1),
                PropertyValue::ObjectIdentifier(pt2),
                PropertyValue::ObjectIdentifier(pt3),
            ])
        );
    }

    #[test]
    fn zone_members_empty() {
        let z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        let val = z
            .read_property(PropertyIdentifier::ZONE_MEMBERS, None)
            .unwrap();
        assert_eq!(val, PropertyValue::List(vec![]));
    }

    #[test]
    fn zone_read_event_state_default() {
        let z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        let val = z
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // NORMAL
    }

    #[test]
    fn zone_read_object_type() {
        let z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        let val = z
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(ObjectType::LIFE_SAFETY_ZONE.to_raw())
        );
    }

    #[test]
    fn zone_property_list() {
        let z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        let props = z.property_list();
        assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(props.contains(&PropertyIdentifier::MODE));
        assert!(props.contains(&PropertyIdentifier::SILENCED));
        assert!(props.contains(&PropertyIdentifier::OPERATION_EXPECTED));
        assert!(props.contains(&PropertyIdentifier::ZONE_MEMBERS));
        assert!(props.contains(&PropertyIdentifier::EVENT_STATE));
        assert!(props.contains(&PropertyIdentifier::STATUS_FLAGS));
        assert!(props.contains(&PropertyIdentifier::OUT_OF_SERVICE));
        assert!(props.contains(&PropertyIdentifier::RELIABILITY));
    }

    #[test]
    fn zone_write_mode() {
        let mut z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        z.write_property(
            PropertyIdentifier::MODE,
            None,
            PropertyValue::Enumerated(LifeSafetyMode::ON.to_raw()),
            None,
        )
        .unwrap();
        let val = z.read_property(PropertyIdentifier::MODE, None).unwrap();
        assert_eq!(val, PropertyValue::Enumerated(LifeSafetyMode::ON.to_raw()));
    }

    #[test]
    fn zone_write_out_of_service() {
        let mut z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        z.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        let val = z
            .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(true));
    }

    #[test]
    fn zone_write_unknown_property_denied() {
        let mut z = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        let result = z.write_property(
            PropertyIdentifier::TRACKING_VALUE,
            None,
            PropertyValue::Enumerated(0),
            None,
        );
        assert!(result.is_err());
    }
}
