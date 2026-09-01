use std::borrow::Cow;

use bacnet_types::enums::{EventState, NotifyType, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::common;
use crate::event::history::EventHistory;
use crate::property_metadata::PropertyMetadata;
use crate::traits::{BACnetObject, WritePropertyRollback};

use super::metadata;
use super::state::AlertEnrollmentWriteRollback;

/// BACnet AlertEnrollment object (type 52).
///
/// Provides the Alert Enrollment property surface from ASHRAE 135-2020 Table
/// 12-61. `Present_Value` identifies the object that last provided an alert;
/// recording that source does not itself evaluate or generate an event.
pub struct AlertEnrollmentObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Event_State: 0 = NORMAL.
    pub(super) event_state: u32,
    /// Object that last provided an alert.
    pub present_value: ObjectIdentifier,
    /// Whether event detection is enabled.
    ///
    /// Prefer [`Self::set_event_detection_enable`] so disabling also clears
    /// stored event state. This field remains public for compatibility;
    /// property reads and internal transition hooks still enforce the
    /// disabled-state invariant after a direct assignment. Re-enable through
    /// the setter as well: a direct FALSE-to-TRUE assignment cannot run the
    /// reset and may expose state stored before the direct disable.
    pub event_detection_enable: bool,
    /// Acknowledged transitions in TO_OFFNORMAL, TO_FAULT, TO_NORMAL order.
    pub(super) acked_transitions: u8,
    pub(super) event_history: EventHistory,
    /// Event enable bits: 3-bit (TO_OFFNORMAL, TO_FAULT, TO_NORMAL).
    pub event_enable: u8,
    /// Notification class number.
    pub notification_class: u32,
    /// Notification category for generated notifications. The local default is
    /// ALARM; ACK_NOTIFICATION is output-only acknowledgement-flow vocabulary.
    notify_type: u32,
}

impl AlertEnrollmentObject {
    /// Create a new AlertEnrollment object with the object that most recently
    /// provided an alert.
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
            event_state: 0,
            present_value: initial_source,
            event_detection_enable: true,
            acked_transitions: 0b111,
            event_history: EventHistory::default(),
            event_enable: 0b111,
            notification_class: 0,
            notify_type: NotifyType::ALARM.to_raw(),
        })
    }

    /// Record the object that most recently provided an alert.
    ///
    /// This source-ownership hook updates only `Present_Value`; it does not
    /// evaluate an alert, change event/acknowledgement history, or generate a
    /// notification.
    pub fn record_alert_source(&mut self, source: ObjectIdentifier) {
        self.present_value = source;
    }

    /// Enable or disable event detection.
    ///
    /// Disabling applies the Clause 13.2.2.1 initial conditions immediately.
    pub fn set_event_detection_enable(&mut self, enabled: bool) {
        if !enabled || !self.event_detection_enable {
            self.event_state = EventState::NORMAL.to_raw();
            self.acked_transitions = 0b111;
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
                let properties = self.property_list();
                Some(common::read_property_list_property(
                    &properties,
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
        if property == PropertyIdentifier::EVENT_TIME_STAMPS {
            if !self.event_detection_enable {
                return EventHistory::default()
                    .read(property, array_index)
                    .expect("EventHistory handles Event_Time_Stamps");
            }
            return self
                .event_history
                .read(property, array_index)
                .expect("EventHistory handles Event_Time_Stamps");
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
                data: vec![bacnet_types::bitstring::pack_octet(self.event_enable)],
            }),
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => {
                Ok(PropertyValue::Unsigned(self.notification_class as u64))
            }
            p if p == PropertyIdentifier::NOTIFY_TYPE => {
                Ok(PropertyValue::Enumerated(self.notify_type))
            }
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(if self.event_detection_enable {
                    self.event_state
                } else {
                    EventState::NORMAL.to_raw()
                }))
            }
            p if p == PropertyIdentifier::ACKED_TRANSITIONS => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![bacnet_types::bitstring::pack_octet(
                    if self.event_detection_enable {
                        self.acked_transitions
                    } else {
                        0b111
                    },
                )],
            }),
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
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.set_event_detection_enable(v);
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::EVENT_ENABLE {
            // BACnetEventTransitionBits is a 3-bit production (Clause 21):
            // the written BitString must declare its canonical shape.
            if let PropertyValue::BitString { unused_bits, data } = &value {
                let byte = common::check_fixed_width_bit_string(*unused_bits, data, 3)?;
                self.event_enable = bacnet_types::bitstring::unpack_octet(&[byte], 3);
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
                if v != NotifyType::ALARM.to_raw() && v != NotifyType::EVENT.to_raw() {
                    return Err(common::value_out_of_range_error());
                }
                self.notify_type = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(common::write_access_denied_error())
    }

    fn capture_write_property_rollback(
        &mut self,
        property: PropertyIdentifier,
        _value: &PropertyValue,
    ) -> Option<WritePropertyRollback> {
        (property == PropertyIdentifier::EVENT_DETECTION_ENABLE).then(|| {
            WritePropertyRollback::new(AlertEnrollmentWriteRollback {
                enabled: self.event_detection_enable,
                event_state: self.event_state,
                acked_transitions: self.acked_transitions,
                event_history: self.event_history.clone(),
            })
        })
    }

    fn restore_write_property_rollback(
        &mut self,
        rollback: WritePropertyRollback,
    ) -> Result<(), Error> {
        let AlertEnrollmentWriteRollback {
            enabled,
            event_state,
            acked_transitions,
            event_history,
        } = rollback.downcast::<AlertEnrollmentWriteRollback>()?;
        self.event_detection_enable = enabled;
        self.event_state = event_state;
        self.acked_transitions = acked_transitions;
        self.event_history = event_history;
        Ok(())
    }

    fn set_event_state_internal(&mut self, state: EventState) -> Result<(), Error> {
        if !self.event_detection_enable && state != EventState::NORMAL {
            return Err(common::write_access_denied_error());
        }
        self.event_state = state.to_raw();
        Ok(())
    }

    fn set_acked_transitions_internal(
        &mut self,
        transition_bit: u8,
        acknowledged: bool,
    ) -> Result<(), Error> {
        if !self.event_detection_enable {
            return Err(common::write_access_denied_error());
        }
        let transition_bit = transition_bit & 0x07;
        if acknowledged {
            self.acked_transitions |= transition_bit;
        } else {
            // Alert Enrollment never requires acknowledgment for TO_NORMAL
            // (Clause 12.52.8), so that bit cannot enter the unacknowledged
            // state even if a generic transition hook asks to clear it.
            self.acked_transitions &= !(transition_bit & 0x03);
        }
        self.acked_transitions |= 0x04;
        Ok(())
    }

    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        matches!(
            property,
            PropertyIdentifier::DESCRIPTION
                | PropertyIdentifier::EVENT_DETECTION_ENABLE
                | PropertyIdentifier::EVENT_ENABLE
                | PropertyIdentifier::NOTIFICATION_CLASS
                | PropertyIdentifier::NOTIFY_TYPE
        )
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        Cow::Borrowed(metadata::ALERT_ENROLLMENT_PROPERTIES)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(metadata::ALERT_ENROLLMENT_PROPERTIES)
    }
}
