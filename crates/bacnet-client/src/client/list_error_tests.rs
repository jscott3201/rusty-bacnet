//! AddListElement and RemoveListElement answer errors with a ChangeList-Error
//! (Clause 21): the error plus the First Failed Element Number. The client
//! surfaces it as `Error::Structured`; a plain class/code error from an older
//! device stays `Error::protocol`.

use super::*;
use crate::tsm::TsmResponse;
use bacnet_encoding::apdu::ErrorPdu;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::error::RemoteError;
use bacnet_types::{
    enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier},
    error::ErrorDetail,
    primitives::ObjectIdentifier,
};

/// The structured error naming `element`.
fn element_error<T>(
    result: &Result<T, Error>,
    class: ErrorClass,
    code: ErrorCode,
    element: u32,
) -> bool {
    matches!(
        result,
        Err(Error::Structured { class: c, code: k, detail })
            if *c == class
                && *k == code
                && **detail == ErrorDetail::FirstFailedElementNumber(element)
    )
}

/// An Error PDU for `service` exactly as `wire` (after the three-octet header)
/// puts it on the network.
fn decoded(service: ConfirmedServiceChoice, wire: &[u8]) -> ErrorPdu {
    let mut apdu = vec![0x50, 1, service.to_raw()];
    apdu.extend_from_slice(wire);
    let Apdu::Error(pdu) = apdu::decode_apdu(Bytes::from(apdu)).unwrap() else {
        panic!("expected an Error PDU");
    };
    pdu
}

#[test]
fn change_list_error_projects_its_element_number() {
    for service in [
        ConfirmedServiceChoice::ADD_LIST_ELEMENT,
        ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
    ] {
        // [0] { SERVICES (5), LIST_ELEMENT_NOT_FOUND (81) } then [1] 3.
        let pdu = decoded(service, &[0x0E, 0x91, 5, 0x91, 81, 0x0F, 0x19, 3]);
        let result = confirmed_response_result(TsmResponse::from_error_pdu(&pdu));
        assert!(
            element_error(
                &result,
                ErrorClass::SERVICES,
                ErrorCode::LIST_ELEMENT_NOT_FOUND,
                3
            ),
            "{service:?}: {result:?}"
        );
        // Zero, for a refusal of the target, is kept too.
        let pdu = decoded(service, &[0x0E, 0x91, 1, 0x91, 31, 0x0F, 0x19, 0]);
        let result = confirmed_response_result(TsmResponse::from_error_pdu(&pdu));
        assert!(
            element_error(&result, ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT, 0),
            "{service:?}: {result:?}"
        );
    }
}

#[test]
fn plain_list_service_error_stays_a_protocol_error() {
    // A device that answers with only the class and code has no element number.
    let pdu = decoded(
        ConfirmedServiceChoice::ADD_LIST_ELEMENT,
        &[0x91, ErrorClass::PROPERTY.to_raw() as u8, 0x91, 9],
    );
    assert!(matches!(
        confirmed_response_result(TsmResponse::from_error_pdu(&pdu)),
        Err(Error::Remote(RemoteError::Error {
            class: ErrorClass::PROPERTY,
            code: ErrorCode::INVALID_DATA_TYPE
        }))
    ));
    // The same body answering another service is not a ChangeList-Error.
    let pdu = decoded(
        ConfirmedServiceChoice::READ_PROPERTY,
        &[0x91, 5, 0x91, 81, 0x0E, 0x91, 5, 0x91, 81, 0x0F, 0x19, 3],
    );
    assert!(matches!(
        confirmed_response_result(TsmResponse::from_error_pdu(&pdu)),
        Err(Error::Remote(RemoteError::Error {
            class: ErrorClass::SERVICES,
            code: ErrorCode::LIST_ELEMENT_NOT_FOUND
        }))
    ));
}

#[tokio::test]
async fn list_services_surface_the_first_failed_element_number() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut received = peer.start().await.unwrap();
    // A reply the client cannot decode times out quickly instead of hanging.
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .apdu_timeout_ms(500)
        .build()
        .await
        .unwrap();
    let oid = ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap();
    let property = PropertyIdentifier::RECIPIENT_LIST;
    for (add, error_class, error_code, element) in [
        (true, ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 2),
        (
            false,
            ErrorClass::SERVICES,
            ErrorCode::LIST_ELEMENT_NOT_FOUND,
            1,
        ),
        (false, ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT, 0),
    ] {
        let elements = vec![0x21, 1, 0x21, 2];
        let send = async {
            if add {
                client
                    .add_list_element(&[2], oid, property, None, elements)
                    .await
            } else {
                client
                    .remove_list_element(&[2], oid, property, None, elements)
                    .await
            }
        };
        let reply = async {
            let received = timeout(Duration::from_secs(1), received.recv())
                .await
                .unwrap()
                .unwrap();
            let npdu = bacnet_encoding::npdu::decode_npdu(received.npdu).unwrap();
            let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap() else {
                panic!("expected a list request")
            };
            // The Error PDU octet by octet: the class and code inside [0]
            // (0x0E ... 0x0F), then the element number under context tag 1.
            let apdu = vec![
                0x50,
                request.invoke_id,
                request.service_choice.to_raw(),
                0x0E,
                0x91,
                error_class.to_raw() as u8,
                0x91,
                error_code.to_raw() as u8,
                0x0F,
                0x19,
                element as u8,
            ];
            let mut npdu = BytesMut::new();
            bacnet_encoding::npdu::encode_npdu(
                &mut npdu,
                &bacnet_encoding::npdu::Npdu {
                    payload: Bytes::from(apdu),
                    ..Default::default()
                },
            )
            .unwrap();
            peer.send_unicast(&npdu, &[1]).await.unwrap();
        };
        let (result, ()) = tokio::join!(send, reply);
        assert!(
            element_error(&result, error_class, error_code, element,),
            "expected a ChangeList-Error, got {result:?}"
        );
        assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
    }
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}
