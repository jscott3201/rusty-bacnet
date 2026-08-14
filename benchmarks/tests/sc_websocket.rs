//! Integration tests for BACnet/SC WebSocket layer behavior.

use std::time::Duration;

use bacnet_benchmarks::sc_helpers::{
    generate_test_certs, make_client_tls_config, make_sc_transport, start_sc_hub, CertMaterial,
};
use bacnet_transport::port::TransportPort;
use bacnet_transport::sc::ScConnectionState;
use bacnet_transport::sc_frame::{
    decode_sc_bvlc_result, decode_sc_message, encode_sc_message, ScBvlcResult, ScFunction,
    ScMessage, ScOption, Vmac, BACNET_SC_HUB_SUBPROTOCOL, BROADCAST_VMAC,
};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::tungstenite::ClientRequestBuilder;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type ClientWs = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[tokio::test]
async fn sc_websocket_hub_subprotocol_handshake_succeeds() {
    let certs = generate_test_certs();
    let (mut hub, url) = start_sc_hub(&certs, [0x10; 6]).await;

    let request = ClientRequestBuilder::new(url.parse().unwrap())
        .with_sub_protocol(BACNET_SC_HUB_SUBPROTOCOL);
    let connector = tokio_tungstenite::Connector::Rustls(make_client_tls_config(&certs));
    let (_ws, response) =
        tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector))
            .await
            .unwrap();

    let selected = response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|value| value.to_str().ok());
    assert_eq!(selected, Some(BACNET_SC_HUB_SUBPROTOCOL));

    hub.stop().await;
}

#[tokio::test]
async fn sc_websocket_hub_rejects_missing_or_wrong_subprotocol() {
    let certs = generate_test_certs();
    let (mut hub, url) = start_sc_hub(&certs, [0x20; 6]).await;

    let missing_request = ClientRequestBuilder::new(url.parse().unwrap());
    let missing_connector = tokio_tungstenite::Connector::Rustls(make_client_tls_config(&certs));
    let missing_result = tokio_tungstenite::connect_async_tls_with_config(
        missing_request,
        None,
        false,
        Some(missing_connector),
    )
    .await;
    assert!(missing_result.is_err());

    let wrong_request =
        ClientRequestBuilder::new(url.parse().unwrap()).with_sub_protocol("dc.bsc.bacnet.org");
    let wrong_connector = tokio_tungstenite::Connector::Rustls(make_client_tls_config(&certs));
    let wrong_result = tokio_tungstenite::connect_async_tls_with_config(
        wrong_request,
        None,
        false,
        Some(wrong_connector),
    )
    .await;
    assert!(wrong_result.is_err());

    hub.stop().await;
}

#[tokio::test]
async fn sc_websocket_text_frame_closes_with_unsupported_data() {
    let certs = generate_test_certs();
    let (mut hub, url) = start_sc_hub(&certs, [0x30; 6]).await;

    let request = ClientRequestBuilder::new(url.parse().unwrap())
        .with_sub_protocol(BACNET_SC_HUB_SUBPROTOCOL);
    let connector = tokio_tungstenite::Connector::Rustls(make_client_tls_config(&certs));
    let (mut ws, _response) =
        tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector))
            .await
            .unwrap();

    ws.send(Message::Text("not a BVLC-SC binary frame".into()))
        .await
        .unwrap();
    let message = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("hub should close promptly after a text frame")
        .expect("hub should send a close frame")
        .expect("close frame should decode");

    match message {
        Message::Close(Some(frame)) => assert_eq!(frame.code, CloseCode::Unsupported),
        other => panic!("expected unsupported-data close frame, got {other:?}"),
    }

    hub.stop().await;
}

#[tokio::test]
async fn sc_websocket_hub_relays_unicast_unknown_and_broadcast_with_vmac_rules() {
    let certs = generate_test_certs();
    let (mut hub, url) = start_sc_hub(&certs, [0x10; 6]).await;

    let vmac_a = [0xA1; 6];
    let vmac_b = [0xB2; 6];
    let vmac_c = [0xC3; 6];

    let mut ws_a = connect_sc_client(&url, &certs, vmac_a).await;
    let mut ws_b = connect_sc_client(&url, &certs, vmac_b).await;
    let mut ws_c = connect_sc_client(&url, &certs, vmac_c).await;

    let unicast = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2001,
        originating_vmac: None,
        destination_vmac: Some(vmac_b),
        dest_options: vec![ScOption {
            option_type: 2,
            must_understand: false,
            data: vec![0xAA, 0xBB],
        }],
        data_options: vec![ScOption {
            option_type: 3,
            must_understand: true,
            data: Vec::new(),
        }],
        payload: Bytes::from_static(&[0x01, 0x20, 0x30]),
    };
    send_sc_message(&mut ws_a, &unicast).await;

    let relayed_unicast = recv_sc_message(&mut ws_b).await;
    assert_eq!(relayed_unicast.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(relayed_unicast.message_id, unicast.message_id);
    assert_eq!(relayed_unicast.originating_vmac, Some(vmac_a));
    assert_eq!(relayed_unicast.destination_vmac, None);
    assert_eq!(relayed_unicast.dest_options, unicast.dest_options);
    assert_eq!(relayed_unicast.data_options, unicast.data_options);
    assert_eq!(relayed_unicast.payload, unicast.payload);
    assert_no_sc_message(&mut ws_a).await;
    assert_no_sc_message(&mut ws_c).await;

    let unknown_unicast = ScMessage {
        destination_vmac: Some([0xD4; 6]),
        message_id: 0x2002,
        ..unicast.clone()
    };
    send_sc_message(&mut ws_a, &unknown_unicast).await;
    assert_no_sc_message(&mut ws_a).await;
    assert_no_sc_message(&mut ws_b).await;
    assert_no_sc_message(&mut ws_c).await;

    let broadcast = ScMessage {
        message_id: 0x2003,
        destination_vmac: Some(BROADCAST_VMAC),
        payload: Bytes::from_static(&[0x01, 0x04, 0x05]),
        ..unicast
    };
    send_sc_message(&mut ws_a, &broadcast).await;

    let relayed_b = recv_sc_message(&mut ws_b).await;
    let relayed_c = recv_sc_message(&mut ws_c).await;
    for relayed in [relayed_b, relayed_c] {
        assert_eq!(relayed.function, ScFunction::EncapsulatedNpdu);
        assert_eq!(relayed.message_id, broadcast.message_id);
        assert_eq!(relayed.originating_vmac, Some(vmac_a));
        assert_eq!(relayed.destination_vmac, Some(BROADCAST_VMAC));
        assert_eq!(relayed.dest_options, broadcast.dest_options);
        assert_eq!(relayed.data_options, broadcast.data_options);
        assert_eq!(relayed.payload, broadcast.payload);
    }
    assert_no_sc_message(&mut ws_a).await;

    hub.stop().await;
}

#[tokio::test]
async fn sc_websocket_routes_destination_option_nak_to_originating_node() {
    let certs = generate_test_certs();
    let (mut hub, url) = start_sc_hub(&certs, [0x10; 6]).await;

    let vmac_a = [0xA5; 6];
    let vmac_b = [0xB5; 6];
    let mut ws_a = connect_sc_client(&url, &certs, vmac_a).await;
    let mut transport_b = make_sc_transport(&url, &certs, vmac_b).await;
    let mut rx_b = transport_b.start().await.unwrap();

    let request = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2201,
        originating_vmac: None,
        destination_vmac: Some(vmac_b),
        dest_options: vec![ScOption {
            option_type: 2,
            must_understand: true,
            data: Vec::new(),
        }],
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x20, 0x30]),
    };
    let mut request_wire = BytesMut::new();
    encode_sc_message(&mut request_wire, &request);
    assert_eq!(request_wire[10], 0x42);
    request_wire[10] = 0x62;
    let mut request_wire = request_wire.to_vec();
    request_wire.splice(11..11, [0, 0]);
    ws_a.send(Message::Binary(request_wire.into()))
        .await
        .unwrap();

    let relayed_result = recv_sc_message(&mut ws_a).await;
    assert_eq!(relayed_result.function, ScFunction::Result);
    assert_eq!(relayed_result.message_id, request.message_id);
    assert_eq!(relayed_result.originating_vmac, Some(vmac_b));
    assert_eq!(relayed_result.destination_vmac, None);
    assert_eq!(
        decode_sc_bvlc_result(&relayed_result).unwrap(),
        ScBvlcResult::Nak {
            result_for: ScFunction::EncapsulatedNpdu,
            error_header_marker: 0x62,
            error_class: ErrorClass::COMMUNICATION.to_raw(),
            error_code: ErrorCode::HEADER_NOT_UNDERSTOOD.to_raw(),
            error_details: String::new(),
        }
    );
    assert!(tokio::time::timeout(Duration::from_millis(50), rx_b.recv())
        .await
        .is_err());
    assert_eq!(
        transport_b.connection().unwrap().lock().await.state,
        ScConnectionState::Connected
    );

    transport_b.stop().await.unwrap();
    hub.stop().await;
}

#[tokio::test]
async fn sc_websocket_hub_drops_peer_result_for_other_function() {
    let certs = generate_test_certs();
    let (mut hub, url) = start_sc_hub(&certs, [0x10; 6]).await;

    let vmac_a = [0xA6; 6];
    let mut ws_a = connect_sc_client(&url, &certs, vmac_a).await;
    let mut ws_b = connect_sc_client(&url, &certs, [0xB6; 6]).await;
    let result = ScMessage {
        function: ScFunction::Result,
        message_id: 0x2202,
        originating_vmac: None,
        destination_vmac: Some(vmac_a),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x06, 0x01, 0x00, 0x00, 0x07, 0x00, 0x01]),
    };
    send_sc_message(&mut ws_b, &result).await;

    assert_no_sc_message(&mut ws_a).await;
    assert_no_sc_message(&mut ws_b).await;

    hub.stop().await;
}

#[tokio::test]
async fn sc_websocket_hub_preserves_large_minimum_size_option_chains() {
    let certs = generate_test_certs();
    let (mut hub, url) = start_sc_hub(&certs, [0x10; 6]).await;

    let vmac_a = [0xA9; 6];
    let vmac_b = [0xB9; 6];

    let mut ws_a = connect_sc_client(&url, &certs, vmac_a).await;
    let mut ws_b = connect_sc_client(&url, &certs, vmac_b).await;

    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2401,
        originating_vmac: None,
        destination_vmac: Some(vmac_b),
        dest_options: minimum_size_options(31),
        data_options: minimum_size_options(31),
        payload: Bytes::from_static(&[0x01, 0x20, 0x31]),
    };
    send_sc_message(&mut ws_a, &msg).await;

    let relayed = recv_sc_message(&mut ws_b).await;
    assert_eq!(relayed.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(relayed.message_id, msg.message_id);
    assert_eq!(relayed.originating_vmac, Some(vmac_a));
    assert_eq!(relayed.destination_vmac, None);
    assert_eq!(relayed.dest_options, msg.dest_options);
    assert_eq!(relayed.data_options, msg.data_options);
    assert_eq!(relayed.payload, msg.payload);
    assert_no_sc_message(&mut ws_a).await;

    hub.stop().await;
}

#[tokio::test]
async fn sc_websocket_hub_naks_direct_address_resolution_as_unsupported() {
    let certs = generate_test_certs();
    let (mut hub, url) = start_sc_hub(&certs, [0x10; 6]).await;

    let mut ws = connect_sc_client(&url, &certs, [0xAD; 6]).await;
    let address_resolution = ScMessage {
        function: ScFunction::AddressResolution,
        message_id: 0x2501,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    send_sc_message(&mut ws, &address_resolution).await;

    let nak = recv_sc_message(&mut ws).await;
    assert_eq!(nak.function, ScFunction::Result);
    assert_eq!(nak.message_id, address_resolution.message_id);
    assert_eq!(
        decode_sc_bvlc_result(&nak).unwrap(),
        ScBvlcResult::Nak {
            result_for: ScFunction::AddressResolution,
            error_header_marker: 0,
            error_class: 7,
            error_code: 150,
            error_details: String::new(),
        }
    );
    assert_no_sc_message(&mut ws).await;

    hub.stop().await;
}

#[tokio::test]
async fn sc_websocket_hub_replaces_known_device_uuid_connection() {
    let certs = generate_test_certs();
    let (mut hub, url) = start_sc_hub(&certs, [0x10; 6]).await;

    let device_uuid = [0x44; 16];
    let old_vmac = [0xA4; 6];
    let replacement_vmac = [0xA5; 6];
    let peer_vmac = [0xB4; 6];

    let mut old_ws = connect_sc_client_with_uuid(&url, &certs, old_vmac, device_uuid).await;
    let mut peer_ws = connect_sc_client(&url, &certs, peer_vmac).await;
    let mut replacement_ws =
        connect_sc_client_with_uuid(&url, &certs, replacement_vmac, device_uuid).await;
    expect_websocket_close(&mut old_ws).await;

    let to_old_vmac = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2101,
        originating_vmac: None,
        destination_vmac: Some(old_vmac),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x20, 0x44]),
    };
    send_sc_message(&mut peer_ws, &to_old_vmac).await;
    assert_no_sc_message(&mut replacement_ws).await;

    let from_old_connection = ScMessage {
        message_id: 0x2102,
        destination_vmac: Some(peer_vmac),
        ..to_old_vmac.clone()
    };
    assert!(try_send_sc_message(&mut old_ws, &from_old_connection)
        .await
        .is_err());
    assert_no_sc_message(&mut peer_ws).await;

    let to_replacement = ScMessage {
        message_id: 0x2103,
        destination_vmac: Some(replacement_vmac),
        ..to_old_vmac
    };
    send_sc_message(&mut peer_ws, &to_replacement).await;

    let relayed = recv_sc_message(&mut replacement_ws).await;
    assert_eq!(relayed.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(relayed.message_id, to_replacement.message_id);
    assert_eq!(relayed.originating_vmac, Some(peer_vmac));
    assert_eq!(relayed.destination_vmac, None);
    assert_eq!(relayed.payload, to_replacement.payload);

    hub.stop().await;
}

#[tokio::test]
async fn sc_websocket_hub_closes_connected_client_on_second_connect_request() {
    let certs = generate_test_certs();
    let (mut hub, url) = start_sc_hub(&certs, [0x10; 6]).await;

    let uuid_a = [0xA7; 16];
    let uuid_b = [0xB7; 16];
    let vmac_a = [0xA7; 6];
    let vmac_b = [0xB7; 6];
    let vmac_c = [0xC7; 6];

    let mut ws_a = connect_sc_client_with_uuid(&url, &certs, vmac_a, uuid_a).await;
    let mut ws_b = connect_sc_client_with_uuid(&url, &certs, vmac_b, uuid_b).await;
    let mut ws_c = connect_sc_client(&url, &certs, vmac_c).await;

    send_connect_request(&mut ws_a, [0xD7; 6], uuid_b, 0x2201).await;
    expect_websocket_close(&mut ws_a).await;

    let to_closed_a = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2204,
        originating_vmac: None,
        destination_vmac: Some(vmac_a),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x20, 0xA7]),
    };
    send_sc_message(&mut ws_c, &to_closed_a).await;
    assert_no_sc_message(&mut ws_b).await;
    assert_no_sc_message(&mut ws_c).await;

    let to_b = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2202,
        originating_vmac: None,
        destination_vmac: Some(vmac_b),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x20, 0x77]),
    };
    send_sc_message(&mut ws_c, &to_b).await;

    let relayed_to_b = recv_sc_message(&mut ws_b).await;
    assert_eq!(relayed_to_b.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(relayed_to_b.message_id, to_b.message_id);
    assert_eq!(relayed_to_b.originating_vmac, Some(vmac_c));
    assert_eq!(relayed_to_b.destination_vmac, None);
    assert_eq!(relayed_to_b.payload, to_b.payload);

    let to_c = ScMessage {
        message_id: 0x2203,
        destination_vmac: Some(vmac_c),
        ..to_b
    };
    send_sc_message(&mut ws_b, &to_c).await;

    let relayed_to_c = recv_sc_message(&mut ws_c).await;
    assert_eq!(relayed_to_c.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(relayed_to_c.message_id, to_c.message_id);
    assert_eq!(relayed_to_c.originating_vmac, Some(vmac_b));
    assert_eq!(relayed_to_c.destination_vmac, None);
    assert_eq!(relayed_to_c.payload, to_c.payload);

    hub.stop().await;
}

#[tokio::test]
async fn sc_websocket_hub_rejects_vmac_collisions_with_result_nak() {
    let certs = generate_test_certs();
    let hub_vmac = [0x10; 6];
    let (mut hub, url) = start_sc_hub(&certs, hub_vmac).await;

    let existing_vmac = [0xA8; 6];
    let mut existing_ws =
        connect_sc_client_with_uuid(&url, &certs, existing_vmac, [0x18; 16]).await;

    let mut duplicate_ws = open_sc_websocket(&url, &certs).await;
    send_connect_request(&mut duplicate_ws, existing_vmac, [0x28; 16], 0x2301).await;
    expect_duplicate_vmac_nak(&mut duplicate_ws, 0x2301).await;
    expect_websocket_closed_or_terminated(&mut duplicate_ws).await;

    let mut hub_collision_ws = open_sc_websocket(&url, &certs).await;
    send_connect_request(&mut hub_collision_ws, hub_vmac, [0x38; 16], 0x2302).await;
    expect_duplicate_vmac_nak(&mut hub_collision_ws, 0x2302).await;
    expect_websocket_closed_or_terminated(&mut hub_collision_ws).await;

    let mut peer_ws = connect_sc_client(&url, &certs, [0xB8; 6]).await;
    let to_existing = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2303,
        originating_vmac: None,
        destination_vmac: Some(existing_vmac),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x20, 0x88]),
    };
    send_sc_message(&mut peer_ws, &to_existing).await;

    let relayed = recv_sc_message(&mut existing_ws).await;
    assert_eq!(relayed.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(relayed.message_id, to_existing.message_id);
    assert_eq!(relayed.originating_vmac, Some([0xB8; 6]));
    assert_eq!(relayed.destination_vmac, None);
    assert_eq!(relayed.payload, to_existing.payload);

    hub.stop().await;
}

async fn connect_sc_client(url: &str, certs: &CertMaterial, vmac: Vmac) -> ClientWs {
    connect_sc_client_with_uuid(url, certs, vmac, [vmac[0]; 16]).await
}

async fn connect_sc_client_with_uuid(
    url: &str,
    certs: &CertMaterial,
    vmac: Vmac,
    device_uuid: [u8; 16],
) -> ClientWs {
    let mut ws = open_sc_websocket(url, certs).await;
    let request = send_connect_request(&mut ws, vmac, device_uuid, 0x1000 | vmac[0] as u16).await;

    let accept = recv_sc_message(&mut ws).await;
    assert_eq!(accept.function, ScFunction::ConnectAccept);
    assert_eq!(accept.message_id, request.message_id);
    assert_eq!(accept.originating_vmac, None);
    assert_eq!(accept.destination_vmac, None);
    assert_eq!(accept.payload.len(), 26);

    ws
}

async fn open_sc_websocket(url: &str, certs: &CertMaterial) -> ClientWs {
    let request = ClientRequestBuilder::new(url.parse().unwrap())
        .with_sub_protocol(BACNET_SC_HUB_SUBPROTOCOL);
    let connector = tokio_tungstenite::Connector::Rustls(make_client_tls_config(certs));
    let (ws, _response) =
        tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector))
            .await
            .unwrap();

    ws
}

async fn send_connect_request(
    ws: &mut ClientWs,
    vmac: Vmac,
    device_uuid: [u8; 16],
    message_id: u16,
) -> ScMessage {
    let mut payload = Vec::with_capacity(26);
    payload.extend_from_slice(&vmac);
    payload.extend_from_slice(&device_uuid);
    payload.extend_from_slice(&1476u16.to_be_bytes());
    payload.extend_from_slice(&1476u16.to_be_bytes());

    let request = ScMessage {
        function: ScFunction::ConnectRequest,
        message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    send_sc_message(ws, &request).await;
    request
}

async fn send_sc_message(ws: &mut ClientWs, msg: &ScMessage) {
    try_send_sc_message(ws, msg).await.unwrap();
}

async fn try_send_sc_message(
    ws: &mut ClientWs,
    msg: &ScMessage,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, msg);
    ws.send(Message::Binary(buf.to_vec().into())).await
}

async fn recv_sc_message(ws: &mut ClientWs) -> ScMessage {
    let message = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("expected SC message before timeout")
        .expect("websocket should still be open")
        .expect("websocket frame should decode");

    match message {
        Message::Binary(data) => decode_sc_message(&data).unwrap(),
        other => panic!("expected SC binary message, got {other:?}"),
    }
}

async fn expect_duplicate_vmac_nak(ws: &mut ClientWs, message_id: u16) {
    let nak = recv_sc_message(ws).await;
    assert_eq!(nak.function, ScFunction::Result);
    assert_eq!(nak.message_id, message_id);
    assert_eq!(
        decode_sc_bvlc_result(&nak).unwrap(),
        ScBvlcResult::Nak {
            result_for: ScFunction::ConnectRequest,
            error_header_marker: 0,
            error_class: 7,
            error_code: 151,
            error_details: String::new(),
        }
    );
}

async fn expect_websocket_close(ws: &mut ClientWs) {
    let message = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("expected WebSocket close before timeout")
        .expect("websocket should produce a close frame")
        .expect("websocket frame should decode");

    match message {
        Message::Close(_) => {}
        other => panic!("expected WebSocket close frame, got {other:?}"),
    }
}

async fn expect_websocket_closed_or_terminated(ws: &mut ClientWs) {
    let result = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("expected WebSocket close or termination before timeout");
    match result {
        None | Some(Err(_)) | Some(Ok(Message::Close(_))) => {}
        Some(Ok(other)) => panic!("expected WebSocket close or termination, got {other:?}"),
    }
}

async fn assert_no_sc_message(ws: &mut ClientWs) {
    let result = tokio::time::timeout(Duration::from_millis(200), ws.next()).await;
    assert!(result.is_err(), "unexpected WebSocket message: {result:?}");
}

fn minimum_size_options(count: usize) -> Vec<ScOption> {
    (0..count)
        .map(|i| ScOption {
            option_type: (i % 31 + 1) as u8,
            must_understand: i % 2 == 0,
            data: Vec::new(),
        })
        .collect()
}
