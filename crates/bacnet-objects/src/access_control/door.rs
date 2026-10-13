use std::sync::Arc;
use std::time::Duration;

use super::door_alarm::{DoorAlarmLists, LISTED_STATES};
use super::door_out_of_service::DoorState;
use super::*;
use crate::event::state_reporting::ChangeOfStateReporting;
use crate::traits::MonotonicClock;

// AccessDoorObject (type 30)
// ---------------------------------------------------------------------------

/// Door_Pulse_Time a new door starts with, in tenths of a second (5 s).
pub const DEFAULT_DOOR_PULSE_TIME: u32 = 50;
/// Door_Extended_Pulse_Time a new door starts with, in tenths of a second
/// (15 s).
pub const DEFAULT_DOOR_EXTENDED_PULSE_TIME: u32 = 150;
/// Door_Open_Too_Long_Time a new door starts with, in tenths of a second
/// (30 s).
pub const DEFAULT_DOOR_OPEN_TOO_LONG_TIME: u32 = 300;

/// BACnet Access Door object (type 30).
///
/// Represents a physical door or barrier in an access control system.
/// Present value carries the door command (BACnetDoorValue); Door_Status
/// reports the physical DoorStatus.
///
/// A PULSE_UNLOCK or EXTENDED_PULSE_UNLOCK command holds its priority slot
/// for Door_Pulse_Time or Door_Extended_Pulse_Time and is then relinquished,
/// so the door relocks (Clauses 12.26.4, 12.26.16 and 12.26.17; #1073). The
/// deadline is taken from the database's monotonic clock when the command is
/// written, and the server's monotonic operation task relinquishes the slot
/// at that deadline, the mechanism Binary Lighting Output egress uses. A
/// pulse written below a priority already commanded is relinquished at once,
/// and a pulse time of zero relinquishes at once too. Changing a pulse time
/// leaves an armed deadline as it was.
///
/// While Out_Of_Service is TRUE a client can simulate Door_Status,
/// Lock_Status and Door_Alarm_State by writing them, and the door's own values
/// come back on the return to service (Clause 12.26.9, Table 12-30 footnote 1;
/// #1131). The module `door_out_of_service` has the details.
///
/// Secured_Status isn't stored: each read works it out from what the door
/// serves (Clause 12.26.14; #1148), so it moves with every command, relock,
/// device report and simulated value. See `secured_status` for how the
/// inputs combine and when the answer is UNKNOWN.
///
/// The door reports intrinsically on Door_Alarm_State with the
/// CHANGE_OF_STATE algorithm (Clause 12.26; #1149): Event_State goes
/// OFFNORMAL once Door_Alarm_State has stayed in Alarm_Values for Time_Delay
/// seconds and back to NORMAL once it has stayed out of them for
/// Time_Delay_Normal. Fault_Values feeds the FAULT_STATE algorithm, so a
/// Door_Alarm_State among them makes Reliability MULTI_STATE_FAULT and
/// Event_State FAULT. Masked_Alarm_Values keeps Door_Alarm_State out of the
/// states it lists. The module `door_alarm` has the rules the three lists
/// set; the event rows are served and written through
/// `ChangeOfStateReporting`, and the server sends the notifications to the
/// Notification Class recipients.
///
/// The application decides when the door is in alarm and reports it with
/// [`Self::set_door_alarm_state`]; Clause 12.26.20 leaves that to the
/// device. That includes DOOR_OPEN_TOO_LONG: the door serves
/// Door_Open_Too_Long_Time for the application's logic to read but runs no
/// timer of its own.
///
/// While the server holds the door, the application reports its
/// Door_Status, Lock_Status and Door_Alarm_State through
/// `BACnetServer::report_door_state_local` (#1132), which stores them as the
/// setters do, out of service included.
#[derive(Clone)]
pub struct AccessDoorObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: DoorValue,
    /// Door_Status, Lock_Status and Door_Alarm_State as served: the device's,
    /// or a client's simulation while Out_Of_Service is TRUE.
    state: DoorState,
    /// The device's own door state, put aside while Out_Of_Service is TRUE.
    device_state: Option<DoorState>,
    door_members: Vec<BACnetDeviceObjectReference>,
    status_flags: StatusFlags,
    out_of_service: bool,
    /// Fault_Values and Masked_Alarm_Values.
    alarm_lists: DoorAlarmLists,
    /// Intrinsic reporting on Door_Alarm_State, Alarm_Values included.
    reporting: ChangeOfStateReporting,
    /// 16-level priority array for commandable Present_Value.
    priority_array: [Option<DoorValue>; 16],
    relinquish_default: DoorValue,
    /// Door_Pulse_Time, tenths of a second.
    door_pulse_time: u32,
    /// Door_Extended_Pulse_Time, tenths of a second.
    door_extended_pulse_time: u32,
    /// Door_Open_Too_Long_Time, tenths of a second. Stored and served for
    /// the application, which decides when the door has been open too long.
    door_open_too_long_time: u32,
    /// For each priority slot holding a pulse, the monotonic instant it is
    /// relinquished.
    pulse_deadlines: [Option<Duration>; 16],
    monotonic_clock: Option<Arc<MonotonicClock>>,
    /// Elapsed time for a door with no bound clock, advanced by
    /// `advance_time_internal`.
    logical_now: Duration,
}

impl AccessDoorObject {
    /// Create a new Access Door object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_DOOR, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: DoorValue::LOCK,
            state: DoorState::SECURE,
            device_state: None,
            door_members: Vec::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            alarm_lists: DoorAlarmLists::default(),
            reporting: ChangeOfStateReporting::new(LISTED_STATES),
            priority_array: Default::default(),
            relinquish_default: DoorValue::LOCK,
            door_pulse_time: DEFAULT_DOOR_PULSE_TIME,
            door_extended_pulse_time: DEFAULT_DOOR_EXTENDED_PULSE_TIME,
            door_open_too_long_time: DEFAULT_DOOR_OPEN_TOO_LONG_TIME,
            pulse_deadlines: [None; 16],
            monotonic_clock: None,
            logical_now: Duration::ZERO,
        })
    }

    /// Set the Relinquish_Default (#270).
    ///
    /// Table 12-30 types it as BACnetDoorValue, but Clause 12.26.11 admits
    /// only LOCK and UNLOCK: a pulse can't be the resting command. Any other
    /// value, including a `DoorValue::from_raw` outside the production, is
    /// refused with VALUE_OUT_OF_RANGE and the stored default kept. After the
    /// store, Present_Value is resolved anew from the priority array so an
    /// empty array falls back to the new default immediately.
    pub fn set_relinquish_default(&mut self, value: DoorValue) -> Result<(), Error> {
        if !matches!(value, DoorValue::LOCK | DoorValue::UNLOCK) {
            return Err(common::value_out_of_range_error());
        }
        self.relinquish_default = value;
        self.recalculate_present_value();
        Ok(())
    }

    /// Set Door_Members, the objects that make up the physical door: its
    /// lock, contact, reader and anything else the application chooses
    /// (Clause 12.26.15 leaves the choice to the device). A reference with no
    /// Device identifier names an object in this device. The array is
    /// read-only over the network.
    ///
    /// A reference whose device identifier isn't a Device object is refused
    /// with VALUE_OUT_OF_RANGE and the members set before are kept (#1285).
    pub fn set_door_members(
        &mut self,
        members: impl IntoIterator<Item = impl Into<BACnetDeviceObjectReference>>,
    ) -> Result<(), Error> {
        let members: Vec<BACnetDeviceObjectReference> =
            members.into_iter().map(Into::into).collect();
        crate::device_reference::check_device_members(&members)?;
        self.door_members = members;
        Ok(())
    }

    /// Set Door_Pulse_Time, in tenths of a second.
    pub fn set_door_pulse_time(&mut self, tenths: u32) {
        self.door_pulse_time = tenths;
    }

    /// Set Door_Extended_Pulse_Time, in tenths of a second.
    pub fn set_door_extended_pulse_time(&mut self, tenths: u32) {
        self.door_extended_pulse_time = tenths;
    }

    /// Set Door_Open_Too_Long_Time, in tenths of a second.
    pub fn set_door_open_too_long_time(&mut self, tenths: u32) {
        self.door_open_too_long_time = tenths;
    }

    /// Set Door_Status, the open or closed state the door's contact reports.
    ///
    /// While Out_Of_Service is TRUE a client's simulated value keeps being
    /// served, and this one takes over on the return to service.
    pub fn set_door_status(&mut self, status: DoorStatus) {
        self.device_state_mut().door_status = status;
    }

    /// Set Lock_Status, the state the door lock's monitor reports.
    ///
    /// While Out_Of_Service is TRUE a client's simulated value keeps being
    /// served, and this one takes over on the return to service.
    pub fn set_lock_status(&mut self, status: LockStatus) {
        self.device_state_mut().lock_status = status;
    }

    /// Set Door_Alarm_State, the alarm condition the application's door
    /// logic has worked out (Clause 12.26.20 leaves that to the device),
    /// DOOR_OPEN_TOO_LONG included.
    ///
    /// The state has to be NORMAL or a member of Alarm_Values or
    /// Fault_Values, and no member of Masked_Alarm_Values; any other is
    /// refused with VALUE_OUT_OF_RANGE and the state held is kept. A change
    /// of the value served triggers a SubscribeCOV notification (Table 13-1)
    /// and feeds the event algorithm, which a running server evaluates at its
    /// next one-second tick. While Out_Of_Service is TRUE a client's
    /// simulated value keeps being served, and this one takes over on the
    /// return to service.
    pub fn set_door_alarm_state(&mut self, state: DoorAlarmState) -> Result<(), Error> {
        if !self.admits(state) {
            return Err(common::value_out_of_range_error());
        }
        self.device_state_mut().door_alarm_state = state;
        Ok(())
    }

    /// Set Alarm_Values, the Door_Alarm_State values the door reports as
    /// offnormal (Clause 12.26.25). NORMAL, or a value outside the
    /// BACnetDoorAlarmState production, named or proprietary (256 to
    /// 65535), is refused with VALUE_OUT_OF_RANGE and the values set before
    /// are kept. A Door_Alarm_State the lists no longer admit drops to
    /// NORMAL. Clients can write the list too, as they can the door's other
    /// event configuration.
    pub fn set_alarm_values(
        &mut self,
        states: impl IntoIterator<Item = DoorAlarmState>,
    ) -> Result<(), Error> {
        self.reporting.set_alarm_values(raw_states(states))?;
        self.settle_door_alarm_state();
        Ok(())
    }

    /// Set Fault_Values, the Door_Alarm_State values that make Reliability
    /// MULTI_STATE_FAULT (Clause 12.26.26), checked as
    /// [`Self::set_alarm_values`] checks its list.
    pub fn set_fault_values(
        &mut self,
        states: impl IntoIterator<Item = DoorAlarmState>,
    ) -> Result<(), Error> {
        self.alarm_lists.set_fault_values(raw_states(states))?;
        self.settle_door_alarm_state();
        Ok(())
    }

    /// Set Masked_Alarm_Values, the states Door_Alarm_State is kept out of
    /// (Clause 12.26.21), checked as [`Self::set_alarm_values`] checks its
    /// list. A door in a state the list now masks returns to NORMAL at once.
    pub fn set_masked_alarm_values(
        &mut self,
        states: impl IntoIterator<Item = DoorAlarmState>,
    ) -> Result<(), Error> {
        self.alarm_lists
            .set_masked_alarm_values(raw_states(states))?;
        self.settle_door_alarm_state();
        Ok(())
    }

    /// Whether the door's lists let Door_Alarm_State take `state`.
    fn admits(&self, state: DoorAlarmState) -> bool {
        self.alarm_lists
            .admits(self.reporting.alarm_values(), state)
    }

    /// Send a Door_Alarm_State the lists no longer admit back to NORMAL,
    /// both the one served and the device's own one put aside out of
    /// service.
    fn settle_door_alarm_state(&mut self) {
        let (lists, alarm_values) = (&self.alarm_lists, self.reporting.alarm_values());
        for state in std::iter::once(&mut self.state).chain(self.device_state.as_mut()) {
            if !lists.admits(alarm_values, state.door_alarm_state) {
                state.door_alarm_state = DoorAlarmState::NORMAL;
            }
        }
    }

    /// The value the event algorithm watches: Door_Alarm_State, raw.
    fn watched_alarm_state(&self) -> u32 {
        self.state.door_alarm_state.to_raw()
    }

    /// Reliability as served: a client's simulated value while out of
    /// service, else what the FAULT_STATE algorithm makes of the
    /// Door_Alarm_State served.
    fn served_reliability(&self) -> Reliability {
        self.state
            .reliability
            .unwrap_or_else(|| self.alarm_lists.fault_state(self.state.door_alarm_state))
    }

    /// The device's own door state: the one put aside while out of service,
    /// else the one served.
    fn device_state_mut(&mut self) -> &mut DoorState {
        self.device_state.as_mut().unwrap_or(&mut self.state)
    }

    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
    }

    fn monotonic_now(&self) -> Duration {
        self.monotonic_clock
            .as_ref()
            .map_or(self.logical_now, |clock| clock())
    }

    /// Store a command (or a NULL relinquish) at `priority`, arming the
    /// relock deadline for a pulse.
    fn command(&mut self, priority: u8, value: Option<DoorValue>) {
        let index = usize::from(priority - 1);
        self.pulse_deadlines[index] = None;
        let pulse = match value {
            Some(DoorValue::PULSE_UNLOCK) => Some(self.door_pulse_time),
            Some(DoorValue::EXTENDED_PULSE_UNLOCK) => Some(self.door_extended_pulse_time),
            _ => None,
        };
        self.priority_array[index] = match pulse {
            // A pulse below a live command, or of zero length, is
            // relinquished as soon as it is written.
            Some(tenths)
                if tenths == 0 || self.priority_array[..index].iter().any(Option::is_some) =>
            {
                None
            }
            Some(tenths) => {
                let length = Duration::from_millis(u64::from(tenths) * 100);
                self.pulse_deadlines[index] = Some(self.monotonic_now().saturating_add(length));
                value
            }
            None => value,
        };
        self.recalculate_present_value();
    }

    /// Relinquish every pulse whose deadline has passed; `true` when one was.
    fn expire_pulses(&mut self, now: Duration) -> bool {
        let mut expired = false;
        for (slot, deadline) in self
            .priority_array
            .iter_mut()
            .zip(&mut self.pulse_deadlines)
        {
            if deadline.is_some_and(|deadline| now >= deadline) {
                *slot = None;
                *deadline = None;
                expired = true;
            }
        }
        if expired {
            self.recalculate_present_value();
        }
        expired
    }

    /// Whether Status_Flags carries IN_ALARM, which follows Event_State
    /// (Clause 12.26.6). Status_Flags and Secured_Status both read it here,
    /// so the two agree.
    fn in_alarm(&self) -> bool {
        self.reporting.event_state() != EventState::NORMAL
    }

    /// Secured_Status, worked out from what the door serves (Clause
    /// 12.26.14).
    ///
    /// Each input comes out met, failed or undetermined. One failed input
    /// makes the door UNSECURED whatever the rest say: a door commanded to
    /// UNLOCK isn't secure even while its contact can't tell whether it is
    /// shut. With none failed, one undetermined input makes it UNKNOWN, and
    /// only a door with every input met is SECURED.
    ///
    /// An input is undetermined only when the door's own monitor says it
    /// can't tell: Door_Status or Lock_Status reads UNKNOWN, or reports its
    /// input faulted (DOOR_FAULT, LOCK_FAULT). Reliability is no input of its
    /// own: a fault moves Event_State to FAULT, which sets IN_ALARM and so
    /// fails the first input. Door_Status and Lock_Status are the served
    /// values, so while Out_Of_Service is TRUE a client's simulation moves
    /// the result as a device report would (Clause 12.26.9).
    fn secured_status(&self) -> DoorSecuredStatus {
        let inputs = [
            Some(!self.in_alarm()),
            // Any masked state fails the door, whichever it is.
            Some(!self.alarm_lists.masks_any()),
            Some(self.present_value == DoorValue::LOCK),
            door_closed(self.state.door_status),
            door_locked(self.state.lock_status),
        ];
        if inputs.contains(&Some(false)) {
            DoorSecuredStatus::UNSECURED
        } else if inputs.contains(&None) {
            DoorSecuredStatus::UNKNOWN
        } else {
            DoorSecuredStatus::SECURED
        }
    }
}

/// Door_Status as a Secured_Status input: met for a shut door or one with no
/// contact fitted (CLOSED, UNUSED), undetermined (`None`) while the contact
/// can't tell or is faulted (UNKNOWN, DOOR_FAULT), and failed for any other
/// value, a proprietary one included.
fn door_closed(status: DoorStatus) -> Option<bool> {
    match status {
        DoorStatus::CLOSED | DoorStatus::UNUSED => Some(true),
        DoorStatus::UNKNOWN | DoorStatus::DOOR_FAULT => None,
        _ => Some(false),
    }
}

/// Lock_Status as a Secured_Status input, read the same way: met for LOCKED
/// or UNUSED, undetermined for UNKNOWN or LOCK_FAULT, failed for UNLOCKED.
fn door_locked(status: LockStatus) -> Option<bool> {
    match status {
        LockStatus::LOCKED | LockStatus::UNUSED => Some(true),
        LockStatus::UNKNOWN | LockStatus::LOCK_FAULT => None,
        _ => Some(false),
    }
}

/// Whether `value` is one of the four BACnetDoorValue members. The
/// bacnet-types test `door_value_values_match_clause_21` pins the
/// production's closed-set length, so this bound cannot drift from the enum.
fn is_door_value(value: DoorValue) -> bool {
    value.to_raw() <= DoorValue::EXTENDED_PULSE_UNLOCK.to_raw()
}

/// Decode a BACnetDoorValue write: an Enumerated within the closed set.
fn checked_door_value(value: PropertyValue) -> Result<DoorValue, Error> {
    let PropertyValue::Enumerated(raw) = value else {
        return Err(common::invalid_data_type_error());
    };
    let value = DoorValue::from_raw(raw);
    if !is_door_value(value) {
        return Err(common::value_out_of_range_error());
    }
    Ok(value)
}

/// Decode a door time write: an Unsigned of at most 32 bits.
fn checked_tenths(value: PropertyValue) -> Result<u32, Error> {
    let PropertyValue::Unsigned(raw) = value else {
        return Err(common::invalid_data_type_error());
    };
    common::u64_to_u32(raw)
}

impl BACnetObject for AccessDoorObject {
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
        if let Some(result) = common::read_identity_properties!(self, property, array_index) {
            return result;
        }
        if let Some(result) = self.reporting.read(property, array_index) {
            return result;
        }
        if let Some(value) = self.alarm_lists.read(property) {
            return Ok(value);
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ACCESS_DOOR.to_raw()))
            }
            // IN_ALARM comes from the door's Event_State, the flag
            // Secured_Status reads, and FAULT from the Reliability served.
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(common::compute_status_flags(
                self.status_flags,
                self.served_reliability(),
                self.out_of_service,
                self.reporting.event_state(),
            )),
            p if p == PropertyIdentifier::OUT_OF_SERVICE => {
                Ok(PropertyValue::Boolean(self.out_of_service))
            }
            p if p == PropertyIdentifier::RELIABILITY => Ok(PropertyValue::Enumerated(
                self.served_reliability().to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value.to_raw()))
            }
            p if p == PropertyIdentifier::DOOR_STATUS => {
                Ok(PropertyValue::Enumerated(self.state.door_status.to_raw()))
            }
            p if p == PropertyIdentifier::LOCK_STATUS => {
                Ok(PropertyValue::Enumerated(self.state.lock_status.to_raw()))
            }
            p if p == PropertyIdentifier::SECURED_STATUS => {
                Ok(PropertyValue::Enumerated(self.secured_status().to_raw()))
            }
            p if p == PropertyIdentifier::DOOR_ALARM_STATE => Ok(PropertyValue::Enumerated(
                self.state.door_alarm_state.to_raw(),
            )),
            p if p == PropertyIdentifier::DOOR_MEMBERS => common::read_array(
                crate::device_reference::reference_elements(&self.door_members),
                array_index,
            ),
            p if p == PropertyIdentifier::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, |v: DoorValue| {
                    PropertyValue::Enumerated(v.to_raw())
                })
            }
            p if p == PropertyIdentifier::RELINQUISH_DEFAULT => {
                Ok(PropertyValue::Enumerated(self.relinquish_default.to_raw()))
            }
            p if p == PropertyIdentifier::DOOR_PULSE_TIME => {
                Ok(PropertyValue::Unsigned(self.door_pulse_time.into()))
            }
            p if p == PropertyIdentifier::DOOR_EXTENDED_PULSE_TIME => Ok(PropertyValue::Unsigned(
                self.door_extended_pulse_time.into(),
            )),
            p if p == PropertyIdentifier::DOOR_OPEN_TOO_LONG_TIME => {
                Ok(PropertyValue::Unsigned(self.door_open_too_long_time.into()))
            }
            // NULL while Present_Value comes from Relinquish_Default
            // (Clause 12.26.39).
            p if p == PropertyIdentifier::CURRENT_COMMAND_PRIORITY => {
                Ok(common::current_command_priority(&self.priority_array))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        // The door's entry edge is always seen (it starts in service and only
        // this write moves Out_Of_Service), so there is no fallback state.
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
        let (lists, alarm_values) = (&self.alarm_lists, self.reporting.alarm_values());
        if let Some(result) = self.state.write(
            self.out_of_service,
            |state| lists.admits(alarm_values, state),
            property,
            &value,
        ) {
            return result;
        }
        // An edit of any of the three lists can leave Door_Alarm_State in a
        // state they no longer admit.
        let list_edit = match property {
            p if p == PropertyIdentifier::ALARM_VALUES => {
                self.reporting.write(property, array_index, &value)
            }
            _ => self.alarm_lists.write(property, array_index, &value),
        };
        if let Some(result) = list_edit {
            result?;
            self.settle_door_alarm_state();
            return Ok(());
        }
        if let Some(result) = self.reporting.write(property, array_index, &value) {
            return result;
        }
        match property {
            // A command (or a NULL relinquish) at the write priority, 16 when
            // absent; Present_Value is then resolved from the priority array.
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                let priority = priority.unwrap_or(16);
                if !(1..=16).contains(&priority) {
                    return Err(common::value_out_of_range_error());
                }
                let value = match value {
                    PropertyValue::Null => None,
                    other => Some(checked_door_value(other)?),
                };
                self.command(priority, value);
                Ok(())
            }
            // Table 12-30 carries Relinquish_Default R (BACnetDoorValue) for
            // the commandable Access Door; the standard permits writability.
            p if p == PropertyIdentifier::RELINQUISH_DEFAULT => {
                self.set_relinquish_default(checked_door_value(value)?)
            }
            p if p == PropertyIdentifier::DOOR_PULSE_TIME => {
                self.door_pulse_time = checked_tenths(value)?;
                Ok(())
            }
            p if p == PropertyIdentifier::DOOR_EXTENDED_PULSE_TIME => {
                self.door_extended_pulse_time = checked_tenths(value)?;
                Ok(())
            }
            p if p == PropertyIdentifier::DOOR_OPEN_TOO_LONG_TIME => {
                self.door_open_too_long_time = checked_tenths(value)?;
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
        super::metadata_topology::for_access_door_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn supports_cov(&self) -> bool {
        true
    }

    crate::event::impl_change_of_state_reporting!(
        reporting,
        Self::watched_alarm_state,
        Self::served_reliability
    );

    fn advance_time_internal(&mut self, elapsed: Duration) -> bool {
        self.logical_now = self.logical_now.saturating_add(elapsed);
        self.expire_pulses(self.logical_now)
    }

    fn bind_monotonic_clock_internal(&mut self, clock: Option<Arc<MonotonicClock>>) {
        self.monotonic_clock = clock;
    }

    fn advance_monotonic_time_internal(&mut self, now: Duration) -> bool {
        self.expire_pulses(now)
    }

    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        self.pulse_deadlines.iter().flatten().min().copied()
    }

    fn cov_snapshot_internal(&self) -> Option<Box<dyn BACnetObject>> {
        Some(Box::new(self.clone()))
    }

    /// Take what the door's hardware reports (#1132): Door_Status,
    /// Lock_Status and Door_Alarm_State, each one given, together. A value
    /// outside its production, or an alarm state the door's lists don't
    /// admit, is VALUE_OUT_OF_RANGE and changes nothing. Stored as the
    /// setters store the device's values: while Out_Of_Service is TRUE the
    /// report replaces the values put aside, and a client's simulated ones
    /// stay served until the return to service. Any other record is
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
    fn report_access_input_internal(&mut self, input: AccessControlInput) -> Result<(), Error> {
        let AccessControlInput::DoorState(report) = input else {
            return Err(common::optional_functionality_not_supported_error());
        };
        let (lists, alarm_values) = (&self.alarm_lists, self.reporting.alarm_values());
        // The device's own state: put aside out of service, else served.
        let device = self.device_state.as_mut().unwrap_or(&mut self.state);
        device.report(report, |state| lists.admits(alarm_values, state))
    }
}

/// The raw values of `states`, for the list setters.
fn raw_states(states: impl IntoIterator<Item = DoorAlarmState>) -> Vec<u32> {
    states.into_iter().map(DoorAlarmState::to_raw).collect()
}

#[cfg(test)]
#[path = "door_secured_status_tests.rs"]
mod secured_status_tests;

// ---------------------------------------------------------------------------
