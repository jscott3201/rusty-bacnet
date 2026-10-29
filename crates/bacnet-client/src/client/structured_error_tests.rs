//! CreateObject, WritePropertyMultiple, SubscribeCOVPropertyMultiple,
//! ConfirmedPrivateTransfer and VT-Close answer errors with their own Clause
//! 21 bodies instead of the plain class/code pair. A peer puts each exact
//! body on the wire; the client must report it as `Error::Structured` with
//! its detail (or `Error::protocol` when the body has none) instead of
//! failing to decode it and timing out.

use super::*;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_services::object_mgmt::ObjectSpecifier;
use bacnet_services::private_transfer::PrivateTransferRequest;
use bacnet_services::virtual_terminal::VTCloseRequest;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::constructed::{BACnetObjectPropertyReference, PropertyReference};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::{ErrorDetail, RemoteError};
use bacnet_types::primitives::ObjectIdentifier;
use std::future::Future;

/// Run `send` while the peer answers the request it receives with an Error
/// PDU whose body after the service choice is exactly `body`.
async fn answered<T>(
    peer: &LoopbackTransport,
    received: &mut mpsc::Receiver<ReceivedNpdu>,
    body: &[u8],
    send: impl Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    let reply = async {
        let received = timeout(Duration::from_secs(1), received.recv())
            .await
            .unwrap()
            .unwrap();
        let npdu = bacnet_encoding::npdu::decode_npdu(received.npdu).unwrap();
        let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap() else {
            panic!("expected a confirmed request")
        };
        let mut apdu = vec![0x50, request.invoke_id, request.service_choice.to_raw()];
        apdu.extend_from_slice(body);
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                payload: Bytes::from(apdu),
                ..Default::default()
            },
        )
        .unwrap();
        peer.send_unicast(&npdu, &[1]).await.unwrap();
    };
    let (result, ()) = tokio::join!(send, reply);
    result
}

fn structured<T: std::fmt::Debug>(
    result: Result<T, Error>,
    class: ErrorClass,
    code: ErrorCode,
) -> ErrorDetail {
    match result {
        Err(Error::Structured {
            class: c,
            code: k,
            detail,
        }) if c == class && k == code => *detail,
        other => panic!("expected a structured {class}/{code} error, got {other:?}"),
    }
}

fn reference(index: Option<u32>) -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 7).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: index,
    }
}

async fn client_and_peer() -> (
    BACnetClient<LoopbackTransport>,
    LoopbackTransport,
    mpsc::Receiver<ReceivedNpdu>,
) {
    let (transport, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let received = peer.start().await.unwrap();
    // A reply the client cannot decode times out quickly instead of hanging.
    let client = BACnetClient::generic_builder()
        .transport(transport)
        .apdu_timeout_ms(500)
        .build()
        .await
        .unwrap();
    (client, peer, received)
}

#[tokio::test]
async fn create_object_error_surfaces_the_failed_initial_value() {
    let (mut client, mut peer, mut received) = client_and_peer().await;
    let value = |property: PropertyIdentifier| BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value: vec![0x44, 0x42, 0x28, 0x00, 0x00],
        priority: None,
    };
    let result = answered(
        &peer,
        &mut received,
        // [0] { PROPERTY (2), WRITE_ACCESS_DENIED (40) }, [1] element 2.
        &[0x0E, 0x91, 2, 0x91, 40, 0x0F, 0x19, 2],
        client.create_object(
            &[2],
            ObjectSpecifier::Type(ObjectType::ANALOG_VALUE),
            vec![
                value(PropertyIdentifier::PRESENT_VALUE),
                value(PropertyIdentifier::OBJECT_TYPE),
            ],
        ),
    )
    .await;
    assert_eq!(
        structured(result, ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED),
        ErrorDetail::FirstFailedElementNumber(2)
    );
    assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn write_property_multiple_error_surfaces_the_failed_write() {
    let (mut client, mut peer, mut received) = client_and_peer().await;
    let result = answered(
        &peer,
        &mut received,
        // [0] { 2, 40 }, [1] { (ANALOG_VALUE, 7), PRESENT_VALUE (85), index 3 }.
        &[
            0x0E, 0x91, 2, 0x91, 40, 0x0F, 0x1E, 0x0C, 0x00, 0x80, 0x00, 0x07, 0x19, 85, 0x29, 3,
            0x1F,
        ],
        client.write_property_multiple(
            &[2],
            vec![WriteAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 7).unwrap(),
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: Some(3),
                    value: vec![0x44, 0x42, 0x28, 0x00, 0x00],
                    priority: None,
                }],
            }],
        ),
    )
    .await;
    assert_eq!(
        structured(result, ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED),
        ErrorDetail::FirstFailedWriteAttempt(reference(Some(3)))
    );
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn subscribe_cov_property_multiple_error_surfaces_the_failed_subscription() {
    let (mut client, mut peer, mut received) = client_and_peer().await;
    let mut request = BytesMut::new();
    SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: false,
        lifetime: Some(300),
        max_notification_delay: Some(10),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 7)
                .unwrap(),
            list_of_cov_references: vec![COVReference {
                monitored_property: PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: Some(8),
                },
                cov_increment: None,
                timestamped: false,
            }],
        }],
    }
    .encode(&mut request)
    .unwrap();
    let send = || {
        client.confirmed_request(
            &[2],
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            &request,
        )
    };

    // The first-failed-subscription choice: [1] { [0] (ANALOG_VALUE, 7),
    // [1] { PRESENT_VALUE, index 8 }, [2] { PROPERTY (2), NOT_COV_PROPERTY (44) } }.
    let subscription = [
        0x1E, 0x0C, 0x00, 0x80, 0x00, 0x07, 0x1E, 0x09, 85, 0x19, 8, 0x1F, 0x2E, 0x91, 2, 0x91, 44,
        0x2F, 0x1F,
    ];
    let result = answered(&peer, &mut received, &subscription, send()).await;
    assert_eq!(
        structured(result, ErrorClass::PROPERTY, ErrorCode::NOT_COV_PROPERTY),
        ErrorDetail::FirstFailedSubscription(reference(Some(8)))
    );
    // The general choice: [0] { SERVICES (5), VALUE_OUT_OF_RANGE (37) }.
    let general = [0x0E, 0x91, 5, 0x91, 37, 0x0F];
    let result = answered(&peer, &mut received, &general, send()).await;
    assert!(
        matches!(
            result,
            Err(Error::Remote(RemoteError::Error {
                class: ErrorClass::SERVICES,
                code: ErrorCode::VALUE_OUT_OF_RANGE
            }))
        ),
        "{result:?}"
    );
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn confirmed_private_transfer_error_surfaces_vendor_service_and_parameters() {
    let (mut client, mut peer, mut received) = client_and_peer().await;
    let mut request = BytesMut::new();
    PrivateTransferRequest {
        vendor_id: 555,
        service_number: 7,
        service_parameters: None,
    }
    .encode(&mut request);
    let result = answered(
        &peer,
        &mut received,
        // [0] { SERVICES (5), OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED (45) },
        // [1] vendor 555, [2] service 7, [3] { Unsigned 1 }.
        &[
            0x0E, 0x91, 5, 0x91, 45, 0x0F, 0x1A, 0x02, 0x2B, 0x29, 7, 0x3E, 0x21, 1, 0x3F,
        ],
        client.confirmed_request(
            &[2],
            ConfirmedServiceChoice::CONFIRMED_PRIVATE_TRANSFER,
            &request,
        ),
    )
    .await;
    assert_eq!(
        structured(
            result,
            ErrorClass::SERVICES,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED
        ),
        ErrorDetail::PrivateTransfer {
            vendor_id: 555,
            service_number: 7,
            error_parameters: Some(vec![0x21, 1]),
        }
    );
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn vt_close_error_surfaces_the_sessions_not_closed() {
    let (mut client, mut peer, mut received) = client_and_peer().await;
    let mut request = BytesMut::new();
    VTCloseRequest {
        list_of_remote_vt_session_identifiers: vec![10, 11],
    }
    .encode(&mut request)
    .unwrap();
    let send = || client.confirmed_request(&[2], ConfirmedServiceChoice::VT_CLOSE, &request);

    // [0] { SERVICES (5), VT_SESSION_TERMINATION_FAILURE (39) }, [1] { 1, 4 }.
    let with_list = [0x0E, 0x91, 5, 0x91, 39, 0x0F, 0x1E, 0x21, 1, 0x21, 4, 0x1F];
    let result = answered(&peer, &mut received, &with_list, send()).await;
    assert_eq!(
        structured(
            result,
            ErrorClass::SERVICES,
            ErrorCode::VT_SESSION_TERMINATION_FAILURE
        ),
        ErrorDetail::VtSessionIdentifiers(vec![1, 4])
    );
    // Any other error omits the list: [0] { SERVICES, UNKNOWN_VT_SESSION (35) }.
    let without_list = [0x0E, 0x91, 5, 0x91, 35, 0x0F];
    let result = answered(&peer, &mut received, &without_list, send()).await;
    assert!(
        matches!(
            result,
            Err(Error::Remote(RemoteError::Error {
                class: ErrorClass::SERVICES,
                code: ErrorCode::UNKNOWN_VT_SESSION
            }))
        ),
        "{result:?}"
    );
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}
