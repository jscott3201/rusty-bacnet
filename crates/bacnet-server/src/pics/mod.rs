//! Protocol Implementation Conformance Statement (PICS) generation per ASHRAE 135-2020 Annex A.
//!
//! A PICS document is a formal declaration of which BACnet features a device supports.
//! It is required for BACnet certification and interoperability testing.

use std::collections::BTreeMap;
use std::fmt;

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::EXECUTED_SERVICES;
use bacnet_types::enums::{ObjectType, PropertyIdentifier, ServiceSupported};

use crate::server::ServerConfig;

// ───────────────────────────── Data model ──────────────────────────────────

/// Complete PICS document per ASHRAE 135-2020 Annex A.
#[derive(Debug, Clone)]
pub struct Pics {
    pub vendor_info: VendorInfo,
    pub device_profile: DeviceProfile,
    pub supported_object_types: Vec<ObjectTypeSupport>,
    pub supported_services: Vec<ServiceSupport>,
    pub data_link_layers: Vec<DataLinkSupport>,
    pub network_layer: NetworkLayerSupport,
    pub character_sets: Vec<CharacterSet>,
    pub special_functionality: Vec<String>,
}

/// Vendor and device identification.
#[derive(Debug, Clone)]
pub struct VendorInfo {
    pub vendor_id: u16,
    pub vendor_name: String,
    pub model_name: String,
    pub firmware_revision: String,
    pub application_software_version: String,
    pub protocol_version: u16,
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

/// Property access flags for a supported property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropertyAccess {
    pub readable: bool,
    pub writable: bool,
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
    pub property_id: PropertyIdentifier,
    pub access: PropertyAccess,
}

/// Object type support declaration.
#[derive(Debug, Clone)]
pub struct ObjectTypeSupport {
    pub object_type: ObjectType,
    pub createable: bool,
    pub deleteable: bool,
    pub supported_properties: Vec<PropertySupport>,
}

/// Service support declaration.
#[derive(Debug, Clone)]
pub struct ServiceSupport {
    pub service_name: String,
    pub initiator: bool,
    pub executor: bool,
}

/// Data link layer support.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataLinkSupport {
    BipV4,
    BipV6,
    Mstp,
    Ethernet,
    BacnetSc,
}

impl fmt::Display for DataLinkSupport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BipV4 => f.write_str("BACnet/IP (Annex J)"),
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
    pub router: bool,
    pub bbmd: bool,
    pub foreign_device: bool,
}

/// Character set support.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CharacterSet {
    Utf8,
    Ansi,
    DbcsIbm,
    DbcsMs,
    Jisx0208,
    Iso8859_1,
}

impl fmt::Display for CharacterSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utf8 => f.write_str("UTF-8"),
            Self::Ansi => f.write_str("ANSI X3.4"),
            Self::DbcsIbm => f.write_str("IBM/Microsoft DBCS"),
            Self::DbcsMs => f.write_str("JIS C 6226"),
            Self::Jisx0208 => f.write_str("JIS X 0208"),
            Self::Iso8859_1 => f.write_str("ISO 8859-1"),
        }
    }
}

// ────────────────────────────── Configuration ──────────────────────────────

/// Configuration for PICS generation that cannot be inferred from the database.
#[derive(Debug, Clone)]
pub struct PicsConfig {
    pub vendor_name: String,
    pub model_name: String,
    pub firmware_revision: String,
    pub application_software_version: String,
    pub protocol_version: u16,
    pub protocol_revision: u16,
    pub device_profile: DeviceProfile,
    pub data_link_layers: Vec<DataLinkSupport>,
    pub network_layer: NetworkLayerSupport,
    pub character_sets: Vec<CharacterSet>,
    pub special_functionality: Vec<String>,
}

impl Default for PicsConfig {
    fn default() -> Self {
        Self {
            vendor_name: String::new(),
            model_name: String::new(),
            firmware_revision: String::new(),
            application_software_version: String::new(),
            protocol_version: 1,
            protocol_revision: 24,
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

/// Generates a [`Pics`] document from an [`ObjectDatabase`] and configuration.
pub struct PicsGenerator<'a> {
    db: &'a ObjectDatabase,
    server_config: &'a ServerConfig,
    pics_config: &'a PicsConfig,
}

impl<'a> PicsGenerator<'a> {
    pub fn new(
        db: &'a ObjectDatabase,
        server_config: &'a ServerConfig,
        pics_config: &'a PicsConfig,
    ) -> Self {
        Self {
            db,
            server_config,
            pics_config,
        }
    }

    /// Generate the complete PICS document.
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
        VendorInfo {
            vendor_id: self.server_config.vendor_id,
            vendor_name: self.pics_config.vendor_name.clone(),
            model_name: self.pics_config.model_name.clone(),
            firmware_revision: self.pics_config.firmware_revision.clone(),
            application_software_version: self.pics_config.application_software_version.clone(),
            protocol_version: self.pics_config.protocol_version,
            protocol_revision: self.pics_config.protocol_revision,
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
            let all_props = representative.property_list();
            let required = representative.required_properties();

            let supported_properties = all_props
                .iter()
                .map(|&pid| {
                    let is_required = required.contains(&pid);
                    let writable = representative.is_writable_property(pid);
                    PropertySupport {
                        property_id: pid,
                        access: PropertyAccess {
                            readable: true,
                            writable,
                            optional: !is_required,
                        },
                    }
                })
                .collect();

            let createable = representative.is_createable();
            let deleteable = representative.is_deleteable();

            result.push(ObjectTypeSupport {
                object_type,
                createable,
                deleteable,
                supported_properties,
            });
        }
        result
    }

    /// Build the service support list based on what the server actually handles.
    /// Services this server initiates (the PICS initiator column): replies
    /// and notifications constructed outbound by `bacnet-server`. Distinct
    /// from [`EXECUTED_SERVICES`], which Clause 12.11 ties to execution.
    const INITIATED_SERVICES: &'static [ServiceSupported] = &[
        ServiceSupported::I_AM,
        ServiceSupported::I_HAVE,
        ServiceSupported::CONFIRMED_COV_NOTIFICATION,
        ServiceSupported::CONFIRMED_EVENT_NOTIFICATION,
        ServiceSupported::UNCONFIRMED_COV_NOTIFICATION,
        ServiceSupported::UNCONFIRMED_EVENT_NOTIFICATION,
        ServiceSupported::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
        ServiceSupported::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
    ];

    fn build_services(&self) -> Vec<ServiceSupport> {
        // Executor column comes from the same constant the Device object's
        // Protocol_Services_Supported is built from, so the PICS cannot drift
        // from the property (both are cross-checked against the dispatch
        // table by `executed_services_match_dispatch_table`).
        let mut service_map: BTreeMap<&'static str, (bool, bool)> = BTreeMap::new();
        for service in EXECUTED_SERVICES {
            service_map
                .entry(service_display_name(*service))
                .or_default()
                .1 = true;
        }
        for service in Self::INITIATED_SERVICES {
            service_map
                .entry(service_display_name(*service))
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

// ─────────────────────────── Text output ───────────────────────────────────

impl Pics {
    /// Render the PICS as human-readable text per Annex A layout.
    pub fn generate_text(&self) -> String {
        let mut out = String::with_capacity(4096);

        out.push_str("=== BACnet Protocol Implementation Conformance Statement (PICS) ===\n");
        out.push_str("    Per ASHRAE 135-2020 Annex A\n\n");

        out.push_str("--- Vendor Information ---\n");
        out.push_str(&format!(
            "Vendor ID:                      {}\n",
            self.vendor_info.vendor_id
        ));
        out.push_str(&format!(
            "Vendor Name:                    {}\n",
            self.vendor_info.vendor_name
        ));
        out.push_str(&format!(
            "Model Name:                     {}\n",
            self.vendor_info.model_name
        ));
        out.push_str(&format!(
            "Firmware Revision:              {}\n",
            self.vendor_info.firmware_revision
        ));
        out.push_str(&format!(
            "Application Software Version:   {}\n",
            self.vendor_info.application_software_version
        ));
        out.push_str(&format!(
            "Protocol Version:               {}\n",
            self.vendor_info.protocol_version
        ));
        out.push_str(&format!(
            "Protocol Revision:              {}\n\n",
            self.vendor_info.protocol_revision
        ));

        out.push_str("--- BACnet Device Profile ---\n");
        out.push_str(&format!("Profile: {}\n\n", self.device_profile));

        out.push_str("--- Supported Object Types ---\n");
        for ot in &self.supported_object_types {
            out.push_str(&format!(
                "\n  Object Type: {} (createable={}, deleteable={})\n",
                ot.object_type, ot.createable, ot.deleteable
            ));
            out.push_str("  Properties:\n");
            for prop in &ot.supported_properties {
                out.push_str(&format!(
                    "    {:<40} {}\n",
                    prop.property_id.to_string(),
                    prop.access
                ));
            }
        }
        out.push('\n');

        out.push_str("--- Supported Services ---\n");
        out.push_str(&format!(
            "  {:<45} {:>9} {:>9}\n",
            "Service", "Initiator", "Executor"
        ));
        out.push_str(&format!("  {:-<45} {:-<9} {:-<9}\n", "", "", ""));
        for svc in &self.supported_services {
            let init = if svc.initiator { "Yes" } else { "No" };
            let exec = if svc.executor { "Yes" } else { "No" };
            out.push_str(&format!(
                "  {:<45} {:>9} {:>9}\n",
                svc.service_name, init, exec
            ));
        }
        out.push('\n');

        out.push_str("--- Data Link Layer Support ---\n");
        for dl in &self.data_link_layers {
            out.push_str(&format!("  {dl}\n"));
        }
        out.push('\n');

        out.push_str("--- Network Layer Options ---\n");
        out.push_str(&format!(
            "  Router:         {}\n",
            self.network_layer.router
        ));
        out.push_str(&format!("  BBMD:           {}\n", self.network_layer.bbmd));
        out.push_str(&format!(
            "  Foreign Device: {}\n\n",
            self.network_layer.foreign_device
        ));

        out.push_str("--- Character Sets Supported ---\n");
        for cs in &self.character_sets {
            out.push_str(&format!("  {cs}\n"));
        }
        out.push('\n');

        if !self.special_functionality.is_empty() {
            out.push_str("--- Special Functionality ---\n");
            for sf in &self.special_functionality {
                out.push_str(&format!("  {sf}\n"));
            }
            out.push('\n');
        }

        out
    }

    /// Render the PICS as Markdown for documentation.
    pub fn generate_markdown(&self) -> String {
        let mut out = String::with_capacity(4096);

        out.push_str("# BACnet Protocol Implementation Conformance Statement (PICS)\n\n");
        out.push_str("*Per ASHRAE 135-2020 Annex A*\n\n");

        out.push_str("## Vendor Information\n\n");
        out.push_str("| Field | Value |\n");
        out.push_str("|-------|-------|\n");
        out.push_str(&format!("| Vendor ID | {} |\n", self.vendor_info.vendor_id));
        out.push_str(&format!(
            "| Vendor Name | {} |\n",
            self.vendor_info.vendor_name
        ));
        out.push_str(&format!(
            "| Model Name | {} |\n",
            self.vendor_info.model_name
        ));
        out.push_str(&format!(
            "| Firmware Revision | {} |\n",
            self.vendor_info.firmware_revision
        ));
        out.push_str(&format!(
            "| Application Software Version | {} |\n",
            self.vendor_info.application_software_version
        ));
        out.push_str(&format!(
            "| Protocol Version | {} |\n",
            self.vendor_info.protocol_version
        ));
        out.push_str(&format!(
            "| Protocol Revision | {} |\n\n",
            self.vendor_info.protocol_revision
        ));

        out.push_str("## BACnet Device Profile\n\n");
        out.push_str(&format!("**{}**\n\n", self.device_profile));

        out.push_str("## Supported Object Types\n\n");
        for ot in &self.supported_object_types {
            out.push_str(&format!(
                "### {}\n\n- Createable: {}\n- Deleteable: {}\n\n",
                ot.object_type, ot.createable, ot.deleteable
            ));
            out.push_str("| Property | Access |\n");
            out.push_str("|----------|--------|\n");
            for prop in &ot.supported_properties {
                out.push_str(&format!("| {} | {} |\n", prop.property_id, prop.access));
            }
            out.push('\n');
        }

        out.push_str("## Supported Services\n\n");
        out.push_str("| Service | Initiator | Executor |\n");
        out.push_str("|---------|-----------|----------|\n");
        for svc in &self.supported_services {
            let init = if svc.initiator { "Yes" } else { "No" };
            let exec = if svc.executor { "Yes" } else { "No" };
            out.push_str(&format!("| {} | {} | {} |\n", svc.service_name, init, exec));
        }
        out.push('\n');

        out.push_str("## Data Link Layer Support\n\n");
        for dl in &self.data_link_layers {
            out.push_str(&format!("- {dl}\n"));
        }
        out.push('\n');

        out.push_str("## Network Layer Options\n\n");
        out.push_str("| Feature | Supported |\n");
        out.push_str("|---------|-----------|\n");
        out.push_str(&format!("| Router | {} |\n", self.network_layer.router));
        out.push_str(&format!("| BBMD | {} |\n", self.network_layer.bbmd));
        out.push_str(&format!(
            "| Foreign Device | {} |\n\n",
            self.network_layer.foreign_device
        ));

        out.push_str("## Character Sets Supported\n\n");
        for cs in &self.character_sets {
            out.push_str(&format!("- {cs}\n"));
        }
        out.push('\n');

        if !self.special_functionality.is_empty() {
            out.push_str("## Special Functionality\n\n");
            for sf in &self.special_functionality {
                out.push_str(&format!("- {sf}\n"));
            }
            out.push('\n');
        }

        out
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

// ─────────────────────────────── Tests ─────────────────────────────────────

/// PICS display name for a `BACnetServicesSupported` bit position.
fn service_display_name(service: ServiceSupported) -> &'static str {
    match service.to_raw() {
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
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod truth_source_tests;
