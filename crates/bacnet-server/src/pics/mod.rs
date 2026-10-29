//! Protocol Implementation Conformance Statement (PICS) generation per ASHRAE 135-2020 Annex A.
//!
//! A PICS document is a formal declaration of which BACnet features a device supports.
//! It is required for BACnet certification and interoperability testing.

use std::collections::BTreeMap;
use std::fmt;

use bacnet_encoding::primitives::charset;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::ServicesSupported;
use bacnet_types::enums::{ObjectType, PropertyIdentifier, ServiceSupported};
use bacnet_types::primitives::PropertyValue;
use bacnet_types::{PROTOCOL_REVISION, PROTOCOL_VERSION};

use crate::local_device::announcement_device;
use crate::server::ServerConfig;

// ───────────────────────────── Data model ──────────────────────────────────

/// Complete PICS document per ASHRAE 135-2020 Annex A.
#[derive(Debug, Clone)]
pub struct Pics {
    /// Vendor and device identification.
    pub vendor_info: VendorInfo,
    /// Device profile claimed.
    pub device_profile: DeviceProfile,
    /// Object types present in the database with their creation, deletion and property support.
    pub supported_object_types: Vec<ObjectTypeSupport>,
    /// Services the server can initiate or execute.
    pub supported_services: Vec<ServiceSupport>,
    /// Data-link layers the device supports.
    pub data_link_layers: Vec<DataLinkSupport>,
    /// Network-layer roles the device fills.
    pub network_layer: NetworkLayerSupport,
    /// Character sets the device supports.
    pub character_sets: Vec<CharacterSet>,
    /// Free-text descriptions of special functionality.
    pub special_functionality: Vec<String>,
}

/// Vendor and device identification.
#[derive(Debug, Clone)]
pub struct VendorInfo {
    /// Vendor identifier assigned by ASHRAE: the Device object's
    /// Vendor_Identifier (see [`PicsGenerator::generate`]).
    pub vendor_id: u16,
    /// The Device object's Vendor_Name, as read (see [`PicsGenerator::generate`]).
    pub vendor_name: String,
    /// The Device object's Model_Name, as read.
    pub model_name: String,
    /// The Device object's Firmware_Revision, as read.
    pub firmware_revision: String,
    /// The Device object's Application_Software_Version, as read.
    pub application_software_version: String,
    /// BACnet protocol version implemented: the Device object's
    /// Protocol_Version (see [`PicsGenerator::generate`]).
    pub protocol_version: u16,
    /// BACnet protocol revision implemented: the Device object's
    /// Protocol_Revision (see [`PicsGenerator::generate`]).
    pub protocol_revision: u16,
}

/// BACnet device profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceProfile {
    /// BACnet Advanced Application Controller.
    BAac,
    /// BACnet Application Specific Controller.
    BAsc,
    /// BACnet Operator Workstation.
    BOws,
    /// BACnet Building Controller.
    BBc,
    /// BACnet Operator Panel.
    BOp,
    /// BACnet Router.
    BRouter,
    /// BACnet Gateway.
    BGw,
    /// BACnet Smart Controller.
    BSc,
    /// Custom / non-standard profile.
    Custom(String),
}

impl fmt::Display for DeviceProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BAac => f.write_str("B-AAC"),
            Self::BAsc => f.write_str("B-ASC"),
            Self::BOws => f.write_str("B-OWS"),
            Self::BBc => f.write_str("B-BC"),
            Self::BOp => f.write_str("B-OP"),
            Self::BRouter => f.write_str("B-ROUTER"),
            Self::BGw => f.write_str("B-GW"),
            Self::BSc => f.write_str("B-SC"),
            Self::Custom(s) => f.write_str(s),
        }
    }
}

/// Property capabilities across configured instances of an object type.
/// Read/write flags mean at least one instance supports that access. Optional is
/// true only when every present metadata row is optional; absent rows do not vote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropertyAccess {
    /// At least one instance can be read.
    pub readable: bool,
    /// At least one instance can be written.
    pub writable: bool,
    /// Every instance that has the property treats it as optional.
    pub optional: bool,
}

impl fmt::Display for PropertyAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = if self.readable { "R" } else { "" };
        let w = if self.writable { "W" } else { "" };
        let o = if self.optional { "O" } else { "" };
        write!(f, "{r}{w}{o}")
    }
}

/// Supported property with its access flags.
#[derive(Debug, Clone)]
pub struct PropertySupport {
    /// Property this row describes.
    pub property_id: PropertyIdentifier,
    /// Aggregated access flags for the property.
    pub access: PropertyAccess,
    /// The property whose writes change this one on some instance, although
    /// a write naming this one is refused: State_Text, written whole or at
    /// index 0, for a multi-state object's Number_Of_States (#1443).
    pub written_through: Option<PropertyIdentifier>,
}

/// Object type support declaration.
#[derive(Debug, Clone)]
pub struct ObjectTypeSupport {
    /// Object type described.
    pub object_type: ObjectType,
    /// Whether the type can be created remotely with CreateObject.
    pub createable: bool,
    /// The properties a CreateObject initial value may set whole on a type
    /// that is createable, although WriteProperty can't change them later
    /// (#1429). Empty when the type isn't createable. A property writes of
    /// another one change, such as Number_Of_States, isn't listed: its row
    /// says so instead ([`PropertySupport::written_through`]).
    pub creation_only_properties: Vec<PropertyIdentifier>,
    /// Whether the type can be deleted remotely with DeleteObject.
    pub deleteable: bool,
    /// Union of effective instance rows in ascending property-ID order.
    pub supported_properties: Vec<PropertySupport>,
}

/// Service support declaration.
#[derive(Debug, Clone)]
pub struct ServiceSupport {
    /// Service name.
    pub service_name: String,
    /// Device can request the service.
    pub initiator: bool,
    /// Device can execute the service.
    pub executor: bool,
}

/// Data link layer support.
///
/// It is `#[non_exhaustive]`, so a new option isn't a breaking change: a
/// `match` on it needs a wildcard arm.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataLinkSupport {
    /// BACnet/IP over IPv4.
    BipV4,
    /// BACnet/IP over IPv4 with NAT traversal: a BBMD behind a NAT router,
    /// reached at a configured global address (Annex J.7.5). Claim it with
    /// [`BipV4`](Self::BipV4) for a device that can run that way.
    BipV4NatTraversal,
    /// BACnet/IP over IPv6.
    BipV6,
    /// MS/TP over RS-485.
    Mstp,
    /// Raw BACnet Ethernet (802.3).
    Ethernet,
    /// BACnet Secure Connect.
    BacnetSc,
}

impl fmt::Display for DataLinkSupport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BipV4 => f.write_str("BACnet/IP (Annex J)"),
            Self::BipV4NatTraversal => f.write_str("BACnet/IP with NAT traversal (Annex J.7.5)"),
            Self::BipV6 => f.write_str("BACnet/IPv6 (Annex U)"),
            Self::Mstp => f.write_str("MS/TP (Clause 9)"),
            Self::Ethernet => f.write_str("BACnet Ethernet (Clause 7)"),
            Self::BacnetSc => f.write_str("BACnet/SC (Annex AB)"),
        }
    }
}

/// Network layer capabilities.
#[derive(Debug, Clone)]
pub struct NetworkLayerSupport {
    /// Device routes between networks.
    pub router: bool,
    /// Device acts as a BACnet Broadcast Management Device.
    pub bbmd: bool,
    /// Device can register as a foreign device with a BBMD.
    pub foreign_device: bool,
}

/// A character set the PICS can claim under Annex A "Character Sets Supported".
///
/// The variants are exactly the six sets Annex A offers. Each discriminant is the
/// set's code from Clause 20.2.9, the initial octet of an encoded CharacterString.
/// `Display` prints the set's Annex A label. Claiming several sets does not mean
/// the device can use them all at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CharacterSet {
    /// ISO 10646 UTF-8, code X'00'.
    Utf8 = charset::UTF8,
    /// IBM/Microsoft DBCS, code X'01'. Its strings carry a two-octet code page.
    IbmMicrosoftDbcs = charset::IBM_MICROSOFT_DBCS,
    /// JIS X 0208, code X'02'.
    JisX0208 = charset::JIS_X_0208,
    /// ISO 10646 UCS-4, code X'03'.
    Ucs4 = charset::UCS4,
    /// ISO 10646 UCS-2, code X'04'.
    Ucs2 = charset::UCS2,
    /// ISO 8859-1, code X'05'.
    Iso8859_1 = charset::ISO_8859_1,
}

impl CharacterSet {
    /// Every set Annex A offers, in Clause 20.2.9 code order.
    pub const ALL: [Self; 6] = [
        Self::Utf8,
        Self::IbmMicrosoftDbcs,
        Self::JisX0208,
        Self::Ucs4,
        Self::Ucs2,
        Self::Iso8859_1,
    ];

    /// The set's Clause 20.2.9 code, one of the `bacnet_encoding::primitives::charset` constants.
    pub const fn code(self) -> u8 {
        self as u8
    }
}

impl fmt::Display for CharacterSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Utf8 => "ISO 10646 (UTF-8)",
            Self::IbmMicrosoftDbcs => "IBM/Microsoft DBCS",
            Self::JisX0208 => "JIS X 0208",
            Self::Ucs4 => "ISO 10646 (UCS-4)",
            Self::Ucs2 => "ISO 10646 (UCS-2)",
            Self::Iso8859_1 => "ISO 8859-1",
        })
    }
}

// ────────────────────────────── Configuration ──────────────────────────────

/// Configuration for PICS generation that cannot be inferred from the database.
#[derive(Debug, Clone)]
pub struct PicsConfig {
    /// Device profile claimed.
    pub device_profile: DeviceProfile,
    /// Data-link layers the device supports.
    pub data_link_layers: Vec<DataLinkSupport>,
    /// Network-layer roles the device fills.
    pub network_layer: NetworkLayerSupport,
    /// Character sets the device supports.
    pub character_sets: Vec<CharacterSet>,
    /// Free-text descriptions of special functionality.
    pub special_functionality: Vec<String>,
}

impl Default for PicsConfig {
    fn default() -> Self {
        Self {
            device_profile: DeviceProfile::BAsc,
            data_link_layers: vec![DataLinkSupport::BipV4],
            network_layer: NetworkLayerSupport {
                router: false,
                bbmd: false,
                foreign_device: false,
            },
            character_sets: vec![CharacterSet::Utf8],
            special_functionality: Vec::new(),
        }
    }
}

// ────────────────────────────── Generator ──────────────────────────────────

/// Generates PICS from raw object declarations and configuration. This standalone
/// generator does not establish service execution; use `BACnetServer::generate_pics`
/// for the full server's effective Device contract.
pub struct PicsGenerator<'a> {
    db: &'a ObjectDatabase,
    server_config: &'a ServerConfig,
    pics_config: &'a PicsConfig,
    served: bool,
}

impl<'a> PicsGenerator<'a> {
    /// Create a generator over the object database and the server and PICS configuration.
    pub fn new(
        db: &'a ObjectDatabase,
        server_config: &'a ServerConfig,
        pics_config: &'a PicsConfig,
    ) -> Self {
        Self {
            db,
            server_config,
            pics_config,
            served: false,
        }
    }

    pub(crate) fn for_server(mut self) -> Self {
        self.served = true;
        self
    }

    /// Generate the complete PICS document.
    ///
    /// The vendor block comes from the Device object, so the PICS states what
    /// the device reports: for the server, the Device at its identity;
    /// otherwise, or when the database lacks that one, the selected Device.
    /// Vendor_Name, Model_Name, Firmware_Revision and
    /// Application_Software_Version are its values at the time of the call,
    /// or empty without a Device or when one isn't a character string. The
    /// protocol version and revision are its Protocol_Version and
    /// Protocol_Revision; without a Device, or when either property isn't an
    /// unsigned value that fits in a `u16`, the stack's own
    /// [`bacnet_types::PROTOCOL_VERSION`] and
    /// [`bacnet_types::PROTOCOL_REVISION`] stand in. The vendor ID is the
    /// same Device's Vendor_Identifier, which for the server's identity
    /// Device is the [`ServerConfig`] identity's vendor ID; without a Device,
    /// or when the value isn't an unsigned that fits in a `u16`, that
    /// identity's vendor ID stands in.
    pub fn generate(&self) -> Pics {
        Pics {
            vendor_info: self.build_vendor_info(),
            device_profile: self.pics_config.device_profile.clone(),
            supported_object_types: self.build_object_types(),
            supported_services: self.build_services(),
            data_link_layers: self.pics_config.data_link_layers.clone(),
            network_layer: self.pics_config.network_layer.clone(),
            character_sets: self.pics_config.character_sets.clone(),
            special_functionality: self.pics_config.special_functionality.clone(),
        }
    }

    fn build_vendor_info(&self) -> VendorInfo {
        // The server reads the Device at its identity, as wildcard reads and
        // announcements do; otherwise the selected (lowest) one. Reads go
        // through the server's view, which for the identity Device serves
        // the identity's Vendor_Identifier; the standalone view has no
        // identity and passes every property read here through raw.
        let device = (self.served)
            .then(|| announcement_device(self.db, &self.server_config.device))
            .flatten()
            .or_else(|| self.db.selected_device())
            .and_then(|oid| self.db.get(&oid));
        let view = crate::device_view::DeviceReadContext::new(
            self.db,
            crate::device_view::DeviceExecution::FullServer,
        )
        .with_identity(self.served.then_some(&self.server_config.device));
        let read = |property| device.map(|d| view.object(d).read_property(property, None));
        let unsigned = |property, fallback| match read(property) {
            Some(Ok(PropertyValue::Unsigned(raw))) => u16::try_from(raw).unwrap_or(fallback),
            _ => fallback,
        };
        let text = |property| match read(property) {
            Some(Ok(PropertyValue::CharacterString(value))) => value,
            _ => String::new(),
        };
        VendorInfo {
            vendor_id: unsigned(
                PropertyIdentifier::VENDOR_IDENTIFIER,
                self.server_config.device.vendor_id(),
            ),
            vendor_name: text(PropertyIdentifier::VENDOR_NAME),
            model_name: text(PropertyIdentifier::MODEL_NAME),
            firmware_revision: text(PropertyIdentifier::FIRMWARE_REVISION),
            application_software_version: text(PropertyIdentifier::APPLICATION_SOFTWARE_VERSION),
            protocol_version: unsigned(PropertyIdentifier::PROTOCOL_VERSION, PROTOCOL_VERSION),
            protocol_revision: unsigned(PropertyIdentifier::PROTOCOL_REVISION, PROTOCOL_REVISION),
        }
    }

    fn build_object_types(&self) -> Vec<ObjectTypeSupport> {
        let mut by_type: BTreeMap<u32, Vec<&dyn bacnet_objects::traits::BACnetObject>> =
            BTreeMap::new();
        for (_oid, obj) in self.db.iter_objects() {
            by_type
                .entry(obj.object_identifier().object_type().to_raw())
                .or_default()
                .push(obj);
        }

        let mut result = Vec::with_capacity(by_type.len());
        for (raw_type, objects) in &by_type {
            let object_type = ObjectType::from_raw(*raw_type);
            let representative = objects[0];
            let view = (self.served && object_type == ObjectType::DEVICE).then(|| {
                crate::device_view::DeviceReadContext::new(
                    self.db,
                    crate::device_view::DeviceExecution::FullServer,
                )
                .with_identity(Some(&self.server_config.device))
            });
            let supported_properties =
                Self::union_property_support(objects.iter().flat_map(|object| {
                    if let Some(view) = &view {
                        Self::object_property_support(&view.object(*object))
                    } else {
                        Self::object_property_support(*object)
                    }
                }));

            // Factory/deletion capabilities are type-level declarations, not
            // per-instance property capabilities. Preserve that separate policy.

            let createable = representative.is_createable();
            let deleteable = representative.is_deleteable();
            let written_through = |property: &PropertyIdentifier| {
                supported_properties
                    .iter()
                    .any(|row| row.property_id == *property && row.written_through.is_some())
            };
            let creation_only_properties = if createable {
                let mut properties = representative.creation_only_properties().to_vec();
                properties.retain(|property| !written_through(property));
                properties
            } else {
                Vec::new()
            };

            result.push(ObjectTypeSupport {
                object_type,
                createable,
                creation_only_properties,
                deleteable,
                supported_properties,
            });
        }
        result
    }

    fn object_property_support(
        object: &dyn bacnet_objects::traits::BACnetObject,
    ) -> Vec<PropertySupport> {
        let metadata = object.property_metadata();
        if metadata.is_empty() {
            let all_props = object.property_list();
            let required = object.required_properties();
            all_props
                .iter()
                .map(|&property_id| PropertySupport {
                    property_id,
                    access: PropertyAccess {
                        readable: true,
                        writable: object.is_writable_property(property_id),
                        optional: !required.contains(&property_id),
                    },
                    written_through: None,
                })
                .collect()
        } else {
            metadata
                .iter()
                .map(|row| PropertySupport {
                    property_id: row.property_identifier,
                    access: PropertyAccess {
                        readable: true,
                        writable: row.write_capability.is_writable(),
                        optional: !row.is_required(),
                    },
                    written_through: row.write_capability.written_through(),
                })
                .collect()
        }
    }

    fn union_property_support(
        rows: impl IntoIterator<Item = PropertySupport>,
    ) -> Vec<PropertySupport> {
        // A type row records capabilities across all configured instances.
        // Missing properties do not vote on conformance; a required present row
        // wins over optional rows. Sort even a single instance by property ID.
        let mut properties: BTreeMap<u32, PropertySupport> = BTreeMap::new();
        for row in rows {
            properties
                .entry(row.property_id.to_raw())
                .and_modify(|existing| {
                    existing.access.readable |= row.access.readable;
                    existing.access.writable |= row.access.writable;
                    existing.access.optional &= row.access.optional;
                    existing.written_through = existing.written_through.or(row.written_through);
                })
                .or_insert(row);
        }
        properties.into_values().collect()
    }

    /// The services the PICS marks as initiated (its initiator column): the
    /// requests `bacnet-server` can build and send itself, in some
    /// configuration. Distinct from
    /// [`EXECUTED_SERVICES`](bacnet_objects::device::EXECUTED_SERVICES), which
    /// Clause 12.11 ties to execution.
    ///
    /// Some go out only when configured: COV and event notifications need a
    /// subscription or a recipient, audit notifications need audit reporting,
    /// and WriteProperty, ReadProperty and Who-Is need a Command action or a
    /// Channel member in another device (Who-Is finds that device). The server
    /// sends them through its own outbound paths, not through `bacnet-client`.
    /// A test scans the server's source and fails on an outbound service this
    /// list leaves out.
    pub const INITIATED_SERVICES: &'static [ServiceSupported] = &[
        ServiceSupported::I_AM,
        ServiceSupported::I_HAVE,
        ServiceSupported::CONFIRMED_COV_NOTIFICATION,
        ServiceSupported::CONFIRMED_EVENT_NOTIFICATION,
        ServiceSupported::UNCONFIRMED_COV_NOTIFICATION,
        ServiceSupported::UNCONFIRMED_EVENT_NOTIFICATION,
        ServiceSupported::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
        ServiceSupported::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
        ServiceSupported::CONFIRMED_AUDIT_NOTIFICATION,
        ServiceSupported::UNCONFIRMED_AUDIT_NOTIFICATION,
        ServiceSupported::READ_PROPERTY,
        ServiceSupported::WRITE_PROPERTY,
        ServiceSupported::WHO_IS,
    ];

    fn build_services(&self) -> Vec<ServiceSupport> {
        // Standalone callers inspect the selected Device's raw declaration. The
        // full server always uses its fixed execution profile, filtered by
        // database clock availability; a mutable/custom Device cannot override
        // that contract.
        let effective_executed = (!self.served)
            .then(|| {
                let device = self.db.get(&self.db.selected_device()?)?;
                match device
                    .read_property(PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED, None)
                    .ok()?
                {
                    PropertyValue::BitString { data, .. } => {
                        Some(ServicesSupported::from_bacnet(&data))
                    }
                    _ => None,
                }
            })
            .flatten();
        let mut service_map: BTreeMap<&'static str, (bool, bool)> = BTreeMap::new();
        let clock_available = self.db.clock_frame().is_some();
        let executed: Box<dyn Iterator<Item = ServiceSupported> + '_> = match &effective_executed {
            Some(services) => Box::new(services.iter()),
            None => {
                Box::new(crate::device_view::DeviceExecution::FullServer.services(clock_available))
            }
        };
        for service in executed {
            service_map
                .entry(service_display_name(service).unwrap_or("Unknown"))
                .or_default()
                .1 = true;
        }
        for service in Self::INITIATED_SERVICES {
            service_map
                .entry(service_display_name(*service).unwrap_or("Unknown"))
                .or_default()
                .0 = true;
        }

        service_map
            .into_iter()
            .map(|(name, (initiator, executor))| ServiceSupport {
                service_name: name.to_string(),
                initiator,
                executor,
            })
            .collect()
    }
}

// ─────────────────────────── Standalone helper ─────────────────────────────

/// Generate a PICS document from an ObjectDatabase and configuration.
///
/// This is a convenience function for use without a running BACnetServer.
pub fn generate_pics(
    db: &ObjectDatabase,
    server_config: &ServerConfig,
    pics_config: &PicsConfig,
) -> Pics {
    PicsGenerator::new(db, server_config, pics_config).generate()
}

/// The name the PICS prints for a `BACnetServicesSupported` bit position,
/// such as `ReadProperty` or `I-Am`, or `None` for a reserved bit. The PICS
/// prints `Unknown` for a reserved bit a Device declares.
pub fn service_display_name(service: ServiceSupported) -> Option<&'static str> {
    Some(match service.to_raw() {
        0 => "AcknowledgeAlarm",
        1 => "ConfirmedCOVNotification",
        2 => "ConfirmedEventNotification",
        3 => "GetAlarmSummary",
        4 => "GetEnrollmentSummary",
        5 => "SubscribeCOV",
        6 => "AtomicReadFile",
        7 => "AtomicWriteFile",
        8 => "AddListElement",
        9 => "RemoveListElement",
        10 => "CreateObject",
        11 => "DeleteObject",
        12 => "ReadProperty",
        14 => "ReadPropertyMultiple",
        15 => "WriteProperty",
        16 => "WritePropertyMultiple",
        17 => "DeviceCommunicationControl",
        18 => "ConfirmedPrivateTransfer",
        19 => "ConfirmedTextMessage",
        20 => "ReinitializeDevice",
        21 => "VT-Open",
        22 => "VT-Close",
        23 => "VT-Data",
        26 => "I-Am",
        27 => "I-Have",
        28 => "UnconfirmedCOVNotification",
        29 => "UnconfirmedEventNotification",
        30 => "UnconfirmedPrivateTransfer",
        31 => "UnconfirmedTextMessage",
        32 => "TimeSynchronization",
        33 => "WhoHas",
        34 => "WhoIs",
        35 => "ReadRange",
        36 => "UTCTimeSynchronization",
        37 => "LifeSafetyOperation",
        38 => "SubscribeCOVProperty",
        39 => "GetEventInformation",
        40 => "WriteGroup",
        41 => "SubscribeCOVPropertyMultiple",
        42 => "ConfirmedCOVNotificationMultiple",
        43 => "UnconfirmedCOVNotificationMultiple",
        44 => "ConfirmedAuditNotification",
        45 => "AuditLogQuery",
        46 => "UnconfirmedAuditNotification",
        47 => "Who-Am-I",
        48 => "You-Are",
        _ => return None,
    })
}

mod device_text;
mod render;

// ─────────────────────────────── Tests ─────────────────────────────────────

#[cfg(test)]
mod tests;

#[cfg(test)]
mod property_metadata_tests;

#[cfg(test)]
mod acked_transitions_policy_tests;

#[cfg(test)]
mod truth_source_tests;

#[cfg(test)]
mod protocol_revision_tests;

#[cfg(test)]
mod identity_text_tests;

#[cfg(test)]
mod property_union_tests;

#[cfg(test)]
mod character_set_tests;

#[cfg(test)]
mod selected_device_tests;

#[cfg(test)]
mod creation_only_tests;

#[cfg(test)]
mod data_link_tests;

#[cfg(test)]
mod initiated_tests;
