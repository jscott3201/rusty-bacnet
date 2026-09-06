use super::deadline_test_support::*;
use super::*;
use crate::sc_frame::ScOption;
use std::time::Duration;

pub(super) fn encoded(message: &ScMessage) -> Bytes {
    let mut wire = BytesMut::new();
    encode_sc_message(&mut wire, message);
    wire.freeze()
}

pub(super) fn npdu_message(destination: Vmac, payload_len: usize, options_len: usize) -> ScMessage {
    let mut payload = vec![0x55; payload_len];
    payload[..2].copy_from_slice(&[1, 0]); // BACnet NPDU version and control
    ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x4321,
        originating_vmac: None,
        destination_vmac: Some(destination),
        dest_options: if options_len == 0 {
            vec![]
        } else {
            vec![ScOption {
                option_type: 31,
                must_understand: false,
                data: vec![0xa5; options_len / 2 - 3],
            }]
        },
        data_options: if options_len == 0 {
            vec![]
        } else {
            vec![ScOption {
                option_type: 31,
                must_understand: false,
                data: vec![0x5a; options_len - options_len / 2 - 3],
            }]
        },
        payload: payload.into(),
    }
}

pub(super) async fn receive(ws: &mut ClientWs) -> Bytes {
    match tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
    {
        Message::Binary(wire) => wire,
        other => panic!("expected binary, got {other:?}"),
    }
}

pub(super) async fn register(ws: &mut ClientWs, vmac: Vmac) -> ScMessage {
    let mut wire = match request(vmac, [vmac[0]; 16]) {
        Message::Binary(wire) => wire.to_vec(),
        _ => unreachable!(),
    };
    // A willing recipient advertises independent, sufficiently large capacities.
    wire[26..30].copy_from_slice(&[0xff; 4]);
    ws.send(Message::Binary(wire.into())).await.unwrap();
    let response = decode_sc_message(&receive(ws).await).unwrap();
    assert_eq!(response.function, ScFunction::ConnectAccept);
    response
}

pub(super) async fn heartbeat(ws: &mut ClientWs, id: u16) {
    ws.send(Message::Binary(
        vec![0x0a, 0, (id >> 8) as u8, id as u8].into(),
    ))
    .await
    .unwrap();
    assert_eq!(
        &receive(ws).await[..],
        &[0x0b, 0, (id >> 8) as u8, id as u8]
    );
}

pub(super) async fn initiating_pair() -> (WebSocketStream<TlsStream>, crate::sc_tls::TlsWebSocket) {
    let tls = TestTls::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let accept = async {
        let (tcp, _) = listener.accept().await.unwrap();
        let stream = tls.acceptor.accept(tcp).await.unwrap();
        #[allow(clippy::result_large_err)]
        tokio_tungstenite::accept_hdr_async(
            stream,
            |_: &_, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                response.headers_mut().insert(
                    "Sec-WebSocket-Protocol",
                    "hub.bsc.bacnet.org".parse().unwrap(),
                );
                Ok(response)
            },
        )
        .await
        .unwrap()
    };
    let url = format!("wss://localhost:{}", address.port());
    let (server, node) = tokio::join!(
        accept,
        crate::sc_tls::TlsWebSocket::connect(&url, tls.client.clone())
    );
    (server, node.unwrap())
}

// Independent RFC 6455 framing oracle. Masked client frames use a fixed nonzero
// key; server frames are unmasked. The payload is never changed by the caller.
pub(super) fn raw_frame(opcode: u8, fin: bool, payload: &[u8], masked: bool) -> Vec<u8> {
    let mut wire = vec![opcode | if fin { 0x80 } else { 0 }];
    let mask_bit = if masked { 0x80 } else { 0 };
    if payload.len() < 126 {
        wire.push(mask_bit | payload.len() as u8);
    } else if payload.len() <= 65535 {
        wire.push(mask_bit | 126);
        wire.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        wire.push(mask_bit | 127);
        wire.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    let mask = [0x12, 0x34, 0x56, 0x78];
    if masked {
        wire.extend_from_slice(&mask);
    }
    wire.extend(
        payload
            .iter()
            .enumerate()
            .map(|(i, b)| if masked { b ^ mask[i % 4] } else { *b }),
    );
    wire
}
