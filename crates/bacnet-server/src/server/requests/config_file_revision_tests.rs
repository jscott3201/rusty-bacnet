//! A File a peer creates or deletes while the Device is in a backup or a
//! restore, a configuration File of the procedure, leaves Database_Revision
//! alone (Clauses 12.11.35 and 19.1, #2669); while the Device is IDLE it
//! moves it as any object does.

use super::mutation_tests::{Fixture, oid};
use super::*;
use bacnet_objects::creation::Creatable;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::file::FileObject;
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::object_mgmt::{CreateObjectRequest, DeleteObjectRequest, ObjectSpecifier};
use bacnet_types::enums::BackupAndRestoreState as State;
use std::borrow::Cow;

/// A File type the application registers for CreateObject, which the
/// built-in table doesn't build.
struct ConfigFile(FileObject);

impl BACnetObject for ConfigFile {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.0.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.0.object_name()
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.0.read_property(property, index)
    }
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.0.write_property(property, index, value, priority)
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.0.property_metadata()
    }
}

impl Creatable for ConfigFile {}

/// The mutation suites' database, with Device 5 in `state` and File
/// creatable through a registered constructor.
async fn fixture(state: State) -> Fixture {
    let mut fixture = Fixture::new(None);
    fixture
        .config
        .object_types
        .register(ObjectType::FILE, |oid, name| {
            Ok(ConfigFile(FileObject::new(
                oid.instance_number(),
                name,
                "config",
            )?))
        })
        .unwrap();
    let mut device = DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(5, 0).unwrap(),
        backup_and_restore: true,
        ..DeviceConfig::default()
    })
    .unwrap();
    device.set_backup_and_restore_state(state);
    fixture.db.write().await.add(Box::new(device)).unwrap();
    fixture
}

/// The first octet of the answer to `body`: 0x20 a SimpleACK, 0x30 a
/// ComplexACK.
async fn answer(fixture: &Fixture, service: ConfirmedServiceChoice, body: BytesMut) -> u8 {
    let npdu = fixture
        .dispatch(service, body.freeze(), 5)
        .await
        .expect("an answer");
    bacnet_encoding::npdu::decode_npdu(npdu).unwrap().payload[0]
}

#[tokio::test]
async fn a_file_created_or_deleted_in_a_backup_or_restore_leaves_the_revision() {
    for (state, moves) in [
        (State::PERFORMING_A_RESTORE, 0),
        (State::PREPARING_FOR_BACKUP, 0),
        (State::IDLE, 1),
    ] {
        let fixture = fixture(state).await;
        let revision = || async { fixture.db.read().await.database_revision() };
        let before = revision().await;
        let mut create = BytesMut::new();
        CreateObjectRequest {
            object_specifier: ObjectSpecifier::Type(ObjectType::FILE),
            list_of_initial_values: vec![],
        }
        .encode(&mut create)
        .unwrap();
        let created = answer(&fixture, ConfirmedServiceChoice::CREATE_OBJECT, create).await;
        assert_eq!(created, 0x30, "{state:?}");
        assert_eq!(revision().await, before + moves, "{state:?}: create");
        let mut delete = BytesMut::new();
        DeleteObjectRequest {
            object_identifier: oid(ObjectType::FILE, 1),
        }
        .encode(&mut delete);
        let deleted = answer(&fixture, ConfirmedServiceChoice::DELETE_OBJECT, delete).await;
        assert_eq!(deleted, 0x20, "{state:?}");
        assert_eq!(revision().await, before + 2 * moves, "{state:?}: delete");
    }
}
