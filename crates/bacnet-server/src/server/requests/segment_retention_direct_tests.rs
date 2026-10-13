//! Retention Aborts keep the verified original direct response capability.
use super::*;
use crate::server::segmented_receive::{initial_state, RequestPayload};
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
        let mut payload = RequestPayload::new(&request);
        payload
            .save_new(0, request.service_request.clone(), Some(0))
            .unwrap();
        let source = MacAddr::from_slice(SOURCE);
        let (mut state, _) = initial_state(
            payload,
            provenance,
            Some(response),
            source.clone(),
            None,
            &request,
        );
        let now = crate::runtime_clock::now();
        state.last_activity = now;
        state.last_progress = now - std::time::Duration::from_secs(16);
        let key =
            crate::server::segmented_send::segmented_receive_key(&source, None, 42, provenance);
        let mut receivers = HashMap::from([(key, state)]);
        if retired {
            listener.stop().await;
        }
        BACnetServer::<TestTransport>::reap_expired_requests(
            &network,
            &mut receivers,
            now,
            std::time::Duration::from_secs(20),
        )
        .await;
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
