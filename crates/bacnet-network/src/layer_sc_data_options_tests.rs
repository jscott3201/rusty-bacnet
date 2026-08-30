use super::NetworkLayer;
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_transport::sc::{LoopbackWebSocket, ScTransport, WebSocketPort};
use bacnet_transport::sc_frame::{
    decode_sc_message, encode_sc_message, ScFunction, ScMessage, ScOption, Vmac,
};
use bacnet_types::enums::NetworkPriority;
use bytes::{Bytes, BytesMut};
use tokio::time::{timeout, Duration};

async fn sc_hub_accept(ws_hub: &LoopbackWebSocket, hub_vmac: Vmac) {
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

#[tokio::test]
async fn sc_data_options_reach_received_apdu_data_attributes() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let hub_vmac = [0x10; 6];
    let mut net = NetworkLayer::new(ScTransport::new(ws_client, [0x01; 6]));

    let hub_accept_task = tokio::spawn(async move {
        sc_hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let mut rx = net.start().await.unwrap();
    let ws_hub = hub_accept_task.await.unwrap();

    let apdu = Bytes::from_static(&[0x10, 0x08]);
    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: NetworkPriority::NORMAL,
        destination: None,
        source: None,
        payload: apdu.clone(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();

    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2345,
        originating_vmac: Some(hub_vmac),
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: vec![
            ScOption {
                option_type: 1,
                must_understand: true,
                data: Vec::new(),
            },
            ScOption {
                option_type: 31,
                must_understand: false,
                data: vec![0x12, 0x34, 0x56],
            },
        ],
        payload: npdu_buf.freeze(),
    };
    let mut sc_buf = BytesMut::new();
    encode_sc_message(&mut sc_buf, &msg);
    ws_hub.send(&sc_buf).await.unwrap();

    let received = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for APDU")
        .expect("APDU channel closed");

    assert_eq!(received.apdu, apdu);
    assert_eq!(received.source_mac.as_slice(), hub_vmac);
    assert!(received.source_network.is_none());
    assert_eq!(received.data_attributes.len(), 2);
    assert_eq!(received.data_attributes[0].option_type, 1);
    assert!(received.data_attributes[0].must_understand);
    assert!(received.data_attributes[0].data.is_empty());
    assert_eq!(received.data_attributes[1].option_type, 31);
    assert!(!received.data_attributes[1].must_understand);
    assert_eq!(received.data_attributes[1].data, vec![0x12, 0x34, 0x56]);

    net.stop().await.unwrap();
}
