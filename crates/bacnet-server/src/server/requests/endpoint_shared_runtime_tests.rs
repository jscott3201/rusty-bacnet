use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bacnet_client::client::ClientConfig;
use bacnet_client::EndpointRequester;
use bacnet_encoding::apdu::{decode_apdu, encode_apdu};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_endpoint_core::coordinator::{
    AdmissionOutcome, CanonicalPeer, LeaseOwner, OutboundTransactionCoordinator,
};
use bacnet_endpoint_core::endpoint_ingress::{ClassifierExit, EndpointIngress, IngressReceivers};
use bacnet_network::layer::NetworkLayer;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::loopback::LoopbackTransport;
use tokio::time::timeout;

use super::endpoint_responder::EndpointResponder;
use super::*;

const WAIT: Duration = Duration::from_secs(1);

fn object_identifier() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap()
}

fn read_request(invoke_id: u8) -> Vec<u8> {
    let mut service_request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: object_identifier(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut service_request);
    let apdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: service_request.freeze(),
    });
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &apdu).unwrap();
    encoded.to_vec()
}

fn read_ack(request: &ConfirmedRequestPdu, value: f32) -> Vec<u8> {
    let mut property_value = BytesMut::new();
    encode_property_value(&mut property_value, &PropertyValue::Real(value)).unwrap();
    let ack = ReadPropertyACK {
        object_identifier: object_identifier(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: property_value.to_vec(),
    };
    let mut service_ack = BytesMut::new();
    ack.encode(&mut service_ack);
    let apdu = Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id: request.invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: request.service_choice,
        service_ack: service_ack.freeze(),
    });
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &apdu).unwrap();
    encoded.to_vec()
}

fn assert_real_value(ack: &ReadPropertyACK, expected: f32) {
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, &PropertyValue::Real(expected)).unwrap();
    assert_eq!(ack.property_value, encoded.to_vec());
}

#[tokio::test]
async fn loopback_shared_runtime_routes_same_invoke_id_by_admitted_role() {
    let (endpoint_transport, peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut endpoint = EndpointIngress::new(endpoint_transport, 8);
    let IngressReceivers {
        inbound_requests: mut inbound_rx,
        terminal_or_segment: mut terminal_rx,
        policy_outcomes: mut policy_rx,
        egress,
    } = endpoint.start().await.unwrap();
    assert!(endpoint.start().await.is_err());

    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let requester = EndpointRequester::new(
        egress.clone(),
        Arc::clone(&coordinator),
        ClientConfig {
            apdu_timeout_ms: 1_000,
            apdu_retries: 0,
            max_apdu_length: 480,
            ..ClientConfig::default()
        },
    )
    .unwrap();
    let notification_transactions =
        NotificationTransactions::with_coordinator(Arc::clone(&coordinator));

    let mut db = ObjectDatabase::new();
    let mut analog = AnalogInputObject::new(7, "endpoint-value", 0).unwrap();
    analog.set_present_value(42.0);
    db.add(Box::new(analog)).unwrap();
    let responder = Arc::new(EndpointResponder::new(
        Arc::new(RwLock::new(db)),
        egress.clone(),
    ));

    let (inbound_seen_tx, inbound_seen_rx) = oneshot::channel();
    let (release_responder_tx, release_responder_rx) = oneshot::channel();
    let responder_task = {
        let responder = Arc::clone(&responder);
        tokio::spawn(async move {
            let received = timeout(WAIT, inbound_rx.recv())
                .await
                .unwrap()
                .expect("inbound request route closed");
            inbound_seen_tx.send(()).unwrap();
            release_responder_rx.await.unwrap();
            responder.handle(received).await.unwrap()
        })
    };

    let admit_calls = Arc::new(AtomicUsize::new(0));
    let terminal_task = {
        let coordinator = Arc::clone(&coordinator);
        let requester = requester.clone();
        let notification_transactions = Arc::clone(&notification_transactions);
        let admit_calls = Arc::clone(&admit_calls);
        tokio::spawn(async move {
            let received = timeout(WAIT, terminal_rx.recv())
                .await
                .unwrap()
                .expect("terminal route closed");
            let peer =
                CanonicalPeer::from_source(&received.source_mac, received.source_network.as_ref());
            let decoded = decode_apdu(received.apdu.clone()).unwrap();
            admit_calls.fetch_add(1, Ordering::SeqCst);
            match coordinator.admit(&peer, &decoded).unwrap() {
                AdmissionOutcome::Admitted(admission) => match admission.metadata().owner() {
                    LeaseOwner::Requester => {
                        requester
                            .complete_pre_admitted(admission, decoded, received)
                            .await
                    }
                    LeaseOwner::ServerNotification => {
                        notification_transactions.complete_pre_admitted(admission, &decoded)
                    }
                },
                AdmissionOutcome::NotOutbound
                | AdmissionOutcome::UnknownInvokeId
                | AdmissionOutcome::PeerMismatch
                | AdmissionOutcome::OwnerMismatch
                | AdmissionOutcome::ServiceMismatch { .. }
                | AdmissionOutcome::PolicyMismatch
                | AdmissionOutcome::DirectionMismatch
                | AdmissionOutcome::DuplicateTerminal => false,
            }
        })
    };

    let mut peer = NetworkLayer::new(peer_transport);
    let mut peer_rx = peer.start().await.unwrap();
    let (responder_wire_tx, responder_wire_rx) = oneshot::channel();
    let peer_task = tokio::spawn(async move {
        peer.send_apdu(&read_request(0), &[0x01], true, NetworkPriority::NORMAL)
            .await
            .unwrap();

        let requester_wire = timeout(WAIT, peer_rx.recv()).await.unwrap().unwrap();
        let requester_request = match decode_apdu(requester_wire.apdu).unwrap() {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected requester ConfirmedRequest, got {other:?}"),
        };
        assert_eq!(requester_request.invoke_id, 0);
        peer.send_apdu(
            &read_ack(&requester_request, 12.5),
            &[0x01],
            false,
            NetworkPriority::NORMAL,
        )
        .await
        .unwrap();

        let responder_wire = timeout(WAIT, peer_rx.recv()).await.unwrap().unwrap();
        let responder_ack = match decode_apdu(responder_wire.apdu).unwrap() {
            Apdu::ComplexAck(ack) => ack,
            other => panic!("expected responder ComplexAck, got {other:?}"),
        };
        assert_eq!(responder_ack.invoke_id, 0);
        let response = ReadPropertyACK::decode(&responder_ack.service_ack).unwrap();
        assert_real_value(&response, 42.0);
        responder_wire_tx.send(()).unwrap();
        assert!(timeout(Duration::from_millis(50), peer_rx.recv())
            .await
            .is_err());
        (requester_request.invoke_id, responder_ack.invoke_id, peer)
    });

    timeout(WAIT, inbound_seen_rx).await.unwrap().unwrap();
    let requester_result = requester
        .read_property(
            &[0x02],
            object_identifier(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await
        .unwrap();
    assert_real_value(&requester_result, 12.5);
    assert!(terminal_task.await.unwrap());
    assert_eq!(admit_calls.load(Ordering::SeqCst), 1);
    assert_eq!(coordinator.active_count().unwrap(), 0);

    release_responder_tx.send(()).unwrap();
    timeout(WAIT, responder_wire_rx).await.unwrap().unwrap();
    requester.close();
    notification_transactions.close();
    responder.close();
    assert!(responder_task.await.unwrap());
    let (requester_invoke_id, responder_invoke_id, mut peer) = peer_task.await.unwrap();
    assert_eq!((requester_invoke_id, responder_invoke_id), (0, 0));

    assert!(matches!(
        endpoint.stop().await.unwrap(),
        ClassifierExit::Cancelled
    ));
    peer.stop().await.unwrap();
    assert!(endpoint.stop().await.is_err());
    assert!(policy_rx.recv().await.is_none());
    assert!(matches!(
        egress
            .send_direct(
                vec![0x20, 0x00, 0x0c],
                MacAddr::from_slice(&[0x02]),
                false,
                NetworkPriority::NORMAL,
            )
            .await,
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
}
