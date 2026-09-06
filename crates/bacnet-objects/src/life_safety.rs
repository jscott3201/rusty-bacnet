//! Life Safety Point (type 21) and Life Safety Zone (type 22) objects
//! per ASHRAE 135-2020 Clauses 12.15 and 12.16.

use bacnet_types::enums::{
    ErrorClass, ErrorCode, LifeSafetyOperation, ObjectType, PropertyIdentifier, SilencedState,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::{BACnetObject, LifeSafetyOperationEffect, LifeSafetyOperationOutcome};

mod reset;

pub use reset::{
    LifeSafetyPointResetCommit, LifeSafetyPointResetContext, LifeSafetyPointResetExecutor,
    LifeSafetyResetError, LifeSafetyZoneResetCommit, LifeSafetyZoneResetContext,
    LifeSafetyZoneResetExecutor,
};

fn life_safety_error(code: ErrorCode) -> Error {
    Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: code.to_raw() as u32,
    }
}

fn apply_silenced_operation(
    silenced: &mut u32,
    operation_expected: &mut u32,
    operation: LifeSafetyOperation,
) -> Result<LifeSafetyOperationEffect, Error> {
    let current = *silenced;
    if current > SilencedState::ALL_SILENCED.to_raw() {
        return Err(life_safety_error(
            ErrorCode::INVALID_OPERATION_IN_THIS_STATE,
        ));
    }

    let desired = if operation == LifeSafetyOperation::SILENCE {
        SilencedState::ALL_SILENCED.to_raw()
    } else if operation == LifeSafetyOperation::SILENCE_AUDIBLE {
        current | SilencedState::AUDIBLE_SILENCED.to_raw()
    } else if operation == LifeSafetyOperation::SILENCE_VISUAL {
        current | SilencedState::VISIBLE_SILENCED.to_raw()
    } else if operation == LifeSafetyOperation::UNSILENCE {
        SilencedState::UNSILENCED.to_raw()
    } else if operation == LifeSafetyOperation::UNSILENCE_AUDIBLE {
        current & !SilencedState::AUDIBLE_SILENCED.to_raw()
    } else if operation == LifeSafetyOperation::UNSILENCE_VISUAL {
        current & !SilencedState::VISIBLE_SILENCED.to_raw()
    } else {
        return Err(life_safety_error(ErrorCode::VALUE_OUT_OF_RANGE));
    };

    if *operation_expected != operation.to_raw() {
        return Err(life_safety_error(
            ErrorCode::INVALID_OPERATION_IN_THIS_STATE,
        ));
    }

    *silenced = desired;
    *operation_expected = LifeSafetyOperation::NONE.to_raw();
    Ok(LifeSafetyOperationEffect::Applied)
}

const POINT_COV_PROPERTIES: [PropertyIdentifier; 5] = [
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::TRACKING_VALUE,
    PropertyIdentifier::SILENCED,
    PropertyIdentifier::OPERATION_EXPECTED,
    PropertyIdentifier::STATUS_FLAGS,
];

const ZONE_COV_PROPERTIES: [PropertyIdentifier; 4] = [
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::SILENCED,
    PropertyIdentifier::OPERATION_EXPECTED,
    PropertyIdentifier::STATUS_FLAGS,
];

fn operation_outcome(
    object: &dyn BACnetObject,
    before: Vec<(PropertyIdentifier, PropertyValue)>,
    effect: LifeSafetyOperationEffect,
) -> LifeSafetyOperationOutcome {
    let changed_properties = before
        .into_iter()
        .filter_map(|(property, previous)| {
            object
                .read_property(property, None)
                .is_ok_and(|current| current != previous)
                .then_some(property)
        })
        .collect();
    LifeSafetyOperationOutcome {
        effect,
        changed_properties,
    }
}

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
    /// Application-owned physical reset integration, configured before insertion.
    reset_executor: Option<LifeSafetyPointResetExecutor>,
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
            reset_executor: None,
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

    /// Set the locally determined silenced state.
    pub fn set_silenced(&mut self, state: SilencedState) {
        self.silenced = state.to_raw();
    }

    /// Set the next LifeSafetyOperation expected by local device logic.
    pub fn set_operation_expected(&mut self, operation: LifeSafetyOperation) {
        self.operation_expected = operation.to_raw();
    }

    /// Configure the application-owned reset executor before database insertion.
    ///
    /// The executor is used only for `RESET`, `RESET_ALARM`, and `RESET_FAULT`.
    /// See [`LifeSafetyPointResetExecutor`] for its synchronous execution contract.
    pub fn set_reset_executor(&mut self, executor: LifeSafetyPointResetExecutor) {
        self.reset_executor = Some(executor);
    }

    /// Configure and return this point for insertion into an object database.
    pub fn with_reset_executor(mut self, executor: LifeSafetyPointResetExecutor) -> Self {
        self.set_reset_executor(executor);
        self
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
        if property == PropertyIdentifier::SILENCED
            || property == PropertyIdentifier::OPERATION_EXPECTED
        {
            return Err(common::write_access_denied_error());
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

    fn supports_cov_property(&self, property: PropertyIdentifier) -> bool {
        POINT_COV_PROPERTIES.contains(&property)
    }

    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        matches!(
            property,
            PropertyIdentifier::MODE
                | PropertyIdentifier::DIRECT_READING
                | PropertyIdentifier::MAINTENANCE_REQUIRED
                | PropertyIdentifier::DESCRIPTION
                | PropertyIdentifier::OUT_OF_SERVICE
        )
    }

    fn apply_life_safety_operation(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<LifeSafetyOperationEffect, Error> {
        if reset::is_reset_operation(operation) {
            self.apply_reset_operation(operation)
        } else {
            apply_silenced_operation(&mut self.silenced, &mut self.operation_expected, operation)
        }
    }

    fn apply_life_safety_operation_detailed(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<LifeSafetyOperationOutcome, Error> {
        let before = POINT_COV_PROPERTIES
            .into_iter()
            .filter_map(|property| {
                self.read_property(property, None)
                    .ok()
                    .map(|value| (property, value))
            })
            .collect();
        let effect = self.apply_life_safety_operation(operation)?;
        Ok(operation_outcome(self, before, effect))
    }

    fn set_life_safety_operation_expected_internal(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<(), Error> {
        self.set_operation_expected(operation);
        Ok(())
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
    /// Application-owned physical reset integration, configured before insertion.
    reset_executor: Option<LifeSafetyZoneResetExecutor>,
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
            reset_executor: None,
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

    /// Set the locally determined silenced state.
    pub fn set_silenced(&mut self, state: SilencedState) {
        self.silenced = state.to_raw();
    }

    /// Set the next LifeSafetyOperation expected by local device logic.
    pub fn set_operation_expected(&mut self, operation: LifeSafetyOperation) {
        self.operation_expected = operation.to_raw();
    }

    /// Configure the application-owned reset executor before database insertion.
    ///
    /// The executor is used only for `RESET`, `RESET_ALARM`, and `RESET_FAULT`.
    /// See [`LifeSafetyZoneResetExecutor`] for its synchronous execution contract.
    pub fn set_reset_executor(&mut self, executor: LifeSafetyZoneResetExecutor) {
        self.reset_executor = Some(executor);
    }

    /// Configure and return this zone for insertion into an object database.
    pub fn with_reset_executor(mut self, executor: LifeSafetyZoneResetExecutor) -> Self {
        self.set_reset_executor(executor);
        self
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
        if property == PropertyIdentifier::SILENCED
            || property == PropertyIdentifier::OPERATION_EXPECTED
        {
            return Err(common::write_access_denied_error());
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

    fn supports_cov_property(&self, property: PropertyIdentifier) -> bool {
        ZONE_COV_PROPERTIES.contains(&property)
    }

    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        matches!(
            property,
            PropertyIdentifier::MODE
                | PropertyIdentifier::DESCRIPTION
                | PropertyIdentifier::OUT_OF_SERVICE
        )
    }

    fn apply_life_safety_operation(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<LifeSafetyOperationEffect, Error> {
        if reset::is_reset_operation(operation) {
            self.apply_reset_operation(operation)
        } else {
            apply_silenced_operation(&mut self.silenced, &mut self.operation_expected, operation)
        }
    }

    fn apply_life_safety_operation_detailed(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<LifeSafetyOperationOutcome, Error> {
        let before = ZONE_COV_PROPERTIES
            .into_iter()
            .filter_map(|property| {
                self.read_property(property, None)
                    .ok()
                    .map(|value| (property, value))
            })
            .collect();
        let effect = self.apply_life_safety_operation(operation)?;
        Ok(operation_outcome(self, before, effect))
    }

    fn set_life_safety_operation_expected_internal(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<(), Error> {
        self.set_operation_expected(operation);
        Ok(())
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests;

#[cfg(test)]
mod reset_tests;
