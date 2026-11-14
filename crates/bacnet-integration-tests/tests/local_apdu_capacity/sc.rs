//! Actual SC server startup and failover preserve the local I-Am declaration.
use super::*;
use bacnet_transport::{
    sc::{LoopbackWebSocket, ScConnectionState, ScReconnectConfig, ScTransport, WebSocketPort},
    sc_frame::{ScFunction, ScMessage, decode_sc_message, encode_sc_message},
    test_transport::{SendMode, TestTransport},
};
use bytes::{Bytes, BytesMut};

async fn accept(hub: &LoopbackWebSocket, npdu: u16, bvlc: u16) {
    let request = decode_sc_message(&hub.recv().await.unwrap()).unwrap();
    assert_eq!(request.function, ScFunction::ConnectRequest);
    assert_eq!(&request.payload[22..], &[0x16, 0x49, 0x05, 0xc6]);
    let mut payload = vec![0x10; 6];
    payload.extend_from_slice(&[0x20; 16]);
    payload.extend_from_slice(&bvlc.to_be_bytes());
    payload.extend_from_slice(&npdu.to_be_bytes());
    let mut bytes = BytesMut::new();
    encode_sc_message(
        &mut bytes,
        &ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: request.message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: vec![],
            data_options: vec![],
            payload: Bytes::from(payload),
        },
    );
    hub.send(&bytes).await.unwrap();
}
async fn assert_iam(hub: &LoopbackWebSocket) {
    let wire = tokio::time::timeout(Duration::from_secs(2), hub.recv())
        .await
        .unwrap()
        .unwrap();
    let frame = decode_sc_message(&wire).unwrap();
    assert_eq!(frame.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(frame.destination_vmac, Some([0xff; 6]));
    let Apdu::UnconfirmedRequest(request) =
        decode_apdu(decode_npdu(frame.payload).unwrap().payload).unwrap()
    else {
        panic!("I-Am expected")
    };
    assert_eq!(request.service_choice.to_raw(), 0);
    assert_eq!(
        IAmRequest::decode(&request.service_request)
            .unwrap()
            .max_apdu_length,
        1476
    );
}
/// `inner` behind a test transport that notes, at each broadcast, the
/// receive capacity and egress limit `inner` reports then.
fn observed_sc(
    inner: ScTransport<LoopbackWebSocket>,
    sends: Arc<std::sync::Mutex<Vec<(u16, u16)>>>,
) -> TestTransport {
    let builder = TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Forward);
    let handle = builder.handle();
    builder
        .on_send(move |frame| {
            if frame.broadcast {
                let limits = handle.inner_limits().expect("the inner transport's limits");
                sends
                    .lock()
                    .unwrap()
                    .push((limits.receive_capacity, limits.egress_limit));
            }
            async { Ok(()) }
        })
        .build()
}

#[tokio::test]
async fn sc_server_egress_1474_then_failover_keeps_device_and_iam_1476() {
    let (node, primary) = LoopbackWebSocket::pair();
    let (backup, failover) = LoopbackWebSocket::pair();
    let transport = ScTransport::new(node, [1; 6])
        .with_device_uuid([1; 16])
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 25,
            max_delay_ms: 25,
            max_retries: 1,
        })
        .with_failover(backup);
    let mut states = transport.connection_state_changes();
    let sends = Arc::new(std::sync::Mutex::new(Vec::new()));
    let transport = observed_sc(transport, Arc::clone(&sends));
    let start = BACnetServer::start(
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::new(893, 0).unwrap(),
            ..Default::default()
        },
        database(Some(1476)),
        transport,
    );
    let (started, ()) = tokio::join!(start, accept(&primary, 1476, 5705));
    let mut server = started.unwrap();
    server.broadcast_i_am().await.unwrap();
    assert_iam(&primary).await;
    drop(primary);
    tokio::time::timeout(Duration::from_secs(2), accept(&failover, 480, 300))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while *states.borrow_and_update() != ScConnectionState::Connected {
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    server.broadcast_i_am().await.unwrap();
    assert_iam(&failover).await;
    assert_eq!(*sends.lock().unwrap(), [(1476, 1474), (1476, 288)]);
    server.stop().await.unwrap();
}
