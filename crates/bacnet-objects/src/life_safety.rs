//! Life Safety Point (type 21) and Life Safety Zone (type 22) objects
//! per ASHRAE 135-2020 Clauses 12.15 and 12.16.
//!
//! R1 matrix (PR-0803 sub-slice 1) — Event_State / Status_Flags post-state for
//! each LifeSafetyOperation on Point and Zone. Verdict: outcome (b), pins only
//! with zero state change. The Standard keeps Event_State purely intrinsic —
//! driven by the object's event algorithm off the monitored LifeSafetyState,
//! mode changes, delays, and re-alert — and no service clause mandates an
//! LSO-driven Event_State or Status_Flags transition:
//!
//! | Operation         | Point Event_State | Point IN_ALARM | Zone Event_State | Zone IN_ALARM |
//! |-------------------|-------------------|----------------|------------------|---------------|
//! | SILENCE           | NORMAL (0)        | FALSE          | NORMAL (0)       | FALSE         |
//! | SILENCE_AUDIBLE   | NORMAL (0)        | FALSE          | NORMAL (0)       | FALSE         |
//! | SILENCE_VISUAL    | NORMAL (0)        | FALSE          | NORMAL (0)       | FALSE         |
//! | UNSILENCE         | NORMAL (0)        | FALSE          | NORMAL (0)       | FALSE         |
//! | UNSILENCE_AUDIBLE | NORMAL (0)        | FALSE          | NORMAL (0)       | FALSE         |
//! | UNSILENCE_VISUAL  | NORMAL (0)        | FALSE          | NORMAL (0)       | FALSE         |
//! | RESET             | NORMAL (0)        | FALSE          | NORMAL (0)       | FALSE         |
//! | RESET_ALARM       | NORMAL (0)        | FALSE          | NORMAL (0)       | FALSE         |
//! | RESET_FAULT       | NORMAL (0)        | FALSE          | NORMAL (0)       | FALSE         |
//!
//! Rationale (paraphrased; see cited pages, never normative text): the object
//! clauses describe Event_State as read-only, mirroring the event algorithm
//! only when intrinsic reporting is supported and otherwise staying NORMAL —
//! and this crate implements no intrinsic reporting, so NORMAL is the
//! spec-correct value. The IN_ALARM flag mirrors a non-NORMAL Event_State.
//! Present_Value latching until reset and Tracking_Value continuous tracking
//! are local matters; Silenced records whether the latest audible/visual
//! transition was silenced via service request or local means, and
//! Operation_Expected names the next operation the local situation calls for.
//! The LifeSafetyOperation service clause only silences/resets/unsilences the
//! addressed (or all applicable) objects and answers Result(+/-), rejecting a
//! reset the object is not ready for; it carries no Event_State/Status_Flags
//! rows. The CHANGE_OF_LIFE_SAFETY algorithm keys transitions off the
//! monitored state versus the alarm lists (plus mode/delay/re-alert) while
//! Operation_Expected and Status_Flags travel as notification inputs only.
//! Page cites in the local licensed PDF (`_spec/2020_ASHRAE_...pdf`):
//! object clauses printed pp. 245-256 (PDF pp. 247-258), LifeSafetyOperation
//! service printed pp. 701-702 (PDF pp. 703-704), CHANGE_OF_LIFE_SAFETY
//! printed pp. 657-658 (PDF pp. 659-660). Refs #177 (no claim change here).
//!
//! Point and Zone both serve Tracking_Value (Clauses 12.15.5 and 12.16.5): the
//! application keeps it current through `set_tracking_value` or a reset
//! commit, and both objects offer it for property COV. While Out_Of_Service is
//! TRUE a client may write Tracking_Value and Reliability instead (#1108); see
//! `out_of_service.rs` for how the simulated values interact with the rest of
//! the object. Once a server holds the object, the application reaches
//! Present_Value and Tracking_Value through `set_present_value_internal` and
//! `set_tracking_value_internal` (#1123); `application.rs` records how that
//! route meets latching, Silenced, the reset executor and Out_Of_Service.
//!
//! Accepted_Modes (Clauses 12.15.13 and 12.16.13) is the configured set of
//! modes a network write of Mode may select. It starts as every standard
//! LifeSafetyMode and an application narrows it with `set_accepted_modes`. A
//! Mode write naming any other value fails with PROPERTY / VALUE_OUT_OF_RANGE
//! and leaves the object unchanged. The local `set_mode` ignores the list,
//! because the object's own logic may move Mode outside it.

use crate::prelude::*;
use alloc::borrow::Cow;
use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, LifeSafetyMode, LifeSafetyOperation, LifeSafetyState,
    ObjectType, PropertyIdentifier, Reliability, SilencedState,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use crate::common::{self, read_common_properties};
use crate::device_reference::reference_list;
use crate::traits::{BACnetObject, LifeSafetyOperationEffect, LifeSafetyOperationOutcome};

mod application;
mod membership;
mod metadata;
mod out_of_service;
mod reset;

use out_of_service::{DeviceValues, Simulation};

pub use reset::{
    LifeSafetyPointResetCommit, LifeSafetyPointResetContext, LifeSafetyPointResetExecutor,
    LifeSafetyResetError, LifeSafetyZoneResetCommit, LifeSafetyZoneResetContext,
    LifeSafetyZoneResetExecutor,
};

/// Whether a state is one BACnetLifeSafetyState defines or one from the
/// proprietary range 256..=65535 (Clause 21).
fn valid_life_safety_state(state: LifeSafetyState) -> bool {
    LifeSafetyState::ALL_NAMED
        .iter()
        .any(|&(_, named)| named == state)
        || (256..=65_535).contains(&state.to_raw())
}

fn life_safety_error(code: ErrorCode) -> Error {
    Error::protocol(ErrorClass::OBJECT, code)
}

fn apply_silenced_operation(
    silenced: &mut SilencedState,
    operation_expected: &mut LifeSafetyOperation,
    operation: LifeSafetyOperation,
) -> Result<LifeSafetyOperationEffect, Error> {
    // The four standard SilencedState values form a two-bit audible/visible
    // set, so the partial operations are bit operations on the wire value. A
    // reserved or proprietary state has no such decomposition and is refused.
    let current = silenced.to_raw();
    if current > SilencedState::ALL_SILENCED.to_raw() {
        return Err(life_safety_error(
            ErrorCode::INVALID_OPERATION_IN_THIS_STATE,
        ));
    }

    let desired = if operation == LifeSafetyOperation::SILENCE {
        SilencedState::ALL_SILENCED
    } else if operation == LifeSafetyOperation::SILENCE_AUDIBLE {
        SilencedState::from_raw(current | SilencedState::AUDIBLE_SILENCED.to_raw())
    } else if operation == LifeSafetyOperation::SILENCE_VISUAL {
        SilencedState::from_raw(current | SilencedState::VISIBLE_SILENCED.to_raw())
    } else if operation == LifeSafetyOperation::UNSILENCE {
        SilencedState::UNSILENCED
    } else if operation == LifeSafetyOperation::UNSILENCE_AUDIBLE {
        SilencedState::from_raw(current & !SilencedState::AUDIBLE_SILENCED.to_raw())
    } else if operation == LifeSafetyOperation::UNSILENCE_VISUAL {
        SilencedState::from_raw(current & !SilencedState::VISIBLE_SILENCED.to_raw())
    } else {
        return Err(life_safety_error(ErrorCode::VALUE_OUT_OF_RANGE));
    };

    if *operation_expected != operation {
        return Err(life_safety_error(
            ErrorCode::INVALID_OPERATION_IN_THIS_STATE,
        ));
    }

    *silenced = desired;
    *operation_expected = LifeSafetyOperation::NONE;
    Ok(LifeSafetyOperationEffect::Applied)
}

/// Properties a Point or Zone offers for property COV, in the order an
/// operation outcome reports their deltas.
const COV_PROPERTIES: [PropertyIdentifier; 5] = [
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::TRACKING_VALUE,
    PropertyIdentifier::SILENCED,
    PropertyIdentifier::OPERATION_EXPECTED,
    PropertyIdentifier::STATUS_FLAGS,
];

/// The Accepted_Modes a new Point or Zone starts with: every standard mode.
fn standard_life_safety_modes() -> Vec<LifeSafetyMode> {
    LifeSafetyMode::ALL_NAMED
        .iter()
        .map(|&(_, mode)| mode)
        .collect()
}

/// Keep each mode once, in the caller's order.
fn distinct_modes(modes: impl IntoIterator<Item = LifeSafetyMode>) -> Vec<LifeSafetyMode> {
    let mut distinct = Vec::new();
    for mode in modes {
        if !distinct.contains(&mode) {
            distinct.push(mode);
        }
    }
    distinct
}

fn read_accepted_modes(accepted_modes: &[LifeSafetyMode]) -> PropertyValue {
    PropertyValue::List(
        accepted_modes
            .iter()
            .map(|mode| PropertyValue::Enumerated(mode.to_raw()))
            .collect(),
    )
}

/// Apply a network write of Mode. A mode missing from Accepted_Modes is
/// refused with PROPERTY / VALUE_OUT_OF_RANGE (Clauses 12.15.13 and 12.16.13)
/// and Mode keeps its value.
fn write_mode(
    mode: &mut LifeSafetyMode,
    accepted_modes: &[LifeSafetyMode],
    value: &PropertyValue,
) -> Result<(), Error> {
    let PropertyValue::Enumerated(raw) = value else {
        return Err(common::invalid_data_type_error());
    };
    let requested = LifeSafetyMode::from_raw(*raw);
    if !accepted_modes.contains(&requested) {
        return Err(common::value_out_of_range_error());
    }
    *mode = requested;
    Ok(())
}

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
/// application via [`set_present_value`](Self::set_present_value), or through
/// [`BACnetObject::set_present_value_internal`] once a server holds the point.
pub struct LifeSafetyPointObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Present value (read-only via protocol).
    present_value: LifeSafetyState,
    /// Operating mode.
    mode: LifeSafetyMode,
    /// Modes a network write of Mode may select.
    accepted_modes: Vec<LifeSafetyMode>,
    /// Silenced state.
    silenced: SilencedState,
    /// Expected operation.
    operation_expected: LifeSafetyOperation,
    /// Tracking value.
    tracking_value: LifeSafetyState,
    /// Zones this point belongs to (Member_Of).
    member_of: Vec<BACnetDeviceObjectReference>,
    /// Raw sensor reading.
    direct_reading: f32,
    /// Whether maintenance is required.
    maintenance_required: bool,
    /// Event_State.
    event_state: EventState,
    status_flags: StatusFlags,
    out_of_service: bool,
    /// Reliability; NO_FAULT_DETECTED until a fault is evaluated or simulated.
    reliability: Reliability,
    /// The application's Tracking_Value and Reliability while a client
    /// simulates them.
    device_values: DeviceValues,
    /// Application-owned physical reset integration, configured before insertion.
    reset_executor: Option<LifeSafetyPointResetExecutor>,
}

impl LifeSafetyPointObject {
    /// Create a new Life Safety Point object.
    ///
    /// Defaults: present_value = QUIET, mode = OFF, accepted_modes = every
    /// standard mode, silenced = UNSILENCED, operation_expected = NONE,
    /// tracking_value = QUIET.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: LifeSafetyState::QUIET,
            mode: LifeSafetyMode::OFF,
            accepted_modes: standard_life_safety_modes(),
            silenced: SilencedState::UNSILENCED,
            operation_expected: LifeSafetyOperation::NONE,
            tracking_value: LifeSafetyState::QUIET,
            member_of: Vec::new(),
            direct_reading: 0.0,
            maintenance_required: false,
            event_state: EventState::NORMAL,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
            device_values: DeviceValues::default(),
            reset_executor: None,
        })
    }

    /// Set the present value.
    pub fn set_present_value(&mut self, state: LifeSafetyState) {
        self.present_value = state;
    }

    /// Set the operating mode from local logic, which Accepted_Modes does not
    /// constrain.
    pub fn set_mode(&mut self, mode: LifeSafetyMode) {
        self.mode = mode;
    }

    /// Configure Accepted_Modes, the modes a network write of Mode may select.
    ///
    /// A repeated mode is kept once. The current Mode stays as it is even when
    /// the new list leaves it out.
    pub fn set_accepted_modes(&mut self, modes: impl IntoIterator<Item = LifeSafetyMode>) {
        self.accepted_modes = distinct_modes(modes);
    }

    /// Set the tracking value the device reports.
    ///
    /// While Out_Of_Service is TRUE a client's simulated value keeps being
    /// served, and this one takes over on the return to service.
    pub fn set_tracking_value(&mut self, state: LifeSafetyState) {
        self.simulation().track(state);
    }

    fn simulation(&mut self) -> Simulation<'_> {
        Simulation {
            out_of_service: &mut self.out_of_service,
            reliability: &mut self.reliability,
            tracking_value: &mut self.tracking_value,
            device_values: &mut self.device_values,
        }
    }

    /// Set the locally determined silenced state.
    pub fn set_silenced(&mut self, state: SilencedState) {
        self.silenced = state;
    }

    /// Set the next LifeSafetyOperation expected by local device logic.
    pub fn set_operation_expected(&mut self, operation: LifeSafetyOperation) {
        self.operation_expected = operation;
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

    /// Add a zone to Member_Of: a Life Safety Zone, in this device (an
    /// `ObjectIdentifier` converts) or in the device the reference names.
    ///
    /// A zone already listed stays listed once. Any other object type, or a
    /// Device member that isn't a Device identifier, fails with PROPERTY /
    /// VALUE_OUT_OF_RANGE and changes nothing.
    pub fn add_member(
        &mut self,
        zone: impl Into<BACnetDeviceObjectReference>,
    ) -> Result<(), Error> {
        membership::add(&mut self.member_of, zone.into(), membership::ZONES)
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
                Ok(PropertyValue::Enumerated(self.present_value.to_raw()))
            }
            p if p == PropertyIdentifier::MODE => Ok(PropertyValue::Enumerated(self.mode.to_raw())),
            p if p == PropertyIdentifier::ACCEPTED_MODES => {
                Ok(read_accepted_modes(&self.accepted_modes))
            }
            p if p == PropertyIdentifier::SILENCED => {
                Ok(PropertyValue::Enumerated(self.silenced.to_raw()))
            }
            p if p == PropertyIdentifier::OPERATION_EXPECTED => {
                Ok(PropertyValue::Enumerated(self.operation_expected.to_raw()))
            }
            p if p == PropertyIdentifier::TRACKING_VALUE => {
                Ok(PropertyValue::Enumerated(self.tracking_value.to_raw()))
            }
            p if p == PropertyIdentifier::MEMBER_OF => Ok(reference_list(&self.member_of)),
            p if p == PropertyIdentifier::DIRECT_READING => {
                Ok(PropertyValue::Real(self.direct_reading))
            }
            p if p == PropertyIdentifier::MAINTENANCE_REQUIRED => {
                Ok(PropertyValue::Boolean(self.maintenance_required))
            }
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state.to_raw()))
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
            return write_mode(&mut self.mode, &self.accepted_modes, &value);
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
        if let Some(result) = self.simulation().write(property, &value) {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            _array_index,
        ))
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_point(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn supports_cov_property(&self, property: PropertyIdentifier) -> bool {
        COV_PROPERTIES.contains(&property)
    }

    fn apply_life_safety_operation(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<LifeSafetyOperationOutcome, Error> {
        let before = COV_PROPERTIES
            .into_iter()
            .filter_map(|property| {
                self.read_property(property, None)
                    .ok()
                    .map(|value| (property, value))
            })
            .collect();
        let effect = if reset::is_reset_operation(operation) {
            self.apply_reset_operation(operation)?
        } else {
            apply_silenced_operation(&mut self.silenced, &mut self.operation_expected, operation)?
        };
        Ok(operation_outcome(self, before, effect))
    }

    fn set_life_safety_operation_expected_internal(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<(), Error> {
        self.set_operation_expected(operation);
        Ok(())
    }

    fn set_reliability_internal(&mut self, reliability: Reliability) -> Result<(), Error> {
        self.simulation().set_reliability(reliability)
    }

    fn set_present_value_internal(&mut self, value: PropertyValue) -> Result<(), Error> {
        self.present_value = application::life_safety_state(&value)?;
        Ok(())
    }

    fn set_tracking_value_internal(&mut self, value: PropertyValue) -> Result<(), Error> {
        let state = application::life_safety_state(&value)?;
        self.set_tracking_value(state);
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
/// (typically the worst-case state among zone members) via
/// [`set_present_value`](Self::set_present_value), or through
/// [`BACnetObject::set_present_value_internal`] once a server holds the zone.
pub struct LifeSafetyZoneObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Present value (read-only via protocol).
    present_value: LifeSafetyState,
    /// Operating mode.
    mode: LifeSafetyMode,
    /// Modes a network write of Mode may select.
    accepted_modes: Vec<LifeSafetyMode>,
    /// Silenced state.
    silenced: SilencedState,
    /// Expected operation.
    operation_expected: LifeSafetyOperation,
    /// Tracking value.
    tracking_value: LifeSafetyState,
    /// Points and zones belonging to this zone (Zone_Members).
    zone_members: Vec<BACnetDeviceObjectReference>,
    /// Zones this zone belongs to (Member_Of).
    member_of: Vec<BACnetDeviceObjectReference>,
    /// Event_State.
    event_state: EventState,
    status_flags: StatusFlags,
    out_of_service: bool,
    /// Reliability; NO_FAULT_DETECTED until a fault is evaluated or simulated.
    reliability: Reliability,
    /// The application's Tracking_Value and Reliability while a client
    /// simulates them.
    device_values: DeviceValues,
    /// Application-owned physical reset integration, configured before insertion.
    reset_executor: Option<LifeSafetyZoneResetExecutor>,
}

impl LifeSafetyZoneObject {
    /// Create a new Life Safety Zone object.
    ///
    /// Defaults: present_value = QUIET, mode = OFF, accepted_modes = every
    /// standard mode, silenced = UNSILENCED, operation_expected = NONE,
    /// tracking_value = QUIET.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: LifeSafetyState::QUIET,
            mode: LifeSafetyMode::OFF,
            accepted_modes: standard_life_safety_modes(),
            silenced: SilencedState::UNSILENCED,
            operation_expected: LifeSafetyOperation::NONE,
            tracking_value: LifeSafetyState::QUIET,
            zone_members: Vec::new(),
            member_of: Vec::new(),
            event_state: EventState::NORMAL,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
            device_values: DeviceValues::default(),
            reset_executor: None,
        })
    }

    /// Set the present value.
    pub fn set_present_value(&mut self, state: LifeSafetyState) {
        self.present_value = state;
    }

    /// Set the operating mode from local logic, which Accepted_Modes does not
    /// constrain.
    pub fn set_mode(&mut self, mode: LifeSafetyMode) {
        self.mode = mode;
    }

    /// Configure Accepted_Modes, the modes a network write of Mode may select.
    ///
    /// A repeated mode is kept once. The current Mode stays as it is even when
    /// the new list leaves it out.
    pub fn set_accepted_modes(&mut self, modes: impl IntoIterator<Item = LifeSafetyMode>) {
        self.accepted_modes = distinct_modes(modes);
    }

    /// Set the tracking value the device reports.
    ///
    /// While Out_Of_Service is TRUE a client's simulated value keeps being
    /// served, and this one takes over on the return to service.
    pub fn set_tracking_value(&mut self, state: LifeSafetyState) {
        self.simulation().track(state);
    }

    fn simulation(&mut self) -> Simulation<'_> {
        Simulation {
            out_of_service: &mut self.out_of_service,
            reliability: &mut self.reliability,
            tracking_value: &mut self.tracking_value,
            device_values: &mut self.device_values,
        }
    }

    /// Set the locally determined silenced state.
    pub fn set_silenced(&mut self, state: SilencedState) {
        self.silenced = state;
    }

    /// Set the next LifeSafetyOperation expected by local device logic.
    pub fn set_operation_expected(&mut self, operation: LifeSafetyOperation) {
        self.operation_expected = operation;
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

    /// Add a member to Zone_Members: a Life Safety Point or Zone, in this
    /// device (an `ObjectIdentifier` converts) or in the device the reference
    /// names.
    ///
    /// A member already listed stays listed once. Any other object type, or a
    /// Device member that isn't a Device identifier, fails with PROPERTY /
    /// VALUE_OUT_OF_RANGE and changes nothing.
    pub fn add_zone_member(
        &mut self,
        member: impl Into<BACnetDeviceObjectReference>,
    ) -> Result<(), Error> {
        membership::add(
            &mut self.zone_members,
            member.into(),
            membership::ZONE_MEMBERS,
        )
    }

    /// Add a zone to Member_Of: a Life Safety Zone this zone belongs to, with
    /// the same refusals as [`add_zone_member`](Self::add_zone_member).
    pub fn add_member(
        &mut self,
        zone: impl Into<BACnetDeviceObjectReference>,
    ) -> Result<(), Error> {
        membership::add(&mut self.member_of, zone.into(), membership::ZONES)
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
                Ok(PropertyValue::Enumerated(self.present_value.to_raw()))
            }
            p if p == PropertyIdentifier::MODE => Ok(PropertyValue::Enumerated(self.mode.to_raw())),
            p if p == PropertyIdentifier::ACCEPTED_MODES => {
                Ok(read_accepted_modes(&self.accepted_modes))
            }
            p if p == PropertyIdentifier::SILENCED => {
                Ok(PropertyValue::Enumerated(self.silenced.to_raw()))
            }
            p if p == PropertyIdentifier::OPERATION_EXPECTED => {
                Ok(PropertyValue::Enumerated(self.operation_expected.to_raw()))
            }
            p if p == PropertyIdentifier::TRACKING_VALUE => {
                Ok(PropertyValue::Enumerated(self.tracking_value.to_raw()))
            }
            p if p == PropertyIdentifier::ZONE_MEMBERS => Ok(reference_list(&self.zone_members)),
            p if p == PropertyIdentifier::MEMBER_OF => Ok(reference_list(&self.member_of)),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state.to_raw()))
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
            return write_mode(&mut self.mode, &self.accepted_modes, &value);
        }
        if property == PropertyIdentifier::SILENCED
            || property == PropertyIdentifier::OPERATION_EXPECTED
        {
            return Err(common::write_access_denied_error());
        }
        if let Some(result) = self.simulation().write(property, &value) {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            _array_index,
        ))
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_zone(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn supports_cov_property(&self, property: PropertyIdentifier) -> bool {
        COV_PROPERTIES.contains(&property)
    }

    fn apply_life_safety_operation(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<LifeSafetyOperationOutcome, Error> {
        let before = COV_PROPERTIES
            .into_iter()
            .filter_map(|property| {
                self.read_property(property, None)
                    .ok()
                    .map(|value| (property, value))
            })
            .collect();
        let effect = if reset::is_reset_operation(operation) {
            self.apply_reset_operation(operation)?
        } else {
            apply_silenced_operation(&mut self.silenced, &mut self.operation_expected, operation)?
        };
        Ok(operation_outcome(self, before, effect))
    }

    fn set_life_safety_operation_expected_internal(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<(), Error> {
        self.set_operation_expected(operation);
        Ok(())
    }

    fn set_reliability_internal(&mut self, reliability: Reliability) -> Result<(), Error> {
        self.simulation().set_reliability(reliability)
    }

    fn set_present_value_internal(&mut self, value: PropertyValue) -> Result<(), Error> {
        self.present_value = application::life_safety_state(&value)?;
        Ok(())
    }

    fn set_tracking_value_internal(&mut self, value: PropertyValue) -> Result<(), Error> {
        let state = application::life_safety_state(&value)?;
        self.set_tracking_value(state);
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

#[cfg(test)]
mod event_state_tests;

#[cfg(test)]
mod accepted_modes_tests;

#[cfg(test)]
mod out_of_service_tests;

#[cfg(test)]
mod application_tests;

#[cfg(test)]
mod membership_tests;
