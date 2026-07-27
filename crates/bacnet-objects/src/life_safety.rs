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
mod tests;
