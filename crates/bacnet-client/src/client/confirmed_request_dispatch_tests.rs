//! Responding-client behavior for inbound ConfirmedRequest APDUs (issue #374).

use super::*;
use bacnet_encoding::apdu::{decode_apdu, ConfirmedRequest};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::AbortReason;

struct ImmediateReplyTransport {
    local_mac: MacAddr,
    inbound_rx: Option<mpsc::Receiver<ReceivedNpdu>>,
    fallback_sends: Arc<Mutex<Vec<Bytes>>>,
}

impl TransportPort for ImmediateReplyTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.inbound_rx.take().expect("transport started once"))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        self.fallback_sends
            .lock()
            .await
            .push(Bytes::copy_from_slice(npdu));
        Ok(())
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.fallback_sends
            .lock()
            .await
            .push(Bytes::copy_from_slice(npdu));
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
}

fn confirmed_request(
    invoke_id: u8,
    service_choice: ConfirmedServiceChoice,
    segmented: bool,
    service_request: Bytes,
) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented,
        more_follows: segmented,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id,
        sequence_number: segmented.then_some(0),
        proposed_window_size: segmented.then_some(1),
        service_choice,
        service_request,
    })
}

fn encode_inbound(apdu: Apdu, source_network: Option<NpduAddress>) -> BytesMut {
    encode_inbound_with_destination(apdu, source_network, None)
}

fn encode_inbound_with_destination(
    apdu: Apdu,
    source_network: Option<NpduAddress>,
    destination: Option<NpduAddress>,
) -> BytesMut {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &apdu).unwrap();
    let mut npdu_buf = BytesMut::new();
    encode_npdu(
        &mut npdu_buf,
        &Npdu {
            source: source_network,
            destination,
            payload: apdu_buf.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    npdu_buf
}

async fn receive_response(
    peer_rx: &mut mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
) -> (Npdu, Apdu) {
    let received = timeout(Duration::from_secs(2), peer_rx.recv())
        .await
        .expect("peer timed out waiting for response")
        .expect("peer channel closed");
    let npdu = decode_npdu(received.npdu).unwrap();
    let apdu = decode_apdu(npdu.payload.clone()).unwrap();
    (npdu, apdu)
}

fn assert_unrecognized_reject(apdu: Apdu, invoke_id: u8) {
    let Apdu::Reject(reject) = apdu else {
        panic!("expected Reject");
    };
    assert_eq!(reject.invoke_id, invoke_id);
    assert_eq!(reject.reject_reason, RejectReason::UNRECOGNIZED_SERVICE);
}

#[tokio::test]
async fn unsupported_local_confirmed_request_receives_reject() {
    let client_mac = vec![0x01];
    let peer_mac = vec![0x02];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac);
    let mut peer_rx = peer_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();

    let request = encode_inbound(
        confirmed_request(
            0x31,
            ConfirmedServiceChoice::READ_PROPERTY,
            false,
            Bytes::new(),
        ),
        None,
    );
    peer_transport
        .send_unicast(&request, &client_mac)
        .await
        .unwrap();

    let (npdu, apdu) = receive_response(&mut peer_rx).await;
    assert!(npdu.destination.is_none());
    assert_unrecognized_reject(apdu, 0x31);

    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn unsupported_routed_confirmed_request_receives_routed_reject() {
    let client_mac = vec![0x11];
    let router_mac = vec![0x12];
    let remote = NpduAddress {
        network: 200,
        mac_address: MacAddr::from_slice(&[0x13, 0x14]),
    };
    let (client_transport, mut router_transport) =
        LoopbackTransport::pair(client_mac.clone(), router_mac);
    let mut router_rx = router_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();

    let request = encode_inbound(
        confirmed_request(
            0x32,
            ConfirmedServiceChoice::READ_PROPERTY,
            false,
            Bytes::new(),
        ),
        Some(remote.clone()),
    );
    router_transport
        .send_unicast(&request, &client_mac)
        .await
        .unwrap();

    let (npdu, apdu) = receive_response(&mut router_rx).await;
    assert_eq!(npdu.destination, Some(remote));
    assert_unrecognized_reject(apdu, 0x32);

    client.stop().await.unwrap();
    router_transport.stop().await.unwrap();
}

#[tokio::test]
async fn unknown_confirmed_service_receives_unrecognized_service_reject() {
    let client_mac = vec![0x21];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), vec![0x22]);
    let mut peer_rx = peer_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();

    let request = encode_inbound(
        confirmed_request(
            0x33,
            ConfirmedServiceChoice::from_raw(0xFE),
            false,
            Bytes::new(),
        ),
        None,
    );
    peer_transport
        .send_unicast(&request, &client_mac)
        .await
        .unwrap();

    let (_, apdu) = receive_response(&mut peer_rx).await;
    assert_unrecognized_reject(apdu, 0x33);

    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn segmented_confirmed_request_is_aborted_by_responding_client() {
    let client_mac = vec![0x31];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), vec![0x32]);
    let mut peer_rx = peer_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();

    let request = encode_inbound(
        confirmed_request(
            0x34,
            ConfirmedServiceChoice::READ_PROPERTY,
            true,
            Bytes::from_static(&[0x0c]),
        ),
        None,
    );
    peer_transport
        .send_unicast(&request, &client_mac)
        .await
        .unwrap();

    let (_, apdu) = receive_response(&mut peer_rx).await;
    let Apdu::Abort(abort) = apdu else {
        panic!("expected Abort");
    };
    assert!(abort.sent_by_server);
    assert_eq!(abort.invoke_id, 0x34);
    assert_eq!(abort.abort_reason, AbortReason::SEGMENTATION_NOT_SUPPORTED);

    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn malformed_confirmed_cov_receives_reject_without_delivery() {
    let client_mac = vec![0x41];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), vec![0x42]);
    let mut peer_rx = peer_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let mut cov_rx = client.cov_notifications();

    let request = encode_inbound(
        confirmed_request(
            0x35,
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            false,
            Bytes::new(),
        ),
        None,
    );
    peer_transport
        .send_unicast(&request, &client_mac)
        .await
        .unwrap();

    let (_, apdu) = receive_response(&mut peer_rx).await;
    let Apdu::Reject(reject) = apdu else {
        panic!("expected Reject");
    };
    assert_eq!(reject.invoke_id, 0x35);
    assert_eq!(
        reject.reject_reason,
        RejectReason::MISSING_REQUIRED_PARAMETER
    );
    assert!(timeout(Duration::from_millis(100), cov_rx.recv())
        .await
        .is_err());

    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn partial_confirmed_cov_reports_missing_required_parameter() {
    let client_mac = vec![0x43];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), vec![0x44]);
    let mut peer_rx = peer_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let mut cov_rx = client.cov_notifications();

    let request = encode_inbound(
        confirmed_request(
            0x39,
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            false,
            Bytes::from_static(&[0x09, 0x01]),
        ),
        None,
    );
    peer_transport
        .send_unicast(&request, &client_mac)
        .await
        .unwrap();

    let (_, apdu) = receive_response(&mut peer_rx).await;
    let Apdu::Reject(reject) = apdu else {
        panic!("expected Reject");
    };
    assert_eq!(reject.invoke_id, 0x39);
    assert_eq!(
        reject.reject_reason,
        RejectReason::MISSING_REQUIRED_PARAMETER
    );
    assert!(timeout(Duration::from_millis(100), cov_rx.recv())
        .await
        .is_err());

    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn invalid_confirmed_cov_representation_reports_invalid_data_encoding() {
    let client_mac = vec![0x45];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), vec![0x46]);
    let mut peer_rx = peer_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let mut cov_rx = client.cov_notifications();

    let request = encode_inbound(
        confirmed_request(
            0x3A,
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            false,
            Bytes::from_static(&[0x09, 0x01, 0x1B, 0x01, 0x02, 0x03]),
        ),
        None,
    );
    peer_transport
        .send_unicast(&request, &client_mac)
        .await
        .unwrap();

    let (_, apdu) = receive_response(&mut peer_rx).await;
    let Apdu::Reject(reject) = apdu else {
        panic!("expected Reject");
    };
    assert_eq!(reject.invoke_id, 0x3A);
    assert_eq!(reject.reject_reason, RejectReason::INVALID_DATA_ENCODING);
    assert!(timeout(Duration::from_millis(100), cov_rx.recv())
        .await
        .is_err());

    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn unsupported_confirmed_group_delivery_remains_silent() {
    let client_mac = vec![0x51];
    let (client_transport, mut peer_transport) = LoopbackTransport::pair(client_mac, vec![0x52]);
    let mut peer_rx = peer_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();

    let request = encode_inbound(
        confirmed_request(
            0x36,
            ConfirmedServiceChoice::READ_PROPERTY,
            false,
            Bytes::new(),
        ),
        None,
    );
    peer_transport.send_broadcast(&request).await.unwrap();

    assert!(timeout(Duration::from_millis(100), peer_rx.recv())
        .await
        .is_err());

    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn unsupported_confirmed_request_uses_immediate_reply_channel() {
    let local_mac = MacAddr::from_slice(&[0x71]);
    let router_mac = MacAddr::from_slice(&[0x72]);
    let remote = NpduAddress {
        network: 300,
        mac_address: MacAddr::from_slice(&[0x73, 0x74]),
    };
    let (inbound_tx, inbound_rx) = mpsc::channel(1);
    let fallback_sends = Arc::new(Mutex::new(Vec::new()));
    let transport = ImmediateReplyTransport {
        local_mac,
        inbound_rx: Some(inbound_rx),
        fallback_sends: Arc::clone(&fallback_sends),
    };
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap();
    let request = encode_inbound(
        confirmed_request(
            0x38,
            ConfirmedServiceChoice::READ_PROPERTY,
            false,
            Bytes::new(),
        ),
        Some(remote.clone()),
    );
    let (reply_tx, reply_rx) = oneshot::channel();

    inbound_tx
        .send(ReceivedNpdu {
            npdu: request.freeze(),
            source_mac: router_mac,
            link_layer_group: false,
            data_attributes: Vec::new(),
            reply_tx: Some(reply_tx),
        })
        .await
        .unwrap();

    let reply = timeout(Duration::from_secs(2), reply_rx)
        .await
        .expect("timed out waiting for immediate response")
        .expect("immediate response channel closed");
    let npdu = decode_npdu(reply).unwrap();
    assert_eq!(npdu.destination, Some(remote));
    assert_unrecognized_reject(decode_apdu(npdu.payload).unwrap(), 0x38);
    assert!(fallback_sends.lock().await.is_empty());

    client.stop().await.unwrap();
}

#[tokio::test]
async fn global_broadcast_confirmed_request_remains_silent() {
    let client_mac = vec![0x61];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), vec![0x62]);
    let mut peer_rx = peer_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();

    let request = encode_inbound_with_destination(
        confirmed_request(
            0x37,
            ConfirmedServiceChoice::READ_PROPERTY,
            false,
            Bytes::new(),
        ),
        None,
        Some(NpduAddress {
            network: 0xFFFF,
            mac_address: MacAddr::new(),
        }),
    );
    // A BBMD can distribute a global broadcast in a unicast data-link frame;
    // the NPDU destination still makes the ConfirmedRequest a broadcast.
    peer_transport
        .send_unicast(&request, &client_mac)
        .await
        .unwrap();

    assert!(timeout(Duration::from_millis(100), peer_rx.recv())
        .await
        .is_err());

    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}
