//! Device object (type 8) per ASHRAE 135-2020 Clause 12.11.
//!
//! The Device object is required in every BACnet device and exposes
//! device-level properties such as vendor info, protocol support,
//! and configuration parameters.

use std::borrow::Cow;
use std::collections::HashMap;

use bacnet_types::constructed::BACnetCOVSubscription;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier, Segmentation};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};

use crate::common::read_property_list_property;
use crate::traits::BACnetObject;

/// Build a BACnet bitstring representing supported object types.
/// Each type N sets bit at byte N/8, position 7-(N%8) (MSB-first within each byte).
fn compute_object_types_supported(types: &[u32]) -> Vec<u8> {
    let max_type = types.iter().copied().max().unwrap_or(0) as usize;
    let num_bytes = (max_type / 8) + 1;
    let mut bitstring = vec![0u8; num_bytes];
    for &t in types {
        let byte_idx = (t as usize) / 8;
        let bit_pos = 7 - ((t as usize) % 8);
        if byte_idx < bitstring.len() {
            bitstring[byte_idx] |= 1 << bit_pos;
        }
    }
    bitstring
}

/// Configuration for creating a Device object.
pub struct DeviceConfig {
    /// Device instance number (0..4194303).
    pub instance: u32,
    /// Device object name.
    pub name: String,
    /// Vendor name string.
    pub vendor_name: String,
    /// ASHRAE-assigned vendor identifier.
    pub vendor_id: u16,
    /// Model name string.
    pub model_name: String,
    /// Firmware revision string.
    pub firmware_revision: String,
    /// Application software version string.
    pub application_software_version: String,
    /// Maximum APDU length accepted (typically 1476 for BIP).
    pub max_apdu_length: u32,
    /// Segmentation support level.
    pub segmentation_supported: Segmentation,
    /// APDU timeout in milliseconds.
    pub apdu_timeout: u32,
    /// Number of APDU retries.
    pub apdu_retries: u32,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            instance: 1,
            name: "BACnet Device".into(),
            vendor_name: "Rusty BACnet".into(),
            vendor_id: 0,
            model_name: "rusty-bacnet".into(),
            firmware_revision: "0.1.0".into(),
            application_software_version: "0.1.0".into(),
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            apdu_timeout: 6000,
            apdu_retries: 3,
        }
    }
}

/// BACnet Device object.
pub struct DeviceObject {
    oid: ObjectIdentifier,
    properties: HashMap<PropertyIdentifier, PropertyValue>,
    /// Cached object list for array-indexed reads.
    object_list: Vec<ObjectIdentifier>,
    /// Protocol_Object_Types_Supported — bitstring indicating which object
    /// types this device supports (one bit per type, MSB-first within each byte).
    protocol_object_types_supported: Vec<u8>,
    /// Protocol_Services_Supported — bitstring indicating which services
    /// this device supports (one bit per service, MSB-first within each byte).
    protocol_services_supported: Vec<u8>,
    /// Active COV subscriptions maintained by the server.
    active_cov_subscriptions: Vec<BACnetCOVSubscription>,
}

impl DeviceObject {
    /// Create a new Device object from configuration.
    pub fn new(config: DeviceConfig) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::DEVICE, config.instance)?;
        let mut properties = HashMap::new();

        properties.insert(
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyValue::ObjectIdentifier(oid),
        );
        properties.insert(
            PropertyIdentifier::OBJECT_NAME,
            PropertyValue::CharacterString(config.name),
        );
        properties.insert(
            PropertyIdentifier::OBJECT_TYPE,
            PropertyValue::Enumerated(ObjectType::DEVICE.to_raw()),
        );
        properties.insert(
            PropertyIdentifier::SYSTEM_STATUS,
            PropertyValue::Enumerated(0), // operational
        );
        properties.insert(
            PropertyIdentifier::VENDOR_NAME,
            PropertyValue::CharacterString(config.vendor_name),
        );
        properties.insert(
            PropertyIdentifier::VENDOR_IDENTIFIER,
            PropertyValue::Unsigned(config.vendor_id as u64),
        );
        properties.insert(
            PropertyIdentifier::MODEL_NAME,
            PropertyValue::CharacterString(config.model_name),
        );
        properties.insert(
            PropertyIdentifier::FIRMWARE_REVISION,
            PropertyValue::CharacterString(config.firmware_revision),
        );
        properties.insert(
            PropertyIdentifier::APPLICATION_SOFTWARE_VERSION,
            PropertyValue::CharacterString(config.application_software_version),
        );
        properties.insert(
            PropertyIdentifier::PROTOCOL_VERSION,
            PropertyValue::Unsigned(1),
        );
        properties.insert(
            PropertyIdentifier::PROTOCOL_REVISION,
            PropertyValue::Unsigned(22), // Revision 22 (2020)
        );
        properties.insert(
            PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED,
            PropertyValue::Unsigned(config.max_apdu_length as u64),
        );
        properties.insert(
            PropertyIdentifier::SEGMENTATION_SUPPORTED,
            PropertyValue::Enumerated(config.segmentation_supported.to_raw() as u32),
        );
        properties.insert(
            PropertyIdentifier::APDU_TIMEOUT,
            PropertyValue::Unsigned(config.apdu_timeout as u64),
        );
        properties.insert(
            PropertyIdentifier::NUMBER_OF_APDU_RETRIES,
            PropertyValue::Unsigned(config.apdu_retries as u64),
        );
        properties.insert(
            PropertyIdentifier::DATABASE_REVISION,
            PropertyValue::Unsigned(0),
        );
        properties.insert(
            PropertyIdentifier::DESCRIPTION,
            PropertyValue::CharacterString(String::new()),
        );

        // Device_Address_Binding — starts empty; populated as devices are discovered.
        properties.insert(
            PropertyIdentifier::DEVICE_ADDRESS_BINDING,
            PropertyValue::List(Vec::new()),
        );

        // Placeholder values updated by the server's time sync or system clock.
        properties.insert(
            PropertyIdentifier::LOCAL_DATE,
            PropertyValue::Date(Date {
                year: 126, // 2026 - 1900
                month: 3,
                day: 18,
                day_of_week: 3, // Wednesday
            }),
        );
        properties.insert(
            PropertyIdentifier::LOCAL_TIME,
            PropertyValue::Time(Time {
                hour: 12,
                minute: 0,
                second: 0,
                hundredths: 0,
            }),
        );

        // UTC_Offset: signed integer minutes from UTC (e.g., -300 for EST).
        properties.insert(
            PropertyIdentifier::UTC_OFFSET,
            PropertyValue::Signed(0), // UTC
        );

        // Last_Restart_Reason: 0=unknown, 1=coldstart, 2=warmstart, etc.
        properties.insert(
            PropertyIdentifier::LAST_RESTART_REASON,
            PropertyValue::Enumerated(0), // unknown
        );

        // Device_UUID: 16-byte UUID stored as OctetString. Default: all zeros.
        properties.insert(
            PropertyIdentifier::DEVICE_UUID,
            PropertyValue::OctetString(vec![0u8; 16]),
        );

        // Max_Segments_Accepted — only included when segmentation is supported.
        if config.segmentation_supported != Segmentation::NONE {
            properties.insert(
                PropertyIdentifier::MAX_SEGMENTS_ACCEPTED,
                PropertyValue::Unsigned(65), // default: more than 64 segments
            );
        }

        // Protocol_Object_Types_Supported: bitstring with one bit per
        // implemented object type.  Computed from the full set of types
        // that have concrete struct implementations in this crate.
        let protocol_object_types_supported = compute_object_types_supported(&[
            ObjectType::ANALOG_INPUT.to_raw(),
            ObjectType::ANALOG_OUTPUT.to_raw(),
            ObjectType::ANALOG_VALUE.to_raw(),
            ObjectType::BINARY_INPUT.to_raw(),
            ObjectType::BINARY_OUTPUT.to_raw(),
            ObjectType::BINARY_VALUE.to_raw(),
            ObjectType::CALENDAR.to_raw(),
            ObjectType::COMMAND.to_raw(),
            ObjectType::DEVICE.to_raw(),
            ObjectType::EVENT_ENROLLMENT.to_raw(),
            ObjectType::FILE.to_raw(),
            ObjectType::GROUP.to_raw(),
            ObjectType::LOOP.to_raw(),
            ObjectType::MULTI_STATE_INPUT.to_raw(),
            ObjectType::MULTI_STATE_OUTPUT.to_raw(),
            ObjectType::NOTIFICATION_CLASS.to_raw(),
            ObjectType::PROGRAM.to_raw(),
            ObjectType::SCHEDULE.to_raw(),
            ObjectType::AVERAGING.to_raw(),
            ObjectType::MULTI_STATE_VALUE.to_raw(),
            ObjectType::TREND_LOG.to_raw(),
            ObjectType::LIFE_SAFETY_POINT.to_raw(),
            ObjectType::LIFE_SAFETY_ZONE.to_raw(),
            ObjectType::ACCUMULATOR.to_raw(),
            ObjectType::PULSE_CONVERTER.to_raw(),
            ObjectType::EVENT_LOG.to_raw(),
            ObjectType::GLOBAL_GROUP.to_raw(),
            ObjectType::TREND_LOG_MULTIPLE.to_raw(),
            ObjectType::LOAD_CONTROL.to_raw(),
            ObjectType::STRUCTURED_VIEW.to_raw(),
            ObjectType::ACCESS_DOOR.to_raw(),
            ObjectType::TIMER.to_raw(),
            ObjectType::ACCESS_CREDENTIAL.to_raw(),
            ObjectType::ACCESS_POINT.to_raw(),
            ObjectType::ACCESS_RIGHTS.to_raw(),
            ObjectType::ACCESS_USER.to_raw(),
            ObjectType::ACCESS_ZONE.to_raw(),
            ObjectType::CREDENTIAL_DATA_INPUT.to_raw(),
            ObjectType::BITSTRING_VALUE.to_raw(),
            ObjectType::CHARACTERSTRING_VALUE.to_raw(),
            ObjectType::DATEPATTERN_VALUE.to_raw(),
            ObjectType::DATE_VALUE.to_raw(),
            ObjectType::DATETIMEPATTERN_VALUE.to_raw(),
            ObjectType::DATETIME_VALUE.to_raw(),
            ObjectType::INTEGER_VALUE.to_raw(),
            ObjectType::LARGE_ANALOG_VALUE.to_raw(),
            ObjectType::OCTETSTRING_VALUE.to_raw(),
            ObjectType::POSITIVE_INTEGER_VALUE.to_raw(),
            ObjectType::TIMEPATTERN_VALUE.to_raw(),
            ObjectType::TIME_VALUE.to_raw(),
            ObjectType::NOTIFICATION_FORWARDER.to_raw(),
            ObjectType::ALERT_ENROLLMENT.to_raw(),
            ObjectType::CHANNEL.to_raw(),
            ObjectType::LIGHTING_OUTPUT.to_raw(),
            ObjectType::BINARY_LIGHTING_OUTPUT.to_raw(),
            ObjectType::NETWORK_PORT.to_raw(),
            ObjectType::ELEVATOR_GROUP.to_raw(),
            ObjectType::ESCALATOR.to_raw(),
            ObjectType::LIFT.to_raw(),
            ObjectType::STAGING.to_raw(),
            ObjectType::AUDIT_REPORTER.to_raw(),
            ObjectType::AUDIT_LOG.to_raw(),
            ObjectType::COLOR.to_raw(),
            ObjectType::COLOR_TEMPERATURE.to_raw(),
        ]);

        // Protocol_Services_Supported: 6 bytes (48 bits).  Bits set for
        // services we handle:
        //   0=AcknowledgeAlarm, 2=ConfirmedEventNotification,
        //   5=SubscribeCOV, 12=ReadProperty, 14=ReadPropertyMultiple,
        //   15=WriteProperty, 16=WritePropertyMultiple,
        //   26=IAm, 27=IHave, 29=UnconfirmedCOVNotification,
        //   31=WhoHas, 32=WhoIs
        //   Byte 0: bits 0,2,5 → 0xA4
        //   Byte 1: bits 12,14,15 → 0x0B
        //   Byte 2: bit 16 → 0x80
        //   Byte 3: bits 26,27,29,31 → 0x35
        //   Byte 4: bit 32 → 0x80
        //   Byte 5: 0x00
        let protocol_services_supported = vec![0xA4, 0x0B, 0x80, 0x35, 0x80, 0x00];

        Ok(Self {
            oid,
            properties,
            object_list: vec![oid], // Device itself is always in the list
            protocol_object_types_supported,
            protocol_services_supported,
            active_cov_subscriptions: Vec::new(),
        })
    }

    /// Update the object-list with the current database contents.
    pub fn set_object_list(&mut self, oids: Vec<ObjectIdentifier>) {
        self.object_list = oids;
    }

    /// Get the device instance number.
    pub fn instance(&self) -> u32 {
        self.oid.instance_number()
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.properties.insert(
            PropertyIdentifier::DESCRIPTION,
            PropertyValue::CharacterString(desc.into()),
        );
    }

    /// Replace the entire active COV subscriptions list.
    pub fn set_active_cov_subscriptions(&mut self, subs: Vec<BACnetCOVSubscription>) {
        self.active_cov_subscriptions = subs;
    }

    /// Add a single COV subscription.
    pub fn add_cov_subscription(&mut self, sub: BACnetCOVSubscription) {
        self.active_cov_subscriptions.push(sub);
    }
}

impl BACnetObject for DeviceObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        match self.properties.get(&PropertyIdentifier::OBJECT_NAME) {
            Some(PropertyValue::CharacterString(s)) => s,
            _ => "Unknown",
        }
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if property == PropertyIdentifier::OBJECT_LIST {
            return match array_index {
                None => {
                    let elements = self
                        .object_list
                        .iter()
                        .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                        .collect();
                    Ok(PropertyValue::List(elements))
                }
                Some(0) => {
                    // Index 0 = array length per BACnet convention
                    Ok(PropertyValue::Unsigned(self.object_list.len() as u64))
                }
                Some(idx) => {
                    let i = (idx - 1) as usize; // BACnet arrays are 1-based
                    if i < self.object_list.len() {
                        Ok(PropertyValue::ObjectIdentifier(self.object_list[i]))
                    } else {
                        Err(Error::Protocol {
                            class: ErrorClass::PROPERTY.to_raw() as u32,
                            code: ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32,
                        })
                    }
                }
            };
        }

        if property == PropertyIdentifier::PROPERTY_LIST {
            return read_property_list_property(&self.property_list(), array_index);
        }

        if property == PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED {
            let num_bytes = self.protocol_object_types_supported.len();
            let total_bits = num_bytes * 8;
            // Find highest set bit to determine actual used bits
            let mut max_type = 0u32;
            for (byte_idx, &byte) in self.protocol_object_types_supported.iter().enumerate() {
                for bit in 0..8 {
                    if byte & (1 << (7 - bit)) != 0 {
                        max_type = (byte_idx * 8 + bit) as u32;
                    }
                }
            }
            let used_bits = max_type as usize + 1;
            let unused = (total_bits - used_bits) as u8;
            return Ok(PropertyValue::BitString {
                unused_bits: unused,
                data: self.protocol_object_types_supported.clone(),
            });
        }

        if property == PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED {
            // 6 bytes = 48 bits; 41 defined (services 0-40), 7 unused bits
            return Ok(PropertyValue::BitString {
                unused_bits: 7,
                data: self.protocol_services_supported.clone(),
            });
        }

        if property == PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS {
            let elements: Vec<PropertyValue> = self
                .active_cov_subscriptions
                .iter()
                .map(|sub| {
                    let mut entry = vec![
                        PropertyValue::ObjectIdentifier(
                            sub.monitored_property_reference.object_identifier,
                        ),
                        PropertyValue::Unsigned(sub.recipient.process_identifier as u64),
                        PropertyValue::Boolean(sub.issue_confirmed_notifications),
                        PropertyValue::Unsigned(sub.time_remaining as u64),
                    ];
                    if let Some(inc) = sub.cov_increment {
                        entry.push(PropertyValue::Real(inc));
                    }
                    PropertyValue::List(entry)
                })
                .collect();
            return Ok(PropertyValue::List(elements));
        }

        self.properties
            .get(&property)
            .cloned()
            .ok_or(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            })
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::DESCRIPTION {
            if let PropertyValue::CharacterString(_) = &value {
                self.properties.insert(property, value);
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        let mut props: Vec<PropertyIdentifier> = self.properties.keys().copied().collect();
        props.push(PropertyIdentifier::OBJECT_LIST);
        props.push(PropertyIdentifier::PROPERTY_LIST);
        props.push(PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED);
        props.push(PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED);
        props.push(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS);
        props.sort_by_key(|p| p.to_raw());
        Cow::Owned(props)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_device() -> DeviceObject {
        DeviceObject::new(DeviceConfig {
            instance: 1234,
            name: "Test Device".into(),
            ..DeviceConfig::default()
        })
        .unwrap()
    }

    #[test]
    fn read_object_identifier() {
        let dev = make_device();
        let val = dev
            .read_property(PropertyIdentifier::OBJECT_IDENTIFIER, None)
            .unwrap();
        let expected_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
        assert_eq!(val, PropertyValue::ObjectIdentifier(expected_oid));
    }

    #[test]
    fn read_object_name() {
        let dev = make_device();
        let val = dev
            .read_property(PropertyIdentifier::OBJECT_NAME, None)
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString("Test Device".into()));
    }

    #[test]
    fn read_object_type() {
        let dev = make_device();
        let val = dev
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(ObjectType::DEVICE.to_raw()));
    }

    #[test]
    fn read_vendor_name() {
        let dev = make_device();
        let val = dev
            .read_property(PropertyIdentifier::VENDOR_NAME, None)
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString("Rusty BACnet".into()));
    }

    #[test]
    fn read_max_apdu_length() {
        let dev = make_device();
        let val = dev
            .read_property(PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(1476));
    }

    #[test]
    fn read_unknown_property_fails() {
        let dev = make_device();
        // Use a property that Device doesn't have
        let result = dev.read_property(PropertyIdentifier::PRESENT_VALUE, None);
        assert!(result.is_err());
    }

    #[test]
    fn write_property_denied() {
        let mut dev = make_device();
        let result = dev.write_property(
            PropertyIdentifier::OBJECT_NAME,
            None,
            PropertyValue::CharacterString("New Name".into()),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn device_description_default_empty() {
        let dev = make_device();
        let val = dev
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString(String::new()));
    }

    #[test]
    fn device_description_write_read() {
        let mut dev = make_device();
        dev.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("Main building controller".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            dev.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("Main building controller".into())
        );
    }

    #[test]
    fn device_set_description_convenience() {
        let mut dev = make_device();
        dev.set_description("Rooftop unit controller");
        assert_eq!(
            dev.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("Rooftop unit controller".into())
        );
    }

    #[test]
    fn device_description_in_property_list() {
        let dev = make_device();
        assert!(dev
            .property_list()
            .contains(&PropertyIdentifier::DESCRIPTION));
    }

    #[test]
    fn object_list_default_contains_device() {
        let dev = make_device();
        // arrayIndex absent: returns the full array as a List
        let val = dev
            .read_property(PropertyIdentifier::OBJECT_LIST, None)
            .unwrap();
        let expected_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
        assert_eq!(
            val,
            PropertyValue::List(vec![PropertyValue::ObjectIdentifier(expected_oid)])
        );
    }

    #[test]
    fn object_list_array_index() {
        let dev = make_device();
        // Index 0 = length
        let val = dev
            .read_property(PropertyIdentifier::OBJECT_LIST, Some(0))
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(1));

        // Index 1 = first element (the device itself)
        let val = dev
            .read_property(PropertyIdentifier::OBJECT_LIST, Some(1))
            .unwrap();
        let expected_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
        assert_eq!(val, PropertyValue::ObjectIdentifier(expected_oid));

        // Index 2 = out of range
        let result = dev.read_property(PropertyIdentifier::OBJECT_LIST, Some(2));
        assert!(result.is_err());
    }

    #[test]
    fn set_object_list() {
        let mut dev = make_device();
        let dev_oid = dev.object_identifier();
        let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let ai2 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();
        dev.set_object_list(vec![dev_oid, ai1, ai2]);

        // arrayIndex absent: returns the full array
        let val = dev
            .read_property(PropertyIdentifier::OBJECT_LIST, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(dev_oid),
                PropertyValue::ObjectIdentifier(ai1),
                PropertyValue::ObjectIdentifier(ai2),
            ])
        );

        // arrayIndex 0: returns the count
        let count = dev
            .read_property(PropertyIdentifier::OBJECT_LIST, Some(0))
            .unwrap();
        assert_eq!(count, PropertyValue::Unsigned(3));
    }

    #[test]
    fn property_list_contains_expected() {
        let dev = make_device();
        let props = dev.property_list();
        assert!(props.contains(&PropertyIdentifier::OBJECT_IDENTIFIER));
        assert!(props.contains(&PropertyIdentifier::OBJECT_NAME));
        assert!(props.contains(&PropertyIdentifier::OBJECT_TYPE));
        assert!(props.contains(&PropertyIdentifier::VENDOR_NAME));
        assert!(props.contains(&PropertyIdentifier::OBJECT_LIST));
        assert!(props.contains(&PropertyIdentifier::PROPERTY_LIST));
        assert!(props.contains(&PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED));
        assert!(props.contains(&PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED));
    }

    #[test]
    fn read_protocol_object_types_supported() {
        let dev = make_device();
        let val = dev
            .read_property(PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED, None)
            .unwrap();
        match val {
            PropertyValue::BitString { unused_bits, data } => {
                assert_eq!(unused_bits, 7);
                assert_eq!(data.len(), 9);
                // Byte 0 (types 0-7): all set
                assert_eq!(data[0], 0xFF);
                // Byte 1 (types 8-15): all set
                assert_eq!(data[1], 0xFF);
                // Byte 2 (types 16-23): all set
                assert_eq!(data[2], 0xFF);
                // Byte 3 (types 24-31): all set
                assert_eq!(data[3], 0xFF);
                // Byte 4 (types 32-39): 32-37,39 set; 38 (NetworkSecurity) unset
                assert_eq!(data[4], 0xFD);
                // Byte 5 (types 40-47): all set
                assert_eq!(data[5], 0xFF);
                // Byte 6 (types 48-55): all set
                assert_eq!(data[6], 0xFF);
                // Byte 7 (types 56-63): all set (56-62 + Color=63)
                assert_eq!(data[7], 0xFF);
                // Byte 8 (type 64): ColorTemperature set, 7 unused bits
                assert_eq!(data[8], 0x80);
            }
            _ => panic!("Expected BitString"),
        }
    }

    #[test]
    fn read_protocol_services_supported() {
        let dev = make_device();
        let val = dev
            .read_property(PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED, None)
            .unwrap();
        match val {
            PropertyValue::BitString { unused_bits, data } => {
                assert_eq!(unused_bits, 7);
                assert_eq!(data.len(), 6);
                // Byte 0: services 0,2,5
                assert_eq!(data[0], 0xA4);
                // Byte 1: services 12,14,15
                assert_eq!(data[1], 0x0B);
                // Byte 4: service 32 (WhoIs)
                assert_eq!(data[4], 0x80);
            }
            _ => panic!("Expected BitString"),
        }
    }

    #[test]
    fn active_cov_subscriptions_default_empty() {
        let dev = make_device();
        let val = dev
            .read_property(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
            .unwrap();
        assert_eq!(val, PropertyValue::List(vec![]));
    }

    #[test]
    fn active_cov_subscriptions_in_property_list() {
        let dev = make_device();
        assert!(dev
            .property_list()
            .contains(&PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS));
    }

    #[test]
    fn active_cov_subscriptions_after_add() {
        use bacnet_types::constructed::{
            BACnetCOVSubscription, BACnetObjectPropertyReference, BACnetRecipient,
            BACnetRecipientProcess,
        };

        let mut dev = make_device();
        let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 200).unwrap();
        let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        dev.add_cov_subscription(BACnetCOVSubscription {
            recipient: BACnetRecipientProcess {
                recipient: BACnetRecipient::Device(dev_oid),
                process_identifier: 7,
            },
            monitored_property_reference: BACnetObjectPropertyReference::new(
                ai_oid,
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            ),
            issue_confirmed_notifications: true,
            time_remaining: 300,
            cov_increment: Some(0.5),
        });

        let val = dev
            .read_property(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
            .unwrap();
        match val {
            PropertyValue::List(subs) => {
                assert_eq!(subs.len(), 1);
                match &subs[0] {
                    PropertyValue::List(entry) => {
                        assert_eq!(entry.len(), 5); // includes cov_increment
                        assert_eq!(entry[0], PropertyValue::ObjectIdentifier(ai_oid));
                        assert_eq!(entry[1], PropertyValue::Unsigned(7));
                        assert_eq!(entry[2], PropertyValue::Boolean(true));
                        assert_eq!(entry[3], PropertyValue::Unsigned(300));
                        assert_eq!(entry[4], PropertyValue::Real(0.5));
                    }
                    _ => panic!("Expected List entry"),
                }
            }
            _ => panic!("Expected List"),
        }
    }

    #[test]
    fn active_cov_subscriptions_without_increment() {
        use bacnet_types::constructed::{
            BACnetCOVSubscription, BACnetObjectPropertyReference, BACnetRecipient,
            BACnetRecipientProcess,
        };

        let mut dev = make_device();
        let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 50).unwrap();
        let bv_oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 3).unwrap();

        dev.add_cov_subscription(BACnetCOVSubscription {
            recipient: BACnetRecipientProcess {
                recipient: BACnetRecipient::Device(dev_oid),
                process_identifier: 1,
            },
            monitored_property_reference: BACnetObjectPropertyReference::new(
                bv_oid,
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            ),
            issue_confirmed_notifications: false,
            time_remaining: 0,
            cov_increment: None,
        });

        let val = dev
            .read_property(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
            .unwrap();
        match val {
            PropertyValue::List(subs) => {
                assert_eq!(subs.len(), 1);
                match &subs[0] {
                    PropertyValue::List(entry) => {
                        assert_eq!(entry.len(), 4); // no cov_increment
                        assert_eq!(entry[2], PropertyValue::Boolean(false));
                    }
                    _ => panic!("Expected List entry"),
                }
            }
            _ => panic!("Expected List"),
        }
    }

    #[test]
    fn active_cov_subscriptions_write_denied() {
        let mut dev = make_device();
        let result = dev.write_property(
            PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS,
            None,
            PropertyValue::List(vec![]),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn set_active_cov_subscriptions_replaces() {
        use bacnet_types::constructed::{
            BACnetCOVSubscription, BACnetObjectPropertyReference, BACnetRecipient,
            BACnetRecipientProcess,
        };

        let mut dev = make_device();
        let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap();
        let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let ai2 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();

        // Add two subscriptions
        let sub1 = BACnetCOVSubscription {
            recipient: BACnetRecipientProcess {
                recipient: BACnetRecipient::Device(dev_oid),
                process_identifier: 1,
            },
            monitored_property_reference: BACnetObjectPropertyReference::new(
                ai1,
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            ),
            issue_confirmed_notifications: true,
            time_remaining: 100,
            cov_increment: None,
        };
        let sub2 = BACnetCOVSubscription {
            recipient: BACnetRecipientProcess {
                recipient: BACnetRecipient::Device(dev_oid),
                process_identifier: 2,
            },
            monitored_property_reference: BACnetObjectPropertyReference::new(
                ai2,
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            ),
            issue_confirmed_notifications: false,
            time_remaining: 200,
            cov_increment: Some(1.0),
        };
        dev.set_active_cov_subscriptions(vec![sub1, sub2]);

        let val = dev
            .read_property(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
            .unwrap();
        match val {
            PropertyValue::List(subs) => assert_eq!(subs.len(), 2),
            _ => panic!("Expected List"),
        }

        // Replace with empty
        dev.set_active_cov_subscriptions(vec![]);
        let val = dev
            .read_property(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
            .unwrap();
        assert_eq!(val, PropertyValue::List(vec![]));
    }

    #[test]
    fn compute_object_types_supported_known_inputs() {
        assert_eq!(compute_object_types_supported(&[0]), vec![0x80]);
        assert_eq!(compute_object_types_supported(&[8]), vec![0x00, 0x80]);
        assert_eq!(
            compute_object_types_supported(&[0, 1, 2, 3, 4, 5]),
            vec![0xFC]
        );
        assert_eq!(compute_object_types_supported(&[]), vec![0x00]);
    }

    #[test]
    fn compute_object_types_supported_old_bits_preserved() {
        let old_types: Vec<u32> = vec![0, 1, 2, 3, 4, 5, 8, 13, 14, 19];
        let bs = compute_object_types_supported(&old_types);
        assert_eq!(bs[0], 0xFC);
        assert_eq!(bs[1], 0x86);
        assert_eq!(bs[2], 0x10);
    }

    #[test]
    fn device_protocol_object_types_has_new_bits() {
        let dev = DeviceObject::new(DeviceConfig {
            instance: 1,
            name: "Test".into(),
            ..DeviceConfig::default()
        })
        .unwrap();
        let val = dev
            .read_property(PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED, None)
            .unwrap();
        let bits = match val {
            PropertyValue::BitString { data, .. } => data,
            _ => panic!("Expected BitString"),
        };
        assert!(bits.len() >= 8, "bitstring should cover types up to 62");
        assert_eq!(bits[0] & 0xFC, 0xFC, "AI/AO/AV/BI/BO/BV");
        assert_ne!(bits[1] & 0x80, 0, "Device (8)");
        assert_ne!(bits[1] & 0x04, 0, "MSI (13)");
        assert_ne!(bits[1] & 0x02, 0, "MSO (14)");
        assert_ne!(bits[2] & 0x10, 0, "MSV (19)");
        assert_ne!(bits[0] & 0x03, 0, "Calendar(6) and Command(7)");
        assert_ne!(bits[3] & 0x80, 0, "Accumulator (24)");
        assert_ne!(bits[7] & 0x80, 0, "NetworkPort (56)");
    }
}
