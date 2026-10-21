//! The native notification owner is independent of the standalone client TSM.
//! Actual transport/network envelopes feed its public dispatch admission API;
//! this fixture does not claim a full BACnetServer COV subscription lifecycle.
use super::*;
use bacnet_encoding::apdu::SimpleAck;
use bacnet_endpoint_core::coordinator::{CanonicalPeer, OutboundTransactionCoordinator};
use bacnet_network::layer::NetworkLayer;
use bacnet_npdu::NetworkPriority;
use bacnet_server::server::{
    __endpoint_NotificationTransactions as NotificationTransactions,
    __endpoint_NotificationWorkerResult as NotificationWorkerResult,
    __endpoint_run_notification_worker as run_notification_worker,
};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn standard_native_notification_owner_accepts_hub_direct_path_switches() {
    // Direct -> Hub, Hub -> accepted direct, and direct A -> replacement B.
    for mode in 0..3 {
        let hub_request = mode == 1;
        let ca = TestCa::new();
        let (mut peer, mut port) = Peer::start(&ca).await;
        if hub_request {
            port.transport = port.transport.with_direct_discovery(false);
        }
        let mut network = NetworkLayer::new(port);
        let mut incoming = network.start().await.unwrap();
        let coordinator = Arc::new(OutboundTransactionCoordinator::new());
        let owner = NotificationTransactions::with_coordinator(coordinator.clone());
        let service = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION;
        let (operation, receiver) = owner
            .reserve(CanonicalPeer::direct(&REMOTE), service)
            .unwrap();
        let mut notification = super::super::notifications::cov(1);
        let Apdu::ConfirmedRequest(ref mut request) = notification else {
            unreachable!()
        };
        request.invoke_id = operation.invoke_id();
        let invoke = request.invoke_id;
        let mut encoded = BytesMut::new();
        encode_apdu(&mut encoded, &notification).unwrap();
        let send =
            run_notification_worker(operation, receiver, Duration::from_secs(3), 0, |attempt| {
                assert_eq!(attempt, 0);
                network.send_apdu(&encoded, &REMOTE, true, NetworkPriority::Normal)
            });
        let remote = async {
            let observed = if hub_request {
                let frame = decode_sc_message(&bounded(peer.hub.recv()).await.unwrap()).unwrap();
                assert_eq!(frame.destination_vmac, Some(REMOTE));
                decode_apdu(decode_npdu(frame.payload).unwrap().payload).unwrap()
            } else {
                peer.receive().await
            };
            assert!(
                matches!(observed,Apdu::ConfirmedRequest(r) if r.invoke_id==invoke && r.service_choice==service)
            );
            let wrong = Apdu::SimpleAck(SimpleAck {
                invoke_id: invoke,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            });
            let correct = Apdu::SimpleAck(SimpleAck {
                invoke_id: invoke,
                service_choice: service,
            });
            if mode == 0 {
                peer.hub_reply(&wrong).await;
                peer.hub_reply(&correct).await;
                None
            } else {
                let ws = bacnet_transport::sc_tls::TlsWebSocket::connect_direct(
                    &format!(
                        "wss://localhost:{}/.bacnet/sc",
                        peer.local.local_addr().port()
                    ),
                    ca.tls("native-responder"),
                )
                .await
                .unwrap();
                let mut connection = bacnet_transport::sc::ScConnection::new(REMOTE, [2; 16]);
                let mut bytes = BytesMut::new();
                encode_sc_message(&mut bytes, &connection.build_connect_request());
                ws.send(&bytes).await.unwrap();
                assert_eq!(
                    decode_sc_message(&ws.recv().await.unwrap())
                        .unwrap()
                        .function,
                    ScFunction::ConnectAccept
                );
                for ack in [wrong, correct] {
                    bytes.clear();
                    encode_sc_message(
                        &mut bytes,
                        &connection
                            .build_direct_encapsulated_npdu(&wire(&ack), &[])
                            .unwrap(),
                    );
                    ws.send(&bytes).await.unwrap();
                }
                Some(ws)
            }
        };
        let dispatch = async {
            for expected in [false, true] {
                let envelope = bounded(incoming.recv()).await.unwrap();
                assert_eq!(envelope.source_mac.as_slice(), &REMOTE);
                assert!(envelope.source_network.is_none());
                assert_eq!(envelope.provenance.is_direct_peer(), mode != 0);
                assert_eq!(envelope.direct_response.is_some(), mode != 0);
                let apdu = decode_apdu(envelope.apdu).unwrap();
                assert_eq!(
                    owner.admit_terminal(
                        &envelope.source_mac,
                        envelope.source_network.as_ref(),
                        None,
                        &apdu
                    ),
                    expected
                );
                assert_eq!(coordinator.active_count().unwrap(), usize::from(!expected));
            }
        };
        let (result, _remote, ()) = tokio::join!(send, remote, dispatch);
        assert!(matches!(result, NotificationWorkerResult::Ack));
        assert_eq!(coordinator.active_count().unwrap(), 0);
        owner.close();
        network.stop().await.unwrap();
        peer.stop().await;
    }
}
