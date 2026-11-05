//! A custom object serving Event_Parameters in the legacy `[255]` form that
//! wouldn't decode back (#2376): ReadProperty answers with an Error PDU and
//! no partial ACK, and ReadPropertyMultiple answers that row with an access
//! error and the other rows as usual.
use super::*;
use bacnet_encoding::apdu::encode_apdu;
use bacnet_objects::device_identity::build_database_with_extra;
use bacnet_objects::property_metadata::{
    PropertyConformance, PropertyMetadata, PropertyWriteCapability,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::PropertyValue;
use std::borrow::Cow;

const DEVICE: u32 = 1234;
const INVOKE: u8 = 23;

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
    fn read_property(&self, property: P, _: Option<u32>) -> Result<PropertyValue, Error> {
        match property {
            P::PRESENT_VALUE => Ok(PropertyValue::Real(1.0)),
            P::EVENT_PARAMETERS => Ok(PropertyValue::ApplicationData(REFUSED.to_vec())),
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }
    fn write_property(
        &mut self,
        _: P,
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
                P::PRESENT_VALUE,
                PropertyConformance::RequiredRead,
                None,
                PropertyWriteCapability::ReadOnly,
            ),
            PropertyMetadata::new(
                P::EVENT_PARAMETERS,
                PropertyConformance::RequiredRead,
                None,
                PropertyWriteCapability::ReadOnly,
            ),
        ];
        Cow::Borrowed(ROWS)
    }
}

fn confirmed(service: ConfirmedServiceChoice, body: &[u8]) -> Bytes {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: INVOKE,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: service,
            service_request: Bytes::copy_from_slice(body),
        }),
    )
    .unwrap();
    apdu.freeze()
}

fn reply_to(apdu: Bytes) -> Apdu {
    let identity = DeviceIdentity::new(DEVICE, 0x105)
        .unwrap()
        .with_services(SERVICES);
    let mut db = build_database_with_extra(&identity, vec![Box::new(Legacy)]).unwrap();
    let mut responder = Responder::new(identity, Limits::default()).unwrap();
    let mut out = BytesMut::new();
    let outcome = responder
        .handle(&mut db, Inbound::new(apdu, 1476), &mut out)
        .outcome;
    assert_eq!(outcome, Outcome::Reply);
    decode_apdu(out.freeze()).unwrap()
}

#[test]
fn read_property_answers_a_refused_value_with_an_error_pdu() {
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: custom(),
        property_identifier: P::EVENT_PARAMETERS,
        property_array_index: None,
    }
    .encode(&mut body);
    match reply_to(confirmed(ConfirmedServiceChoice::READ_PROPERTY, &body)) {
        Apdu::Error(error) => {
            assert_eq!(error.invoke_id, INVOKE);
            assert_eq!(error.service_choice, ConfirmedServiceChoice::READ_PROPERTY);
            assert_eq!(
                (error.error_class, error.error_code),
                (ErrorClass::SERVICES, ErrorCode::OTHER)
            );
        }
        other => panic!("expected an Error PDU, got {other:?}"),
    }
}

#[test]
fn read_property_multiple_answers_a_refused_value_in_its_row() {
    let mut body = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: custom(),
            list_of_property_references: [P::PRESENT_VALUE, P::EVENT_PARAMETERS, P::PRESENT_VALUE]
                .into_iter()
                .map(|property_identifier| PropertyReference {
                    property_identifier,
                    property_array_index: None,
                })
                .collect(),
        }],
    }
    .encode(&mut body)
    .unwrap();
    let Apdu::ComplexAck(ack) = reply_to(confirmed(
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        &body,
    )) else {
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
}
