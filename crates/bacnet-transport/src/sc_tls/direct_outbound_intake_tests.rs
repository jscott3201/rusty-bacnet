//! Outbound wire admission and saturated intake do not stall control progress.
use super::*;
use crate::sc::npdu_admission::DEFAULT_NPDU_PER_ORIGIN_LIMIT as QUOTA;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn outbound_wire_shapes_options_and_saturated_npdu_intake_preserve_controls() {
    let ca = TestCa::generate();
    let server = ca.node_config(vec!["localhost".into()]);
    let listener = tokio::net::TcpListener::bind(loopback_addr())
        .await
        .unwrap();
    let uri = direct_url(&listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let tls = server.acceptor().accept(tcp).await.unwrap();
        let mut ws = tokio_tungstenite::accept_hdr_async(
            tls,
            super::super::super::direct_subprotocol_response,
        )
        .await
        .unwrap();
        let Message::Binary(connect) = ws.next().await.unwrap().unwrap() else {
            panic!("Connect")
        };
        let config = DirectAcceptConfig::new(loopback_addr(), LISTENER_VMAC, LISTENER_UUID, server);
        let mut bytes = BytesMut::new();
        encode_sc_message(
            &mut bytes,
            &super::super::super::build_connect_accept(
                decode_sc_message(&connect).unwrap().message_id,
                &config,
            ),
        );
        ws.send(Message::Binary(bytes.to_vec().into()))
            .await
            .unwrap();
        let Message::Binary(initial) = ws.next().await.unwrap().unwrap() else {
            panic!("NPDU")
        };
        let base = decode_sc_message(&initial).unwrap();
        assert_eq!(base.payload.as_ref(), NPDU);
        let mut invalid = Vec::new();
        let mut frame = base.clone();
        frame.originating_vmac = Some(LISTENER_VMAC);
        invalid.push(frame);
        let mut frame = base.clone();
        frame.destination_vmac = Some(DIAL_VMAC);
        invalid.push(frame);
        let mut frame = base.clone();
        frame.destination_vmac = Some([0xff; 6]);
        invalid.push(frame);
        let mut frame = base.clone();
        frame.payload = Bytes::new();
        invalid.push(frame);
        let mut frame = base.clone();
        frame.payload = Bytes::from(vec![1; 1479]);
        invalid.push(frame);
        for frame in invalid {
            bytes.clear();
            encode_sc_message(&mut bytes, &frame);
            ws.send(Message::Binary(bytes.to_vec().into()))
                .await
                .unwrap();
        }
        let mut mu = base.clone();
        mu.message_id = 67;
        mu.dest_options = vec![crate::sc_frame::ScOption {
            option_type: 2,
            must_understand: true,
            data: vec![],
        }];
        bytes.clear();
        encode_sc_message(&mut bytes, &mu);
        ws.send(Message::Binary(bytes.to_vec().into()))
            .await
            .unwrap();
        let Message::Binary(nak) = ws.next().await.unwrap().unwrap() else {
            panic!("NAK")
        };
        let nak = decode_sc_message(&nak).unwrap();
        assert_eq!(nak.function, ScFunction::Result);
        assert_eq!(nak.message_id, 67);
        assert_eq!(nak.payload.as_ref(), &[1, 1, 0x42, 0, 7, 0, 146]);
        // The default per-origin intake quota is 32, even though aggregate
        // storage has 256 slots. Do not drain until Disconnect ACK proves all
        // 33 application frames passed the worker's read turn.
        for index in 0..=QUOTA as u8 {
            let mut frame = base.clone();
            frame.payload = Bytes::from(vec![1, 0, index]);
            frame.data_options = vec![crate::sc_frame::ScOption {
                option_type: 3,
                must_understand: false,
                data: vec![9, 8],
            }];
            bytes.clear();
            encode_sc_message(&mut bytes, &frame);
            ws.send(Message::Binary(bytes.to_vec().into()))
                .await
                .unwrap();
        }
        let disconnect = crate::sc::direct_membership::disconnect_request();
        bytes.clear();
        encode_sc_message(&mut bytes, &disconnect);
        ws.send(Message::Binary(bytes.to_vec().into()))
            .await
            .unwrap();
        let Message::Binary(ack) = ws.next().await.unwrap().unwrap() else {
            panic!("Disconnect ACK")
        };
        let ack = decode_sc_message(&ack).unwrap();
        assert_eq!(ack.function, ScFunction::DisconnectAck);
        assert_eq!(ack.message_id, disconnect.message_id);
    });
    let (mut transport, mut rx, hub) = start_outbound(&ca).await;
    discover_send(&transport, &hub, &uri).await;
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(transport.npdu_drop_counts().fairness_drops, 1);
    for index in 0..QUOTA as u8 {
        let received = rx.try_recv().unwrap();
        assert_eq!(received.npdu.as_ref(), &[1, 0, index]);
        assert_eq!(received.source_mac.as_slice(), &LISTENER_VMAC);
        assert!(!received.link_layer_group);
        assert_eq!(
            received.data_attributes,
            vec![crate::port::DataAttribute {
                option_type: 3,
                must_understand: false,
                data: vec![9, 8]
            }]
        );
        assert!(received.provenance.is_direct_peer());
        assert_eq!(
            received.direct_response.as_ref().unwrap().identity(),
            received.provenance.direct_sc_identity().unwrap()
        );
        assert!(received
            .direct_response
            .unwrap()
            .send(NPDU, &DirectResponseScope::default())
            .await
            .is_err());
    }
    assert!(rx.try_recv().is_err());
    assert!(transport.direct_route_for_test(&LISTENER_VMAC).is_none());
    transport.stop().await.unwrap();
}
