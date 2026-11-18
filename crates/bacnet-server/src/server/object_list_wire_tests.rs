//! The Device's Object_List and Database_Revision follow the database
//! (#2669): a peer's CreateObject and DeleteObject, and the application's
//! own adds and removes on a running server, each read on the wire with
//! ReadProperty and ReadPropertyMultiple, whole, by count and at every
//! index from the first to one past the last.

use super::*;
use crate::server::test_transport::{SendLog, TestTransport};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_encoding::primitives::decode_application_value;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_services::object_mgmt::{
    CreateObjectAck, CreateObjectRequest, DeleteObjectRequest, ObjectSpecifier,
};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::EngineeringUnits;
use std::future::Future;
use std::pin::Pin;

const LIST: PropertyIdentifier = PropertyIdentifier::OBJECT_LIST;
const REVISION: PropertyIdentifier = PropertyIdentifier::DATABASE_REVISION;

/// One row's answer: its values, or its error.
type Row = Result<Vec<PropertyValue>, (ErrorClass, ErrorCode)>;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn ai1() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, 1)
}

fn bv(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::BINARY_VALUE, instance)
}

fn av1() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_VALUE, 1)
}

fn values(bytes: &[u8]) -> Vec<PropertyValue> {
    let mut offset = 0;
    let mut out = Vec::new();
    while offset < bytes.len() {
        let (value, next) = decode_application_value(bytes, offset).unwrap();
        out.push(value);
        offset = next;
    }
    out
}

fn ids(list: &[ObjectIdentifier]) -> Vec<PropertyValue> {
    list.iter()
        .copied()
        .map(PropertyValue::ObjectIdentifier)
        .collect()
}

/// A running server for Device 2669, with AI-1 and BV-1, on the shared
/// test transport, and a peer at MAC 1 that sends it requests.
struct Wire {
    server: BACnetServer<TestTransport>,
    tx: mpsc::Sender<ReceivedNpdu>,
    sent: SendLog,
    invoke: u8,
    device: ObjectIdentifier,
}

impl Wire {
    async fn start() -> Self {
        let identity = DeviceIdentity::new(2669, 0).unwrap();
        let db = build_database_with_extra(
            &identity,
            vec![
                Box::new(AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap()),
                Box::new(BinaryValueObject::new(1, "BV-1").unwrap()),
            ],
        )
        .unwrap();
        let (tx, rx) = mpsc::channel(16);
        let transport = TestTransport::builder().local_mac(&[2]).inbound(rx).build();
        let sent = transport.handle().sent();
        let config = ServerConfig {
            device: identity.clone(),
            ..Default::default()
        };
        let server = BACnetServer::start(config, db, transport).await.unwrap();
        Self {
            server,
            tx,
            sent,
            invoke: 0,
            device: identity.device_oid(),
        }
    }

    /// Send one confirmed request from the peer and return the server's
    /// answer to it. Boxed, so the many exchanges of one test don't share
    /// one large poll frame.
    fn request(
        &mut self,
        service: ConfirmedServiceChoice,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Apdu> + Send + '_>> {
        Box::pin(async move {
            self.invoke = self.invoke.wrapping_add(1);
            let id = self.invoke;
            let mut payload = BytesMut::new();
            encode_apdu(
                &mut payload,
                &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                    segmented: false,
                    more_follows: false,
                    segmented_response_accepted: false,
                    max_segments: None,
                    max_apdu_length: 1476,
                    invoke_id: id,
                    sequence_number: None,
                    proposed_window_size: None,
                    service_choice: service,
                    service_request: body,
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
            self.tx
                .send(ReceivedNpdu::unverified(
                    npdu.freeze(),
                    MacAddr::from_slice(&[1]),
                    false,
                    vec![],
                    None,
                ))
                .await
                .unwrap();
            let sent = self.sent.clone();
            tokio::time::timeout(Duration::from_secs(5), async move {
                loop {
                    let seen = sent.len();
                    let answer = sent.frames().into_iter().map(|f| f.apdu()).find(|apdu| {
                        let invoke = match apdu {
                            Apdu::ComplexAck(ack) => ack.invoke_id,
                            Apdu::SimpleAck(ack) => ack.invoke_id,
                            Apdu::Error(error) => error.invoke_id,
                            Apdu::Reject(reject) => reject.invoke_id,
                            Apdu::Abort(abort) => abort.invoke_id,
                            _ => return false,
                        };
                        invoke == id
                    });
                    if let Some(answer) = answer {
                        return answer;
                    }
                    sent.wait_for_len(seen + 1).await;
                }
            })
            .await
            .expect("an answer")
        })
    }

    /// ReadProperty of the Device's `property` at `index`.
    async fn read(&mut self, property: PropertyIdentifier, index: Option<u32>) -> Row {
        let mut body = BytesMut::new();
        ReadPropertyRequest {
            object_identifier: self.device,
            property_identifier: property,
            property_array_index: index,
        }
        .encode(&mut body);
        match self
            .request(ConfirmedServiceChoice::READ_PROPERTY, body.freeze())
            .await
        {
            Apdu::ComplexAck(ack) => Ok(values(
                &ReadPropertyACK::decode(&ack.service_ack)
                    .unwrap()
                    .property_value,
            )),
            Apdu::Error(error) => Err((error.error_class, error.error_code)),
            other => panic!("ReadProperty answered {other:?}"),
        }
    }

    /// One ReadPropertyMultiple of the Device's Object_List at each of
    /// `indexes`, and its Database_Revision.
    async fn rpm(&mut self, indexes: &[Option<u32>]) -> Vec<Row> {
        let mut references: Vec<_> = indexes
            .iter()
            .map(|&property_array_index| PropertyReference {
                property_identifier: LIST,
                property_array_index,
            })
            .collect();
        references.push(PropertyReference {
            property_identifier: REVISION,
            property_array_index: None,
        });
        let mut body = BytesMut::new();
        ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: self.device,
                list_of_property_references: references,
            }],
        }
        .encode(&mut body)
        .unwrap();
        let Apdu::ComplexAck(ack) = self
            .request(
                ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                body.freeze(),
            )
            .await
        else {
            panic!("ReadPropertyMultiple refused");
        };
        let ack = ReadPropertyMultipleACK::decode(&ack.service_ack).unwrap();
        ack.list_of_read_access_results[0]
            .list_of_results
            .iter()
            .map(|row| match (&row.property_value, row.error) {
                (Some(value), _) => Ok(values(value)),
                (None, Some(error)) => Err(error),
                (None, None) => panic!("a row with neither value nor error"),
            })
            .collect()
    }

    /// The Device reads `list` as its Object_List and `revision` as its
    /// Database_Revision, through both services: whole, its count at index
    /// 0, each entry from 1 to the last, and one past the last refused.
    async fn expect(&mut self, list: &[ObjectIdentifier], revision: u64) {
        let last = list.len() as u32;
        let invalid = Err((ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX));
        assert_eq!(self.read(LIST, None).await, Ok(ids(list)));
        assert_eq!(
            self.read(LIST, Some(0)).await,
            Ok(vec![PropertyValue::Unsigned(u64::from(last))])
        );
        for (index, entry) in (1..).zip(list) {
            assert_eq!(
                self.read(LIST, Some(index)).await,
                Ok(vec![PropertyValue::ObjectIdentifier(*entry)]),
                "Object_List[{index}]"
            );
        }
        assert_eq!(self.read(LIST, Some(last + 1)).await, invalid);
        assert_eq!(
            self.read(REVISION, None).await,
            Ok(vec![PropertyValue::Unsigned(revision)])
        );
        let rows = self
            .rpm(&[None, Some(0), Some(1), Some(last), Some(last + 1)])
            .await;
        assert_eq!(
            rows,
            [
                Ok(ids(list)),
                Ok(vec![PropertyValue::Unsigned(u64::from(last))]),
                Ok(ids(&list[..1])),
                Ok(ids(&list[list.len() - 1..])),
                invalid,
                Ok(vec![PropertyValue::Unsigned(revision)]),
            ]
        );
    }

    async fn create(&mut self, request: CreateObjectRequest) -> Apdu {
        let mut body = BytesMut::new();
        request.encode(&mut body).unwrap();
        self.request(ConfirmedServiceChoice::CREATE_OBJECT, body.freeze())
            .await
    }

    async fn delete(&mut self, object: ObjectIdentifier) -> Apdu {
        let mut body = BytesMut::new();
        DeleteObjectRequest {
            object_identifier: object,
        }
        .encode(&mut body);
        self.request(ConfirmedServiceChoice::DELETE_OBJECT, body.freeze())
            .await
    }
}

/// A peer's CreateObject adds its object last and moves the revision; its
/// DeleteObject takes one out and moves it again. Before #2669 the list
/// stayed as built (the created object missing, the deleted one listed)
/// and the revision stayed 0.
#[tokio::test]
async fn a_peers_create_and_delete_move_object_list_and_revision() {
    let mut wire = Wire::start().await;
    let device = wire.device;
    wire.expect(&[device, ai1(), bv(1)], 3).await;

    let created = wire
        .create(CreateObjectRequest {
            object_specifier: ObjectSpecifier::Type(ObjectType::ANALOG_VALUE),
            list_of_initial_values: vec![],
        })
        .await;
    let Apdu::ComplexAck(ack) = created else {
        panic!("CreateObject refused: {created:?}");
    };
    assert_eq!(
        CreateObjectAck::decode(&ack.service_ack)
            .unwrap()
            .object_identifier,
        av1()
    );
    wire.expect(&[device, ai1(), bv(1), av1()], 4).await;

    let deleted = wire.delete(bv(1)).await;
    assert!(matches!(deleted, Apdu::SimpleAck(_)), "{deleted:?}");
    wire.expect(&[device, ai1(), av1()], 5).await;
    wire.server.stop().await.unwrap();
}

/// The application's own add and remove on the running server's database
/// move both too.
#[tokio::test]
async fn the_applications_own_add_and_remove_move_them_too() {
    let mut wire = Wire::start().await;
    let device = wire.device;
    let database = Arc::clone(wire.server.database());
    database
        .write()
        .await
        .add(Box::new(BinaryValueObject::new(9, "BV-9").unwrap()))
        .unwrap();
    wire.expect(&[device, ai1(), bv(1), bv(9)], 4).await;
    let removed = database.write().await.remove(&ai1()).unwrap();
    assert!(removed.is_some());
    drop(removed);
    wire.expect(&[device, bv(1), bv(9)], 5).await;
    wire.server.stop().await.unwrap();
}

/// A CreateObject refused for an initial value adds nothing, so neither
/// the list nor the revision moves, though the server added the object
/// before refusing the value and took it out again. A DeleteObject refused
/// leaves both alone too.
#[tokio::test]
async fn refused_creates_and_deletes_move_nothing() {
    let mut wire = Wire::start().await;
    let device = wire.device;
    let mut name = BytesMut::new();
    encode_property_value(&mut name, &PropertyValue::CharacterString("AI-1".into())).unwrap();
    let refused = wire
        .create(CreateObjectRequest {
            object_specifier: ObjectSpecifier::Type(ObjectType::ANALOG_VALUE),
            list_of_initial_values: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::OBJECT_NAME,
                property_array_index: None,
                value: name.to_vec(),
                priority: None,
            }],
        })
        .await;
    assert!(matches!(refused, Apdu::Error(_)), "{refused:?}");
    let refused = wire.delete(device).await;
    assert!(matches!(refused, Apdu::Error(_)), "{refused:?}");
    let refused = wire.delete(av1()).await;
    assert!(matches!(refused, Apdu::Error(_)), "{refused:?}");
    wire.expect(&[device, ai1(), bv(1)], 3).await;
    wire.server.stop().await.unwrap();
}
