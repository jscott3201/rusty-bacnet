use super::*;
use crate::sc_frame::{decode_sc_bvlc_result, ScBvlcResult, ScOption};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use tokio::time::{timeout, Duration};

async fn hub_accept(ws_hub: &LoopbackWebSocket, hub_vmac: Vmac) {
    let data = ws_hub.recv().await.unwrap();
    let req = decode_sc_message(&data).unwrap();
    assert_eq!(req.function, ScFunction::ConnectRequest);

    let mut accept_payload = Vec::with_capacity(26);
    accept_payload.extend_from_slice(&hub_vmac);
    accept_payload.extend_from_slice(&[0u8; 16]);
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());

    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(accept_payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &accept);
    ws_hub.send(&buf).await.unwrap();
}

fn encapsulated_npdu_with_data_option(
    message_id: u16,
    destination_vmac: Option<Vmac>,
    option: ScOption,
) -> ScMessage {
    ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id,
        originating_vmac: Some([0x10; 6]),
        destination_vmac,
        dest_options: Vec::new(),
        data_options: vec![option],
        payload: Bytes::from_static(&[0x01, 0x00, 0x30]),
    }
}

async fn start_transport() -> (
    ScTransport<LoopbackWebSocket>,
    mpsc::Receiver<ReceivedNpdu>,
    LoopbackWebSocket,
) {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6]);
    let hub_task = tokio::spawn(async move {
        hub_accept(&ws_hub, [0x10; 6]).await;
        ws_hub
    });

    let rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();
    (transport, rx, ws_hub)
}

#[tokio::test]
async fn unsupported_must_understand_data_option_unicast_returns_nak() {
    let (mut transport, mut rx, ws_hub) = start_transport().await;
    let option = ScOption {
        option_type: 2,
        must_understand: true,
        data: Vec::new(),
    };
    let msg = encapsulated_npdu_with_data_option(0x2233, None, option);
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &msg);
    ws_hub.send(&buf).await.unwrap();

    let nak_data = timeout(Duration::from_secs(1), ws_hub.recv())
        .await
        .expect("timed out waiting for BVLC-Result NAK")
        .unwrap();
    let nak = decode_sc_message(&nak_data).unwrap();
    assert_eq!(nak.message_id, msg.message_id);
    assert_eq!(nak.data_options.len(), 0);
    assert_eq!(
        decode_sc_bvlc_result(&nak).unwrap(),
        ScBvlcResult::Nak {
            result_for: ScFunction::EncapsulatedNpdu,
            error_header_marker: 0x42,
            error_class: ErrorClass::COMMUNICATION.to_raw(),
            error_code: ErrorCode::HEADER_NOT_UNDERSTOOD.to_raw(),
            error_details: String::new(),
        }
    );
    assert!(timeout(Duration::from_millis(50), rx.recv()).await.is_err());

    let state = transport.connection().unwrap().lock().await.state;
    assert_eq!(state, ScConnectionState::Connected);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn unsupported_must_understand_data_option_broadcast_drops_without_nak() {
    let (mut transport, mut rx, ws_hub) = start_transport().await;
    let option = ScOption {
        option_type: 31,
        must_understand: true,
        data: vec![0x12, 0x34, 0x56],
    };
    let msg = encapsulated_npdu_with_data_option(0x3344, Some(BROADCAST_VMAC), option);
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &msg);
    ws_hub.send(&buf).await.unwrap();

    assert!(timeout(Duration::from_millis(50), rx.recv()).await.is_err());
    assert!(timeout(Duration::from_millis(50), ws_hub.recv())
        .await
        .is_err());

    let state = transport.connection().unwrap().lock().await.state;
    assert_eq!(state, ScConnectionState::Connected);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn unsupported_non_must_understand_data_option_is_preserved() {
    let (mut transport, mut rx, ws_hub) = start_transport().await;
    let option = ScOption {
        option_type: 2,
        must_understand: false,
        data: vec![0xAA],
    };
    let msg = encapsulated_npdu_with_data_option(0x4455, None, option.clone());
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &msg);
    ws_hub.send(&buf).await.unwrap();

    let received = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for SC NPDU")
        .expect("SC NPDU channel closed");
    assert_eq!(received.npdu, msg.payload);
    assert_eq!(received.data_attributes.len(), 1);
    assert_eq!(received.data_attributes[0].option_type, option.option_type);
    assert_eq!(
        received.data_attributes[0].must_understand,
        option.must_understand
    );
    assert_eq!(received.data_attributes[0].data, option.data);
    assert!(timeout(Duration::from_millis(50), ws_hub.recv())
        .await
        .is_err());

    transport.stop().await.unwrap();
}
