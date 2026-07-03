use bacnet_encoding::apdu::{self, encode_apdu, Apdu, ConfirmedRequest, SegmentAck, SimpleAck};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::BytesMut;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use super::{BACnetClient, ClientConfig};

async fn send_local_response<T: TransportPort>(transport: &T, client_mac: &[u8], apdu: Apdu) {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &apdu).expect("valid APDU encoding");
    let npdu = Npdu {
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();
    transport.send_unicast(&npdu_buf, client_mac).await.unwrap();
}

async fn recv_confirmed_segment(
    rx: &mut mpsc::Receiver<ReceivedNpdu>,
    context: &str,
) -> ConfirmedRequest {
    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap_or_else(|_| panic!("{context}: timed out waiting for segment"))
        .unwrap_or_else(|| panic!("{context}: server channel closed"));
    let npdu = decode_npdu(received.npdu).unwrap();
    let decoded = apdu::decode_apdu(npdu.payload).unwrap();
    let Apdu::ConfirmedRequest(req) = decoded else {
        panic!("{context}: expected ConfirmedRequest, got {:?}", decoded);
    };
    assert!(req.segmented, "{context}: request must be segmented");
    req
}

#[tokio::test]
async fn segmented_request_retransmits_segment_zero_after_negative_ack_zero() {
    let client_mac = vec![0x01];
    let server_mac = vec![0x02];
    let (client_transport, mut server_transport) =
        LoopbackTransport::pair(client_mac.clone(), server_mac.clone());
    let mut server_rx = server_transport.start().await.unwrap();

    let mut config = ClientConfig {
        apdu_timeout_ms: 2000,
        max_apdu_length: 50,
        proposed_window_size: 2,
        ..ClientConfig::default()
    };
    config.apdu_retries = 2;
    let mut client = BACnetClient::start(config, client_transport).await.unwrap();

    let request_server_mac = server_mac.clone();
    let service_data: Vec<u8> = (0u8..100).collect();
    let request_task = tokio::spawn(async move {
        let result = client
            .confirmed_request(
                &request_server_mac,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    let mut invoke_id = None;
    for expected_seq in [0, 1] {
        let received = timeout(Duration::from_secs(2), server_rx.recv())
            .await
            .expect("server timed out waiting for initial window segment")
            .expect("server channel closed");
        let npdu = decode_npdu(received.npdu).unwrap();
        let decoded = apdu::decode_apdu(npdu.payload).unwrap();
        let Apdu::ConfirmedRequest(req) = decoded else {
            panic!("Expected ConfirmedRequest, got {:?}", decoded);
        };
        assert!(req.segmented);
        assert_eq!(req.sequence_number, Some(expected_seq));
        invoke_id = Some(req.invoke_id);
    }

    let invoke_id = invoke_id.unwrap();
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: true,
            sent_by_server: true,
            invoke_id,
            sequence_number: 0,
            actual_window_size: 2,
        }),
    )
    .await;

    for expected_seq in [0, 1] {
        let received = timeout(Duration::from_secs(2), server_rx.recv())
            .await
            .expect("server timed out waiting for retransmitted window segment")
            .expect("server channel closed");
        let npdu = decode_npdu(received.npdu).unwrap();
        let decoded = apdu::decode_apdu(npdu.payload).unwrap();
        let Apdu::ConfirmedRequest(req) = decoded else {
            panic!("Expected ConfirmedRequest, got {:?}", decoded);
        };
        assert_eq!(req.invoke_id, invoke_id);
        assert_eq!(req.sequence_number, Some(expected_seq));
    }

    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 1,
            actual_window_size: 1,
        }),
    )
    .await;

    loop {
        let received = timeout(Duration::from_secs(2), server_rx.recv())
            .await
            .expect("server timed out waiting for remaining segment")
            .expect("server channel closed");
        let npdu = decode_npdu(received.npdu).unwrap();
        let decoded = apdu::decode_apdu(npdu.payload).unwrap();
        let Apdu::ConfirmedRequest(req) = decoded else {
            panic!("Expected ConfirmedRequest, got {:?}", decoded);
        };
        assert_eq!(req.invoke_id, invoke_id);
        let seq = req.sequence_number.unwrap();
        assert!(seq >= 2);
        let more_follows = req.more_follows;

        send_local_response(
            &server_transport,
            &client_mac,
            Apdu::SegmentAck(SegmentAck {
                negative_ack: false,
                sent_by_server: true,
                invoke_id,
                sequence_number: seq,
                actual_window_size: 1,
            }),
        )
        .await;

        if !more_follows {
            break;
        }
    }

    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .await;

    let result = request_task.await.unwrap();
    assert!(result.unwrap().is_empty());
    server_transport.stop().await.unwrap();
}

#[tokio::test]
async fn segmented_request_ignores_stale_negative_ack_zero_after_progress() {
    let client_mac = vec![0x01];
    let server_mac = vec![0x02];
    let (client_transport, mut server_transport) =
        LoopbackTransport::pair(client_mac.clone(), server_mac.clone());
    let mut server_rx = server_transport.start().await.unwrap();

    let mut config = ClientConfig {
        apdu_timeout_ms: 2000,
        max_apdu_length: 50,
        proposed_window_size: 2,
        ..ClientConfig::default()
    };
    config.apdu_retries = 2;
    let mut client = BACnetClient::start(config, client_transport).await.unwrap();

    let request_server_mac = server_mac.clone();
    let service_data: Vec<u8> = (0..260).map(|i| (i % 251) as u8).collect();
    let request_task = tokio::spawn(async move {
        let result = client
            .confirmed_request(
                &request_server_mac,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    let mut invoke_id = None;
    for expected_seq in [0, 1] {
        let req = recv_confirmed_segment(&mut server_rx, "initial window").await;
        assert_eq!(req.sequence_number, Some(expected_seq));
        invoke_id = Some(req.invoke_id);
    }

    let invoke_id = invoke_id.unwrap();
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 1,
            actual_window_size: 2,
        }),
    )
    .await;

    for expected_seq in [2, 3] {
        let req = recv_confirmed_segment(&mut server_rx, "second window").await;
        assert_eq!(req.invoke_id, invoke_id);
        assert_eq!(req.sequence_number, Some(expected_seq));
    }

    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: true,
            sent_by_server: true,
            invoke_id,
            sequence_number: 0,
            actual_window_size: 2,
        }),
    )
    .await;
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 3,
            actual_window_size: 1,
        }),
    )
    .await;

    loop {
        let req = recv_confirmed_segment(&mut server_rx, "post-stale-ack window").await;
        assert_eq!(req.invoke_id, invoke_id);
        let seq = req.sequence_number.unwrap();
        assert!(
            seq >= 4,
            "stale negative SegmentACK 0 rewound transfer to segment {seq}"
        );
        let more_follows = req.more_follows;

        send_local_response(
            &server_transport,
            &client_mac,
            Apdu::SegmentAck(SegmentAck {
                negative_ack: false,
                sent_by_server: true,
                invoke_id,
                sequence_number: seq,
                actual_window_size: 1,
            }),
        )
        .await;

        if !more_follows {
            break;
        }
    }

    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .await;

    let result = request_task.await.unwrap();
    assert!(result.unwrap().is_empty());
    server_transport.stop().await.unwrap();
}
