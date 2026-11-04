use crate::prelude::*;
use alloc::borrow::Cow;

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{EventState, NotifyType, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::common;
use crate::event::history::EventHistory;
use crate::property_metadata::PropertyMetadata;
use crate::traits::BACnetObject;

use super::metadata;

/// BACnet AlertEnrollment object (type 52).
///
/// Provides the Alert Enrollment property surface from ASHRAE 135-2020 Table
/// 12-61. `Present_Value` names whichever object most recently handed this
/// enrollment an alert to distribute; recording that source does not itself
/// evaluate or generate an event. Event_Message_Texts,
/// Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair come from
/// the shared reporting options, as on Event Enrollment (#1865).
pub struct AlertEnrollmentObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Event_State; NORMAL while event detection is disabled.
    pub(super) event_state: EventState,
    /// Source of the most recent alert handed to this enrollment.
    pub present_value: ObjectIdentifier,
    /// Whether event detection is enabled; set through
    /// [`Self::set_event_detection_enable`], so disabling also clears stored
    /// event state. Property reads still enforce the disabled-state
    /// invariant after a direct assignment in this module's tests.
    pub(super) event_detection_enable: bool,
    /// Acked_Transitions: a set flag means that transition was acknowledged.
    pub(super) acked_transitions: EventTransitionBits,
    pub(super) event_history: EventHistory,
    /// Event_Enable: the transitions whose notifications are distributed.
    event_enable: EventTransitionBits,
    /// Notification class number.
    notification_class: u32,
    /// Notification category for generated notifications. The local default is
    /// ALARM; ACK_NOTIFICATION is output-only acknowledgement-flow vocabulary.
    notify_type: NotifyType,
    /// The reporting settings a write set, kept in storage once the
    /// enrollment opts in (#2331).
    reporting_store: crate::write_config::reporting::ReportingStore,
}

/// Whether an Alert Enrollment takes `notify_type` as its Notify_Type: only
/// ALARM and EVENT, the categories its notifications go out as.
fn takes_notify_type(notify_type: NotifyType) -> bool {
    notify_type == NotifyType::ALARM || notify_type == NotifyType::EVENT
}

impl AlertEnrollmentObject {
    /// Create a new AlertEnrollment object whose Present_Value starts as
    /// `initial_source`, the latest alert source.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        initial_source: ObjectIdentifier,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ALERT_ENROLLMENT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            event_state: EventState::NORMAL,
            present_value: initial_source,
            event_detection_enable: true,
            acked_transitions: EventTransitionBits::all(),
            event_history: EventHistory::default(),
            event_enable: EventTransitionBits::all(),
            notification_class: 0,
            notify_type: NotifyType::ALARM,
            reporting_store: Default::default(),
        })
    }

    /// Record `source` as the object behind the latest alert.
    ///
    /// This source-ownership hook updates only `Present_Value`; it does not
    /// evaluate an alert, change event/acknowledgement history, or generate a
    /// notification.
    pub fn record_alert_source(&mut self, source: ObjectIdentifier) {
        self.present_value = source;
    }

    /// Seed a committed, unacknowledged transition to `state` for a test:
    /// Alert Enrollment has no transition commit of its own.
    #[cfg(all(test, feature = "event-reporting"))]
    pub(crate) fn seed_unacknowledged(
        &mut self,
        state: EventState,
        transition: EventTransitionBits,
    ) {
        self.event_state = state;
        self.acked_transitions.remove(transition);
    }

    /// Enable or disable event detection.
    ///
    /// Disabling applies the Clause 13.2.2.1 initial conditions immediately.
    /// On an enrollment keeping its reporting settings, a value a write
    /// saved wins until `clear_written_reporting` (#2331), as for the next
    /// two setters.
    pub fn set_event_detection_enable(&mut self, enabled: bool) {
        if !self.kept_over_setter(PropertyIdentifier::EVENT_DETECTION_ENABLE) {
            self.apply_event_detection_enable(enabled);
        }
    }

    /// Set Notification_Class.
    pub fn set_notification_class(&mut self, class: u32) {
        if !self.kept_over_setter(PropertyIdentifier::NOTIFICATION_CLASS) {
            self.notification_class = class;
        }
    }

    /// Set Event_Enable: the transitions whose notifications are
    /// distributed. Flags outside the three named transitions are dropped.
    pub fn set_event_enable(&mut self, enable: EventTransitionBits) {
        if !self.kept_over_setter(PropertyIdentifier::EVENT_ENABLE) {
            self.event_enable = enable & EventTransitionBits::all();
        }
    }

    fn kept_over_setter(&self, property: PropertyIdentifier) -> bool {
        crate::write_config::reporting::setter_gives_way(&self.reporting_store, self.oid, property)
    }

    /// Event_Detection_Enable as a write or an unguarded setter makes it.
    fn apply_event_detection_enable(&mut self, enabled: bool) {
        if !enabled || !self.event_detection_enable {
            self.event_state = EventState::NORMAL;
            self.acked_transitions = EventTransitionBits::all();
            self.event_history.reset();
        }
        self.event_detection_enable = enabled;
    }

    /// Alert-local projection of the universal properties that Table 12-61
    /// serves. In particular, the shared common-property macro is intentionally
    /// not used because it would also expose Status_Flags, Out_Of_Service, and
    /// Reliability, none of which is a Table 12-61 row.
    fn read_alert_common_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => {
                Some(Ok(PropertyValue::ObjectIdentifier(self.oid)))
            }
            PropertyIdentifier::OBJECT_NAME => {
                Some(Ok(PropertyValue::CharacterString(self.name.clone())))
            }
            PropertyIdentifier::DESCRIPTION => {
                Some(Ok(PropertyValue::CharacterString(self.description.clone())))
            }
            PropertyIdentifier::PROPERTY_LIST => {
                Some(crate::property_metadata::read_property_list(
                    &self.property_metadata(),
                    array_index,
                ))
            }
            _ => None,
        }
    }
}

impl BACnetObject for AlertEnrollmentObject {
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
        crate::property_metadata::check_read_array_index(self, property, array_index)?;
        // The history arrays read their initial condition while detection is
        // off, even after a direct field assignment skipped the reset.
        if !self.event_detection_enable
            && matches!(
                property,
                PropertyIdentifier::EVENT_TIME_STAMPS | PropertyIdentifier::EVENT_MESSAGE_TEXTS
            )
        {
            return EventHistory::default()
                .read(property, array_index)
                .expect("EventHistory handles the history arrays");
        }
        // Event_Time_Stamps, Event_Message_Texts and the reporting options
        // (Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair),
        // which Table 12-61 lists as optional rows (#1865).
        if let Some(result) = self.event_history.read(property, array_index) {
            return result;
        }
        if let Some(result) = self.read_alert_common_property(property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::ALERT_ENROLLMENT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::ObjectIdentifier(self.present_value))
            }
            p if p == PropertyIdentifier::EVENT_DETECTION_ENABLE => {
                Ok(PropertyValue::Boolean(self.event_detection_enable))
            }
            p if p == PropertyIdentifier::EVENT_ENABLE => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.event_enable.to_bacnet()],
            }),
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => {
                Ok(PropertyValue::Unsigned(self.notification_class as u64))
            }
            p if p == PropertyIdentifier::NOTIFY_TYPE => {
                Ok(PropertyValue::Enumerated(self.notify_type.to_raw()))
            }
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(
                if self.event_detection_enable {
                    self.event_state
                } else {
                    EventState::NORMAL
                }
                .to_raw(),
            )),
            p if p == PropertyIdentifier::ACKED_TRANSITIONS => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![
                    if self.event_detection_enable {
                        self.acked_transitions
                    } else {
                        EventTransitionBits::all()
                    }
                    .to_bacnet(),
                ],
            }),
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
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        crate::write_config::reporting::admit(self, property, array_index, &value)?;
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.apply_event_detection_enable(v);
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::EVENT_ENABLE {
            // BACnetEventTransitionBits is a 3-bit production (Clause 21):
            // the written BitString must declare its canonical shape.
            if let PropertyValue::BitString { unused_bits, data } = &value {
                let byte = common::check_fixed_width_bit_string(*unused_bits, data, 3)?;
                self.event_enable = EventTransitionBits::from_bacnet(&[byte]);
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::NOTIFICATION_CLASS {
            if let PropertyValue::Unsigned(v) = value {
                self.notification_class = common::u64_to_u32(v)?;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::NOTIFY_TYPE {
            if let PropertyValue::Enumerated(v) = value {
                let notify_type = NotifyType::from_raw(v);
                if !takes_notify_type(notify_type) {
                    return Err(common::value_out_of_range_error());
                }
                self.notify_type = notify_type;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) =
            self.event_history
                .write(property, array_index, &value, self.event_detection_enable)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
        ))
    }

    crate::traits::lend_event_reporting!();

    #[cfg(feature = "durable-writes")]
    fn durable_writes(&mut self) -> Option<&mut dyn crate::durable::DurableWrites> {
        self.durable_capability()
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        Cow::Borrowed(metadata::ALERT_ENROLLMENT_PROPERTIES)
    }
}

crate::write_config::reporting::impl_reporting_persistence!(AlertEnrollmentObject, without_tags);
keep_enrollment_reporting!(AlertEnrollmentObject, takes_notify_type: takes_notify_type);

/// An Alert Enrollment's alerts come from its server-side evaluator, so it
/// lends event reporting for its Event_Algorithm_Inhibit_Ref alone.
impl crate::traits::EventReporting for AlertEnrollmentObject {
    fn event_algorithm_inhibit_reference(
        &self,
    ) -> Option<bacnet_types::constructed::BACnetObjectPropertyReference> {
        self.event_history.options.inhibit_reference().cloned()
    }

    fn follow_event_algorithm_inhibit(&mut self, inhibit: bool) -> bool {
        self.event_history.options.follow(inhibit)
    }
}
