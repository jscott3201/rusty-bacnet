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
