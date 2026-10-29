use super::super::envelope_harness::Harness;
use super::*;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_transport::sc::{LoopbackWebSocket, ScTransport, WebSocketPort};
use bacnet_transport::sc_frame::{
    ScFunction, ScMessage, ScOption, Vmac, decode_sc_message, encode_sc_message,
};
use tokio::time::{Duration, timeout};

async fn sc_hub_accept(ws_hub: &LoopbackWebSocket, hub_vmac: Vmac) {
    sc_hub_accept_with_limits(ws_hub, hub_vmac, 1476, 1476).await;
}

/// Act as a hub: take the node's Connect-Request and accept it with the
/// hub's Max-BVLC-Length and Max-NPDU-Length.
pub(super) async fn sc_hub_accept_with_limits(
    ws_hub: &LoopbackWebSocket,
    hub_vmac: Vmac,
    max_bvlc: u16,
    max_npdu: u16,
) {
    let data = ws_hub.recv().await.unwrap();
    let req = decode_sc_message(&data).unwrap();
    assert_eq!(req.function, ScFunction::ConnectRequest);

    let mut accept_payload = Vec::with_capacity(26);
    accept_payload.extend_from_slice(&hub_vmac);
    accept_payload.extend_from_slice(&[0x33; 16]);
    accept_payload.extend_from_slice(&max_bvlc.to_be_bytes());
    accept_payload.extend_from_slice(&max_npdu.to_be_bytes());

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
async fn forward_unicast_preserves_data_attributes() {
    let mut h = Harness::two_port();
    let npdu = Npdu {
        expecting_reply: false,
        priority: bacnet_npdu::NetworkPriority::Normal,
        destination: Some(NpduAddress {
            network: 2000,
            mac_address: MacAddr::from_slice(&[0x01, 0x02]),
        }),
        source: None,
        hop_count: 10,
        payload: Bytes::from_static(&[0x10, 0x08]),
        ..Npdu::default()
    };
    let data_attributes = vec![
        DataAttribute {
            option_type: 1,
            must_understand: true,
            data: Vec::new(),
        },
        DataAttribute {
            option_type: 31,
            must_understand: false,
            data: vec![0x12, 0x34, 0x56],
        },
    ];
    let mut ctx = h.ctx(0, &[0x0A], npdu);
    ctx.data_attributes = data_attributes.clone();
    h.dispatch(ctx).await;

    match h.rxs[1].try_recv().unwrap() {
        SendRequest::Unicast {
            npdu: data,
            data_attributes: sent_attributes,
            ..
        } => {
            let decoded = decode_npdu(data).unwrap();
            assert!(decoded.destination.is_none());
            assert!(decoded.source.is_some());
            assert_eq!(sent_attributes, data_attributes);
        }
        SendRequest::Broadcast { .. } => panic!("expected forwarded unicast"),
    }
}

#[tokio::test]
async fn forward_broadcast_preserves_data_attributes_on_each_output_port() {
    let mut h = Harness::with_networks(&[1000, 2000, 3000]);
    let npdu = Npdu {
        expecting_reply: false,
        priority: bacnet_npdu::NetworkPriority::Normal,
        destination: Some(NpduAddress {
            network: 0xFFFF,
            mac_address: MacAddr::new(),
        }),
        source: None,
        hop_count: 10,
        payload: Bytes::from_static(&[0x10, 0x08]),
        ..Npdu::default()
    };
    let data_attributes = vec![DataAttribute {
        option_type: 2,
        must_understand: true,
        data: vec![0xAA],
    }];
    let mut ctx = h.ctx(1, &[0x0A], npdu);
    ctx.data_attributes = data_attributes.clone();
    h.dispatch(ctx).await;

    for port in [0, 2] {
        match h.rxs[port].try_recv().unwrap() {
            SendRequest::Broadcast {
                npdu: data,
                data_attributes: sent_attributes,
                ..
            } => {
                let decoded = decode_npdu(data).unwrap();
                assert_eq!(decoded.destination.as_ref().unwrap().network, 0xFFFF);
                assert!(decoded.source.is_some());
                assert_eq!(sent_attributes, data_attributes);
            }
            SendRequest::Unicast { .. } => panic!("expected forwarded broadcast"),
        }
    }
    h.assert_quiet();
}

#[tokio::test]
async fn sc_to_sc_forwarding_preserves_data_attributes_as_data_options() {
    let (ws_client_a, ws_hub_a) = LoopbackWebSocket::pair();
    let (ws_client_b, ws_hub_b) = LoopbackWebSocket::pair();
    let hub_a_vmac: Vmac = [0xA0; 6];
    let hub_b_vmac: Vmac = [0xB0; 6];
    let dest_vmac: Vmac = [0x22; 6];

    let accept_a = tokio::spawn(async move {
        sc_hub_accept(&ws_hub_a, hub_a_vmac).await;
        ws_hub_a
    });
    let accept_b = tokio::spawn(async move {
        sc_hub_accept(&ws_hub_b, hub_b_vmac).await;
        ws_hub_b
    });

    let port_a = RouterPort {
        transport: ScTransport::new(ws_client_a, [0x01; 6]).with_device_uuid([1; 16]),
        network_number: 1000,
    };
    let port_b = RouterPort {
        transport: ScTransport::new(ws_client_b, [0x02; 6]).with_device_uuid([1; 16]),
        network_number: 2000,
    };
    let StartedRouter {
        mut router,
        apdus: _local_rx,
        ..
    } = BACnetRouter::start(vec![port_a, port_b], RouterOptions::new())
        .await
        .unwrap();
    let ws_hub_a = accept_a.await.unwrap();
    let ws_hub_b = accept_b.await.unwrap();

    let apdu = Bytes::from_static(&[0x10, 0x08]);
    let npdu = Npdu {
        expecting_reply: false,
        priority: bacnet_npdu::NetworkPriority::Normal,
        destination: Some(NpduAddress {
            network: 2000,
            mac_address: MacAddr::from_slice(&dest_vmac),
        }),
        source: None,
        hop_count: 255,
        payload: apdu.clone(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();

    let inbound = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2233,
        originating_vmac: Some(hub_a_vmac),
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
    encode_sc_message(&mut sc_buf, &inbound);
    ws_hub_a.send(&sc_buf).await.unwrap();

    let forwarded = async {
        for _ in 0..5 {
            let data = ws_hub_b.recv().await.unwrap();
            let msg = decode_sc_message(&data).unwrap();
            if msg.function != ScFunction::EncapsulatedNpdu {
                continue;
            }
            let decoded = decode_npdu(msg.payload.clone()).unwrap();
            if !decoded.is_network_message() && decoded.payload == apdu {
                return msg;
            }
        }
        panic!("did not receive forwarded APDU");
    };
    let msg = timeout(Duration::from_secs(1), forwarded)
        .await
        .expect("timed out waiting for forwarded SC NPDU");

    assert_eq!(msg.destination_vmac, Some(dest_vmac));
    assert_eq!(msg.data_options.len(), 2);
    assert_eq!(msg.data_options[0].option_type, 1);
    assert!(msg.data_options[0].must_understand);
    assert!(msg.data_options[0].data.is_empty());
    assert_eq!(msg.data_options[1].option_type, 31);
    assert!(!msg.data_options[1].must_understand);
    assert_eq!(msg.data_options[1].data, vec![0x12, 0x34, 0x56]);

    router.stop().await.unwrap();
}
