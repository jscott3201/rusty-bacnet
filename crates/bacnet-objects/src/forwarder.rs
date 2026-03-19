//! NotificationForwarder object (type 51) per ASHRAE 135-2020 Clause 12.51.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

/// BACnet NotificationForwarder object (type 51).
///
/// Forwards event notifications to remote devices. Filters which
/// notifications to forward based on process identifier and local-only flags.
pub struct NotificationForwarderObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
    /// Process identifier filter — list of process IDs to forward for.
    pub process_identifier_filter: Vec<u32>,
    /// Number of subscribed recipients (stored as count).
    pub subscribed_recipients: u32,
    /// Whether to forward only local notifications.
    pub local_forwarding_only: bool,
    /// Whether event detection is enabled.
    pub event_detection_enable: bool,
}

impl NotificationForwarderObject {
    /// Create a new NotificationForwarder object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
            process_identifier_filter: Vec::new(),
            subscribed_recipients: 0,
            local_forwarding_only: false,
            event_detection_enable: true,
        })
    }
}

impl BACnetObject for NotificationForwarderObject {
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
                ObjectType::NOTIFICATION_FORWARDER.to_raw(),
            )),
            p if p == PropertyIdentifier::PROCESS_IDENTIFIER_FILTER => Ok(PropertyValue::List(
                self.process_identifier_filter
                    .iter()
                    .map(|id| PropertyValue::Unsigned(*id as u64))
                    .collect(),
            )),
            p if p == PropertyIdentifier::SUBSCRIBED_RECIPIENTS => {
                Ok(PropertyValue::Unsigned(self.subscribed_recipients as u64))
            }
            p if p == PropertyIdentifier::LOCAL_FORWARDING_ONLY => {
                Ok(PropertyValue::Boolean(self.local_forwarding_only))
            }
            p if p == PropertyIdentifier::EVENT_DETECTION_ENABLE => {
                Ok(PropertyValue::Boolean(self.event_detection_enable))
            }
            p if p == PropertyIdentifier::RECIPIENT_LIST => {
                Ok(PropertyValue::List(Vec::new())) // Empty recipient list by default
            }
            p if p == PropertyIdentifier::PROCESS_IDENTIFIER_FILTER => {
                Ok(PropertyValue::List(Vec::new()))
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
        if property == PropertyIdentifier::LOCAL_FORWARDING_ONLY {
            if let PropertyValue::Boolean(v) = value {
                self.local_forwarding_only = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.event_detection_enable = v;
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
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::PROCESS_IDENTIFIER_FILTER,
            PropertyIdentifier::SUBSCRIBED_RECIPIENTS,
            PropertyIdentifier::LOCAL_FORWARDING_ONLY,
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
        ];
        Cow::Borrowed(PROPS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_notification_forwarder() {
        let nf = NotificationForwarderObject::new(1, "NF-1").unwrap();
        assert_eq!(
            nf.object_identifier().object_type(),
            ObjectType::NOTIFICATION_FORWARDER
        );
        assert_eq!(nf.object_identifier().instance_number(), 1);
        assert_eq!(nf.object_name(), "NF-1");
    }

    #[test]
    fn object_type() {
        let nf = NotificationForwarderObject::new(1, "NF").unwrap();
        let val = nf
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(ObjectType::NOTIFICATION_FORWARDER.to_raw())
        );
    }

    #[test]
    fn process_identifier_filter_empty() {
        let nf = NotificationForwarderObject::new(1, "NF").unwrap();
        let val = nf
            .read_property(PropertyIdentifier::PROCESS_IDENTIFIER_FILTER, None)
            .unwrap();
        if let PropertyValue::List(items) = val {
            assert!(items.is_empty());
        } else {
            panic!("Expected List");
        }
    }

    #[test]
    fn process_identifier_filter_with_values() {
        let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
        nf.process_identifier_filter.push(100);
        nf.process_identifier_filter.push(200);

        let val = nf
            .read_property(PropertyIdentifier::PROCESS_IDENTIFIER_FILTER, None)
            .unwrap();
        if let PropertyValue::List(items) = val {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], PropertyValue::Unsigned(100));
            assert_eq!(items[1], PropertyValue::Unsigned(200));
        } else {
            panic!("Expected List");
        }
    }

    #[test]
    fn local_forwarding_only_default() {
        let nf = NotificationForwarderObject::new(1, "NF").unwrap();
        let val = nf
            .read_property(PropertyIdentifier::LOCAL_FORWARDING_ONLY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(false));
    }

    #[test]
    fn write_local_forwarding_only() {
        let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
        nf.write_property(
            PropertyIdentifier::LOCAL_FORWARDING_ONLY,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        let val = nf
            .read_property(PropertyIdentifier::LOCAL_FORWARDING_ONLY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(true));
    }

    #[test]
    fn write_local_forwarding_only_wrong_type() {
        let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
        let result = nf.write_property(
            PropertyIdentifier::LOCAL_FORWARDING_ONLY,
            None,
            PropertyValue::Unsigned(1),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn event_detection_enable_default() {
        let nf = NotificationForwarderObject::new(1, "NF").unwrap();
        let val = nf
            .read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(true));
    }

    #[test]
    fn write_event_detection_enable() {
        let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
        nf.write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
        let val = nf
            .read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(false));
    }

    #[test]
    fn subscribed_recipients_default() {
        let nf = NotificationForwarderObject::new(1, "NF").unwrap();
        let val = nf
            .read_property(PropertyIdentifier::SUBSCRIBED_RECIPIENTS, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(0));
    }

    #[test]
    fn property_list() {
        let nf = NotificationForwarderObject::new(1, "NF").unwrap();
        let props = nf.property_list();
        assert!(props.contains(&PropertyIdentifier::PROCESS_IDENTIFIER_FILTER));
        assert!(props.contains(&PropertyIdentifier::SUBSCRIBED_RECIPIENTS));
        assert!(props.contains(&PropertyIdentifier::LOCAL_FORWARDING_ONLY));
        assert!(props.contains(&PropertyIdentifier::EVENT_DETECTION_ENABLE));
    }
}
