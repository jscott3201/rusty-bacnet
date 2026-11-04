//! One object of every catalogue type, served over the wire (#2175).
//!
//! A server holds the whole catalogue (its Device configured as the
//! server's own) on the shared [`TestTransport`], in paused time, and a
//! peer sends it confirmed requests. For each type:
//! - RPM ALL, REQUIRED and OPTIONAL expand to the rows: every row but
//!   Property_List, the required ones but Property_List, and the optional
//!   ones. Each value read decodes into well-formed elements that encode
//!   back to the same octets, of the row's datatype where that is a
//!   primitive, and equal to what the object itself reads
//!   ([`values`]);
//! - a WriteProperty of its own value to a row with no write route, or to
//!   an Out_Of_Service row while the object is in service, is refused with
//!   PROPERTY / WRITE_ACCESS_DENIED;
//! - CreateObject and DeleteObject answer as the server's type registry
//!   says: a created object reads every required row, and a type the
//!   registry doesn't build or doesn't delete gets the OBJECT error for it.

use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::property_metadata::{PropertyMetadata, PropertyWriteCapability};
use bacnet_services::object_mgmt::{
    CreateObjectAck, CreateObjectRequest, DeleteObjectRequest, ObjectSpecifier,
};
use bacnet_services::rpm::{
    ReadPropertyMultipleACK, ReadPropertyMultipleRequest, ReadResultElement,
};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::primitives::{Date, Time};

#[path = "catalogue_wire_values.rs"]
mod values;

const DEVICE_INSTANCE: u32 = 2175;

/// One object per catalogue type. A type added to the catalogue changes it.
const CATALOGUE_TYPES: usize = 64;
/// Values an RPM ALL reads over the catalogue: every row but Property_List.
/// A row added or dropped changes it.
const VALUES_READ: usize = 1138;
/// Rows a write over the wire is refused for access: the rows without a
/// write route and the Out_Of_Service rows, the arrays that read empty
/// included (#2305).
const REFUSED_ROWS: usize = 690;
/// Types the built-in table creates. A table entry changes it.
const CREATED_TYPES: usize = 27;
/// Types DeleteObject removes: all but Device and Network Port.
const DELETED_TYPES: usize = 62;

/// A Device clock that always reads the same instant, so every value read
/// from it is the same on the wire and in the object's own read.
struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: Date {
                year: 126,
                month: 11,
                day: 2,
                day_of_week: 1,
            },
            local_time: Time {
                hour: 10,
                minute: 30,
                second: 15,
                hundredths: 31,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

struct Wire {
    server: BACnetServer<TestTransport>,
    tx: mpsc::Sender<ReceivedNpdu>,
    invoke_id: u8,
}

/// A request's outcome, as the peer sees it.
#[derive(Debug)]
enum Reply {
    Ack(Bytes),
    Simple,
    Error(ErrorClass, ErrorCode),
}

/// The catalogue, its Device replaced by the server's own.
fn catalogue_database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(DEVICE_INSTANCE, 0)
                .unwrap()
                .with_name("Catalogue device"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    for object in bacnet_objects::catalogue::representatives() {
        if object.object_identifier().object_type() != ObjectType::DEVICE {
            db.add(object).unwrap();
        }
    }
    db
}

impl Wire {
    async fn start() -> Self {
        let config = ServerConfig {
            device: bacnet_objects::DeviceIdentity::new(DEVICE_INSTANCE, 0).unwrap(),
            ..ServerConfig::default()
        };
        let (tx, rx) = mpsc::channel(16);
        let transport = TestTransport::builder()
            .local_mac(&[0x0A, 0, 0, 2, 0xBA, 0xC0])
            .inbound(rx)
            .build();
        let server = BACnetServer::start(config, catalogue_database(), transport)
            .await
            .unwrap();
        // Start installs the system clock; replace it with the fixed one.
        server
            .database()
            .write()
            .await
            .set_clock_reader(Some(Arc::new(FixedClock)));
        Self {
            server,
            tx,
            invoke_id: 0,
        }
    }

    /// One confirmed request from a peer, and the server's reply.
    async fn send(&mut self, service_choice: ConfirmedServiceChoice, request: BytesMut) -> Reply {
        self.invoke_id = self.invoke_id.wrapping_add(1);
        let mut payload = BytesMut::new();
        encode_apdu(
            &mut payload,
            &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: 1476,
                invoke_id: self.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice,
                service_request: request.freeze(),
            }),
        )
        .unwrap();
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                expecting_reply: true,
                payload: payload.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ReceivedNpdu {
                direct_response: None,
                npdu: npdu.freeze(),
                source_mac: MacAddr::from_slice(&[0x0A, 0, 0, 5, 0xBA, 0xC0]),
                link_layer_group: false,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: Some(reply_tx),
            })
            .await
            .unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(30), reply_rx)
            .await
            .expect("the server replies")
            .unwrap();
        match apdu::decode_apdu(decode_npdu(reply).unwrap().payload).unwrap() {
            Apdu::ComplexAck(ack) => Reply::Ack(ack.service_ack),
            Apdu::SimpleAck(_) => Reply::Simple,
            Apdu::Error(error) => Reply::Error(error.error_class, error.error_code),
            other => panic!("a reply this test doesn't expect: {other:?}"),
        }
    }

    /// RPM of one property reference, or ALL, REQUIRED or OPTIONAL, on
    /// `oid`: its results.
    async fn rpm(
        &mut self,
        oid: ObjectIdentifier,
        property: PropertyIdentifier,
    ) -> Vec<ReadResultElement> {
        let mut request = BytesMut::new();
        ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: oid,
                list_of_property_references: vec![PropertyReference {
                    property_identifier: property,
                    property_array_index: None,
                }],
            }],
        }
        .encode(&mut request)
        .unwrap();
        match self
            .send(ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE, request)
            .await
        {
            Reply::Ack(ack) => {
                let mut ack = ReadPropertyMultipleACK::decode(&ack).unwrap();
                assert_eq!(ack.list_of_read_access_results.len(), 1);
                let result = ack.list_of_read_access_results.remove(0);
                assert_eq!(result.object_identifier, oid);
                result.list_of_results
            }
            other => panic!("{oid:?} RPM {property:?}: {other:?}"),
        }
    }

    async fn write(
        &mut self,
        oid: ObjectIdentifier,
        property: PropertyIdentifier,
        value: Vec<u8>,
    ) -> Reply {
        let mut request = BytesMut::new();
        WritePropertyRequest {
            object_identifier: oid,
            property_identifier: property,
            property_array_index: None,
            property_value: value,
            priority: None,
        }
        .encode(&mut request)
        .unwrap();
        self.send(ConfirmedServiceChoice::WRITE_PROPERTY, request)
            .await
    }

    async fn create(&mut self, oid: ObjectIdentifier) -> Reply {
        let mut request = BytesMut::new();
        CreateObjectRequest {
            object_specifier: ObjectSpecifier::Identifier(oid),
            list_of_initial_values: Vec::new(),
        }
        .encode(&mut request)
        .unwrap();
        self.send(ConfirmedServiceChoice::CREATE_OBJECT, request)
            .await
    }

    async fn delete(&mut self, oid: ObjectIdentifier) -> Reply {
        let mut request = BytesMut::new();
        DeleteObjectRequest {
            object_identifier: oid,
        }
        .encode(&mut request);
        self.send(ConfirmedServiceChoice::DELETE_OBJECT, request)
            .await
    }

    /// Each object's identifier and rows, in object type order.
    async fn objects(&self) -> Vec<(ObjectIdentifier, Vec<PropertyMetadata>)> {
        let db = self.server.database().read().await;
        let mut objects: Vec<_> = db
            .iter_objects()
            .map(|(oid, object)| (oid, object.property_metadata().into_owned()))
            .collect();
        objects.sort_by_key(|(oid, _)| oid.object_type().to_raw());
        objects
    }
}

/// The properties RPM's ALL, REQUIRED and OPTIONAL expand to on `rows`.
fn expansion(rows: &[PropertyMetadata], property: PropertyIdentifier) -> Vec<PropertyIdentifier> {
    rows.iter()
        .filter(|row| match property {
            PropertyIdentifier::ALL => row.property_identifier != PropertyIdentifier::PROPERTY_LIST,
            PropertyIdentifier::REQUIRED => {
                row.is_required() && row.property_identifier != PropertyIdentifier::PROPERTY_LIST
            }
            _ => !row.is_required(),
        })
        .map(|row| row.property_identifier)
        .collect()
}

#[tokio::test(start_paused = true)]
async fn every_catalogue_type_reads_all_required_and_optional_over_the_wire() {
    let mut wire = Wire::start().await;
    let objects = wire.objects().await;
    assert_eq!(
        objects.len(),
        CATALOGUE_TYPES,
        "one object per catalogue type"
    );
    let mut failures = Vec::new();
    let mut values_checked = 0;
    for (oid, rows) in &objects {
        for selector in [
            PropertyIdentifier::ALL,
            PropertyIdentifier::REQUIRED,
            PropertyIdentifier::OPTIONAL,
        ] {
            let results = wire.rpm(*oid, selector).await;
            let read: Vec<PropertyIdentifier> =
                results.iter().map(|r| r.property_identifier).collect();
            if read != expansion(rows, selector) {
                failures.push(format!("{oid:?} {selector:?}: read {read:?}"));
            }
            if selector != PropertyIdentifier::ALL {
                continue;
            }
            let db = wire.server.database().read().await;
            let object = db.get(oid).unwrap();
            for result in &results {
                let row = rows
                    .iter()
                    .find(|row| row.property_identifier == result.property_identifier)
                    .unwrap();
                if let Err(failure) = values::check(object, row, result) {
                    failures.push(format!(
                        "{oid:?} {:?}: {failure}",
                        result.property_identifier
                    ));
                }
                values_checked += 1;
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(values_checked, VALUES_READ, "values read");
}

#[tokio::test(start_paused = true)]
async fn rows_without_a_write_route_refuse_writes_over_the_wire() {
    let mut wire = Wire::start().await;
    let objects = wire.objects().await;
    let mut failures = Vec::new();
    let mut refused = 0;
    for (oid, rows) in &objects {
        let results = wire.rpm(*oid, PropertyIdentifier::ALL).await;
        for row in rows {
            // A row only Out_Of_Service opens is shut: every catalogue object
            // starts in service.
            if !matches!(
                row.write_capability,
                PropertyWriteCapability::ReadOnly
                    | PropertyWriteCapability::Through(_)
                    | PropertyWriteCapability::WhenOutOfService
            ) {
                continue;
            }
            let property = row.property_identifier;
            // The value it reads, so the refusal is for access, not shape;
            // NULL for Property_List and a log's ReadRange-only buffer.
            let value = results
                .iter()
                .find(|result| result.property_identifier == property)
                .and_then(|result| result.property_value.clone())
                .unwrap_or_else(|| vec![0x00]);
            match wire.write(*oid, property, value).await {
                Reply::Error(ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED) => refused += 1,
                other => failures.push(format!("{oid:?} {property:?}: {other:?}")),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(refused, REFUSED_ROWS, "rows refused for access");
}

#[tokio::test(start_paused = true)]
async fn create_and_delete_follow_the_type_registry_over_the_wire() {
    let mut wire = Wire::start().await;
    let objects = wire.objects().await;
    // The registry the running server answers from.
    let config = Arc::clone(&wire.server.config);
    let registry = &config.object_types;
    let mut failures = Vec::new();
    let (mut created, mut deleted) = (0, 0);
    for (oid, _) in &objects {
        let object_type = oid.object_type();
        let new = ObjectIdentifier::new(object_type, 2).unwrap();
        let reply = wire.create(new).await;
        match (&reply, registry.creates(object_type)) {
            (Reply::Ack(ack), true) => {
                assert_eq!(CreateObjectAck::decode(ack).unwrap().object_identifier, new);
                // The new object serves its required rows: each reads, but a
                // log's Log_Buffer, which refuses.
                let results = wire.rpm(new, PropertyIdentifier::REQUIRED).await;
                let db = wire.server.database().read().await;
                let object = db.get(&new).unwrap();
                let rows = object.property_metadata();
                let read: Vec<PropertyIdentifier> =
                    results.iter().map(|r| r.property_identifier).collect();
                if results.is_empty() || read != expansion(&rows, PropertyIdentifier::REQUIRED) {
                    failures.push(format!("{new:?} REQUIRED read {read:?}"));
                }
                for result in &results {
                    if result.error.is_some() && !values::range_only_refusal(object, result) {
                        failures.push(format!("{new:?} {result:?}"));
                    }
                }
                drop(rows);
                drop(db);
                match wire.delete(new).await {
                    Reply::Simple => {}
                    other => failures.push(format!("{new:?} delete: {other:?}")),
                }
                created += 1;
            }
            (
                Reply::Error(ErrorClass::OBJECT, ErrorCode::DYNAMIC_CREATION_NOT_SUPPORTED),
                false,
            ) => {}
            (_, creates) => failures.push(format!("{new:?} create (creates {creates}): {reply:?}")),
        }
    }
    // Deletions last, so every create above ran beside the whole catalogue.
    for (oid, _) in &objects {
        let reply = wire.delete(*oid).await;
        match (&reply, registry.deletes(oid.object_type())) {
            (Reply::Simple, true) => {
                let gone = wire.server.database().read().await.get(oid).is_none();
                if !gone {
                    failures.push(format!("{oid:?} acknowledged but still there"));
                }
                deleted += 1;
            }
            (Reply::Error(ErrorClass::OBJECT, ErrorCode::OBJECT_DELETION_NOT_PERMITTED), false) => {
            }
            (_, deletes) => failures.push(format!("{oid:?} delete (deletes {deletes}): {reply:?}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(
        created, CREATED_TYPES,
        "the built-in table's createable types"
    );
    assert_eq!(
        deleted, DELETED_TYPES,
        "every type but Device and Network Port"
    );
}
