use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bacnet_encoding::apdu::{
    decode_apdu, encode_apdu, Apdu, ComplexAck, SegmentAck as SegmentAckPdu,
};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_endpoint_core::coordinator::{
    AdmissionKind, AdmissionOutcome, CanonicalPeer, LeaseOwner, OutboundTransactionCoordinator,
};
use bacnet_endpoint_core::endpoint_ingress::EndpointIngress;
use bacnet_network::layer::{NetworkLayer, ReceivedApdu};
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, NetworkPriority, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::time::{timeout, Duration};

use super::*;
use bacnet_transport::port::TransportProvenance;

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
        direct_response: None,
        apdu,
        source_mac: MacAddr::from_slice(source),
        ingress_network: None,
        source_network: None,
        link_layer_group: false,
        is_group: false,
        global_broadcast: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

fn encoded_apdu(apdu: &Apdu) -> Bytes {
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, apdu).unwrap();
    encoded.freeze()
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
    assert!(
        !requester
            .complete_pre_admitted(
                admission.clone(),
                segment_ack,
                received(Bytes::from_static(&[0x40, 0x00, 0x00, 0x01]), &[0x02]),
            )
            .await
    );
    assert_eq!(coordinator.active_count().unwrap(), 1);

    assert!(
        requester
            .complete_pre_admitted(admission, ack, received(ack_bytes, &[0x02]),)
            .await
    );
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

#[tokio::test]
async fn pre_admitted_segmented_response_sends_one_abort_and_releases_exact_lease() {
    let (endpoint_transport, peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut endpoint = EndpointIngress::new(endpoint_transport, 4);
    let mut ingress = endpoint.start().await.unwrap();
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

    let segmented = Apdu::ComplexAck(ComplexAck {
        segmented: true,
        more_follows: true,
        invoke_id,
        sequence_number: Some(0),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::from_static(&[0xaa]),
    });
    peer.send_apdu(
        &encoded_apdu(&segmented),
        &[0x01],
        false,
        NetworkPriority::NORMAL,
    )
    .await
    .unwrap();
    let segmented_received = timeout(WAIT, ingress.terminal_or_segment.recv())
        .await
        .unwrap()
        .unwrap();
    let decoded = decode_apdu(segmented_received.apdu.clone()).unwrap();
    let admit_calls = AtomicUsize::new(0);
    admit_calls.fetch_add(1, Ordering::SeqCst);
    let admission = match coordinator
        .admit(&CanonicalPeer::direct(&[0x02]), &decoded)
        .unwrap()
    {
        AdmissionOutcome::Admitted(admission) => admission,
        other => panic!("expected segmented requester admission, got {other:?}"),
    };
    assert_eq!(admission.kind(), AdmissionKind::NonTerminal);

    let wrong_sequence = match decoded.clone() {
        Apdu::ComplexAck(mut ack) => {
            ack.sequence_number = Some(1);
            Apdu::ComplexAck(ack)
        }
        _ => unreachable!(),
    };
    assert!(
        !requester
            .complete_pre_admitted(
                admission.clone(),
                wrong_sequence.clone(),
                received(encoded_apdu(&wrong_sequence), &[0x02]),
            )
            .await
    );
    let wrong_service = match decoded.clone() {
        Apdu::ComplexAck(mut ack) => {
            ack.service_choice = ConfirmedServiceChoice::WRITE_PROPERTY;
            Apdu::ComplexAck(ack)
        }
        _ => unreachable!(),
    };
    assert!(
        !requester
            .complete_pre_admitted(
                admission.clone(),
                wrong_service.clone(),
                received(encoded_apdu(&wrong_service), &[0x02]),
            )
            .await
    );
    let non_segment = Apdu::SegmentAck(SegmentAckPdu {
        negative_ack: false,
        sent_by_server: true,
        invoke_id,
        sequence_number: 0,
        actual_window_size: 1,
    });
    assert!(
        !requester
            .complete_pre_admitted(
                admission.clone(),
                non_segment.clone(),
                received(encoded_apdu(&non_segment), &[0x02]),
            )
            .await
    );
    assert_eq!(coordinator.active_count().unwrap(), 1);
    assert!(timeout(Duration::from_millis(25), peer_rx.recv())
        .await
        .is_err());

    assert!(
        requester
            .complete_pre_admitted(admission.clone(), decoded.clone(), segmented_received,)
            .await
    );
    let abort = timeout(WAIT, peer_rx.recv()).await.unwrap().unwrap();
    match decode_apdu(abort.apdu).unwrap() {
        Apdu::Abort(abort) => {
            assert!(!abort.sent_by_server);
            assert_eq!(abort.invoke_id, invoke_id);
            assert_eq!(abort.abort_reason, AbortReason::SEGMENTATION_NOT_SUPPORTED);
        }
        other => panic!("expected client Abort, got {other:?}"),
    }
    assert!(matches!(
        request_handle.await.unwrap(),
        Err(Error::Abort { reason })
            if reason == AbortReason::INVALID_APDU_IN_THIS_STATE.to_raw()
    ));
    assert_eq!(coordinator.active_count().unwrap(), 0);
    assert_eq!(admit_calls.load(Ordering::SeqCst), 1);

    assert!(
        !requester
            .complete_pre_admitted(
                admission,
                decoded.clone(),
                received(encoded_apdu(&decoded), &[0x02]),
            )
            .await
    );
    assert!(timeout(Duration::from_millis(25), peer_rx.recv())
        .await
        .is_err());

    requester.close();
    endpoint.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[path = "endpoint_operation_tests.rs"]
mod operation_tests;

/// A routed destination names one device, as a client's routed confirmed
/// request does (#1278). DNET 0, DNET 65535 or an empty DADR fails with
/// `Error::Encoding` before a transaction is reserved, whether the router is
/// known or reached by local broadcast, while DNET 65534 prepares normally.
#[tokio::test]
async fn routed_destination_to_no_single_device_fails_before_registration() {
    let (endpoint_transport, _peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut endpoint = EndpointIngress::new(endpoint_transport, 2);
    let ingress = endpoint.start().await.unwrap();
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let requester =
        EndpointRequester::new(ingress.egress.clone(), Arc::clone(&coordinator), config()).unwrap();
    let prepare = |destination| {
        requester.prepare_read_property(
            destination,
            Vec::new(),
            object_identifier(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
    };
    let routed = |destination_network, destination_mac: &[u8]| EndpointApduDestination::Routed {
        destination_network,
        destination_mac: MacAddr::from_slice(destination_mac),
        router_mac: MacAddr::from_slice(&[0x02]),
    };
    let via_broadcast = |destination_network, destination_mac: &[u8]| {
        EndpointApduDestination::RoutedViaLocalBroadcast {
            destination_network,
            destination_mac: MacAddr::from_slice(destination_mac),
        }
    };

    for (dnet, dadr) in [(0, &[3][..]), (u16::MAX, &[3]), (100, &[])] {
        for destination in [routed(dnet, dadr), via_broadcast(dnet, dadr)] {
            let refused = prepare(destination.clone()).await.err();
            assert!(
                matches!(refused, Some(Error::Encoding(_))),
                "{destination:?}: {refused:?}"
            );
            assert_eq!(coordinator.active_count().unwrap(), 0);
            assert_eq!(requester.inner.tsm.lock().unwrap().pending_count(), 0);
        }
    }
    let prepared = prepare(routed(u16::MAX - 1, &[3])).await.unwrap();
    assert_eq!(coordinator.active_count().unwrap(), 1);
    drop(prepared);
    assert_eq!(coordinator.active_count().unwrap(), 0);

    endpoint.stop().await.unwrap();
}

/// A direct destination that reaches a group of nodes, as named or once a
/// routed one naming the endpoint's own network is localized to it, is a
/// local broadcast (#1479). The requester refuses it before it registers a
/// transaction, so no lease or invoke ID is held afterwards, rather than
/// leaving the refusal to the egress.
#[tokio::test(start_paused = true)]
async fn a_group_destination_fails_before_registration() {
    use bacnet_types::network_number::NetworkNumber;

    let (mut endpoint_transport, _peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    endpoint_transport.set_broadcast_mac(vec![0xFF]);
    let mut endpoint = EndpointIngress::new(endpoint_transport, 2);
    let ingress = endpoint.start().await.unwrap();
    ingress
        .egress
        .local_network_number()
        .publish(NetworkNumber::configured(7).unwrap());
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let requester =
        EndpointRequester::new(ingress.egress.clone(), Arc::clone(&coordinator), config()).unwrap();
    let broadcast = || MacAddr::from_slice(&[0xFF]);
    for destination in [
        EndpointApduDestination::Direct {
            destination_mac: broadcast(),
        },
        EndpointApduDestination::Routed {
            destination_network: 7,
            destination_mac: broadcast(),
            router_mac: MacAddr::from_slice(&[0x02]),
        },
        EndpointApduDestination::RoutedViaLocalBroadcast {
            destination_network: 7,
            destination_mac: broadcast(),
        },
    ] {
        let refused = requester
            .prepare_read_property(
                destination.clone(),
                Vec::new(),
                object_identifier(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
            .err();
        assert!(
            matches!(&refused, Some(Error::Encoding(message))
                if message.contains("broadcast or group address")),
            "{destination:?}: {refused:?}"
        );
        assert_eq!(coordinator.active_count().unwrap(), 0);
        assert_eq!(requester.inner.tsm.lock().unwrap().pending_count(), 0);
    }

    endpoint.stop().await.unwrap();
}
