use std::sync::Arc;

use bacnet_encoding::apdu::{
    decode_apdu, encode_apdu, Apdu, ComplexAck, SegmentAck as SegmentAckPdu,
};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_endpoint_core::coordinator::{
    AdmissionOutcome, CanonicalPeer, LeaseOwner, OutboundTransactionCoordinator,
};
use bacnet_endpoint_core::endpoint_ingress::EndpointIngress;
use bacnet_network::layer::{NetworkLayer, ReceivedApdu};
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::time::{timeout, Duration};

use super::*;

const WAIT: Duration = Duration::from_secs(1);

fn config() -> ClientConfig {
    ClientConfig {
        apdu_timeout_ms: 1_000,
        apdu_retries: 0,
        max_apdu_length: 480,
        ..ClientConfig::default()
    }
}

fn object_identifier() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

fn encoded_ack(invoke_id: u8, property_value: PropertyValue) -> (Apdu, Bytes) {
    let mut value = BytesMut::new();
    encode_property_value(&mut value, &property_value).unwrap();
    let ack = ReadPropertyACK {
        object_identifier: object_identifier(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: value.to_vec(),
    };
    let mut service_ack = BytesMut::new();
    ack.encode(&mut service_ack);
    let apdu = Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: service_ack.freeze(),
    });
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &apdu).unwrap();
    (apdu, encoded.freeze())
}

fn received(apdu: Bytes, source: &[u8]) -> ReceivedApdu {
    ReceivedApdu {
        apdu,
        source_mac: MacAddr::from_slice(source),
        source_network: None,
        is_group: false,
        data_attributes: Vec::new(),
        reply_tx: None,
    }
}

#[tokio::test]
async fn pre_admitted_completion_checks_role_token_owner_peer_and_service_without_readmitting() {
    let (endpoint_transport, peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut endpoint = EndpointIngress::new(endpoint_transport, 4);
    let ingress = endpoint.start().await.unwrap();
    let mut peer = NetworkLayer::new(peer_transport);
    let mut peer_rx = peer.start().await.unwrap();
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let requester =
        EndpointRequester::new(ingress.egress.clone(), Arc::clone(&coordinator), config()).unwrap();

    let request_handle = {
        let requester = requester.clone();
        tokio::spawn(async move {
            requester
                .read_property(
                    &[0x02],
                    object_identifier(),
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                )
                .await
        })
    };
    let request = timeout(WAIT, peer_rx.recv()).await.unwrap().unwrap();
    let invoke_id = match decode_apdu(request.apdu).unwrap() {
        Apdu::ConfirmedRequest(request) => request.invoke_id,
        other => panic!("expected confirmed request, got {other:?}"),
    };
    assert_eq!(invoke_id, 0);

    let (ack, ack_bytes) = encoded_ack(invoke_id, PropertyValue::Real(12.5));
    let admission = match coordinator
        .admit(&CanonicalPeer::direct(&[0x02]), &ack)
        .unwrap()
    {
        AdmissionOutcome::Admitted(admission) => admission,
        other => panic!("expected requester admission, got {other:?}"),
    };
    assert_eq!(admission.metadata().owner(), LeaseOwner::Requester);

    let segment_ack = Apdu::SegmentAck(SegmentAckPdu {
        negative_ack: false,
        sent_by_server: true,
        invoke_id,
        sequence_number: 0,
        actual_window_size: 1,
    });
    assert!(!requester.complete_pre_admitted(
        admission.clone(),
        segment_ack,
        received(Bytes::from_static(&[0x40, 0x00, 0x00, 0x01]), &[0x02]),
    ));
    assert_eq!(coordinator.active_count().unwrap(), 1);

    assert!(requester.complete_pre_admitted(admission, ack, received(ack_bytes, &[0x02]),));
    let result = request_handle.await.unwrap().unwrap();
    let mut expected_value = BytesMut::new();
    encode_property_value(&mut expected_value, &PropertyValue::Real(12.5)).unwrap();
    assert_eq!(result.property_value, expected_value.to_vec());
    assert_eq!(coordinator.active_count().unwrap(), 0);

    requester.close();
    endpoint.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn close_cancels_exact_pending_lease_and_rejects_future_requests() {
    let (endpoint_transport, peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut endpoint = EndpointIngress::new(endpoint_transport, 2);
    let ingress = endpoint.start().await.unwrap();
    let mut peer = NetworkLayer::new(peer_transport);
    let mut peer_rx = peer.start().await.unwrap();
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let requester =
        EndpointRequester::new(ingress.egress.clone(), Arc::clone(&coordinator), config()).unwrap();

    let request_handle = {
        let requester = requester.clone();
        tokio::spawn(async move {
            requester
                .read_property(
                    &[0x02],
                    object_identifier(),
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                )
                .await
        })
    };
    timeout(WAIT, peer_rx.recv()).await.unwrap().unwrap();
    assert_eq!(coordinator.active_count().unwrap(), 1);

    requester.close();
    assert_eq!(coordinator.active_count().unwrap(), 0);
    assert!(matches!(
        request_handle.await.unwrap(),
        Err(Error::Encoding(message)) if message == "endpoint shutdown"
    ));
    assert!(matches!(
        requester
            .read_property(
                &[0x02],
                object_identifier(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await,
        Err(Error::Encoding(message)) if message == "endpoint shutdown"
    ));

    endpoint.stop().await.unwrap();
    peer.stop().await.unwrap();
}
