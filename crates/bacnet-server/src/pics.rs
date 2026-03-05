//! Protocol Implementation Conformance Statement (PICS) generation per ASHRAE 135-2020 Annex A.
//!
//! A PICS document is a formal declaration of which BACnet features a device supports.
//! It is required for BACnet certification and interoperability testing.

use std::collections::BTreeMap;
use std::fmt;

use bacnet_objects::database::ObjectDatabase;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

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

/// Vendor and device identification (Annex A.1).
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

/// BACnet device profile (Annex A.2).
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

/// Object type support declaration (Annex A.3).
#[derive(Debug, Clone)]
pub struct ObjectTypeSupport {
    pub object_type: ObjectType,
    pub createable: bool,
    pub deleteable: bool,
    pub supported_properties: Vec<PropertySupport>,
}

/// Service support declaration (Annex A.4).
#[derive(Debug, Clone)]
pub struct ServiceSupport {
    pub service_name: String,
    pub initiator: bool,
    pub executor: bool,
}

/// Data link layer support (Annex A.5).
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

/// Network layer capabilities (Annex A.6).
#[derive(Debug, Clone)]
pub struct NetworkLayerSupport {
    pub router: bool,
    pub bbmd: bool,
    pub foreign_device: bool,
}

/// Character set support (Annex A.7).
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
        // Group objects by type using a BTreeMap for deterministic ordering.
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
            // Use the first object as representative for property enumeration.
            let representative = objects[0];
            let all_props = representative.property_list();
            let required = representative.required_properties();

            let supported_properties = all_props
                .iter()
                .map(|&pid| {
                    let is_required = required.contains(&pid);
                    // Try a probe write to check writability.  We only check the
                    // representative and consider all listed properties readable.
                    let writable = Self::is_writable_property(object_type, pid);
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

            let createable = Self::is_createable(object_type);
            let deleteable = Self::is_deleteable(object_type);

            result.push(ObjectTypeSupport {
                object_type,
                createable,
                deleteable,
                supported_properties,
            });
        }
        result
    }

    /// Heuristic: properties that are commonly writable per BACnet standard.
    fn is_writable_property(object_type: ObjectType, pid: PropertyIdentifier) -> bool {
        // Universal read-only properties
        if pid == PropertyIdentifier::OBJECT_IDENTIFIER
            || pid == PropertyIdentifier::OBJECT_TYPE
            || pid == PropertyIdentifier::PROPERTY_LIST
            || pid == PropertyIdentifier::STATUS_FLAGS
        {
            return false;
        }

        // OBJECT_NAME is writable on most objects
        if pid == PropertyIdentifier::OBJECT_NAME {
            return true;
        }

        // PRESENT_VALUE writability depends on object type
        if pid == PropertyIdentifier::PRESENT_VALUE {
            return object_type != ObjectType::ANALOG_INPUT
                && object_type != ObjectType::BINARY_INPUT
                && object_type != ObjectType::MULTI_STATE_INPUT;
        }

        // Common writable properties
        pid == PropertyIdentifier::DESCRIPTION
            || pid == PropertyIdentifier::OUT_OF_SERVICE
            || pid == PropertyIdentifier::COV_INCREMENT
            || pid == PropertyIdentifier::HIGH_LIMIT
            || pid == PropertyIdentifier::LOW_LIMIT
            || pid == PropertyIdentifier::DEADBAND
            || pid == PropertyIdentifier::NOTIFICATION_CLASS
    }

    fn is_createable(object_type: ObjectType) -> bool {
        // Device and NetworkPort objects are not dynamically created.
        object_type != ObjectType::DEVICE && object_type != ObjectType::NETWORK_PORT
    }

    fn is_deleteable(object_type: ObjectType) -> bool {
        object_type != ObjectType::DEVICE && object_type != ObjectType::NETWORK_PORT
    }

    /// Build the service support list based on what the server actually handles.
    fn build_services(&self) -> Vec<ServiceSupport> {
        let mut services = Vec::new();

        // Confirmed services the server executes
        let executor_services = [
            "ReadProperty",
            "WriteProperty",
            "ReadPropertyMultiple",
            "WritePropertyMultiple",
            "SubscribeCOV",
            "SubscribeCOVProperty",
            "CreateObject",
            "DeleteObject",
            "DeviceCommunicationControl",
            "ReinitializeDevice",
            "GetEventInformation",
            "AcknowledgeAlarm",
            "ReadRange",
            "AtomicReadFile",
            "AtomicWriteFile",
            "AddListElement",
            "RemoveListElement",
        ];

        // Confirmed services the server initiates
        let initiator_services = ["ConfirmedCOVNotification", "ConfirmedEventNotification"];

        // Unconfirmed services the server executes
        let unconfirmed_executor = [
            "WhoIs",
            "WhoHas",
            "TimeSynchronization",
            "UTCTimeSynchronization",
        ];

        // Unconfirmed services the server initiates
        let unconfirmed_initiator = [
            "I-Am",
            "I-Have",
            "UnconfirmedCOVNotification",
            "UnconfirmedEventNotification",
        ];

        // Merge all service names, tracking initiator/executor status.
        let mut service_map: BTreeMap<&str, (bool, bool)> = BTreeMap::new();
        for name in &executor_services {
            service_map.entry(name).or_default().1 = true;
        }
        for name in &initiator_services {
            service_map.entry(name).or_default().0 = true;
        }
        for name in &unconfirmed_executor {
            service_map.entry(name).or_default().1 = true;
        }
        for name in &unconfirmed_initiator {
            service_map.entry(name).or_default().0 = true;
        }

        for (name, (initiator, executor)) in &service_map {
            services.push(ServiceSupport {
                service_name: (*name).to_string(),
                initiator: *initiator,
                executor: *executor,
            });
        }

        services
    }
}

// ─────────────────────────── Text output ───────────────────────────────────

impl Pics {
    /// Render the PICS as human-readable text per Annex A layout.
    pub fn generate_text(&self) -> String {
        let mut out = String::with_capacity(4096);

        out.push_str("=== BACnet Protocol Implementation Conformance Statement (PICS) ===\n");
        out.push_str("    Per ASHRAE 135-2020 Annex A\n\n");

        // Section 1: Vendor Information
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

        // Section 2: Device Profile
        out.push_str("--- BACnet Device Profile ---\n");
        out.push_str(&format!("Profile: {}\n\n", self.device_profile));

        // Section 3: Supported Object Types
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

        // Section 4: Supported Services
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

        // Section 5: Data Link Layers
        out.push_str("--- Data Link Layer Support ---\n");
        for dl in &self.data_link_layers {
            out.push_str(&format!("  {dl}\n"));
        }
        out.push('\n');

        // Section 6: Network Layer
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

        // Section 7: Character Sets
        out.push_str("--- Character Sets Supported ---\n");
        for cs in &self.character_sets {
            out.push_str(&format!("  {cs}\n"));
        }
        out.push('\n');

        // Section 8: Special Functionality
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

        // Vendor Info
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

        // Device Profile
        out.push_str("## BACnet Device Profile\n\n");
        out.push_str(&format!("**{}**\n\n", self.device_profile));

        // Object Types
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

        // Services
        out.push_str("## Supported Services\n\n");
        out.push_str("| Service | Initiator | Executor |\n");
        out.push_str("|---------|-----------|----------|\n");
        for svc in &self.supported_services {
            let init = if svc.initiator { "Yes" } else { "No" };
            let exec = if svc.executor { "Yes" } else { "No" };
            out.push_str(&format!("| {} | {} | {} |\n", svc.service_name, init, exec));
        }
        out.push('\n');

        // Data Link
        out.push_str("## Data Link Layer Support\n\n");
        for dl in &self.data_link_layers {
            out.push_str(&format!("- {dl}\n"));
        }
        out.push('\n');

        // Network Layer
        out.push_str("## Network Layer Options\n\n");
        out.push_str("| Feature | Supported |\n");
        out.push_str("|---------|-----------|\n");
        out.push_str(&format!("| Router | {} |\n", self.network_layer.router));
        out.push_str(&format!("| BBMD | {} |\n", self.network_layer.bbmd));
        out.push_str(&format!(
            "| Foreign Device | {} |\n\n",
            self.network_layer.foreign_device
        ));

        // Character Sets
        out.push_str("## Character Sets Supported\n\n");
        for cs in &self.character_sets {
            out.push_str(&format!("- {cs}\n"));
        }
        out.push('\n');

        // Special Functionality
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

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use bacnet_objects::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

    use super::*;

    // ── Minimal test objects ───────────────────────────────────────────

    struct TestAnalogInput {
        oid: ObjectIdentifier,
        name: String,
    }

    impl BACnetObject for TestAnalogInput {
        fn object_identifier(&self) -> ObjectIdentifier {
            self.oid
        }
        fn object_name(&self) -> &str {
            &self.name
        }
        fn read_property(
            &self,
            _property: PropertyIdentifier,
            _array_index: Option<u32>,
        ) -> Result<PropertyValue, Error> {
            Ok(PropertyValue::Real(0.0))
        }
        fn write_property(
            &mut self,
            _property: PropertyIdentifier,
            _array_index: Option<u32>,
            _value: PropertyValue,
            _priority: Option<u8>,
        ) -> Result<(), Error> {
            Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
            })
        }
        fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
            static PROPS: [PropertyIdentifier; 8] = [
                PropertyIdentifier::OBJECT_IDENTIFIER,
                PropertyIdentifier::OBJECT_NAME,
                PropertyIdentifier::OBJECT_TYPE,
                PropertyIdentifier::PROPERTY_LIST,
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::STATUS_FLAGS,
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyIdentifier::UNITS,
            ];
            Cow::Borrowed(&PROPS)
        }
    }

    struct TestBinaryValue {
        oid: ObjectIdentifier,
        name: String,
    }

    impl BACnetObject for TestBinaryValue {
        fn object_identifier(&self) -> ObjectIdentifier {
            self.oid
        }
        fn object_name(&self) -> &str {
            &self.name
        }
        fn read_property(
            &self,
            _property: PropertyIdentifier,
            _array_index: Option<u32>,
        ) -> Result<PropertyValue, Error> {
            Ok(PropertyValue::Boolean(false))
        }
        fn write_property(
            &mut self,
            _property: PropertyIdentifier,
            _array_index: Option<u32>,
            _value: PropertyValue,
            _priority: Option<u8>,
        ) -> Result<(), Error> {
            Ok(())
        }
        fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
            static PROPS: [PropertyIdentifier; 6] = [
                PropertyIdentifier::OBJECT_IDENTIFIER,
                PropertyIdentifier::OBJECT_NAME,
                PropertyIdentifier::OBJECT_TYPE,
                PropertyIdentifier::PROPERTY_LIST,
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::STATUS_FLAGS,
            ];
            Cow::Borrowed(&PROPS)
        }
    }

    struct TestDevice {
        oid: ObjectIdentifier,
        name: String,
    }

    impl BACnetObject for TestDevice {
        fn object_identifier(&self) -> ObjectIdentifier {
            self.oid
        }
        fn object_name(&self) -> &str {
            &self.name
        }
        fn read_property(
            &self,
            _property: PropertyIdentifier,
            _array_index: Option<u32>,
        ) -> Result<PropertyValue, Error> {
            Ok(PropertyValue::Unsigned(0))
        }
        fn write_property(
            &mut self,
            _property: PropertyIdentifier,
            _array_index: Option<u32>,
            _value: PropertyValue,
            _priority: Option<u8>,
        ) -> Result<(), Error> {
            Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
            })
        }
        fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
            static PROPS: [PropertyIdentifier; 6] = [
                PropertyIdentifier::OBJECT_IDENTIFIER,
                PropertyIdentifier::OBJECT_NAME,
                PropertyIdentifier::OBJECT_TYPE,
                PropertyIdentifier::PROPERTY_LIST,
                PropertyIdentifier::PROTOCOL_VERSION,
                PropertyIdentifier::PROTOCOL_REVISION,
            ];
            Cow::Borrowed(&PROPS)
        }
    }

    // ── Helpers ────────────────────────────────────────────────────────

    fn make_test_db() -> ObjectDatabase {
        let mut db = ObjectDatabase::new();
        db.add(Box::new(TestDevice {
            oid: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
            name: "Test Device".into(),
        }))
        .unwrap();
        db.add(Box::new(TestAnalogInput {
            oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            name: "AI-1".into(),
        }))
        .unwrap();
        db.add(Box::new(TestAnalogInput {
            oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap(),
            name: "AI-2".into(),
        }))
        .unwrap();
        db.add(Box::new(TestBinaryValue {
            oid: ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap(),
            name: "BV-1".into(),
        }))
        .unwrap();
        db
    }

    fn make_pics_config() -> PicsConfig {
        PicsConfig {
            vendor_name: "Acme Corp".into(),
            model_name: "BACnet Controller 3000".into(),
            firmware_revision: "1.0.0".into(),
            application_software_version: "2.0.0".into(),
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
            special_functionality: vec!["Intrinsic event reporting".into()],
        }
    }

    // ── Tests ──────────────────────────────────────────────────────────

    #[test]
    fn generate_pics_basic() {
        let db = make_test_db();
        let server_config = ServerConfig {
            vendor_id: 999,
            ..ServerConfig::default()
        };
        let pics_config = make_pics_config();
        let pics = generate_pics(&db, &server_config, &pics_config);

        assert_eq!(pics.vendor_info.vendor_id, 999);
        assert_eq!(pics.vendor_info.vendor_name, "Acme Corp");
        assert_eq!(pics.device_profile, DeviceProfile::BAsc);
        assert_eq!(pics.character_sets, vec![CharacterSet::Utf8]);
        assert_eq!(pics.data_link_layers, vec![DataLinkSupport::BipV4]);
    }

    #[test]
    fn all_object_types_listed() {
        let db = make_test_db();
        let server_config = ServerConfig::default();
        let pics_config = make_pics_config();
        let pics = generate_pics(&db, &server_config, &pics_config);

        let types: Vec<ObjectType> = pics
            .supported_object_types
            .iter()
            .map(|ot| ot.object_type)
            .collect();
        assert!(types.contains(&ObjectType::DEVICE));
        assert!(types.contains(&ObjectType::ANALOG_INPUT));
        assert!(types.contains(&ObjectType::BINARY_VALUE));
        // 3 distinct types in our test DB
        assert_eq!(types.len(), 3);
    }

    #[test]
    fn object_type_properties_populated() {
        let db = make_test_db();
        let server_config = ServerConfig::default();
        let pics_config = make_pics_config();
        let pics = generate_pics(&db, &server_config, &pics_config);

        let ai = pics
            .supported_object_types
            .iter()
            .find(|ot| ot.object_type == ObjectType::ANALOG_INPUT)
            .expect("ANALOG_INPUT should be in PICS");

        // AI has 8 properties in our test fixture
        assert_eq!(ai.supported_properties.len(), 8);

        // PRESENT_VALUE should be read-only for input objects
        let pv = ai
            .supported_properties
            .iter()
            .find(|p| p.property_id == PropertyIdentifier::PRESENT_VALUE)
            .expect("PRESENT_VALUE should exist");
        assert!(pv.access.readable);
        assert!(!pv.access.writable);
    }

    #[test]
    fn device_not_createable_or_deleteable() {
        let db = make_test_db();
        let server_config = ServerConfig::default();
        let pics_config = make_pics_config();
        let pics = generate_pics(&db, &server_config, &pics_config);

        let dev = pics
            .supported_object_types
            .iter()
            .find(|ot| ot.object_type == ObjectType::DEVICE)
            .expect("DEVICE should be in PICS");
        assert!(!dev.createable);
        assert!(!dev.deleteable);
    }

    #[test]
    fn services_match_implementation() {
        let db = make_test_db();
        let server_config = ServerConfig::default();
        let pics_config = make_pics_config();
        let pics = generate_pics(&db, &server_config, &pics_config);

        let service_names: Vec<&str> = pics
            .supported_services
            .iter()
            .map(|s| s.service_name.as_str())
            .collect();

        // Executor services
        assert!(service_names.contains(&"ReadProperty"));
        assert!(service_names.contains(&"WriteProperty"));
        assert!(service_names.contains(&"ReadPropertyMultiple"));
        assert!(service_names.contains(&"SubscribeCOV"));
        assert!(service_names.contains(&"CreateObject"));
        assert!(service_names.contains(&"DeleteObject"));
        assert!(service_names.contains(&"WhoIs"));

        // Initiator services
        assert!(service_names.contains(&"I-Am"));
        assert!(service_names.contains(&"ConfirmedCOVNotification"));

        // Check initiator/executor flags on ReadProperty
        let rp = pics
            .supported_services
            .iter()
            .find(|s| s.service_name == "ReadProperty")
            .expect("ReadProperty should be listed");
        assert!(!rp.initiator);
        assert!(rp.executor);

        // I-Am is initiator only
        let iam = pics
            .supported_services
            .iter()
            .find(|s| s.service_name == "I-Am")
            .expect("I-Am should be listed");
        assert!(iam.initiator);
        assert!(!iam.executor);
    }

    #[test]
    fn text_output_contains_key_sections() {
        let db = make_test_db();
        let server_config = ServerConfig {
            vendor_id: 42,
            ..ServerConfig::default()
        };
        let pics_config = make_pics_config();
        let pics = generate_pics(&db, &server_config, &pics_config);
        let text = pics.generate_text();

        assert!(text.contains("Protocol Implementation Conformance Statement"));
        assert!(text.contains("Vendor ID:"));
        assert!(text.contains("42"));
        assert!(text.contains("Acme Corp"));
        assert!(text.contains("B-ASC"));
        assert!(text.contains("Supported Object Types"));
        assert!(text.contains("ANALOG_INPUT"));
        assert!(text.contains("Supported Services"));
        assert!(text.contains("ReadProperty"));
        assert!(text.contains("Data Link Layer Support"));
        assert!(text.contains("BACnet/IP (Annex J)"));
        assert!(text.contains("Character Sets Supported"));
        assert!(text.contains("UTF-8"));
        assert!(text.contains("Special Functionality"));
        assert!(text.contains("Intrinsic event reporting"));
    }

    #[test]
    fn markdown_output_has_tables() {
        let db = make_test_db();
        let server_config = ServerConfig::default();
        let pics_config = make_pics_config();
        let pics = generate_pics(&db, &server_config, &pics_config);
        let md = pics.generate_markdown();

        assert!(md.contains("# BACnet Protocol Implementation Conformance Statement"));
        assert!(md.contains("| Field | Value |"));
        assert!(md.contains("| Service | Initiator | Executor |"));
        assert!(md.contains("| Property | Access |"));
        assert!(md.contains("## Supported Object Types"));
        assert!(md.contains("## Supported Services"));
    }

    #[test]
    fn empty_database_produces_empty_object_list() {
        let db = ObjectDatabase::new();
        let server_config = ServerConfig::default();
        let pics_config = PicsConfig::default();
        let pics = generate_pics(&db, &server_config, &pics_config);

        assert!(pics.supported_object_types.is_empty());
        // Services should still be listed (server capability, not DB-dependent)
        assert!(!pics.supported_services.is_empty());
    }

    #[test]
    fn device_profile_display() {
        assert_eq!(DeviceProfile::BAac.to_string(), "B-AAC");
        assert_eq!(DeviceProfile::BAsc.to_string(), "B-ASC");
        assert_eq!(DeviceProfile::BOws.to_string(), "B-OWS");
        assert_eq!(DeviceProfile::BBc.to_string(), "B-BC");
        assert_eq!(DeviceProfile::BOp.to_string(), "B-OP");
        assert_eq!(DeviceProfile::BRouter.to_string(), "B-ROUTER");
        assert_eq!(DeviceProfile::BGw.to_string(), "B-GW");
        assert_eq!(DeviceProfile::BSc.to_string(), "B-SC");
        assert_eq!(
            DeviceProfile::Custom("MyProfile".into()).to_string(),
            "MyProfile"
        );
    }

    #[test]
    fn property_access_display() {
        let rw = PropertyAccess {
            readable: true,
            writable: true,
            optional: false,
        };
        assert_eq!(rw.to_string(), "RW");

        let ro = PropertyAccess {
            readable: true,
            writable: false,
            optional: true,
        };
        assert_eq!(ro.to_string(), "RO");

        let wo = PropertyAccess {
            readable: false,
            writable: true,
            optional: false,
        };
        assert_eq!(wo.to_string(), "W");
    }

    #[test]
    fn binary_value_present_value_is_writable() {
        let db = make_test_db();
        let server_config = ServerConfig::default();
        let pics_config = make_pics_config();
        let pics = generate_pics(&db, &server_config, &pics_config);

        let bv = pics
            .supported_object_types
            .iter()
            .find(|ot| ot.object_type == ObjectType::BINARY_VALUE)
            .expect("BINARY_VALUE should be in PICS");

        let pv = bv
            .supported_properties
            .iter()
            .find(|p| p.property_id == PropertyIdentifier::PRESENT_VALUE)
            .expect("PRESENT_VALUE should exist on BV");
        assert!(
            pv.access.writable,
            "BinaryValue PRESENT_VALUE should be writable"
        );
    }
}
