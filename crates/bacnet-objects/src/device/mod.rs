//! Device object (type 8) per ASHRAE 135-2020 Clause 12.11.
//!
//! The Device object is required in every BACnet device and exposes
//! device-level properties such as vendor info, protocol support,
//! and configuration parameters.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use bacnet_encoding::primitives::encode_timestamp_choice;
use bacnet_types::enums::{
    BackupAndRestoreState, DeviceStatus, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
    Segmentation, ServiceSupported,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

use crate::clock::{ClockFrame, ClockReader, UNSPECIFIED_DATETIME};
use crate::common::{
    invalid_data_type_error, property_is_not_an_array_error, read_array,
    read_property_list_property, value_out_of_range_error,
};
use crate::object_profile::{ObjectProfile, ProfileState, TagsPersistence};
use crate::traits::BACnetObject;

mod audit_recipient;
mod metadata;
pub use audit_recipient::{AuditRecipientChangeSink, AuditWriteSource, DeviceAuthority};

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
    // Received notifications go to the Notification Forwarder objects
    // (Clause 12.51); the server acknowledges each confirmed one.
    ServiceSupported::CONFIRMED_EVENT_NOTIFICATION,
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
    ServiceSupported::UNCONFIRMED_EVENT_NOTIFICATION,
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
    // Applied to the local Channel objects (Clause 15.11).
    ServiceSupported::WRITE_GROUP,
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
    /// One source for Device identity and runtime declarations.
    pub identity: crate::DeviceIdentity,
    /// Vendor name string.
    pub vendor_name: String,
    /// Model name string.
    pub model_name: String,
    /// Firmware revision string.
    pub firmware_revision: String,
    /// Application software version string.
    pub application_software_version: String,
    /// APDU timeout in milliseconds.
    pub apdu_timeout: u32,
    /// Number of APDU retries.
    pub apdu_retries: u32,
    /// Whether the device performs the backup and restore procedures (Clause 19.1), which
    /// adds Configuration_Files, Last_Restore_Time, Backup_Failure_Timeout and
    /// Backup_And_Restore_State. The application carries out the procedures themselves.
    pub backup_and_restore: bool,
    /// Initial Backup_Failure_Timeout in seconds. Only served with `backup_and_restore`.
    pub backup_failure_timeout: u16,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            identity: crate::DeviceIdentity::default(),
            vendor_name: "Rusty BACnet".into(),
            model_name: "rusty-bacnet".into(),
            firmware_revision: "0.1.0".into(),
            application_software_version: "0.1.0".into(),
            apdu_timeout: 6000,
            apdu_retries: 3,
            backup_and_restore: false,
            backup_failure_timeout: 60,
        }
    }
}

fn timestamp_value(stamp: &BACnetTimeStamp) -> PropertyValue {
    let mut encoded = BytesMut::new();
    encode_timestamp_choice(&mut encoded, stamp)
        .expect("BACnetTimeStamp CHOICE encoding is infallible");
    PropertyValue::ApplicationData(encoded.to_vec())
}

/// BACnet Device object.
pub struct DeviceObject {
    oid: ObjectIdentifier,
    recipient: audit_recipient::RecipientState,
    properties: HashMap<PropertyIdentifier, PropertyValue>,
    profile: ProfileState,
    /// Cached object list for array-indexed reads.
    object_list: Vec<ObjectIdentifier>,
    /// Protocol_Object_Types_Supported — bitstring indicating which object
    /// types this device supports (one bit per type, MSB-first within each byte).
    protocol_object_types_supported: Vec<u8>,
    /// Configured executed services before clock-availability filtering.
    configured_services_supported: Vec<ServiceSupported>,
    /// Shared dynamic clock sample source. `None` is explicitly clockless.
    clock: Option<Arc<dyn ClockReader>>,
}

impl DeviceObject {
    /// Create a new Device object from configuration.
    pub fn new(config: DeviceConfig) -> Result<Self, Error> {
        config.identity.validate()?;
        let oid = config.identity.device_oid();
        let mut properties = HashMap::new();

        properties.insert(
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyValue::ObjectIdentifier(oid),
        );
        properties.insert(
            PropertyIdentifier::OBJECT_NAME,
            PropertyValue::CharacterString(config.identity.name().to_owned()),
        );
        properties.insert(
            PropertyIdentifier::OBJECT_TYPE,
            PropertyValue::Enumerated(ObjectType::DEVICE.to_raw()),
        );
        properties.insert(
            PropertyIdentifier::SYSTEM_STATUS,
            PropertyValue::Enumerated(DeviceStatus::OPERATIONAL.to_raw()),
        );
        properties.insert(
            PropertyIdentifier::VENDOR_NAME,
            PropertyValue::CharacterString(config.vendor_name),
        );
        properties.insert(
            PropertyIdentifier::VENDOR_IDENTIFIER,
            PropertyValue::Unsigned(config.identity.vendor_id() as u64),
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
            PropertyValue::Unsigned(config.identity.max_apdu_length() as u64),
        );
        properties.insert(
            PropertyIdentifier::SEGMENTATION_SUPPORTED,
            PropertyValue::Enumerated(config.identity.segmentation().to_raw() as u32),
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

        // Device_Address_Binding: stored empty. A running server serves its
        // device bindings in place of this value (bacnet-server, #1369).
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
            PropertyValue::OctetString(config.identity.device_uuid().to_vec()),
        );

        // Table 12-13 footnotes 7 and 8: present only on a device that performs the backup
        // and restore procedures. The application owns the File objects Configuration_Files
        // names, so the list starts empty.
        if config.backup_and_restore {
            if config.backup_failure_timeout == 0 {
                return Err(Error::Encoding(
                    "Backup_Failure_Timeout must be positive when backup and restore is on".into(),
                ));
            }
            properties.insert(
                PropertyIdentifier::CONFIGURATION_FILES,
                PropertyValue::List(Vec::new()),
            );
            // No restore has happened yet.
            let (date, time) = UNSPECIFIED_DATETIME;
            properties.insert(
                PropertyIdentifier::LAST_RESTORE_TIME,
                timestamp_value(&BACnetTimeStamp::DateTime { date, time }),
            );
            properties.insert(
                PropertyIdentifier::BACKUP_FAILURE_TIMEOUT,
                PropertyValue::Unsigned(u64::from(config.backup_failure_timeout)),
            );
            properties.insert(
                PropertyIdentifier::BACKUP_AND_RESTORE_STATE,
                PropertyValue::Enumerated(BackupAndRestoreState::IDLE.to_raw()),
            );
        }

        // Max_Segments_Accepted — only included when segmentation is supported.
        if config.identity.segmentation() != Segmentation::NONE {
            if config.identity.apdu_segment_timeout_ms() == 0 {
                return Err(Error::Encoding(
                    "APDU_Segment_Timeout must be positive when segmentation is supported".into(),
                ));
            }
            properties.insert(
                PropertyIdentifier::APDU_SEGMENT_TIMEOUT,
                PropertyValue::Unsigned(config.identity.apdu_segment_timeout_ms()),
            );
            let max_segments_accepted = if config.identity.segmentation() == Segmentation::TRANSMIT
            {
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
            ObjectType::AUDIT_LOG.to_raw(),
            ObjectType::AUDIT_REPORTER.to_raw(),
            ObjectType::COLOR.to_raw(),
            ObjectType::COLOR_TEMPERATURE.to_raw(),
        ]);

        Ok(Self {
            oid,
            recipient: Default::default(),
            profile: ProfileState::default(),
            properties,
            object_list: vec![oid], // Device itself is always in the list
            protocol_object_types_supported,
            configured_services_supported: EXECUTED_SERVICES.to_vec(),
            clock: None,
        })
    }

    /// Build with application-owned Tags storage. Saved Tags override configured
    /// Tags only while [`Self::set_profile`] provisions the row. Loaded data must
    /// satisfy the shared Tags rules and opt-in 1 MiB snapshot limit.
    pub fn with_tags_persistence(
        config: DeviceConfig,
        persistence: Arc<dyn TagsPersistence>,
    ) -> Result<Self, Error> {
        let mut object = Self::new(config)?;
        object.profile = ProfileState::persistent(object.oid, persistence)?;
        Ok(object)
    }

    /// Provision independent optional Tags, Profile_Location and Profile_Name
    /// rows before registration. Ordinary servers permit Tags writes; an endpoint
    /// still applies its narrower Device write allowlist. The profile text is
    /// network read-only. Invalid configuration leaves the old profile unchanged.
    pub fn set_profile(&mut self, profile: ObjectProfile) -> Result<(), Error> {
        self.profile.provision(profile)
    }

    /// Wait for queued save attempts; each write reports its own outcome.
    /// This is not a success receipt. Unstaged synchronous writes block their caller.
    pub fn wait_for_tag_saves(&self) {
        self.profile.wait_for_saves();
    }

    /// Update the object-list with the current database contents.
    pub fn set_object_list(&mut self, oids: Vec<ObjectIdentifier>) {
        self.object_list = oids;
    }

    /// Set `System_Status`, the device's reported operating state.
    pub fn set_system_status(&mut self, status: DeviceStatus) {
        self.properties.insert(
            PropertyIdentifier::SYSTEM_STATUS,
            PropertyValue::Enumerated(status.to_raw()),
        );
    }

    /// Set `Database_Revision`, which a client compares against its cached copy to see whether
    /// objects were created, deleted, renamed or re-identified, or a restore happened.
    pub fn set_database_revision(&mut self, revision: u32) {
        self.properties.insert(
            PropertyIdentifier::DATABASE_REVISION,
            PropertyValue::Unsigned(u64::from(revision)),
        );
    }

    /// Name the File objects that hold this device's configuration (`Configuration_Files`).
    /// Ignored unless the device was built with [`DeviceConfig::backup_and_restore`].
    pub fn set_configuration_files(&mut self, files: Vec<ObjectIdentifier>) {
        self.set_backup_property(
            PropertyIdentifier::CONFIGURATION_FILES,
            PropertyValue::List(
                files
                    .into_iter()
                    .map(PropertyValue::ObjectIdentifier)
                    .collect(),
            ),
        );
    }

    /// Set `Backup_And_Restore_State`, where the device is in a backup or restore.
    /// Ignored unless the device was built with [`DeviceConfig::backup_and_restore`].
    pub fn set_backup_and_restore_state(&mut self, state: BackupAndRestoreState) {
        self.set_backup_property(
            PropertyIdentifier::BACKUP_AND_RESTORE_STATE,
            PropertyValue::Enumerated(state.to_raw()),
        );
    }

    /// Set `Last_Restore_Time`, when the last restore completed.
    /// Ignored unless the device was built with [`DeviceConfig::backup_and_restore`].
    pub fn set_last_restore_time(&mut self, time: BACnetTimeStamp) {
        self.set_backup_property(
            PropertyIdentifier::LAST_RESTORE_TIME,
            timestamp_value(&time),
        );
    }

    fn set_backup_property(&mut self, property: PropertyIdentifier, value: PropertyValue) {
        if let Some(current) = self.properties.get_mut(&property) {
            *current = value;
        }
    }

    fn write_backup_failure_timeout(
        &mut self,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        if array_index.is_some() {
            return Err(property_is_not_an_array_error());
        }
        // Clause 15.9: relinquishing a non-commandable writable property
        // succeeds without changing its value when no other error exists.
        match value {
            PropertyValue::Null => Ok(()),
            PropertyValue::Unsigned(seconds) if (1..=u64::from(u16::MAX)).contains(&seconds) => {
                self.properties.insert(
                    PropertyIdentifier::BACKUP_FAILURE_TIMEOUT,
                    PropertyValue::Unsigned(seconds),
                );
                Ok(())
            }
            PropertyValue::Unsigned(_) => Err(value_out_of_range_error()),
            _ => Err(invalid_data_type_error()),
        }
    }

    /// Replace the advertised executed-service set (`Protocol_Services_Supported`,
    /// Clause 12.11). For deployments whose dispatch surface differs from the
    /// bundled server's [`EXECUTED_SERVICES`]. This is a standalone declaration,
    /// not a runtime dispatch switch. Raw COV presence follows these services;
    /// a running server overrides served Device data with its own execution view.
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

    /// Sync the durable Device UUID (Clause 12.11 DEVICE_UUID).
    ///
    /// RB-16 endpoint sync (not a fork): the caller owns the 16-byte UUID
    /// durably (Annex AB.1.5.3); this setter only stores the supplied bytes
    /// as an OctetString. No generation, no persistence, no version/variant
    /// checks — startup validation elsewhere rejects only all-zero where a
    /// real SC identity is required.
    pub fn set_device_uuid(&mut self, uuid: [u8; 16]) {
        self.properties.insert(
            PropertyIdentifier::DEVICE_UUID,
            PropertyValue::OctetString(uuid.to_vec()),
        );
    }

    fn clock_frame(&self) -> Option<ClockFrame> {
        self.clock.as_ref()?.read_clock()
    }

    fn cov_property_present(&self, property: PropertyIdentifier) -> bool {
        match property {
            PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS => {
                self.configured_services_supported.iter().any(|service| {
                    matches!(
                        *service,
                        ServiceSupported::SUBSCRIBE_COV | ServiceSupported::SUBSCRIBE_COV_PROPERTY
                    )
                })
            }
            PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS => self
                .configured_services_supported
                .contains(&ServiceSupported::SUBSCRIBE_COV_PROPERTY_MULTIPLE),
            _ => true,
        }
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
    fn device_authority_internal(&mut self) -> Option<DeviceAuthority<'_>> {
        Some(DeviceAuthority(self))
    }

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
        if property == PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT {
            return self.read_audit_recipient(array_index);
        }
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
                            class: ErrorClass::PROPERTY,
                            code: ErrorCode::INVALID_ARRAY_INDEX,
                        })
                    }
                }
            };
        }

        if property == PropertyIdentifier::PROPERTY_LIST {
            return read_property_list_property(&self.property_list(), array_index);
        }

        if property == PropertyIdentifier::CONFIGURATION_FILES {
            if let Some(PropertyValue::List(files)) = self.properties.get(&property) {
                return read_array(files.clone(), array_index);
            }
        }

        if matches!(
            property,
            PropertyIdentifier::LOCAL_DATE
                | PropertyIdentifier::LOCAL_TIME
                | PropertyIdentifier::UTC_OFFSET
                | PropertyIdentifier::DAYLIGHT_SAVINGS_STATUS
        ) {
            let frame = self.clock_frame().ok_or(Error::Protocol {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::UNKNOWN_PROPERTY,
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

        if matches!(
            property,
            PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS
                | PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS
        ) {
            // Standalone default: the object holds no subscription state. A
            // running `BACnetServer` owns both live lists in its COV
            // subscription table and projects them for network reads and
            // `read_local`.
            return if self.cov_property_present(property) {
                Ok(PropertyValue::ApplicationData(Vec::new()))
            } else {
                Err(Error::Protocol {
                    class: ErrorClass::PROPERTY,
                    code: ErrorCode::UNKNOWN_PROPERTY,
                })
            };
        }

        if let Some(value) = self.profile.read(property, array_index) {
            return value;
        }
        self.properties
            .get(&property)
            .cloned()
            .ok_or(Error::Protocol {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::UNKNOWN_PROPERTY,
            })
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT {
            return self.write_audit_recipient(array_index, value, _priority, None);
        }
        if let Some(result) = self.profile.write(property, array_index, &value) {
            return result;
        }
        if property == PropertyIdentifier::DESCRIPTION {
            if array_index.is_some() {
                return Err(Error::Protocol {
                    class: ErrorClass::PROPERTY,
                    code: ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
                });
            }
            // Clause 15.9: relinquishing a non-commandable writable property
            // succeeds without changing its value when no other error exists.
            if value == PropertyValue::Null {
                return Ok(());
            }
            if let PropertyValue::CharacterString(_) = &value {
                self.properties.insert(property, value);
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::INVALID_DATA_TYPE,
            });
        }
        if property == PropertyIdentifier::BACKUP_FAILURE_TIMEOUT
            && self.properties.contains_key(&property)
        {
            return self.write_backup_failure_timeout(array_index, value);
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            array_index,
        ))
    }

    fn durable_writes_internal(&mut self) -> Option<&mut dyn crate::durable::DurableWrites> {
        self.profile.capability()
    }

    fn advance_monotonic_time_internal(&mut self, now: std::time::Duration) -> bool {
        self.profile.expire(now);
        false
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        // Preserve Device's legacy inclusion of PROPERTY_LIST. The wire reader
        // still removes it and the identity rows as before.
        Cow::Owned(
            self.property_metadata()
                .iter()
                .map(|row| row.property_identifier)
                .collect(),
        )
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

#[cfg(test)]
mod service_profile_tests;
