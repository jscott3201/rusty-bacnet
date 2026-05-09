//! EventEnrollment (type 9) object per ASHRAE 135-2020 Clause 12.12.

use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, FaultParameters};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

/// BACnet EventEnrollment object.
///
/// Provides algorithmic event detection for a referenced object property.
/// The event_parameters are stored as raw bytes; full structured decoding
/// is deferred to a future enhancement.
pub struct EventEnrollmentObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    event_type: u32,
    notify_type: u32,
    event_parameters: Vec<u8>,
    object_property_reference: Option<BACnetDeviceObjectPropertyReference>,
    event_state: u32,
    event_enable: u8,
    acked_transitions: u8,
    notification_class: u32,
    fault_parameters: Option<FaultParameters>,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl EventEnrollmentObject {
    /// Create a new EventEnrollment object.
    ///
    /// `event_type` is the BACnet EventType enumeration value.
    pub fn new(instance: u32, name: impl Into<String>, event_type: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            event_type,
            notify_type: 0,
            event_parameters: Vec::new(),
            object_property_reference: None,
            event_state: 0,
            event_enable: 0b111,
            acked_transitions: 0b111,
            notification_class: 0,
            fault_parameters: None,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set the object property reference.
    pub fn set_object_property_reference(
        &mut self,
        reference: Option<BACnetDeviceObjectPropertyReference>,
    ) {
        self.object_property_reference = reference;
    }

    /// Set the event parameters (raw bytes).
    pub fn set_event_parameters(&mut self, params: Vec<u8>) {
        self.event_parameters = params;
    }

    /// Set the fault parameters for this event enrollment.
    pub fn set_fault_parameters(&mut self, fp: Option<FaultParameters>) {
        self.fault_parameters = fp;
    }

    /// Set the event state (raw u32).
    pub fn set_event_state(&mut self, state: u32) {
        self.event_state = state;
    }

    /// Set the notification class.
    pub fn set_notification_class(&mut self, nc: u32) {
        self.notification_class = nc;
    }

    /// Set the event enable bitmask (3 bits: TO_OFFNORMAL, TO_FAULT, TO_NORMAL).
    pub fn set_event_enable(&mut self, enable: u8) {
        self.event_enable = enable & 0x07;
    }
}

impl BACnetObject for EventEnrollmentObject {
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
                ObjectType::EVENT_ENROLLMENT.to_raw(),
            )),
            p if p == PropertyIdentifier::EVENT_TYPE => {
                Ok(PropertyValue::Enumerated(self.event_type))
            }
            p if p == PropertyIdentifier::NOTIFY_TYPE => {
                Ok(PropertyValue::Enumerated(self.notify_type))
            }
            p if p == PropertyIdentifier::EVENT_PARAMETERS => {
                Ok(PropertyValue::OctetString(self.event_parameters.clone()))
            }
            p if p == PropertyIdentifier::OBJECT_PROPERTY_REFERENCE => {
                match &self.object_property_reference {
                    None => Ok(PropertyValue::Null),
                    Some(r) => Ok(PropertyValue::List(vec![
                        PropertyValue::ObjectIdentifier(r.object_identifier),
                        PropertyValue::Unsigned(r.property_identifier as u64),
                        match r.property_array_index {
                            Some(idx) => PropertyValue::Unsigned(idx as u64),
                            None => PropertyValue::Null,
                        },
                        match r.device_identifier {
                            Some(dev) => PropertyValue::ObjectIdentifier(dev),
                            None => PropertyValue::Null,
                        },
                    ])),
                }
            }
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state))
            }
            p if p == PropertyIdentifier::EVENT_ENABLE => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.event_enable << 5],
            }),
            p if p == PropertyIdentifier::ACKED_TRANSITIONS => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.acked_transitions << 5],
            }),
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => {
                Ok(PropertyValue::Unsigned(self.notification_class as u64))
            }
            p if p == PropertyIdentifier::FAULT_PARAMETERS => match &self.fault_parameters {
                None => Ok(PropertyValue::Null),
                Some(fp) => {
                    let variant_tag = match fp {
                        FaultParameters::FaultNone => 0u64,
                        FaultParameters::FaultCharacterString { .. } => 1,
                        FaultParameters::FaultExtended { .. } => 2,
                        FaultParameters::FaultLifeSafety { .. } => 3,
                        FaultParameters::FaultState { .. } => 4,
                        FaultParameters::FaultStatusFlags { .. } => 5,
                        FaultParameters::FaultOutOfRange { .. } => 6,
                        FaultParameters::FaultListed { .. } => 7,
                    };
                    Ok(PropertyValue::Unsigned(variant_tag))
                }
            },
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
        if property == PropertyIdentifier::NOTIFY_TYPE {
            if let PropertyValue::Enumerated(v) = value {
                self.notify_type = v;
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
        if property == PropertyIdentifier::EVENT_ENABLE {
            if let PropertyValue::BitString { data, .. } = &value {
                if let Some(&byte) = data.first() {
                    self.event_enable = byte >> 5;
                    return Ok(());
                }
                return Err(common::invalid_data_type_error());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::EVENT_STATE {
            if let PropertyValue::Enumerated(v) = value {
                self.event_state = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::EVENT_PARAMETERS {
            if let PropertyValue::OctetString(bytes) = value {
                self.event_parameters = bytes;
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
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::EVENT_TYPE,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::EVENT_PARAMETERS,
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::FAULT_PARAMETERS,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// AlertEnrollmentObject (type 52)
// ---------------------------------------------------------------------------

/// BACnet AlertEnrollment object (type 52).
///
/// Provides alert-based event enrollment. The PRESENT_VALUE is an enumerated
/// AlertState. Supports EVENT_DETECTION_ENABLE, EVENT_ENABLE (3-bit),
/// and NOTIFICATION_CLASS.
pub struct AlertEnrollmentObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    status_flags: StatusFlags,
    /// Event_State: 0 = NORMAL.
    event_state: u32,
    out_of_service: bool,
    reliability: u32,
    /// Present value — AlertState enumeration.
    pub present_value: u32,
    /// Whether event detection is enabled.
    pub event_detection_enable: bool,
    /// Event enable bits: 3-bit (TO_OFFNORMAL, TO_FAULT, TO_NORMAL).
    pub event_enable: u8,
    /// Notification class number.
    pub notification_class: u32,
}

impl AlertEnrollmentObject {
    /// Create a new AlertEnrollment object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ALERT_ENROLLMENT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            status_flags: StatusFlags::empty(),
            event_state: 0, // NORMAL
            out_of_service: false,
            reliability: 0,
            present_value: 0,
            event_detection_enable: true,
            event_enable: 0b111,
            notification_class: 0,
        })
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
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::ALERT_ENROLLMENT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::EVENT_DETECTION_ENABLE => {
                Ok(PropertyValue::Boolean(self.event_detection_enable))
            }
            p if p == PropertyIdentifier::EVENT_ENABLE => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.event_enable << 5],
            }),
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => {
                Ok(PropertyValue::Unsigned(self.notification_class as u64))
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
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.event_detection_enable = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::EVENT_ENABLE {
            if let PropertyValue::BitString { data, .. } = &value {
                if let Some(&byte) = data.first() {
                    self.event_enable = byte >> 5;
                    return Ok(());
                }
                return Err(common::invalid_data_type_error());
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
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

#[cfg(test)]
mod tests;
