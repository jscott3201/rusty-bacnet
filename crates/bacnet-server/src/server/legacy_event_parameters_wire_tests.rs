//! A custom object serving Event_Parameters in the legacy `[255]` form that
//! wouldn't decode back (#2376), read over the wire from a running server:
//! ReadProperty answers with an Error PDU and no partial ACK, and
//! ReadPropertyMultiple answers that row with an access error and the other
//! rows as usual.

use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::property_metadata::{
    PropertyConformance, PropertyMetadata, PropertyWriteCapability,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::primitives::PropertyValue;
use std::borrow::Cow;

const DEVICE_INSTANCE: u32 = 2376;

/// A legacy value with no closing pair: every property value encoder
/// refuses it.
const REFUSED: [u8; 4] = [0xfe, 0xff, 1, 2];

fn custom() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 9).unwrap()
}

/// Serves Present_Value, and Event_Parameters as raw [`REFUSED`] octets.
struct Legacy;

impl BACnetObject for Legacy {
    fn object_identifier(&self) -> ObjectIdentifier {
        custom()
    }
    fn object_name(&self) -> &str {
        "legacy"
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        _: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::PRESENT_VALUE => Ok(PropertyValue::Real(1.0)),
            PropertyIdentifier::EVENT_PARAMETERS => {
                Ok(PropertyValue::ApplicationData(REFUSED.to_vec()))
            }
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        const ROWS: &[PropertyMetadata] = &[
            PropertyMetadata::new(
                PropertyIdentifier::PRESENT_VALUE,
                PropertyConformance::RequiredRead,
                None,
                PropertyWriteCapability::ReadOnly,
            ),
            PropertyMetadata::new(
                PropertyIdentifier::EVENT_PARAMETERS,
                PropertyConformance::RequiredRead,
                None,
                PropertyWriteCapability::ReadOnly,
            ),
        ];
        Cow::Borrowed(ROWS)
    }
}

async fn start() -> (BACnetServer<TestTransport>, mpsc::Sender<ReceivedNpdu>) {
    let identity = bacnet_objects::DeviceIdentity::new(DEVICE_INSTANCE, 0).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: identity.clone(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(Legacy)).unwrap();
    let config = ServerConfig {
        device: identity,
        ..ServerConfig::default()
    };
    let (tx, rx) = mpsc::channel(4);
    let transport = TestTransport::builder()
        .local_mac(&[0x0A, 0, 0, 2, 0xBA, 0xC0])
        .inbound(rx)
        .build();
    let server = BACnetServer::start(config, db, transport).await.unwrap();
    (server, tx)
}

/// One confirmed request from a peer, and the server's reply.
async fn send(
    tx: &mpsc::Sender<ReceivedNpdu>,
    service_choice: ConfirmedServiceChoice,
    request: BytesMut,
) -> Apdu {
    let mut payload = BytesMut::new();
    encode_apdu(
        &mut payload,
        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 5,
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
    tx.send(ReceivedNpdu {
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
    apdu::decode_apdu(decode_npdu(reply).unwrap().payload).unwrap()
}

#[tokio::test(start_paused = true)]
async fn read_property_answers_a_refused_value_with_an_error_pdu() {
    let (mut server, tx) = start().await;
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: custom(),
        property_identifier: PropertyIdentifier::EVENT_PARAMETERS,
        property_array_index: None,
    }
    .encode(&mut request);
    match send(&tx, ConfirmedServiceChoice::READ_PROPERTY, request).await {
        Apdu::Error(error) => assert_eq!(
            (error.error_class, error.error_code),
            (ErrorClass::SERVICES, ErrorCode::OTHER)
        ),
        other => panic!("expected an Error PDU, got {other:?}"),
    }
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn read_property_multiple_answers_a_refused_value_in_its_row() {
    let (mut server, tx) = start().await;
    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: custom(),
            list_of_property_references: [
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::EVENT_PARAMETERS,
                PropertyIdentifier::PRESENT_VALUE,
            ]
            .into_iter()
            .map(|property_identifier| PropertyReference {
                property_identifier,
                property_array_index: None,
            })
            .collect(),
        }],
    }
    .encode(&mut request)
    .unwrap();
    let Apdu::ComplexAck(ack) =
        send(&tx, ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE, request).await
    else {
        panic!("expected a ComplexACK");
    };
    let ack = ReadPropertyMultipleACK::decode(&ack.service_ack).unwrap();
    let rows = &ack.list_of_read_access_results[0].list_of_results;
    let real = Some(vec![0x44, 0x3f, 0x80, 0x00, 0x00]);
    assert_eq!(rows.len(), 3);
    assert_eq!((&rows[0].property_value, rows[0].error), (&real, None));
    assert_eq!(rows[1].property_value, None);
    assert_eq!(
        rows[1].error,
        Some((ErrorClass::SERVICES, ErrorCode::OTHER))
    );
    assert_eq!((&rows[2].property_value, rows[2].error), (&real, None));
    server.stop().await.unwrap();
}
