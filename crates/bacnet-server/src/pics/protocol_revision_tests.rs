//! The PICS takes Protocol_Version and Protocol_Revision from the Device it
//! describes, so the two can't disagree (#2125).

use std::borrow::Cow;

use bacnet_objects::DeviceIdentity;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::{PROTOCOL_REVISION, PROTOCOL_VERSION};

use super::{PicsConfig, PicsGenerator, generate_pics};
use crate::server::test_transport::TestTransport;
use crate::server::{BACnetServer, ServerConfig};

const INSTANCE: u32 = 2125;

fn identity() -> DeviceIdentity {
    DeviceIdentity::new(INSTANCE, 0).expect("valid Device identity")
}

fn device() -> DeviceObject {
    DeviceObject::new(DeviceConfig {
        identity: identity(),
        ..DeviceConfig::default()
    })
    .unwrap()
}

/// A library user's Device that reports its own protocol version and
/// revision and otherwise behaves as the bundled one.
struct RevisedDevice {
    inner: DeviceObject,
    version: u64,
    revision: u64,
}

impl RevisedDevice {
    /// The bundled Device at `instance`, reporting `version` and `revision`.
    fn boxed(instance: u32, version: u64, revision: u64) -> Box<Self> {
        let identity = DeviceIdentity::new(instance, 0).expect("valid Device identity");
        let inner = DeviceObject::new(DeviceConfig {
            identity: identity.with_name(format!("Device {instance}")),
            ..DeviceConfig::default()
        })
        .unwrap();
        Box::new(Self {
            inner,
            version,
            revision,
        })
    }
}

impl BACnetObject for RevisedDevice {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.inner.object_name()
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::PROTOCOL_VERSION => {
                return Ok(PropertyValue::Unsigned(self.version));
            }
            PropertyIdentifier::PROTOCOL_REVISION => {
                return Ok(PropertyValue::Unsigned(self.revision));
            }
            _ => {}
        }
        self.inner.read_property(property, array_index)
    }
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.inner
            .write_property(property, array_index, value, priority)
    }
    fn property_metadata(&self) -> Cow<'_, [bacnet_objects::property_metadata::PropertyMetadata]> {
        self.inner.property_metadata()
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.inner.property_list()
    }
    fn is_createable(&self) -> bool {
        false
    }
    fn is_deleteable(&self) -> bool {
        false
    }
}

fn database(device: Box<dyn BACnetObject>) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(device).unwrap();
    db
}

fn unsigned(device: &dyn BACnetObject, property: PropertyIdentifier) -> u16 {
    match device.read_property(property, None).unwrap() {
        PropertyValue::Unsigned(raw) => u16::try_from(raw).unwrap(),
        other => panic!("{property} is {other:?}"),
    }
}

/// The bundled Device and the PICS both report the stack's constants.
#[test]
fn the_device_and_the_pics_share_the_protocol_constants() {
    let device = device();
    assert_eq!(
        unsigned(&device, PropertyIdentifier::PROTOCOL_VERSION),
        PROTOCOL_VERSION
    );
    assert_eq!(
        unsigned(&device, PropertyIdentifier::PROTOCOL_REVISION),
        PROTOCOL_REVISION
    );
    let pics = generate_pics(
        &database(Box::new(device)),
        &ServerConfig::default(),
        &PicsConfig::default(),
    );
    assert_eq!(pics.vendor_info.protocol_version, PROTOCOL_VERSION);
    assert_eq!(pics.vendor_info.protocol_revision, PROTOCOL_REVISION);
    // The text and Markdown print the Device's values.
    let row = format!("| Protocol Revision | {PROTOCOL_REVISION} |");
    assert!(pics.generate_markdown().contains(&row));
}

/// A Device that reports another version and revision is what the
/// standalone PICS states.
#[test]
fn the_standalone_pics_follows_the_devices_version_and_revision() {
    let db = database(RevisedDevice::boxed(INSTANCE, 2, 30));
    let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
    assert_eq!(pics.vendor_info.protocol_version, 2);
    assert_eq!(pics.vendor_info.protocol_revision, 30);
}

/// With two Devices, the server's PICS follows the one at its identity, as
/// wildcard reads and announcements do, even when another has a lower
/// instance; the standalone PICS follows the lowest. Without the identity
/// Device the server's PICS falls back to the lowest too.
#[test]
fn the_server_pics_follows_the_identity_device() {
    let mut db = database(RevisedDevice::boxed(INSTANCE, 1, 22));
    db.add(RevisedDevice::boxed(100, 2, 30)).unwrap();
    let config = ServerConfig {
        device: identity(),
        ..ServerConfig::default()
    };
    let pics_config = PicsConfig::default();
    let protocol = |pics: super::Pics| {
        (
            pics.vendor_info.protocol_version,
            pics.vendor_info.protocol_revision,
        )
    };
    let server = PicsGenerator::new(&db, &config, &pics_config).for_server();
    assert_eq!(protocol(server.generate()), (1, 22));
    assert_eq!(protocol(generate_pics(&db, &config, &pics_config)), (2, 30));
    let elsewhere = ServerConfig {
        device: DeviceIdentity::new(7, 0).unwrap(),
        ..ServerConfig::default()
    };
    let server = PicsGenerator::new(&db, &elsewhere, &pics_config).for_server();
    assert_eq!(protocol(server.generate()), (2, 30));
}

/// Without a Device, or with a value no `u16` holds, the constants stand in.
#[test]
fn the_constants_stand_in_without_a_usable_device_value() {
    let empty = generate_pics(
        &ObjectDatabase::new(),
        &ServerConfig::default(),
        &PicsConfig::default(),
    );
    assert_eq!(empty.vendor_info.protocol_version, PROTOCOL_VERSION);
    assert_eq!(empty.vendor_info.protocol_revision, PROTOCOL_REVISION);
    let db = database(RevisedDevice::boxed(INSTANCE, 1, u64::from(u16::MAX) + 1));
    let wide = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
    assert_eq!(wide.vendor_info.protocol_revision, PROTOCOL_REVISION);
}

/// The server's PICS states what its served Device reports, including a
/// revision the library user's Device chose.
#[tokio::test]
async fn the_server_pics_matches_the_served_device() {
    for (version, revision) in [(1, u64::from(PROTOCOL_REVISION)), (2, 30)] {
        let server = BACnetServer::generic_builder()
            .device(identity())
            .transport(TestTransport::new())
            .database(database(RevisedDevice::boxed(INSTANCE, version, revision)))
            .build()
            .await
            .unwrap();
        let oid = identity().device_oid();
        let mut served = [0u16; 2];
        let properties = [
            PropertyIdentifier::PROTOCOL_VERSION,
            PropertyIdentifier::PROTOCOL_REVISION,
        ];
        for (value, property) in served.iter_mut().zip(properties) {
            let PropertyValue::Unsigned(raw) =
                server.read_local(&oid, property, None).await.unwrap()
            else {
                panic!("{property} is not unsigned");
            };
            *value = u16::try_from(raw).unwrap();
        }
        let pics = server.generate_pics(&PicsConfig::default()).await;
        assert_eq!(
            served,
            [version, revision].map(|v| u16::try_from(v).unwrap())
        );
        assert_eq!(
            [
                pics.vendor_info.protocol_version,
                pics.vendor_info.protocol_revision
            ],
            served,
            "revision {revision}"
        );
    }
}
