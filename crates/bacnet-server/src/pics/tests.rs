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
    /// Device is not createable or deleteable; mirrors the real DeviceObject.
    fn is_createable(&self) -> bool {
        false
    }
    fn is_deleteable(&self) -> bool {
        false
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

    // The TestAnalogInput stub does not override is_writable_property, so it
    // inherits the default historical_writable_default heuristic, which reports
    // PRESENT_VALUE read-only for ANALOG_INPUT. The real AnalogInputObject
    // overrides this to writable-when-out-of-service — see
    // pics_input_present_value_writable_only_when_out_of_service.
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
fn real_device_and_network_port_overrides_not_createable_or_deleteable() {
    // Directly exercise the real DeviceObject and NetworkPortObject trait
    // overrides (not the TestDevice stub) so a regression flipping either
    // override to true fails here rather than silently changing PICS output.
    use bacnet_objects::device::{DeviceConfig, DeviceObject};
    use bacnet_objects::network_port::NetworkPortObject;

    let device = DeviceObject::new(DeviceConfig {
        instance: 1,
        name: "Dev".into(),
        ..Default::default()
    })
    .unwrap();
    assert!(!device.is_createable(), "Device must not be createable");
    assert!(!device.is_deleteable(), "Device must not be deleteable");

    let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
    assert!(!np.is_createable(), "NetworkPort must not be createable");
    assert!(!np.is_deleteable(), "NetworkPort must not be deleteable");
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
    assert!(
        !service_names.contains(&"WriteGroup"),
        "server PICS must not list unsupported inbound WriteGroup"
    );

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

// ── Real-object PICS writability/createability tests ───────────────────────
//
// The tests above use minimal stub objects. The tests below use the real
// bacnet-objects implementations to verify that PICS writable flags and
// createability match the objects' own `write_property` arms and the runtime
// `handle_create_object` factory. This is the core regression guard for the
// shared-truth-source fix (issue #115): PICS and runtime dispatch must agree.

use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};

/// Build a database with one of each of the 9 core I/O/V object types.
fn make_real_objects_db() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(AnalogInputObject::new(1, "ai-1", 95).unwrap()))
        .unwrap();
    db.add(Box::new(AnalogOutputObject::new(1, "ao-1", 95).unwrap()))
        .unwrap();
    db.add(Box::new(AnalogValueObject::new(1, "av-1", 95).unwrap()))
        .unwrap();
    db.add(Box::new(BinaryInputObject::new(1, "bi-1").unwrap()))
        .unwrap();
    db.add(Box::new(BinaryOutputObject::new(1, "bo-1").unwrap()))
        .unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "bv-1").unwrap()))
        .unwrap();
    db.add(Box::new(MultiStateInputObject::new(1, "msi-1", 2).unwrap()))
        .unwrap();
    db.add(Box::new(
        MultiStateOutputObject::new(1, "mso-1", 2).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(MultiStateValueObject::new(1, "msv-1", 2).unwrap()))
        .unwrap();
    db
}

/// Helper: look up a property's writable flag in a PICS ObjectTypeSupport.
fn pics_writable<'a>(pics: &'a Pics, object_type: ObjectType, pid: PropertyIdentifier) -> bool {
    pics.supported_object_types
        .iter()
        .find(|ot| ot.object_type == object_type)
        .and_then(|ot| {
            ot.supported_properties
                .iter()
                .find(|p| p.property_id == pid)
        })
        .map(|p| p.access.writable)
        .expect("property should be in the PICS list")
}

#[test]
fn pics_event_properties_writable_on_analog_types() {
    // The old heuristic omitted LIMIT_ENABLE, NOTIFY_TYPE, TIME_DELAY,
    // EVENT_ENABLE — which the objects actually accept via write_generic_event_properties!.
    let db = make_real_objects_db();
    let pics = generate_pics(&db, &ServerConfig::default(), &make_pics_config());

    for ot in [
        ObjectType::ANALOG_INPUT,
        ObjectType::ANALOG_OUTPUT,
        ObjectType::ANALOG_VALUE,
    ] {
        for pid in [
            PropertyIdentifier::LIMIT_ENABLE,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::TIME_DELAY,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::HIGH_LIMIT,
            PropertyIdentifier::LOW_LIMIT,
            PropertyIdentifier::DEADBAND,
            PropertyIdentifier::NOTIFICATION_CLASS,
        ] {
            assert!(
                pics_writable(&pics, ot, pid),
                "{ot:?}: {pid:?} should be writable (accepted by write_generic_event_properties!)"
            );
        }
    }
}

/// Before #229 the four types below had no write path for the event set at all
/// — Time_Delay and Notify_Type were absent entirely — so PICS advertised what
/// no client could commission and no notification could ever be distributed.
#[test]
fn pics_event_properties_writable_on_binary_and_multistate_types() {
    let db = make_real_objects_db();
    let pics = generate_pics(&db, &ServerConfig::default(), &make_pics_config());

    for ot in [
        ObjectType::BINARY_INPUT,
        ObjectType::BINARY_VALUE,
        ObjectType::MULTI_STATE_INPUT,
        ObjectType::MULTI_STATE_VALUE,
    ] {
        for pid in [
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::TIME_DELAY,
            PropertyIdentifier::NOTIFICATION_CLASS,
        ] {
            assert!(
                pics_writable(&pics, ot, pid),
                "{ot:?}: {pid:?} should be writable (accepted by write_generic_event_properties!)"
            );
        }
        // Denied by the same macro, so PICS must not advertise it writable.
        assert!(
            !pics_writable(&pics, ot, PropertyIdentifier::ACKED_TRANSITIONS),
            "{ot:?}: ACKED_TRANSITIONS must stay read-only"
        );
    }
}

#[test]
fn pics_priority_array_writable_on_commandable_types() {
    let db = make_real_objects_db();
    let pics = generate_pics(&db, &ServerConfig::default(), &make_pics_config());

    // Commandable types accept PRIORITY_ARRAY (direct) and PRESENT_VALUE
    // (via the priority array). RELINQUISH_DEFAULT grew a validated write arm
    // in #270 (the standard permits writability), so the PICS advertises it.
    for ot in [
        ObjectType::ANALOG_OUTPUT,
        ObjectType::ANALOG_VALUE,
        ObjectType::BINARY_OUTPUT,
        ObjectType::BINARY_VALUE,
        ObjectType::MULTI_STATE_OUTPUT,
        ObjectType::MULTI_STATE_VALUE,
    ] {
        assert!(
            pics_writable(&pics, ot, PropertyIdentifier::PRIORITY_ARRAY),
            "{ot:?}: PRIORITY_ARRAY should be writable"
        );
        assert!(
            pics_writable(&pics, ot, PropertyIdentifier::PRESENT_VALUE),
            "{ot:?}: PRESENT_VALUE should be writable"
        );
        assert!(
            pics_writable(&pics, ot, PropertyIdentifier::RELINQUISH_DEFAULT),
            "{ot:?}: RELINQUISH_DEFAULT should be writable (#270)"
        );
    }
}

#[test]
fn pics_input_present_value_writable_only_when_out_of_service() {
    let db = make_real_objects_db();
    let pics = generate_pics(&db, &ServerConfig::default(), &make_pics_config());

    // Input types (AI, BI, MSI) accept PRESENT_VALUE writes when
    // out-of-service. PICS reports the type-level writability (the runtime
    // enforces the out-of-service guard), so PRESENT_VALUE is writable.
    // This was a false-negative in the old heuristic for AI (AI was excluded);
    // the trait override now mirrors the real write_property arm.
    for ot in [
        ObjectType::ANALOG_INPUT,
        ObjectType::BINARY_INPUT,
        ObjectType::MULTI_STATE_INPUT,
    ] {
        assert!(
            pics_writable(&pics, ot, PropertyIdentifier::PRESENT_VALUE),
            "{ot:?}: PRESENT_VALUE should be writable (accepted when out-of-service)"
        );
    }
}

#[test]
fn pics_state_text_writable_on_multistate_types() {
    let db = make_real_objects_db();
    let pics = generate_pics(&db, &ServerConfig::default(), &make_pics_config());

    // All three multistate types accept STATE_TEXT writes (array-indexed).
    for ot in [
        ObjectType::MULTI_STATE_INPUT,
        ObjectType::MULTI_STATE_OUTPUT,
        ObjectType::MULTI_STATE_VALUE,
    ] {
        assert!(
            pics_writable(&pics, ot, PropertyIdentifier::STATE_TEXT),
            "{ot:?}: STATE_TEXT should be writable"
        );
    }
}

#[test]
fn pics_universal_readonly_properties_never_writable() {
    let db = make_real_objects_db();
    let pics = generate_pics(&db, &ServerConfig::default(), &make_pics_config());

    // OBJECT_IDENTIFIER, OBJECT_TYPE, PROPERTY_LIST, STATUS_FLAGS are never
    // writable on any object type.
    for ot in pics.supported_object_types.iter() {
        for pid in [
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PROPERTY_LIST,
            PropertyIdentifier::STATUS_FLAGS,
        ] {
            if ot.supported_properties.iter().any(|p| p.property_id == pid) {
                assert!(
                    !pics_writable(&pics, ot.object_type, pid),
                    "{ot:?}: {pid:?} must never be writable"
                );
            }
        }
    }
}

#[test]
fn pics_createability_matches_runtime_factory() {
    let db = make_real_objects_db();
    let pics = generate_pics(&db, &ServerConfig::default(), &make_pics_config());

    // The 8 types handle_create_object actually constructs must be createable.
    for ot in [
        ObjectType::ANALOG_INPUT,
        ObjectType::ANALOG_OUTPUT,
        ObjectType::BINARY_INPUT,
        ObjectType::BINARY_OUTPUT,
        ObjectType::BINARY_VALUE,
        ObjectType::MULTI_STATE_INPUT,
        ObjectType::MULTI_STATE_OUTPUT,
        ObjectType::MULTI_STATE_VALUE,
    ] {
        let entry = pics
            .supported_object_types
            .iter()
            .find(|e| e.object_type == ot)
            .expect("type should be in PICS");
        assert!(
            entry.createable,
            "{ot:?} should be createable (factory constructs it)"
        );
    }

    // AnalogValue is NOT in the factory — PICS must not advertise createability.
    let av = pics
        .supported_object_types
        .iter()
        .find(|e| e.object_type == ObjectType::ANALOG_VALUE)
        .expect("ANALOG_VALUE should be in PICS");
    assert!(
        !av.createable,
        "AnalogValue must NOT be createable (factory rejects it with UNSUPPORTED_OBJECT_TYPE)"
    );
}

#[test]
fn pics_writability_matches_runtime_write_property() {
    // Cross-check: PICS reports LIMIT_ENABLE writable on AnalogInput AND
    // write_property actually accepts it. The old heuristic reported it
    // non-writable (false-negative); the trait override fixes both.
    use bacnet_objects::event::LimitEnable;
    use bacnet_objects::traits::BACnetObject;

    let mut ai = AnalogInputObject::new(1, "ai-1", 95).unwrap();
    // PICS (via the trait method) must report it writable.
    assert!(
        ai.is_writable_property(PropertyIdentifier::LIMIT_ENABLE),
        "is_writable_property must report LIMIT_ENABLE writable on AnalogInput"
    );
    // And the runtime write_property must accept it.
    let bits = LimitEnable::BOTH.to_bits();
    let result = ai.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![bits],
        },
        None,
    );
    assert!(
        result.is_ok(),
        "write_property must accept LIMIT_ENABLE, got: {result:?}"
    );

    // Cross-check a property PICS reports non-writable: STATUS_FLAGS.
    assert!(
        !ai.is_writable_property(PropertyIdentifier::STATUS_FLAGS),
        "STATUS_FLAGS must not be writable on AnalogInput"
    );
    let result = ai.write_property(
        PropertyIdentifier::STATUS_FLAGS,
        None,
        PropertyValue::Boolean(true),
        None,
    );
    assert!(
        result.is_err(),
        "write_property must reject STATUS_FLAGS, got: {result:?}"
    );
}

#[test]
fn executed_services_match_dispatch_table() {
    // The three-way truth chain for #192: dispatch arms (requests/mod.rs +
    // unconfirmed.rs choice consts) -> BACnetServicesSupported bits ->
    // device::EXECUTED_SERVICES, which feeds both the Device object's
    // Protocol_Services_Supported and the PICS executor column. If a dispatch
    // arm is added or removed without updating the choice const or the device
    // constant, this test fails.
    use bacnet_types::enums::{ServiceSupported, UnconfirmedServiceChoice};

    assert!(
        !crate::server::EXECUTED_UNCONFIRMED.contains(&UnconfirmedServiceChoice::WRITE_GROUP),
        "inbound WriteGroup has no execution path"
    );

    let mut from_dispatch: Vec<u8> = crate::server::EXECUTED_CONFIRMED
        .iter()
        .map(|c| {
            ServiceSupported::from_confirmed_choice(*c)
                .expect("dispatched confirmed choice has a defined bit")
                .to_raw()
        })
        .chain(crate::server::EXECUTED_UNCONFIRMED.iter().map(|c| {
            ServiceSupported::from_unconfirmed_choice(*c)
                .expect("dispatched unconfirmed choice has a defined bit")
                .to_raw()
        }))
        .collect();
    from_dispatch.sort_unstable();

    let mut declared: Vec<u8> = bacnet_objects::device::EXECUTED_SERVICES
        .iter()
        .map(|s| s.to_raw())
        .collect();
    declared.sort_unstable();

    assert_eq!(
        declared, from_dispatch,
        "device::EXECUTED_SERVICES must equal the dispatch table's executed set"
    );
}
