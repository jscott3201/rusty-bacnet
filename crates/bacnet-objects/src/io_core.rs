//! What the analog, binary and multi-state Input, Output and Value objects
//! share (#2182): [`IoCore`], the state each holds beside its event
//! detector and its own Present_Value, and the property routes over them.
//!
//! The core serves Object_Identifier, Object_Name, Description and
//! Object_Type, Status_Flags, Out_Of_Service, Reliability and
//! Reliability_Evaluation_Inhibit, Event_Detection_Enable, the event
//! history and the optional profile rows, and with them the event rows of
//! the object's detector ([`IoDetector`]). An object answers its own rows
//! (Present_Value, Units, a priority array, state texts) and hands every
//! other property to [`IoCore::read`] and [`IoCore::write`]. Property_List
//! and the answer for a property nobody serves stay with the object, which
//! owns its metadata.
//!
//! The detector stays a field of the object, `event_detector`, beside the
//! core: the core's own code is the same for every detector, so all nine
//! objects share one copy of it, and only the small entry points that reach
//! the detector are made once per detector type. The shared macros that
//! wire an object's reporting name the core's fields by their path
//! (`core.event_history` for
//! [`impl_builtin_intrinsic_reporting!`](crate::event::impl_builtin_intrinsic_reporting),
//! `in core` for
//! [`impl_reporting_persistence!`](crate::write_config::reporting::impl_reporting_persistence)).

use alloc::borrow::Cow;
use alloc::sync::Arc;

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{EventState, ObjectType, PropertyIdentifier as P, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use crate::common::{
    self, ReliabilityInhibitState, read_analog_event_properties, read_generic_event_properties,
    write_analog_event_properties, write_generic_event_properties,
};
use crate::event::OutOfRangeDetector;
use crate::event::history::EventHistory;
use crate::event::options::REPORTING_OPTION_METADATA;
use crate::object_profile::{ProfileState, TagsPersistence};
use crate::prelude::*;
use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyPresenceCondition::{IntrinsicReportingOptional, IntrinsicReportingRequired},
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
};

/// The state an analog, binary or multi-state Input, Output or Value holds
/// beside its event detector and its Present_Value.
pub(crate) struct IoCore {
    pub(crate) oid: ObjectIdentifier,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) out_of_service: bool,
    /// Reliability; NO_FAULT_DETECTED until a fault is evaluated or simulated.
    pub(crate) reliability: Reliability,
    /// The object's own Reliability, set aside while Out_Of_Service is TRUE
    /// and a client may simulate one.
    pub(crate) reliability_before_out_of_service: Option<Reliability>,
    pub(crate) reliability_inhibit: ReliabilityInhibitState,
    /// Event_Detection_Enable. FALSE suspends the event state machine
    /// (Clause 13.2.2.1).
    pub(crate) event_detection_enable: bool,
    pub(crate) event_history: EventHistory,
    /// Tags, Profile_Location and Profile_Name, once provisioned.
    pub(crate) profile: ProfileState,
    /// The reporting settings a write set, kept in storage once the object
    /// opts in (#2331).
    #[cfg(feature = "event-reporting")]
    pub(crate) reporting_store: crate::write_config::reporting::ReportingStore,
}

impl IoCore {
    /// A core for the object `instance` of `object_type`: in service, no
    /// fault, detection on, no profile rows. Fails as
    /// [`ObjectIdentifier::new`] does for an instance out of range.
    pub(crate) fn new(object_type: ObjectType, instance: u32, name: String) -> Result<Self, Error> {
        Ok(Self {
            oid: ObjectIdentifier::new(object_type, instance)?,
            name,
            description: String::new(),
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
            reliability_before_out_of_service: None,
            reliability_inhibit: ReliabilityInhibitState::default(),
            event_detection_enable: true,
            event_history: EventHistory::default(),
            profile: ProfileState::default(),
            #[cfg(feature = "event-reporting")]
            reporting_store: Default::default(),
        })
    }

    /// Keep written Tags in `persistence`, starting from what it holds for
    /// this object: the `with_tags_persistence` constructors.
    pub(crate) fn keep_tags(&mut self, persistence: Arc<dyn TagsPersistence>) -> Result<(), Error> {
        self.profile = ProfileState::persistent(self.oid, persistence, false)?;
        Ok(())
    }

    /// Add the provisioned profile rows to `rows`, after the object's own.
    pub(crate) fn add_profile_rows(&self, rows: &mut Cow<'_, [PropertyMetadata]>) {
        if self.profile.metadata().next().is_some() {
            rows.to_mut().extend(self.profile.metadata());
        }
    }

    /// Whether Reliability is the object's own to work out now: neither
    /// Out_Of_Service, under which a client may simulate it, nor
    /// Reliability_Evaluation_Inhibit is TRUE.
    pub(crate) fn owns_reliability(&self) -> bool {
        !self.out_of_service && !self.reliability_inhibit.enabled()
    }

    /// The route behind `ReliabilityEval::set_reliability`: WRITE_ACCESS_DENIED
    /// unless the object owns its Reliability now, VALUE_OUT_OF_RANGE for a
    /// value outside BACnetReliability. A refusal changes nothing.
    pub(crate) fn set_reliability(&mut self, reliability: Reliability) -> Result<(), Error> {
        if !self.owns_reliability() {
            return Err(common::write_access_denied_error());
        }
        if !common::is_reliability_value_valid(reliability) {
            return Err(common::value_out_of_range_error());
        }
        self.reliability = reliability;
        Ok(())
    }

    /// Read one of the properties the core or `detector` serves; `None` for
    /// any other. The object has checked `array_index` against its metadata
    /// first.
    ///
    /// Only this entry and [`Self::write`] are made per detector type; the
    /// rest is one copy for every object, as far as the inliner leaves it
    /// one. An image with a single I/O type gets [`Self::read_own`] and
    /// [`Self::write_own`] inlined into that type's own routes, which costs a
    /// few dozen octets there; check the shared copy once all nine objects
    /// are on the core, and mark the two `#[inline(never)]` if the images
    /// need it.
    pub(crate) fn read<D: IoDetector>(
        &self,
        detector: &D,
        property: P,
        array_index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        if let Some(result) = detector.read(property) {
            return Some(result);
        }
        self.read_own(property, array_index, detector.event_state())
    }

    /// [`Self::read`] past the detector's rows; `event_state` is its
    /// Event_State, which Status_Flags' IN_ALARM follows.
    fn read_own(
        &self,
        property: P,
        array_index: Option<u32>,
        event_state: EventState,
    ) -> Option<Result<PropertyValue, Error>> {
        let value = match property {
            P::OBJECT_IDENTIFIER => PropertyValue::ObjectIdentifier(self.oid),
            P::OBJECT_NAME => PropertyValue::CharacterString(self.name.clone()),
            P::DESCRIPTION => PropertyValue::CharacterString(self.description.clone()),
            P::OBJECT_TYPE => PropertyValue::Enumerated(self.oid.object_type().to_raw()),
            P::STATUS_FLAGS => common::compute_status_flags(
                StatusFlags::empty(),
                self.reliability,
                self.out_of_service,
                event_state,
            ),
            P::OUT_OF_SERVICE => PropertyValue::Boolean(self.out_of_service),
            P::RELIABILITY => PropertyValue::Enumerated(self.reliability.to_raw()),
            P::RELIABILITY_EVALUATION_INHIBIT => {
                PropertyValue::Boolean(self.reliability_inhibit.enabled())
            }
            P::EVENT_DETECTION_ENABLE => PropertyValue::Boolean(self.event_detection_enable),
            _ => {
                if let Some(result) = self.event_history.read(property, array_index) {
                    return Some(result);
                }
                return self.profile.read(property, array_index);
            }
        };
        Some(Ok(value))
    }

    /// Take a write of one of the properties the core or `detector` serves;
    /// `None` for any other. A refused write changes nothing.
    ///
    /// Out_Of_Service hands Reliability to a client and back
    /// ([`ReliabilityInhibitState::write_out_of_service`]); Reliability
    /// itself takes a client's write only while Out_Of_Service is TRUE.
    pub(crate) fn write<D: IoDetector>(
        &mut self,
        detector: &mut D,
        property: P,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        if property == P::EVENT_DETECTION_ENABLE {
            return Some(self.write_detection_enable(value).map(|disabled| {
                if disabled {
                    detector.reset();
                }
            }));
        }
        if let Some(result) = detector.write(property, value) {
            return Some(result);
        }
        self.write_own(property, array_index, value)
    }

    /// [`Self::write`] past Event_Detection_Enable and the detector's rows.
    fn write_own(
        &mut self,
        property: P,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        let inhibit = &mut self.reliability_inhibit;
        if let Some(result) =
            inhibit.write_inhibit(&mut self.reliability, self.out_of_service, property, value)
        {
            return Some(result);
        }
        if let Some(result) = inhibit.write_out_of_service(
            &mut self.out_of_service,
            &mut self.reliability,
            &mut self.reliability_before_out_of_service,
            property,
            value,
        ) {
            return Some(result.map(drop));
        }
        if let Some(result) = common::write_object_name(&mut self.name, property, value) {
            return Some(result);
        }
        if let Some(result) = common::write_description(&mut self.description, property, value) {
            return Some(result);
        }
        if let Some(result) = inhibit.write_client_reliability(
            self.out_of_service,
            &mut self.reliability,
            property,
            value,
        ) {
            return Some(result);
        }
        // Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair (#1329).
        if let Some(result) =
            self.event_history
                .write(property, array_index, value, self.event_detection_enable)
        {
            return Some(result);
        }
        self.profile.write(property, array_index, value)
    }

    /// Event_Detection_Enable. FALSE also puts the event history back to
    /// its initial values (Clause 13.2.2.1), the reporting options kept;
    /// `Ok(true)` then asks the caller to reset its detector too.
    fn write_detection_enable(&mut self, value: &PropertyValue) -> Result<bool, Error> {
        let &PropertyValue::Boolean(enabled) = value else {
            return Err(common::invalid_data_type_error());
        };
        self.event_detection_enable = enabled;
        if !enabled {
            self.event_history.reset();
        }
        Ok(!enabled)
    }
}

/// The intrinsic-reporting detector an I/O object holds beside its
/// [`IoCore`]: OUT_OF_RANGE on the analog objects. Its event rows are the
/// ones every detector has (Event_State, Event_Enable, Notify_Type,
/// Notification_Class, the two time delays and Acked_Transitions) and its
/// algorithm's parameters.
pub(crate) trait IoDetector {
    /// Event_State, as Status_Flags' IN_ALARM reads it.
    fn event_state(&self) -> EventState;

    /// Back to the initial event state detection off asks for: NORMAL,
    /// every transition acknowledged, no delay running and no fault
    /// recorded. The parameters stay.
    fn reset(&mut self);

    /// Read one of the detector's event rows; `None` for any other property.
    fn read(&self, property: P) -> Option<Result<PropertyValue, Error>>;

    /// Take a write of one of the detector's event rows; `None` for any
    /// other property. Acked_Transitions is refused with
    /// WRITE_ACCESS_DENIED: only transitions and acknowledgments move it.
    fn write(&mut self, property: P, value: &PropertyValue) -> Option<Result<(), Error>>;
}

impl IoDetector for OutOfRangeDetector {
    fn event_state(&self) -> EventState {
        self.event_state
    }

    fn reset(&mut self) {
        self.event_state = EventState::NORMAL;
        self.acked_transitions = EventTransitionBits::all();
        self.pending = None;
        self.fault_reliability = None;
    }

    fn read(&self, property: P) -> Option<Result<PropertyValue, Error>> {
        if let Some(result) = read_analog_event_properties!(self, property) {
            return Some(result);
        }
        read_generic_event_properties!(self, property)
    }

    fn write(&mut self, property: P, value: &PropertyValue) -> Option<Result<(), Error>> {
        if let Some(result) = write_analog_event_properties!(self, property, value) {
            return Some(result);
        }
        write_generic_event_properties!(self, property, value)
    }
}

/// Object_Identifier, Object_Name, Description and Object_Type: the rows
/// each of the nine tables opens with.
pub(crate) const IDENTITY_ROWS: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
];

/// Reliability, which a client writes while Out_Of_Service is TRUE, and
/// Reliability_Evaluation_Inhibit.
pub(crate) const RELIABILITY_ROWS: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::RELIABILITY, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::RELIABILITY_EVALUATION_INHIBIT, Optional, None, Always),
];

/// Acked_Transitions, Event_Time_Stamps and Event_Message_Texts, which an
/// intrinsic reporter needs (Event_Message_Texts is only permitted, #1485),
/// then Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair
/// (#1329).
pub(crate) const EVENT_HISTORY_ROWS: &[PropertyMetadata] = &[
    PropertyMetadata::new(
        P::ACKED_TRANSITIONS,
        Optional,
        Some(IntrinsicReportingRequired),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::EVENT_TIME_STAMPS,
        Optional,
        Some(IntrinsicReportingRequired),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::EVENT_MESSAGE_TEXTS,
        Optional,
        Some(IntrinsicReportingOptional),
        ReadOnly,
    ),
    REPORTING_OPTION_METADATA[0],
    REPORTING_OPTION_METADATA[1],
    REPORTING_OPTION_METADATA[2],
];

/// Give `$object`, which holds an [`IoCore`] in `core`, the profile setters
/// every one of the nine has: `set_profile` and `wait_for_tag_saves`.
macro_rules! impl_profile {
    ($object:ty) => {
        impl $object {
            /// Provision independent optional Tags, Profile_Location and
            /// Profile_Name rows before registration. Tags is writable over
            /// the network whatever Out_Of_Service or a command says, and on
            /// an Analog Value in every Present_Value access mode; the text
            /// rows are network read-only. Invalid configuration leaves the
            /// previous profile and its attached storage unchanged.
            pub fn set_profile(
                &mut self,
                profile: $crate::object_profile::ObjectProfile,
            ) -> Result<(), bacnet_types::error::Error> {
                self.core.profile.provision(profile)
            }

            /// Wait for queued save attempts to finish; writes report their
            /// own outcomes. This is not a success receipt. Unstaged
            /// synchronous writes block their caller.
            pub fn wait_for_tag_saves(&self) {
                self.core.profile.wait_for_saves();
            }
        }
    };
}
pub(crate) use impl_profile;
