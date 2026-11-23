//! Retention Aborts keep the verified original direct response capability.
use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_endpoint_core::segmented_request::{Receipt, Segment, SegmentedRequests};
use bacnet_transport::sc::WebSocketPort;

#[tokio::test]
async fn progress_abort_uses_original_direct_capability_and_retirement_never_falls_back() {
    for retired in [false, true] {
        let (provenance, response, mut listener, peer) = direct_peer().await;
        let transport = TestTransport::builder().build();
        let sent = transport.sent();
        let network = Arc::new(NetworkLayer::new(transport));
        let request = ConfirmedRequestPdu {
            segmented: true,
            more_follows: true,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 42,
            sequence_number: Some(0),
            proposed_window_size: Some(1),
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: Bytes::from_static(&[1]),
        };
        // Opened 16 s before `now` and never heard from since; four
        // segment timeouts of 5 s aren't up yet, so it ends with an Abort.
        let start = crate::runtime_clock::now();
        let now = start + std::time::Duration::from_secs(16);
        let mut receivers = SegmentedRequests::new(std::time::Duration::from_secs(5));
        let route =
            bacnet_network::response_route::ResponseRoute::new(provenance.clone(), Some(response));
        let receipt = receivers.receive(
            Segment {
                source_mac: SOURCE,
                source_network: None,
                route: &route,
                request: &request,
            },
            start,
        );
        assert!(matches!(receipt, Receipt::Ack(_)));
        if retired {
            listener.stop().await;
        }
        BACnetServer::<TestTransport>::reap_expired_requests(&network, &mut receivers, now).await;
        assert!(receivers.is_empty());
        assert!(
            sent.is_empty(),
            "direct authority never falls back to a MAC send"
        );
        if !retired {
            let wire = tokio::time::timeout(std::time::Duration::from_secs(2), peer.recv())
                .await
                .unwrap()
                .unwrap();
            let message = bacnet_transport::sc_frame::decode_sc_message(&wire).unwrap();
            let npdu = decode_npdu(message.payload).unwrap();
            assert_eq!(npdu.destination, None);
            assert!(
                matches!(decode_apdu(npdu.payload).unwrap(), Apdu::Abort(abort)
                if abort.sent_by_server && abort.invoke_id == 42 && abort.abort_reason == AbortReason::OTHER)
            );
            listener.stop().await;
        }
    }
}
