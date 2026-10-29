use bacnet_types::enums::AccessZoneOccupancyState;

use super::zone_occupancy::{Occupancy, adjusted};
use super::zone_out_of_service::ZoneState;
use super::*;
use crate::event::state_reporting::{ChangeOfStateReporting, ListedStates};

// AccessZoneObject (type 36)
// ---------------------------------------------------------------------------

/// BACnet Access Zone object (type 36).
///
/// Represents a physical zone or area controlled by access points. Table
/// 12-37 has neither Present_Value nor Access_Doors, so the object serves
/// neither (#1064).
///
/// The zone counts occupancy (#1284): Occupancy_State follows
/// Occupancy_Count, the occupancy limits and Occupancy_Count_Enable, and a
/// client adjusts the count through Adjust_Value. The module
/// `zone_occupancy` has the rules.
///
/// The zone reports intrinsically on Occupancy_State with the
/// CHANGE_OF_STATE algorithm (Clause 12.32; #1305): Event_State goes
/// OFFNORMAL once Occupancy_State has stayed in Alarm_Values for Time_Delay
/// seconds and back to NORMAL once it has stayed out of them for
/// Time_Delay_Normal, and a Reliability other than NO_FAULT_DETECTED puts it
/// in FAULT. The event rows are served and written through
/// `ChangeOfStateReporting`, and the server sends the notifications to the
/// Notification Class recipients. Alarm_Values never holds NORMAL, the state
/// with no limit crossed: listed, it would put the zone in alarm whenever its
/// count sat inside its limits (#1401).
///
/// Entry_Points and Exit_Points list the Access Points leading into and out
/// of the zone as `BACnetDeviceObjectReference` values (Clauses 12.32.23 and
/// 12.32.24; #1306), set by the application and read-only over the network.
///
/// While Out_Of_Service is TRUE a client can simulate Occupancy_Count and
/// Reliability by writing them, and the zone's own values come back on the
/// return to service (Table 12-37 footnote 1; #1247). The module
/// `zone_out_of_service` has the details.
pub struct AccessZoneObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    global_identifier: u64,
    /// Occupancy_Count and Reliability as served: the zone's own, or a
    /// client's simulation while Out_Of_Service is TRUE.
    state: ZoneState,
    /// The zone's own values, put aside while Out_Of_Service is TRUE.
    device_state: Option<ZoneState>,
    /// Occupancy_Count_Enable, Adjust_Value and the occupancy limits.
    occupancy: Occupancy,
    entry_points: Vec<BACnetDeviceObjectReference>,
    exit_points: Vec<BACnetDeviceObjectReference>,
    out_of_service: bool,
    /// Intrinsic reporting on Occupancy_State.
    reporting: ChangeOfStateReporting,
}

impl AccessZoneObject {
    /// Create a new Access Zone object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_ZONE, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            global_identifier: 0,
            state: ZoneState {
                occupancy_count: 0,
                reliability: Reliability::NO_FAULT_DETECTED,
            },
            device_state: None,
            occupancy: Occupancy::NEW,
            entry_points: Vec::new(),
            exit_points: Vec::new(),
            out_of_service: false,
            reporting: ChangeOfStateReporting::new(ALARM_STATES),
        })
    }

    /// Set Entry_Points, the Access Points leading into the zone
    /// (Clause 12.32.23). A reference with no device identifier names an
    /// object in this device.
    ///
    /// Each reference has to name an Access Point object, and its device
    /// identifier, when given, a Device object (#1285); a list breaking
    /// either rule is refused with VALUE_OUT_OF_RANGE and the points set
    /// before are kept. The list is read-only over the network.
    pub fn set_entry_points(
        &mut self,
        points: impl IntoIterator<Item = impl Into<BACnetDeviceObjectReference>>,
    ) -> Result<(), Error> {
        self.entry_points = references_to(ObjectType::ACCESS_POINT, points)?;
        Ok(())
    }

    /// Set Exit_Points, the Access Points leading out of the zone
    /// (Clause 12.32.24), with the checks [`Self::set_entry_points`] makes.
    pub fn set_exit_points(
        &mut self,
        points: impl IntoIterator<Item = impl Into<BACnetDeviceObjectReference>>,
    ) -> Result<(), Error> {
        self.exit_points = references_to(ObjectType::ACCESS_POINT, points)?;
        Ok(())
    }

    /// Set Alarm_Values, the occupancy states the zone reports as offnormal
    /// (Clause 12.32.27). NORMAL, or a value outside the
    /// BACnetAccessZoneOccupancyState production, named or proprietary (64
    /// to 65535), is refused with VALUE_OUT_OF_RANGE naming the element, and
    /// the values set before are kept. Clients can write the list too, as
    /// they can the zone's other event configuration (Time_Delay,
    /// Notification_Class, Event_Enable and the rest), and meet the same
    /// checks.
    pub fn set_alarm_values(
        &mut self,
        states: impl IntoIterator<Item = AccessZoneOccupancyState>,
    ) -> Result<(), Error> {
        self.reporting
            .set_alarm_values(states.into_iter().map(|state| state.to_raw()).collect())
    }

    /// Record the zone's occupancy count, as the application's counting
    /// works it out (Clause 12.32.11). Over the network Occupancy_Count takes
    /// writes only while Out_Of_Service is TRUE; meanwhile the client's
    /// simulated count keeps being served, and this one takes over on the
    /// return to service. While counting is off the count stays zero and the
    /// call changes nothing.
    pub fn set_occupancy_count(&mut self, count: u64) {
        if self.occupancy.enabled {
            self.device_state_mut().occupancy_count = count;
        }
    }

    /// Set Occupancy_Count_Enable (Clause 12.32.12), read-only over the
    /// network. Turning counting off zeroes Occupancy_Count, the count set
    /// aside while out of service and Adjust_Value, and Occupancy_State reads
    /// DISABLED until counting is turned on again, starting from zero.
    pub fn set_occupancy_count_enable(&mut self, enabled: bool) {
        self.occupancy.enabled = enabled;
        if !enabled {
            self.occupancy.adjust_value = 0;
            self.state.occupancy_count = 0;
            if let Some(own) = &mut self.device_state {
                own.occupancy_count = 0;
            }
        }
    }

    /// Set Occupancy_Lower_Limit and Occupancy_Upper_Limit together, zero
    /// meaning no limit (Clauses 12.32.14 and 12.32.15). Both are read-only
    /// over the network. A nonzero upper limit at or below the lower one is
    /// refused with VALUE_OUT_OF_RANGE and both limits are kept.
    pub fn set_occupancy_limits(&mut self, lower: u64, upper: u64) -> Result<(), Error> {
        self.occupancy.set_limits(lower, upper)
    }

    /// Occupancy_State as served: DISABLED while counting is off, else the
    /// served count against the limits (Clause 12.32.6).
    pub fn occupancy_state(&self) -> AccessZoneOccupancyState {
        self.occupancy.state(self.state.occupancy_count)
    }

    /// The value the event algorithm watches: Occupancy_State, raw.
    fn watched_state(&self) -> u32 {
        self.occupancy_state().to_raw()
    }

    /// The Reliability served, which the algorithm's fault check reads.
    fn served_reliability(&self) -> Reliability {
        self.state.reliability
    }

    /// A client's Adjust_Value write (Clause 12.32.13): an Integer, else
    /// INVALID_DATA_TYPE. With counting on, the value is kept and moves the
    /// count served, unless Out_Of_Service is TRUE (12.32.10); with counting
    /// off, Adjust_Value stays zero.
    fn write_adjust_value(&mut self, value: &PropertyValue) -> Result<(), Error> {
        let PropertyValue::Signed(adjust) = *value else {
            return Err(common::invalid_data_type_error());
        };
        if !self.occupancy.enabled {
            return Ok(());
        }
        self.occupancy.adjust_value = adjust;
        if !self.out_of_service {
            self.state.occupancy_count = adjusted(self.state.occupancy_count, adjust);
        }
        Ok(())
    }

    /// The zone's own values: the ones put aside while out of service, else
    /// the ones served.
    fn device_state_mut(&mut self) -> &mut ZoneState {
        self.device_state.as_mut().unwrap_or(&mut self.state)
    }
}

impl BACnetObject for AccessZoneObject {
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
        if let Some(result) = crate::common::read_identity_properties!(self, property, array_index)
        {
            return result;
        }
        if let Some(result) = self.reporting.read(property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ACCESS_ZONE.to_raw()))
            }
            // OVERRIDDEN stays clear (Clause 12.32.7).
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(common::compute_status_flags(
                StatusFlags::empty(),
                self.state.reliability,
                self.out_of_service,
                self.reporting.event_state(),
            )),
            p if p == PropertyIdentifier::OUT_OF_SERVICE => {
                Ok(PropertyValue::Boolean(self.out_of_service))
            }
            p if p == PropertyIdentifier::RELIABILITY => {
                Ok(PropertyValue::Enumerated(self.state.reliability.to_raw()))
            }
            p if p == PropertyIdentifier::GLOBAL_IDENTIFIER => {
                Ok(PropertyValue::Unsigned(self.global_identifier))
            }
            p if p == PropertyIdentifier::OCCUPANCY_COUNT => {
                Ok(PropertyValue::Unsigned(self.state.occupancy_count))
            }
            p if p == PropertyIdentifier::OCCUPANCY_STATE => {
                Ok(PropertyValue::Enumerated(self.occupancy_state().to_raw()))
            }
            p if p == PropertyIdentifier::OCCUPANCY_COUNT_ENABLE => {
                Ok(PropertyValue::Boolean(self.occupancy.enabled))
            }
            p if p == PropertyIdentifier::ADJUST_VALUE => {
                Ok(PropertyValue::Signed(self.occupancy.adjust_value))
            }
            p if p == PropertyIdentifier::OCCUPANCY_UPPER_LIMIT => {
                Ok(PropertyValue::Unsigned(self.occupancy.upper_limit))
            }
            p if p == PropertyIdentifier::OCCUPANCY_LOWER_LIMIT => {
                Ok(PropertyValue::Unsigned(self.occupancy.lower_limit))
            }
            p if p == PropertyIdentifier::ENTRY_POINTS => {
                Ok(crate::device_reference::reference_list(&self.entry_points))
            }
            p if p == PropertyIdentifier::EXIT_POINTS => {
                Ok(crate::device_reference::reference_list(&self.exit_points))
            }
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
        // The zone starts in service and only this write moves Out_Of_Service,
        // so the entry edge is always seen and there is no fallback.
        if let Some(result) = common::write_out_of_service_with_restore(
            &mut self.out_of_service,
            &mut self.state,
            &mut self.device_state,
            None,
            property,
            &value,
        ) {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if let Some(result) = self.state.write(
            self.out_of_service,
            self.occupancy.enabled,
            property,
            &value,
        ) {
            return result;
        }
        if property == PropertyIdentifier::ADJUST_VALUE {
            return self.write_adjust_value(&value);
        }
        if let Some(result) = self.reporting.write(property, array_index, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::GLOBAL_IDENTIFIER => {
                if let PropertyValue::Unsigned(v) = value {
                    self.global_identifier = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
                array_index,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        super::metadata_topology::for_access_zone_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    crate::event::impl_change_of_state_reporting!(
        reporting,
        Self::watched_state,
        Self::served_reliability
    );

    /// The Reliability the zone's fault detection works out. Like the other
    /// Reliability carriers, the zone refuses it with WRITE_ACCESS_DENIED
    /// while Out_Of_Service is TRUE, so a client's simulated value stays
    /// served; the return to service brings back the value from before. A
    /// value outside the BACnetReliability production is refused with
    /// VALUE_OUT_OF_RANGE.
    fn set_reliability_internal(&mut self, reliability: Reliability) -> Result<(), Error> {
        if self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        self.state.reliability = checked_reliability(reliability)?;
        Ok(())
    }
}

/// The states Alarm_Values can hold: a BACnetAccessZoneOccupancyState other
/// than NORMAL.
const ALARM_STATES: ListedStates = ListedStates {
    normal: AccessZoneOccupancyState::NORMAL.to_raw(),
    in_range: occupancy_state_in_range,
};

/// Whether `raw` is a BACnetAccessZoneOccupancyState: a named state or a
/// proprietary one from 64 to 65535 (Clause 23.1).
fn occupancy_state_in_range(raw: u32) -> bool {
    AccessZoneOccupancyState::ALL_NAMED
        .iter()
        .any(|&(_, named)| named.to_raw() == raw)
        || (64..=65_535).contains(&raw)
}

// ---------------------------------------------------------------------------
