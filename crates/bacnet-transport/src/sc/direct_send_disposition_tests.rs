//! Public custom factories remain send-only; uncertain writes never replay.
use super::*;

#[tokio::test]
async fn custom_direct_factory_cannot_admit_application_npdu_or_fabricate_identity() {
    let (mut transport, mut rx, hub, _, mut peers) = start_direct(500).await;
    let (sent, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar = decode_sc_message(&hub_recv(&hub).await).unwrap();
        hub.send(&ack_for(ar.message_id, b"wss://peer.example/sc"))
            .await
            .unwrap();
    });
    sent.unwrap();
    let peer = peers.recv().await.unwrap();
    let request = peer.recv().await.unwrap();
    peer.send(&request).await.unwrap();
    let mut bytes = BytesMut::new();
    encode_sc_message(
        &mut bytes,
        &crate::sc::direct_membership::disconnect_request(),
    );
    peer.send(&bytes).await.unwrap();
    // Ordered Disconnect ACK proves the preceding application frame was read.
    let ack = timeout(Duration::from_secs(2), peer.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        decode_sc_message(&ack).unwrap().function,
        ScFunction::DisconnectAck
    );
    assert!(rx.try_recv().is_err());
    transport.stop().await.unwrap();
}
struct AfterWriteFailure {
    ws: LoopbackWebSocket,
    fail: bool,
}
impl WebSocketPort for AfterWriteFailure {
    async fn recv(&self) -> Result<Vec<u8>, Error> {
        self.ws.recv().await
    }
    async fn send(&self, bytes: &[u8]) -> Result<(), Error> {
        self.ws.send(bytes).await?;
        if self.fail && decode_sc_message(bytes).unwrap().function == ScFunction::EncapsulatedNpdu {
            Err(Error::Transport(std::io::Error::other(
                "failure after actual fixture write",
            )))
        } else {
            Ok(())
        }
    }
}
#[tokio::test]
async fn uncertain_started_direct_write_is_not_redialed_or_replayed_through_hub() {
    let attempted = Arc::new(Mutex::new(Vec::new()));
    let (peers, mut received) = mpsc::unbounded_channel();
    let factory = fake_dialer(attempted.clone(), peers);
    let (ws, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(AfterWriteFailure { ws, fail: false }, [1; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(500)
        .with_custom_direct_dialer(move |uri| {
            let socket = factory(uri);
            async move {
                Ok(AfterWriteFailure {
                    ws: socket.await?,
                    fail: true,
                })
            }
        });
    let (started, ()) = tokio::join!(
        transport.start(),
        data_attribute_tests::hub_accept(&hub, [0x10; 6])
    );
    let _rx = started.unwrap();
    let (sent, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar = decode_sc_message(&hub_recv(&hub).await).unwrap();
        hub.send(&ack_for(
            ar.message_id,
            b"wss://one.example/sc wss://two.example/sc",
        ))
        .await
        .unwrap();
    });
    assert!(sent.is_err());
    assert_eq!(attempted.lock().await.as_slice(), &["wss://one.example/sc"]);
    let peer = received.recv().await.unwrap();
    let actual = decode_sc_message(&peer.recv().await.unwrap()).unwrap();
    assert_eq!(actual.payload.as_ref(), NPDU);
    assert_eq!(
        decode_sc_message(&peer.recv().await.unwrap())
            .unwrap()
            .function,
        ScFunction::DisconnectRequest
    );
    assert!(
        hub_try_recv(&hub).await.is_none(),
        "uncertain payload must never go to Hub"
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn direct_length_refusals_never_write_to_peer_or_fall_back_to_hub() {
    let (mut transport, _rx, hub, attempted, mut peers) = start_direct(500).await;
    let (sent, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar = decode_sc_message(&hub_recv(&hub).await).unwrap();
        hub.send(&ack_for(ar.message_id, b"wss://peer.example/sc"))
            .await
            .unwrap();
    });
    sent.unwrap();
    let peer = peers.recv().await.unwrap();
    assert_eq!(
        decode_sc_message(&peer.recv().await.unwrap())
            .unwrap()
            .payload
            .as_ref(),
        NPDU
    );
    for attributes_only in [false, true] {
        let npdu = if attributes_only {
            NPDU.to_vec()
        } else {
            vec![0x10; 1477]
        };
        let attributes = if attributes_only {
            vec![crate::port::DataAttribute {
                option_type: 31,
                must_understand: false,
                data: vec![0x55; 1476],
            }]
        } else {
            Vec::new()
        };
        let error = transport
            .send_unicast_with_data_attributes(&npdu, &TARGET, &attributes)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::SendTooLong { length, limit: 1476 } if length > 1476));
        // A subsequent fitting write is an ordered barrier on this route.
        transport.send_unicast(NPDU, &TARGET).await.unwrap();
        let actual = decode_sc_message(&peer.recv().await.unwrap()).unwrap();
        assert_eq!(actual.payload.as_ref(), NPDU);
        assert!(actual.data_options.is_empty());
    }
    assert_eq!(
        attempted.lock().await.as_slice(),
        &["wss://peer.example/sc"]
    );
    assert!(
        hub_try_recv(&hub).await.is_none(),
        "no length refusal is replayed through Hub"
    );
    transport.stop().await.unwrap();
}
