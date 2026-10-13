use std::sync::Arc;

use bacnet_encoding::constructed::encode_authentication_factor;
use bacnet_types::constructed::{BACnetAuthenticationFactor, BACnetAuthenticationPolicy};
use bacnet_types::enums::AuthorizationMode;

use super::credential_data_input_formats::{is_factor_type, undefined_factor};
use super::point_authorization::Authorization;
use super::point_event_time::next_event_time;
use super::*;
use crate::clock::ClockReader;

// AccessPointObject (type 33)
// ---------------------------------------------------------------------------

/// The most authentication policies an Access Point holds while it serves
/// Authentication_Policy_List and Authentication_Policy_Names, a resource
/// cap: a count or list past it is VALUE_OUT_OF_RANGE.
pub const MAX_AUTHENTICATION_POLICIES: u32 = 256;

/// BACnet Access Point object (type 33).
///
/// Represents an access point (reader/controller at a door) in an access control system.
/// The most recent access event is Access_Event; Table 12-36 has no
/// Present_Value row, so the object serves none (#1064).
///
/// Each change of Out_Of_Service is an access event of the point's own
/// (Clause 12.31.8; #1248): entering out of service records OUT_OF_SERVICE
/// and the return to service OUT_OF_SERVICE_RELINQUISHED. Recording one
/// follows Clause 12.31.27.1, which moves Access_Event_Tag on when an event
/// starts a new access transaction and stamps Access_Event_Time with the
/// current date and time. The introduction to Clause 12.31 counts an
/// operator action as an access transaction, so each change of
/// Out_Of_Service starts a new one: the tag moves on by one (wrapping at the
/// top of the Unsigned range), and the time comes from the Device clock. A
/// write that leaves Out_Of_Service as it was, NULL included, records
/// nothing. No credential or factor belongs to an edge, so each one stores
/// the no-credential reference in Access_Event_Credential (Clause 12.31.30)
/// and the UNDEFINED factor in Access_Event_Authentication_Factor (Clause
/// 12.31.31).
///
/// The application's own access events reach a point the server holds
/// through `BACnetServer::report_access_event_local` (#1132), which refuses
/// them while Out_Of_Service is TRUE: the point then performs no
/// authentication or authorization (Clause 12.31.8), so it has no events of
/// its own to report.
///
/// Authentication_Status is the status the application reports for its
/// authentication process (Clause 12.31.9; #1284), READY until it reports
/// another. While Out_Of_Service is TRUE no authentication runs, so the
/// property reads DISABLED; the reported status is kept and served again on
/// the return to service.
///
/// Each event the point stamps itself, an edge or a reported event with no
/// time, gets an Access_Event_Time strictly after the one served before it,
/// so every event moves the time (Clause 12.31.29), several events of one
/// transaction included: the clock's time, or one hundredth past the time
/// served when the clock isn't later, and with no usable clock the next
/// sequence number (Clause 21.6). The module `point_event_time` has the
/// rule.
///
/// Access_Event_Time is the Table 13-1 trigger, and each edge moves it, so
/// each edge sends the SubscribeCOV report, which also carries the changed
/// OUT_OF_SERVICE flag. That holds for a round trip in one
/// WritePropertyMultiple too, where Status_Flags ends where it started. The
/// point runs no intrinsic reporting (no ACCESS_EVENT algorithm;
/// Event_State stays NORMAL), so an edge raises no event notification.
///
/// Active_Authentication_Policy, Number_Of_Authentication_Policies,
/// Authentication_Policy_List, Authentication_Policy_Names,
/// Authorization_Mode and Priority_For_Writing hold the settings the
/// application's authentication and authorization work from (#1307, #1325).
/// A client picks the policy in effect and the authorization mode; the
/// application sets the policies, the modes it carries out and the door
/// command priority. The module `point_authorization` has the rules.
///
/// Reliability is CONFIGURATION_ERROR while Active_Authentication_Policy is
/// zero, which happens when the policies leave none usable in effect
/// (Clause 12.31.10), or while Authentication_Policy_List holds any invalid
/// policy (Clause 12.31.12), and NO_FAULT_DETECTED otherwise. While it isn't
/// NO_FAULT_DETECTED the point performs no authentication and generates no
/// access events (Clause 12.31.7), so `report_access_event_local` refuses
/// them. Since it can leave
/// NO_FAULT_DETECTED, a client may write it while Out_Of_Service is TRUE
/// to simulate a fault, and only then (Clause 12.31.8): an Enumerated
/// inside the BACnetReliability production, else INVALID_DATA_TYPE or
/// VALUE_OUT_OF_RANGE, and WRITE_ACCESS_DENIED in service. Out of service
/// the value served stays decoupled from the policies: it starts as the
/// value served at the edge, follows only those writes, and the return to
/// service serves the derived value again. Status_Flags carries FAULT from
/// the value served.
pub struct AccessPointObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    access_event: AccessEvent,
    access_event_tag: u64,
    access_event_time: BACnetTimeStamp,
    access_event_credential: BACnetDeviceObjectReference,
    /// Access_Event_Authentication_Factor, the UNDEFINED factor until an
    /// event carries one.
    access_event_authentication_factor: BACnetAuthenticationFactor,
    /// The status the application reports; DISABLED is served instead while
    /// Out_Of_Service is TRUE.
    authentication_status: AuthenticationStatus,
    access_doors: Vec<BACnetDeviceObjectReference>,
    /// The policy, authorization mode and door command priority settings.
    authorization: Authorization,
    event_state: EventState,
    status_flags: StatusFlags,
    out_of_service: bool,
    /// Reliability as served: derived from the active policy in service, a
    /// client's simulation out of service.
    reliability: Reliability,
    clock: Option<Arc<dyn ClockReader>>,
}

impl AccessPointObject {
    /// Create a new Access Point object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_POINT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            access_event: AccessEvent::NONE,
            access_event_tag: 0,
            access_event_time: never_updated(),
            access_event_credential: no_credential(),
            access_event_authentication_factor: undefined_factor(),
            authentication_status: AuthenticationStatus::READY,
            access_doors: Vec::new(),
            authorization: Authorization::new(),
            event_state: EventState::NORMAL,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
            clock: None,
        })
    }

    /// Record the most recent access event: Access_Event, Access_Event_Tag,
    /// Access_Event_Time, Access_Event_Credential and
    /// Access_Event_Authentication_Factor, which the application's access
    /// logic produces. They change together, as Clause 12.31.27.1 has them
    /// stored for each event; [`AccessEventReport`] documents each field.
    ///
    /// Access_Event_Time is a `BACnetTimeStamp` (Clause 12.31.29) and goes
    /// out in its Clause 21 CHOICE form. A report without one is stamped
    /// strictly after the time served, from the Device clock or a sequence
    /// number, as an Out_Of_Service edge is (`point_event_time`). A change of the
    /// time triggers a SubscribeCOV notification; the other four only ride
    /// along (Table 13-1). Over the network all five stay read-only.
    ///
    /// The credential, when given, names the Access Credential object
    /// behind the event; without one the point stores the no-credential
    /// reference, instance 4194303, for an event no credential belongs to or
    /// one whose credential is unknown or kept back (Clause 12.31.30). Given
    /// explicitly, that reference carries 4194303 as the object instance
    /// and, when it names a device, as the device instance too. The factor
    /// is stored as given, or the UNDEFINED factor without one (Clause
    /// 12.31.31). Refused with VALUE_OUT_OF_RANGE, with none of the five
    /// changing: a reference to another object type, one whose device
    /// identifier isn't a Device (#1285), one with 4194303 in only one of
    /// the two instances, which is neither a credential nor the
    /// no-credential reference, a factor whose format type isn't a named
    /// BACnetAuthenticationFactorType, and an event that is neither a named
    /// BACnetAccessEvent nor a proprietary one from 512 to 65535 (Clause
    /// 21). These rows have no network write route, so this setter is the
    /// only check.
    ///
    /// This setter takes an event in or out of service, to set a point up
    /// before the server holds it. A running server's route,
    /// `BACnetServer::report_access_event_local`, refuses one while
    /// Out_Of_Service is TRUE (#1132).
    pub fn set_access_event(&mut self, report: AccessEventReport) -> Result<(), Error> {
        let credential = report.credential.unwrap_or_else(no_credential);
        crate::device_reference::check_device_member(credential.device_identifier)?;
        if credential.object_identifier.object_type() != ObjectType::ACCESS_CREDENTIAL {
            return Err(common::value_out_of_range_error());
        }
        let empty = |instance: u32| instance == ObjectIdentifier::MAX_INSTANCE;
        if credential.device_identifier.is_some_and(|device| {
            empty(device.instance_number()) != empty(credential.object_identifier.instance_number())
        }) {
            return Err(common::value_out_of_range_error());
        }
        let factor = report
            .authentication_factor
            .unwrap_or_else(undefined_factor);
        if !is_factor_type(factor.format_type) || !access_event_in_range(report.event) {
            return Err(common::value_out_of_range_error());
        }
        self.access_event_time = report
            .time
            .unwrap_or_else(|| next_event_time(self.clock.as_deref(), &self.access_event_time));
        self.access_event = report.event;
        self.access_event_tag = report.tag;
        self.access_event_credential = credential;
        self.access_event_authentication_factor = factor;
        Ok(())
    }

    /// Report the status of the point's authentication process
    /// (Clause 12.31.9), read-only over the network. A value outside the
    /// closed BACnetAuthenticationStatus production is refused with
    /// VALUE_OUT_OF_RANGE. While Out_Of_Service is TRUE the status is kept
    /// but DISABLED is served until the return to service.
    pub fn set_authentication_status(&mut self, status: AuthenticationStatus) -> Result<(), Error> {
        if status.to_raw() > AuthenticationStatus::IN_PROGRESS.to_raw() {
            return Err(common::value_out_of_range_error());
        }
        self.authentication_status = status;
        Ok(())
    }

    /// Authentication_Status as served: DISABLED while Out_Of_Service is
    /// TRUE, else the status last reported.
    pub fn authentication_status(&self) -> AuthenticationStatus {
        if self.out_of_service {
            AuthenticationStatus::DISABLED
        } else {
            self.authentication_status
        }
    }

    /// Set Access_Doors, the Access Door objects whose Present_Value this
    /// point commands once access is granted (Clause 12.31.32). A point that
    /// commands no door keeps the array empty. A reference with no Device
    /// identifier names an object in this device.
    ///
    /// Every reference must name an Access Door object, and a device
    /// identifier, when given, a Device object (#1285); a list breaking
    /// either rule is refused with VALUE_OUT_OF_RANGE and the doors set
    /// before are kept. The array is read-only over the network.
    pub fn set_access_doors(
        &mut self,
        doors: impl IntoIterator<Item = impl Into<BACnetDeviceObjectReference>>,
    ) -> Result<(), Error> {
        self.access_doors = references_to(ObjectType::ACCESS_DOOR, doors)?;
        Ok(())
    }

    /// Set Number_Of_Authentication_Policies, how many authentication
    /// policies the application defines (Clause 12.31.11), 1 until set. It
    /// is read-only over the network. Zero is refused with
    /// VALUE_OUT_OF_RANGE.
    ///
    /// Once [`Self::set_authentication_policies`] has set the policy arrays,
    /// they follow the count, which can then be at most
    /// [`MAX_AUTHENTICATION_POLICIES`] (VALUE_OUT_OF_RANGE above it): a
    /// smaller count drops the last policies, and a larger one adds empty
    /// policies, not enforcing order and with no timeout (Clause 12.31.12.2),
    /// each with an empty name. An empty policy is invalid, so a grown point
    /// reads Reliability CONFIGURATION_ERROR until the application fills the
    /// new policies in. A count below Active_Authentication_Policy, or one
    /// that leaves it naming an invalid policy, drops it to zero (Clause
    /// 12.31.10) until a client writes a usable policy.
    pub fn set_number_of_authentication_policies(&mut self, count: u32) -> Result<(), Error> {
        self.authorization.set_policies(count)?;
        self.refresh_reliability();
        Ok(())
    }

    /// Set Authentication_Policy_List and Authentication_Policy_Names, the
    /// policies the point defines, as `(name, policy)` pairs in policy
    /// order (Clauses 12.31.12 and 12.31.13). The point serves both arrays
    /// from then on, and Number_Of_Authentication_Policies takes the number
    /// of pairs, since both arrays are that long (Table 12-36 footnote 1).
    /// All three are read-only over the network. No pairs at all, or more
    /// than [`MAX_AUTHENTICATION_POLICIES`], is refused with
    /// VALUE_OUT_OF_RANGE, and nothing changes.
    ///
    /// A policy is stored as given, but only a well-formed one is usable:
    /// at least one entry, each naming a Credential Data Input object (in a
    /// Device, when it names a device), with indexes that start at 1 and, in
    /// list order, either repeat (a second factor that completes the same
    /// step) or go up by one. A client can't make an invalid policy the
    /// active one (VALUE_OUT_OF_RANGE), and while the list holds any invalid
    /// policy Reliability reads CONFIGURATION_ERROR (Clause 12.31.12). When
    /// the new list leaves Active_Authentication_Policy beyond the count or
    /// on an invalid policy, it drops to zero (Clause 12.31.10) until a
    /// client writes a usable policy; the point doesn't pick one itself.
    pub fn set_authentication_policies(
        &mut self,
        policies: impl IntoIterator<Item = (impl Into<String>, BACnetAuthenticationPolicy)>,
    ) -> Result<(), Error> {
        self.authorization.set_policy_list(
            policies
                .into_iter()
                .map(|(name, policy)| (name.into(), policy))
                .collect(),
        )?;
        self.refresh_reliability();
        Ok(())
    }

    /// Derive Reliability from the policies, in service only: out of
    /// service a client's simulated value stays served.
    fn refresh_reliability(&mut self) {
        if !self.out_of_service {
            self.reliability = if self.authorization.active_policy() == 0
                || self.authorization.holds_invalid_policy()
            {
                Reliability::CONFIGURATION_ERROR
            } else {
                Reliability::NO_FAULT_DETECTED
            };
        }
    }

    /// Whether the point serves Authentication_Policy_List and
    /// Authentication_Policy_Names.
    pub(super) fn serves_policy_list(&self) -> bool {
        self.authorization.serves_policy_list()
    }

    /// Set the authorization modes the application carries out, the values
    /// a write of Authorization_Mode can take (Clause 12.31.14). A new point
    /// takes AUTHORIZE alone, since the point enforces no mode itself: an
    /// application that acts on Authorization_Mode declares the other modes
    /// it carries out here. The set must hold AUTHORIZE and the mode in
    /// effect, and each mode must be a standard one or a proprietary one from
    /// 64 to 65535; otherwise it is refused with VALUE_OUT_OF_RANGE and the
    /// set before is kept. A repeated mode counts once.
    pub fn set_supported_authorization_modes(
        &mut self,
        modes: impl IntoIterator<Item = AuthorizationMode>,
    ) -> Result<(), Error> {
        self.authorization.set_supported_modes(modes)
    }

    /// Set Priority_For_Writing, the priority at which the application
    /// commands the Access_Doors after a grant (Clauses 12.31.32.1 and
    /// 12.31.33), 16 until set. It is read-only over the network. A
    /// priority outside 1..=16 is refused with VALUE_OUT_OF_RANGE.
    pub fn set_priority_for_writing(&mut self, priority: u8) -> Result<(), Error> {
        self.authorization.set_priority_for_writing(priority)
    }

    /// Record the access event an Out_Of_Service edge raises: a new
    /// transaction, stamped after the time served (`point_event_time`), with
    /// no credential and no factor.
    fn record_out_of_service_event(&mut self, event: AccessEvent) {
        self.access_event = event;
        self.access_event_tag = self.access_event_tag.wrapping_add(1);
        self.access_event_time = next_event_time(self.clock.as_deref(), &self.access_event_time);
        self.access_event_credential = no_credential();
        self.access_event_authentication_factor = undefined_factor();
    }
}

/// The Access_Event_Credential of an event without a credential
/// (Clause 12.31.30): an Access Credential identifier at instance 4194303,
/// with no device identifier.
fn no_credential() -> BACnetDeviceObjectReference {
    ObjectIdentifier::new(
        ObjectType::ACCESS_CREDENTIAL,
        ObjectIdentifier::MAX_INSTANCE,
    )
    .expect("4194303 is within the instance range")
    .into()
}

/// Whether `event` is a named BACnetAccessEvent or a proprietary one
/// (512..=65535, Clause 21).
fn access_event_in_range(event: AccessEvent) -> bool {
    AccessEvent::ALL_NAMED
        .iter()
        .any(|&(_, named)| named == event)
        || (512..=65_535).contains(&event.to_raw())
}

impl BACnetObject for AccessPointObject {
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
        if let Some(result) = self.authorization.read(property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ACCESS_POINT.to_raw()))
            }
            p if p == PropertyIdentifier::ACCESS_EVENT => {
                Ok(PropertyValue::Enumerated(self.access_event.to_raw()))
            }
            p if p == PropertyIdentifier::ACCESS_EVENT_TAG => {
                Ok(PropertyValue::Unsigned(self.access_event_tag))
            }
            p if p == PropertyIdentifier::ACCESS_EVENT_TIME => {
                timestamp_value(&self.access_event_time)
            }
            p if p == PropertyIdentifier::ACCESS_EVENT_CREDENTIAL => Ok(
                crate::device_reference::reference_value(&self.access_event_credential),
            ),
            p if p == PropertyIdentifier::ACCESS_EVENT_AUTHENTICATION_FACTOR => {
                let mut buf = BytesMut::new();
                encode_authentication_factor(&mut buf, &self.access_event_authentication_factor);
                Ok(PropertyValue::ApplicationData(buf.to_vec()))
            }
            p if p == PropertyIdentifier::AUTHENTICATION_STATUS => Ok(PropertyValue::Enumerated(
                self.authentication_status().to_raw(),
            )),
            p if p == PropertyIdentifier::ACCESS_DOORS => common::read_array(
                crate::device_reference::reference_elements(&self.access_doors),
                array_index,
            ),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state.to_raw()))
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
        let was_out_of_service = self.out_of_service;
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            match (was_out_of_service, self.out_of_service) {
                (false, true) => self.record_out_of_service_event(AccessEvent::OUT_OF_SERVICE),
                (true, false) => {
                    self.record_out_of_service_event(AccessEvent::OUT_OF_SERVICE_RELINQUISHED);
                    self.refresh_reliability();
                }
                _ => {}
            }
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if property == PropertyIdentifier::RELIABILITY {
            if !self.out_of_service {
                return Err(common::write_access_denied_error());
            }
            self.reliability = simulated_reliability(&value)?;
            return Ok(());
        }
        if let Some(result) = self.authorization.write(property, array_index, &value) {
            self.refresh_reliability();
            return result;
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            array_index,
        ))
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        super::metadata_topology::for_access_point_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    /// Table 13-1 lists Access Point, so it takes SubscribeCOV; its report
    /// leads with Access_Event, as the point has no Present_Value.
    fn supports_cov(&self) -> bool {
        true
    }

    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.clock = clock;
    }

    /// Take an access event the application reports (#1132), through
    /// [`Self::set_access_event`]. Refused with WRITE_ACCESS_DENIED while
    /// Out_Of_Service is TRUE (Clause 12.31.8) or Reliability isn't
    /// NO_FAULT_DETECTED (Clause 12.31.7), since the point performs no
    /// authentication then, and any other record with
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
    fn report_access_input_internal(&mut self, input: AccessControlInput) -> Result<(), Error> {
        let AccessControlInput::AccessEvent(report) = input else {
            return Err(common::optional_functionality_not_supported_error());
        };
        if self.out_of_service || self.reliability != Reliability::NO_FAULT_DETECTED {
            return Err(common::write_access_denied_error());
        }
        self.set_access_event(report)
    }
}

// ---------------------------------------------------------------------------
