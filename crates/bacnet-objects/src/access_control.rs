//! Access Control objects (ASHRAE 135-2020 Clause 12).
//!
//! This module implements the seven BACnet access control object types:
//! - AccessDoor (type 30)
//! - AccessCredential (type 32)
//! - AccessPoint (type 33)
//! - AccessRights (type 34)
//! - AccessUser (type 35)
//! - AccessZone (type 36)
//! - CredentialDataInput (type 37)

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

// ---------------------------------------------------------------------------
// AccessDoorObject (type 30)
// ---------------------------------------------------------------------------

/// BACnet Access Door object (type 30).
///
/// Represents a physical door or barrier in an access control system.
/// Present value indicates the door command status (DoorStatus enumeration).
pub struct AccessDoorObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,    // DoorStatus: 0=closed, 1=opened, 2=unknown
    door_status: u32,      // DoorStatus enumeration
    lock_status: u32,      // LockStatus enumeration
    secured_status: u32,   // DoorSecuredStatus enumeration
    door_alarm_state: u32, // DoorAlarmState enumeration
    door_members: Vec<ObjectIdentifier>,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl AccessDoorObject {
    /// Create a new Access Door object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_DOOR, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0, // closed
            door_status: 0,   // closed
            lock_status: 0,
            secured_status: 0,
            door_alarm_state: 0,
            door_members: Vec::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }
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
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ACCESS_DOOR.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::DOOR_STATUS => {
                Ok(PropertyValue::Enumerated(self.door_status))
            }
            p if p == PropertyIdentifier::LOCK_STATUS => {
                Ok(PropertyValue::Enumerated(self.lock_status))
            }
            p if p == PropertyIdentifier::SECURED_STATUS => {
                Ok(PropertyValue::Enumerated(self.secured_status))
            }
            p if p == PropertyIdentifier::DOOR_ALARM_STATE => {
                Ok(PropertyValue::Enumerated(self.door_alarm_state))
            }
            p if p == PropertyIdentifier::DOOR_MEMBERS => Ok(PropertyValue::List(
                self.door_members
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .collect(),
            )),
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
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                if !self.out_of_service {
                    return Err(common::write_access_denied_error());
                }
                if let PropertyValue::Enumerated(v) = value {
                    self.present_value = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(common::write_access_denied_error()),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::DOOR_STATUS,
            PropertyIdentifier::LOCK_STATUS,
            PropertyIdentifier::SECURED_STATUS,
            PropertyIdentifier::DOOR_ALARM_STATE,
            PropertyIdentifier::DOOR_MEMBERS,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// AccessCredentialObject (type 32)
// ---------------------------------------------------------------------------

/// BACnet Access Credential object (type 32).
///
/// Represents a credential (card, fob, biometric, etc.) used for access control.
/// Present value indicates active/inactive status (BinaryPV).
pub struct AccessCredentialObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32, // BinaryPV: 0=inactive, 1=active
    credential_status: u32,
    assigned_access_rights_count: u32,
    authentication_factors: Vec<Vec<u8>>,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl AccessCredentialObject {
    /// Create a new Access Credential object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0, // inactive
            credential_status: 0,
            assigned_access_rights_count: 0,
            authentication_factors: Vec::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }
}

impl BACnetObject for AccessCredentialObject {
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
                ObjectType::ACCESS_CREDENTIAL.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::CREDENTIAL_STATUS => {
                Ok(PropertyValue::Enumerated(self.credential_status))
            }
            p if p == PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS => Ok(PropertyValue::Unsigned(
                self.assigned_access_rights_count as u64,
            )),
            p if p == PropertyIdentifier::AUTHENTICATION_FACTORS => Ok(PropertyValue::List(
                self.authentication_factors
                    .iter()
                    .map(|f| PropertyValue::OctetString(f.clone()))
                    .collect(),
            )),
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
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                if let PropertyValue::Enumerated(v) = value {
                    self.present_value = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::CREDENTIAL_STATUS => {
                if let PropertyValue::Enumerated(v) = value {
                    self.credential_status = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(common::write_access_denied_error()),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::CREDENTIAL_STATUS,
            PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS,
            PropertyIdentifier::AUTHENTICATION_FACTORS,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// AccessPointObject (type 33)
// ---------------------------------------------------------------------------

/// BACnet Access Point object (type 33).
///
/// Represents an access point (reader/controller at a door) in an access control system.
/// Present value indicates the most recent access event.
pub struct AccessPointObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32, // AccessEvent enumeration
    access_event: u32,
    access_event_tag: u64,
    access_event_time: ([u8; 4], [u8; 4]), // (Date, Time) as raw bytes
    access_doors: Vec<ObjectIdentifier>,
    event_state: u32,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl AccessPointObject {
    /// Create a new Access Point object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_POINT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            access_event: 0,
            access_event_tag: 0,
            access_event_time: ([0xFF, 0xFF, 0xFF, 0xFF], [0xFF, 0xFF, 0xFF, 0xFF]),
            access_doors: Vec::new(),
            event_state: 0,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }
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
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ACCESS_POINT.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::ACCESS_EVENT => {
                Ok(PropertyValue::Enumerated(self.access_event))
            }
            p if p == PropertyIdentifier::ACCESS_EVENT_TAG => {
                Ok(PropertyValue::Unsigned(self.access_event_tag))
            }
            p if p == PropertyIdentifier::ACCESS_EVENT_TIME => {
                let (d, t) = &self.access_event_time;
                Ok(PropertyValue::List(vec![
                    PropertyValue::Date(Date {
                        year: d[0],
                        month: d[1],
                        day: d[2],
                        day_of_week: d[3],
                    }),
                    PropertyValue::Time(Time {
                        hour: t[0],
                        minute: t[1],
                        second: t[2],
                        hundredths: t[3],
                    }),
                ]))
            }
            p if p == PropertyIdentifier::ACCESS_DOORS => Ok(PropertyValue::List(
                self.access_doors
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .collect(),
            )),
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
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                if let PropertyValue::Enumerated(v) = value {
                    self.present_value = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(common::write_access_denied_error()),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::ACCESS_EVENT,
            PropertyIdentifier::ACCESS_EVENT_TAG,
            PropertyIdentifier::ACCESS_EVENT_TIME,
            PropertyIdentifier::ACCESS_DOORS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// AccessRightsObject (type 34)
// ---------------------------------------------------------------------------

/// BACnet Access Rights object (type 34).
///
/// Defines a set of access rules (positive and negative) that can be
/// assigned to credentials and users.
pub struct AccessRightsObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    global_identifier: u64,
    positive_access_rules_count: u32,
    negative_access_rules_count: u32,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl AccessRightsObject {
    /// Create a new Access Rights object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_RIGHTS, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            global_identifier: 0,
            positive_access_rules_count: 0,
            negative_access_rules_count: 0,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }
}

impl BACnetObject for AccessRightsObject {
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
                ObjectType::ACCESS_RIGHTS.to_raw(),
            )),
            p if p == PropertyIdentifier::GLOBAL_IDENTIFIER => {
                Ok(PropertyValue::Unsigned(self.global_identifier))
            }
            p if p == PropertyIdentifier::POSITIVE_ACCESS_RULES => Ok(PropertyValue::Unsigned(
                self.positive_access_rules_count as u64,
            )),
            p if p == PropertyIdentifier::NEGATIVE_ACCESS_RULES => Ok(PropertyValue::Unsigned(
                self.negative_access_rules_count as u64,
            )),
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
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::GLOBAL_IDENTIFIER => {
                if let PropertyValue::Unsigned(v) = value {
                    self.global_identifier = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(common::write_access_denied_error()),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::GLOBAL_IDENTIFIER,
            PropertyIdentifier::POSITIVE_ACCESS_RULES,
            PropertyIdentifier::NEGATIVE_ACCESS_RULES,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// AccessUserObject (type 35)
// ---------------------------------------------------------------------------

/// BACnet Access User object (type 35).
///
/// Represents a person or entity that uses credentials to gain access.
/// Present value indicates the user type (AccessUserType enumeration).
pub struct AccessUserObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32, // AccessUserType enumeration
    user_type: u32,
    credentials: Vec<ObjectIdentifier>,
    assigned_access_rights_count: u32,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl AccessUserObject {
    /// Create a new Access User object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_USER, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            user_type: 0,
            credentials: Vec::new(),
            assigned_access_rights_count: 0,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }
}

impl BACnetObject for AccessUserObject {
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
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ACCESS_USER.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::USER_TYPE => {
                Ok(PropertyValue::Enumerated(self.user_type))
            }
            p if p == PropertyIdentifier::CREDENTIALS => Ok(PropertyValue::List(
                self.credentials
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .collect(),
            )),
            p if p == PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS => Ok(PropertyValue::Unsigned(
                self.assigned_access_rights_count as u64,
            )),
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
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                if let PropertyValue::Enumerated(v) = value {
                    self.present_value = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::USER_TYPE => {
                if let PropertyValue::Enumerated(v) = value {
                    self.user_type = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(common::write_access_denied_error()),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::USER_TYPE,
            PropertyIdentifier::CREDENTIALS,
            PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// AccessZoneObject (type 36)
// ---------------------------------------------------------------------------

/// BACnet Access Zone object (type 36).
///
/// Represents a physical zone or area controlled by access points.
/// Present value indicates the occupancy state (AccessZoneOccupancyState).
pub struct AccessZoneObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32, // AccessZoneOccupancyState enumeration
    global_identifier: u64,
    occupancy_count: u64,
    access_doors: Vec<ObjectIdentifier>,
    entry_points: Vec<ObjectIdentifier>,
    exit_points: Vec<ObjectIdentifier>,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl AccessZoneObject {
    /// Create a new Access Zone object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_ZONE, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            global_identifier: 0,
            occupancy_count: 0,
            access_doors: Vec::new(),
            entry_points: Vec::new(),
            exit_points: Vec::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }
}

impl BACnetObject for AccessZoneObject {
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
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ACCESS_ZONE.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::GLOBAL_IDENTIFIER => {
                Ok(PropertyValue::Unsigned(self.global_identifier))
            }
            p if p == PropertyIdentifier::OCCUPANCY_COUNT => {
                Ok(PropertyValue::Unsigned(self.occupancy_count))
            }
            p if p == PropertyIdentifier::ACCESS_DOORS => Ok(PropertyValue::List(
                self.access_doors
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .collect(),
            )),
            p if p == PropertyIdentifier::ENTRY_POINTS => Ok(PropertyValue::List(
                self.entry_points
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .collect(),
            )),
            p if p == PropertyIdentifier::EXIT_POINTS => Ok(PropertyValue::List(
                self.exit_points
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .collect(),
            )),
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
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                if let PropertyValue::Enumerated(v) = value {
                    self.present_value = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::GLOBAL_IDENTIFIER => {
                if let PropertyValue::Unsigned(v) = value {
                    self.global_identifier = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(common::write_access_denied_error()),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::GLOBAL_IDENTIFIER,
            PropertyIdentifier::OCCUPANCY_COUNT,
            PropertyIdentifier::ACCESS_DOORS,
            PropertyIdentifier::ENTRY_POINTS,
            PropertyIdentifier::EXIT_POINTS,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// CredentialDataInputObject (type 37)
// ---------------------------------------------------------------------------

/// BACnet Credential Data Input object (type 37).
///
/// Represents a credential reader device (card reader, biometric scanner, etc.).
/// Present value indicates the authentication status.
pub struct CredentialDataInputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,              // AuthenticationStatus: 0=notReady, 1=waiting
    update_time: ([u8; 4], [u8; 4]), // (Date, Time) as raw bytes
    supported_formats: Vec<u64>,
    supported_format_classes: Vec<u64>,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl CredentialDataInputObject {
    /// Create a new Credential Data Input object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::CREDENTIAL_DATA_INPUT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0, // notReady
            update_time: ([0xFF, 0xFF, 0xFF, 0xFF], [0xFF, 0xFF, 0xFF, 0xFF]),
            supported_formats: Vec::new(),
            supported_format_classes: Vec::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }
}

impl BACnetObject for CredentialDataInputObject {
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
                ObjectType::CREDENTIAL_DATA_INPUT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::UPDATE_TIME => {
                let (d, t) = &self.update_time;
                Ok(PropertyValue::List(vec![
                    PropertyValue::Date(Date {
                        year: d[0],
                        month: d[1],
                        day: d[2],
                        day_of_week: d[3],
                    }),
                    PropertyValue::Time(Time {
                        hour: t[0],
                        minute: t[1],
                        second: t[2],
                        hundredths: t[3],
                    }),
                ]))
            }
            p if p == PropertyIdentifier::SUPPORTED_FORMATS => Ok(PropertyValue::List(
                self.supported_formats
                    .iter()
                    .map(|v| PropertyValue::Unsigned(*v))
                    .collect(),
            )),
            p if p == PropertyIdentifier::SUPPORTED_FORMAT_CLASSES => Ok(PropertyValue::List(
                self.supported_format_classes
                    .iter()
                    .map(|v| PropertyValue::Unsigned(*v))
                    .collect(),
            )),
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
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        // CredentialDataInput is primarily read-only (driven by hardware)
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::UPDATE_TIME,
            PropertyIdentifier::SUPPORTED_FORMATS,
            PropertyIdentifier::SUPPORTED_FORMAT_CLASSES,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- AccessDoorObject ---

    #[test]
    fn access_door_create_and_read_defaults() {
        let door = AccessDoorObject::new(1, "DOOR-1").unwrap();
        assert_eq!(door.object_name(), "DOOR-1");
        assert_eq!(
            door.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0) // closed
        );
    }

    #[test]
    fn access_door_object_type() {
        let door = AccessDoorObject::new(1, "DOOR-1").unwrap();
        assert_eq!(
            door.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::ACCESS_DOOR.to_raw())
        );
    }

    #[test]
    fn access_door_property_list() {
        let door = AccessDoorObject::new(1, "DOOR-1").unwrap();
        let list = door.property_list();
        assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(list.contains(&PropertyIdentifier::DOOR_STATUS));
        assert!(list.contains(&PropertyIdentifier::LOCK_STATUS));
        assert!(list.contains(&PropertyIdentifier::SECURED_STATUS));
        assert!(list.contains(&PropertyIdentifier::DOOR_ALARM_STATE));
        assert!(list.contains(&PropertyIdentifier::DOOR_MEMBERS));
    }

    #[test]
    fn access_door_read_door_members_empty() {
        let door = AccessDoorObject::new(1, "DOOR-1").unwrap();
        assert_eq!(
            door.read_property(PropertyIdentifier::DOOR_MEMBERS, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
    }

    #[test]
    fn access_door_write_present_value() {
        let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
        // Must be out-of-service to write present value
        door.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        door.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1), // opened
            None,
        )
        .unwrap();
        assert_eq!(
            door.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(1)
        );
    }

    #[test]
    fn access_door_write_present_value_not_out_of_service() {
        let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
        // Writing present value when not out-of-service should fail
        let result = door.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn access_door_write_present_value_wrong_type() {
        let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
        door.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        let result = door.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(1.0),
            None,
        );
        assert!(result.is_err());
    }

    // --- AccessCredentialObject ---

    #[test]
    fn access_credential_create_and_read_defaults() {
        let cred = AccessCredentialObject::new(1, "CRED-1").unwrap();
        assert_eq!(cred.object_name(), "CRED-1");
        assert_eq!(
            cred.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0) // inactive
        );
    }

    #[test]
    fn access_credential_object_type() {
        let cred = AccessCredentialObject::new(1, "CRED-1").unwrap();
        assert_eq!(
            cred.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::ACCESS_CREDENTIAL.to_raw())
        );
    }

    #[test]
    fn access_credential_property_list() {
        let cred = AccessCredentialObject::new(1, "CRED-1").unwrap();
        let list = cred.property_list();
        assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(list.contains(&PropertyIdentifier::CREDENTIAL_STATUS));
        assert!(list.contains(&PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS));
        assert!(list.contains(&PropertyIdentifier::AUTHENTICATION_FACTORS));
    }

    #[test]
    fn access_credential_read_assigned_access_rights() {
        let cred = AccessCredentialObject::new(1, "CRED-1").unwrap();
        assert_eq!(
            cred.read_property(PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
    }

    #[test]
    fn access_credential_read_authentication_factors() {
        let cred = AccessCredentialObject::new(1, "CRED-1").unwrap();
        assert_eq!(
            cred.read_property(PropertyIdentifier::AUTHENTICATION_FACTORS, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
    }

    // --- AccessPointObject ---

    #[test]
    fn access_point_create_and_read_defaults() {
        let point = AccessPointObject::new(1, "AP-1").unwrap();
        assert_eq!(point.object_name(), "AP-1");
        assert_eq!(
            point
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0)
        );
    }

    #[test]
    fn access_point_object_type() {
        let point = AccessPointObject::new(1, "AP-1").unwrap();
        assert_eq!(
            point
                .read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::ACCESS_POINT.to_raw())
        );
    }

    #[test]
    fn access_point_property_list() {
        let point = AccessPointObject::new(1, "AP-1").unwrap();
        let list = point.property_list();
        assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(list.contains(&PropertyIdentifier::ACCESS_EVENT));
        assert!(list.contains(&PropertyIdentifier::ACCESS_EVENT_TAG));
        assert!(list.contains(&PropertyIdentifier::ACCESS_EVENT_TIME));
        assert!(list.contains(&PropertyIdentifier::ACCESS_DOORS));
        assert!(list.contains(&PropertyIdentifier::EVENT_STATE));
    }

    #[test]
    fn access_point_read_access_event_time() {
        let point = AccessPointObject::new(1, "AP-1").unwrap();
        let val = point
            .read_property(PropertyIdentifier::ACCESS_EVENT_TIME, None)
            .unwrap();
        match val {
            PropertyValue::List(items) => {
                assert_eq!(items.len(), 2);
            }
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn access_point_read_access_doors_empty() {
        let point = AccessPointObject::new(1, "AP-1").unwrap();
        assert_eq!(
            point
                .read_property(PropertyIdentifier::ACCESS_DOORS, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
    }

    // --- AccessRightsObject ---

    #[test]
    fn access_rights_create_and_read_defaults() {
        let rights = AccessRightsObject::new(1, "AR-1").unwrap();
        assert_eq!(rights.object_name(), "AR-1");
        assert_eq!(
            rights
                .read_property(PropertyIdentifier::GLOBAL_IDENTIFIER, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
    }

    #[test]
    fn access_rights_object_type() {
        let rights = AccessRightsObject::new(1, "AR-1").unwrap();
        assert_eq!(
            rights
                .read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::ACCESS_RIGHTS.to_raw())
        );
    }

    #[test]
    fn access_rights_property_list() {
        let rights = AccessRightsObject::new(1, "AR-1").unwrap();
        let list = rights.property_list();
        assert!(list.contains(&PropertyIdentifier::GLOBAL_IDENTIFIER));
        assert!(list.contains(&PropertyIdentifier::POSITIVE_ACCESS_RULES));
        assert!(list.contains(&PropertyIdentifier::NEGATIVE_ACCESS_RULES));
    }

    #[test]
    fn access_rights_read_rules_counts() {
        let rights = AccessRightsObject::new(1, "AR-1").unwrap();
        assert_eq!(
            rights
                .read_property(PropertyIdentifier::POSITIVE_ACCESS_RULES, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            rights
                .read_property(PropertyIdentifier::NEGATIVE_ACCESS_RULES, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
    }

    #[test]
    fn access_rights_write_global_identifier() {
        let mut rights = AccessRightsObject::new(1, "AR-1").unwrap();
        rights
            .write_property(
                PropertyIdentifier::GLOBAL_IDENTIFIER,
                None,
                PropertyValue::Unsigned(42),
                None,
            )
            .unwrap();
        assert_eq!(
            rights
                .read_property(PropertyIdentifier::GLOBAL_IDENTIFIER, None)
                .unwrap(),
            PropertyValue::Unsigned(42)
        );
    }

    // --- AccessUserObject ---

    #[test]
    fn access_user_create_and_read_defaults() {
        let user = AccessUserObject::new(1, "USER-1").unwrap();
        assert_eq!(user.object_name(), "USER-1");
        assert_eq!(
            user.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0)
        );
    }

    #[test]
    fn access_user_object_type() {
        let user = AccessUserObject::new(1, "USER-1").unwrap();
        assert_eq!(
            user.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::ACCESS_USER.to_raw())
        );
    }

    #[test]
    fn access_user_property_list() {
        let user = AccessUserObject::new(1, "USER-1").unwrap();
        let list = user.property_list();
        assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(list.contains(&PropertyIdentifier::USER_TYPE));
        assert!(list.contains(&PropertyIdentifier::CREDENTIALS));
        assert!(list.contains(&PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS));
    }

    #[test]
    fn access_user_read_credentials_empty() {
        let user = AccessUserObject::new(1, "USER-1").unwrap();
        assert_eq!(
            user.read_property(PropertyIdentifier::CREDENTIALS, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
    }

    #[test]
    fn access_user_write_user_type() {
        let mut user = AccessUserObject::new(1, "USER-1").unwrap();
        user.write_property(
            PropertyIdentifier::USER_TYPE,
            None,
            PropertyValue::Enumerated(1),
            None,
        )
        .unwrap();
        assert_eq!(
            user.read_property(PropertyIdentifier::USER_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(1)
        );
    }

    // --- AccessZoneObject ---

    #[test]
    fn access_zone_create_and_read_defaults() {
        let zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
        assert_eq!(zone.object_name(), "ZONE-1");
        assert_eq!(
            zone.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0)
        );
    }

    #[test]
    fn access_zone_object_type() {
        let zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
        assert_eq!(
            zone.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::ACCESS_ZONE.to_raw())
        );
    }

    #[test]
    fn access_zone_property_list() {
        let zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
        let list = zone.property_list();
        assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(list.contains(&PropertyIdentifier::GLOBAL_IDENTIFIER));
        assert!(list.contains(&PropertyIdentifier::OCCUPANCY_COUNT));
        assert!(list.contains(&PropertyIdentifier::ACCESS_DOORS));
        assert!(list.contains(&PropertyIdentifier::ENTRY_POINTS));
        assert!(list.contains(&PropertyIdentifier::EXIT_POINTS));
    }

    #[test]
    fn access_zone_read_lists_empty() {
        let zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
        assert_eq!(
            zone.read_property(PropertyIdentifier::ACCESS_DOORS, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
        assert_eq!(
            zone.read_property(PropertyIdentifier::ENTRY_POINTS, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
        assert_eq!(
            zone.read_property(PropertyIdentifier::EXIT_POINTS, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
    }

    #[test]
    fn access_zone_read_occupancy_count() {
        let zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
        assert_eq!(
            zone.read_property(PropertyIdentifier::OCCUPANCY_COUNT, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
    }

    #[test]
    fn access_zone_write_global_identifier() {
        let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
        zone.write_property(
            PropertyIdentifier::GLOBAL_IDENTIFIER,
            None,
            PropertyValue::Unsigned(99),
            None,
        )
        .unwrap();
        assert_eq!(
            zone.read_property(PropertyIdentifier::GLOBAL_IDENTIFIER, None)
                .unwrap(),
            PropertyValue::Unsigned(99)
        );
    }

    // --- CredentialDataInputObject ---

    #[test]
    fn credential_data_input_create_and_read_defaults() {
        let cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
        assert_eq!(cdi.object_name(), "CDI-1");
        assert_eq!(
            cdi.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0) // notReady
        );
    }

    #[test]
    fn credential_data_input_object_type() {
        let cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
        assert_eq!(
            cdi.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::CREDENTIAL_DATA_INPUT.to_raw())
        );
    }

    #[test]
    fn credential_data_input_property_list() {
        let cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
        let list = cdi.property_list();
        assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(list.contains(&PropertyIdentifier::UPDATE_TIME));
        assert!(list.contains(&PropertyIdentifier::SUPPORTED_FORMATS));
        assert!(list.contains(&PropertyIdentifier::SUPPORTED_FORMAT_CLASSES));
    }

    #[test]
    fn credential_data_input_read_update_time() {
        let cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
        let val = cdi
            .read_property(PropertyIdentifier::UPDATE_TIME, None)
            .unwrap();
        match val {
            PropertyValue::List(items) => {
                assert_eq!(items.len(), 2);
            }
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn credential_data_input_read_supported_formats_empty() {
        let cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
        assert_eq!(
            cdi.read_property(PropertyIdentifier::SUPPORTED_FORMATS, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
    }

    #[test]
    fn credential_data_input_write_denied() {
        let mut cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
        let result = cdi.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            None,
        );
        assert!(result.is_err());
    }
}
