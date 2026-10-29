//! A real peer independently advertises each receive bound in Connect-Accept.
use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{
    Message,
    handshake::server::{ErrorResponse, Request, Response},
};
#[allow(clippy::result_large_err)]
fn direct(request: &Request, mut response: Response) -> Result<Response, ErrorResponse> {
    assert_eq!(
        request.headers()["Sec-WebSocket-Protocol"],
        "dc.bsc.bacnet.org"
    );
    response.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        "dc.bsc.bacnet.org".parse().unwrap(),
    );
    Ok(response)
}
#[tokio::test]
async fn outbound_tls_endpoint_original_budget_uses_each_peer_limit_without_hub() {
    for (bvlc, npdu_limit) in [(40u16, 1476u16), (100, 36)] {
        let ca = TestCa::new();
        let tls = ca.server_tls();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!(
            "wss://localhost:{}/.bacnet/sc",
            listener.local_addr().unwrap().port()
        );
        let peer = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let tls = tokio_rustls::TlsAcceptor::from(tls)
                .accept(tcp)
                .await
                .unwrap();
            let mut ws = tokio_tungstenite::accept_hdr_async(tls, direct)
                .await
                .unwrap();
            let Message::Binary(connect) = ws.next().await.unwrap().unwrap() else {
                panic!("Connect-Request")
            };
            let request = decode_sc_message(&connect).unwrap();
            assert_eq!(request.function, ScFunction::ConnectRequest);
            assert_eq!(&request.payload[..6], &LOCAL);
            let mut payload = Vec::new();
            payload.extend_from_slice(&REMOTE);
            payload.extend_from_slice(&[2; 16]);
            payload.extend_from_slice(&bvlc.to_be_bytes());
            payload.extend_from_slice(&npdu_limit.to_be_bytes());
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
            ws.send(Message::Binary(bytes.to_vec().into()))
                .await
                .unwrap();
            let Message::Binary(initial) = ws.next().await.unwrap().unwrap() else {
                panic!("initial marker")
            };
            assert_eq!(
                decode_sc_message(&initial).unwrap().payload.as_ref(),
                &[1, 0, 0x10, 8]
            );
            bytes.clear();
            encode_sc_message(
                &mut bytes,
                &ScMessage {
                    function: ScFunction::EncapsulatedNpdu,
                    message_id: 99,
                    originating_vmac: None,
                    destination_vmac: None,
                    dest_options: vec![],
                    data_options: vec![],
                    payload: Bytes::from(wire(&read_name(75))),
                },
            );
            ws.send(Message::Binary(bytes.to_vec().into()))
                .await
                .unwrap();
            let Message::Binary(reply) = ws.next().await.unwrap().unwrap() else {
                panic!("bounded reply")
            };
            assert!(reply.len() <= usize::from(bvlc));
            let frame = decode_sc_message(&reply).unwrap();
            assert!(frame.payload.len() <= usize::from(npdu_limit));
            assert_eq!(
                (frame.originating_vmac, frame.destination_vmac),
                (None, None)
            );
            assert!(frame.data_options.is_empty());
            let npdu = decode_npdu(frame.payload).unwrap();
            assert!(matches!(decode_apdu(npdu.payload).unwrap(),Apdu::Abort(a) if a.invoke_id==75));
        });
        let (port, mut local, hub) = start_port(&ca, uri).await;
        let mut session = EndpointSession::new(port, SessionRole::Both, SessionConfig::default())
            .unwrap()
            .with_database(database(&"long".repeat(30)));
        session.start().await.unwrap();
        bounded(async { tokio::select! {
            result=peer=>result.unwrap(),
            escaped=hub.recv()=>panic!("direct budget failure escaped through Hub: {escaped:?}"),
        }}).await;
        session.stop().await.unwrap();
        local.stop().await;
    }
}
