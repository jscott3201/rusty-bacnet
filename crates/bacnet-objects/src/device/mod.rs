//! Device object (type 8) per ASHRAE 135-2020 Clause 12.11.
//!
//! The Device object is required in every BACnet device and exposes
//! device-level properties such as vendor info, protocol support,
//! and configuration parameters.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use bacnet_types::constructed::BACnetCOVSubscription;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, ObjectType, PropertyIdentifier, Segmentation, ServiceSupported,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::clock::{ClockFrame, ClockReader};
use crate::common::read_property_list_property;
use crate::traits::BACnetObject;

/// Every service the bundled `bacnet-server` dispatch executes, as
/// `BACnetServicesSupported` bit positions (Clause 21).
///
/// Default source for `Protocol_Services_Supported`, which Clause 12.11 ties
/// to services *executed* — the server's initiate-only services (I-Am,
/// I-Have, COV/event notifications) are deliberately absent. Kept in lockstep
/// with the dispatch arms by `bacnet-server`'s executed-services cross-check
/// test; deployments with a different dispatch surface override via
/// [`DeviceObject::set_services_supported`].
pub const EXECUTED_SERVICES: &[ServiceSupported] = &[
    ServiceSupported::ACKNOWLEDGE_ALARM,
    ServiceSupported::GET_ALARM_SUMMARY,
    ServiceSupported::GET_ENROLLMENT_SUMMARY,
    ServiceSupported::SUBSCRIBE_COV,
    ServiceSupported::ATOMIC_READ_FILE,
    ServiceSupported::ATOMIC_WRITE_FILE,
    ServiceSupported::ADD_LIST_ELEMENT,
    ServiceSupported::REMOVE_LIST_ELEMENT,
    ServiceSupported::CREATE_OBJECT,
    ServiceSupported::DELETE_OBJECT,
    ServiceSupported::READ_PROPERTY,
    ServiceSupported::READ_PROPERTY_MULTIPLE,
    ServiceSupported::WRITE_PROPERTY,
    ServiceSupported::WRITE_PROPERTY_MULTIPLE,
    ServiceSupported::DEVICE_COMMUNICATION_CONTROL,
    ServiceSupported::CONFIRMED_TEXT_MESSAGE,
    ServiceSupported::REINITIALIZE_DEVICE,
    ServiceSupported::UNCONFIRMED_TEXT_MESSAGE,
    ServiceSupported::TIME_SYNCHRONIZATION,
    ServiceSupported::WHO_HAS,
    ServiceSupported::WHO_IS,
    ServiceSupported::READ_RANGE,
    ServiceSupported::UTC_TIME_SYNCHRONIZATION,
    // Executes authorized silence/unsilence operations. Built-in reset execution
    // stays unsupported pending application-executor and replay semantics.
    ServiceSupported::LIFE_SAFETY_OPERATION,
    ServiceSupported::SUBSCRIBE_COV_PROPERTY,
    ServiceSupported::GET_EVENT_INFORMATION,
    ServiceSupported::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
    ServiceSupported::CONFIRMED_AUDIT_NOTIFICATION,
    ServiceSupported::UNCONFIRMED_AUDIT_NOTIFICATION,
    ServiceSupported::AUDIT_LOG_QUERY,
];

/// Number of bits in the `BACnetServicesSupported` production: bits 0..=48
/// (you-Are is the highest defined bit, Clause 21).
const SERVICES_SUPPORTED_BITS: usize = 49;

/// Build the `Protocol_Services_Supported` bit string from a service set,
/// sized for the full production (7 octets, 7 unused bits) and packed
/// MSB-first per Clause 20.2.10: bit N at byte N/8, position 7-(N%8).
fn compute_services_supported(services: &[ServiceSupported]) -> Vec<u8> {
    let mut bits = vec![0u8; SERVICES_SUPPORTED_BITS.div_ceil(8)];
    for service in services {
        let n = service.to_raw() as usize;
        if n < SERVICES_SUPPORTED_BITS {
            bits[n / 8] |= 0x80 >> (n % 8);
        }
    }
    bits
}

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
    /// Configured executed services before clock-availability filtering.
    configured_services_supported: Vec<ServiceSupported>,
    /// Shared dynamic clock sample source. `None` is explicitly clockless.
    clock: Option<Arc<dyn ClockReader>>,
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
            let max_segments_accepted = if config.segmentation_supported == Segmentation::TRANSMIT {
                1
            } else {
                65
            };
            properties.insert(
                PropertyIdentifier::MAX_SEGMENTS_ACCEPTED,
                PropertyValue::Unsigned(max_segments_accepted),
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
            ObjectType::ALERT_ENROLLMENT.to_raw(),
            ObjectType::LIGHTING_OUTPUT.to_raw(),
            ObjectType::BINARY_LIGHTING_OUTPUT.to_raw(),
            ObjectType::NETWORK_PORT.to_raw(),
            ObjectType::ELEVATOR_GROUP.to_raw(),
            ObjectType::ESCALATOR.to_raw(),
            ObjectType::LIFT.to_raw(),
            ObjectType::STAGING.to_raw(),
            ObjectType::AUDIT_LOG.to_raw(),
            ObjectType::AUDIT_REPORTER.to_raw(),
            ObjectType::COLOR.to_raw(),
            ObjectType::COLOR_TEMPERATURE.to_raw(),
        ]);

        Ok(Self {
            oid,
            properties,
            object_list: vec![oid], // Device itself is always in the list
            protocol_object_types_supported,
            configured_services_supported: EXECUTED_SERVICES.to_vec(),
            clock: None,
            active_cov_subscriptions: Vec::new(),
        })
    }

    /// Update the object-list with the current database contents.
    pub fn set_object_list(&mut self, oids: Vec<ObjectIdentifier>) {
        self.object_list = oids;
    }

    /// Replace the advertised executed-service set (`Protocol_Services_Supported`,
    /// Clause 12.11). For deployments whose dispatch surface differs from the
    /// bundled server's [`EXECUTED_SERVICES`].
    ///
    /// The production is closed at you-Are (bit 48); values past it are not
    /// representable and are dropped.
    pub fn set_services_supported(&mut self, services: &[ServiceSupported]) {
        self.configured_services_supported = services.to_vec();
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

    fn clock_frame(&self) -> Option<ClockFrame> {
        self.clock.as_ref()?.read_clock()
    }

    fn services_supported(&self) -> Vec<u8> {
        let clock_available = self.clock_frame().is_some();
        let services = self
            .configured_services_supported
            .iter()
            .copied()
            .filter(|service| {
                clock_available
                    || (*service != ServiceSupported::TIME_SYNCHRONIZATION
                        && *service != ServiceSupported::UTC_TIME_SYNCHRONIZATION)
            })
            .collect::<Vec<_>>();
        compute_services_supported(&services)
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

        if matches!(
            property,
            PropertyIdentifier::LOCAL_DATE
                | PropertyIdentifier::LOCAL_TIME
                | PropertyIdentifier::UTC_OFFSET
                | PropertyIdentifier::DAYLIGHT_SAVINGS_STATUS
        ) {
            let frame = self.clock_frame().ok_or(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            })?;
            return Ok(match property {
                PropertyIdentifier::LOCAL_DATE => PropertyValue::Date(frame.local_date),
                PropertyIdentifier::LOCAL_TIME => PropertyValue::Time(frame.local_time),
                PropertyIdentifier::UTC_OFFSET => {
                    PropertyValue::Signed(i32::from(frame.utc_offset))
                }
                PropertyIdentifier::DAYLIGHT_SAVINGS_STATUS => {
                    PropertyValue::Boolean(frame.daylight_savings_status)
                }
                _ => unreachable!(),
            });
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
            let services_supported = self.services_supported();
            let unused = (services_supported.len() * 8 - SERVICES_SUPPORTED_BITS) as u8;
            return Ok(PropertyValue::BitString {
                unused_bits: unused,
                data: services_supported,
            });
        }

        if property == PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS {
            let mut buf = bytes::BytesMut::new();
            bacnet_encoding::constructed::encode_cov_subscription_list(
                &mut buf,
                &self.active_cov_subscriptions,
            );
            return Ok(PropertyValue::ApplicationData(buf.to_vec()));
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
        if self.clock_frame().is_some() {
            props.extend([
                PropertyIdentifier::LOCAL_DATE,
                PropertyIdentifier::LOCAL_TIME,
                PropertyIdentifier::UTC_OFFSET,
                PropertyIdentifier::DAYLIGHT_SAVINGS_STATUS,
            ]);
        }
        props.sort_by_key(|p| p.to_raw());
        Cow::Owned(props)
    }

    /// Device is not createable or deleteable at runtime.
    fn is_createable(&self) -> bool {
        false
    }
    fn is_deleteable(&self) -> bool {
        false
    }

    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.clock = clock;
    }
}

#[cfg(test)]
mod tests;
