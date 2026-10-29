//! The lists that govern an Access Door's Door_Alarm_State (Clauses
//! 12.26.20, 12.26.21, 12.26.25 and 12.26.26, Table 12-30, #1149).
//!
//! Three BACnetLISTs of BACnetDoorAlarmState shape the value:
//!
//! - Alarm_Values, the CHANGE_OF_STATE algorithm's alarm values, which the
//!   door's `ChangeOfStateReporting` holds with the other event rows;
//! - Fault_Values, the FAULT_STATE algorithm's fault values, held here;
//! - Masked_Alarm_Values, the states the door is kept out of, held here.
//!
//! Door_Alarm_State itself stays NORMAL or one of the alarm or fault values,
//! and never takes a masked state. The door applies that one check to every
//! route that sets the value (the application's `set_door_alarm_state` and a
//! client's simulated write while Out_Of_Service is TRUE) and refuses any
//! other state with VALUE_OUT_OF_RANGE. When a list changes so that the
//! value held no longer passes, the value drops back to NORMAL at once: the
//! standard asks this of a state written into Masked_Alarm_Values, and the
//! door treats a state taken out of Alarm_Values or Fault_Values the same
//! way, since it is then no alarm or fault the door recognises.
//!
//! The FAULT_STATE algorithm (Clause 13.4.5) is what Fault_Values feeds: a
//! Door_Alarm_State among them makes Reliability MULTI_STATE_FAULT, and any
//! other one NO_FAULT_DETECTED. The door has no other reliability source, so
//! nothing takes precedence over the check, and it works Reliability out on
//! each read rather than storing it. Moving from one fault value to another
//! stays MULTI_STATE_FAULT without a new transition; the optional
//! re-indication of Clause 13.4.5 is left out, as Clause 13.3.2's is for
//! alarm values.
//!
//! Each list takes WriteProperty, WritePropertyMultiple and the list services
//! in the form the door's Alarm_Values does (`state_reporting::
//! enumerated_list`): named states or proprietary ones from 256 to 65535,
//! up to `MAX_ALARM_VALUES` of them. None of the three takes NORMAL, which
//! is refused with VALUE_OUT_OF_RANGE naming the element (`state_reporting::
//! ListedStates`, which the Access Zone's Alarm_Values shares): NORMAL is
//! the state with no alarm or fault, so as an alarm value it would put a
//! quiet door in alarm, as a fault value it would fault one, and masked it
//! would leave Door_Alarm_State nothing to hold.

use crate::prelude::*;
use bacnet_types::enums::{DoorAlarmState, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::door_out_of_service::door_alarm_state_in_range;
use crate::event::state_reporting::{
    ListedStates, checked_raw_list, enumerated_list, enumerated_list_value,
};

/// Fault_Values and Masked_Alarm_Values, raw.
#[derive(Debug, Clone, Default)]
pub(super) struct DoorAlarmLists {
    fault_values: Vec<u32>,
    masked_alarm_values: Vec<u32>,
}

impl DoorAlarmLists {
    /// Whether the door may hold `state` while Alarm_Values is
    /// `alarm_values`: NORMAL or a member of either value list, and no
    /// masked state.
    pub(super) fn admits(&self, alarm_values: &[u32], state: DoorAlarmState) -> bool {
        let raw = state.to_raw();
        !self.masked_alarm_values.contains(&raw)
            && (state == DoorAlarmState::NORMAL
                || alarm_values.contains(&raw)
                || self.fault_values.contains(&raw))
    }

    /// The Reliability the FAULT_STATE algorithm gives `state`.
    pub(super) fn fault_state(&self, state: DoorAlarmState) -> Reliability {
        if self.fault_values.contains(&state.to_raw()) {
            Reliability::MULTI_STATE_FAULT
        } else {
            Reliability::NO_FAULT_DETECTED
        }
    }

    /// Whether Masked_Alarm_Values holds any state.
    pub(super) fn masks_any(&self) -> bool {
        !self.masked_alarm_values.is_empty()
    }

    /// Replace Fault_Values, checked as a network write would be.
    pub(super) fn set_fault_values(&mut self, values: Vec<u32>) -> Result<(), Error> {
        self.fault_values = checked_raw_list(values, LISTED_STATES)?;
        Ok(())
    }

    /// Replace Masked_Alarm_Values, checked as a network write would be.
    pub(super) fn set_masked_alarm_values(&mut self, values: Vec<u32>) -> Result<(), Error> {
        self.masked_alarm_values = checked_raw_list(values, LISTED_STATES)?;
        Ok(())
    }

    /// Serve Fault_Values or Masked_Alarm_Values; `None` for any other
    /// property.
    pub(super) fn read(&self, property: PropertyIdentifier) -> Option<PropertyValue> {
        match property {
            p if p == PropertyIdentifier::FAULT_VALUES => {
                Some(enumerated_list_value(&self.fault_values))
            }
            p if p == PropertyIdentifier::MASKED_ALARM_VALUES => {
                Some(enumerated_list_value(&self.masked_alarm_values))
            }
            _ => None,
        }
    }

    /// Take a network write of Fault_Values or Masked_Alarm_Values; `None`
    /// for any other property. A refused list leaves the one held.
    pub(super) fn write(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        let list = match property {
            p if p == PropertyIdentifier::FAULT_VALUES => &mut self.fault_values,
            p if p == PropertyIdentifier::MASKED_ALARM_VALUES => &mut self.masked_alarm_values,
            _ => return None,
        };
        Some(enumerated_list(array_index, value, LISTED_STATES).map(|values| *list = values))
    }
}

/// The states each of the three lists can hold: a BACnetDoorAlarmState,
/// named or proprietary, other than NORMAL.
pub(super) const LISTED_STATES: ListedStates = ListedStates {
    normal: DoorAlarmState::NORMAL.to_raw(),
    in_range: |raw| door_alarm_state_in_range(DoorAlarmState::from_raw(raw)),
};
