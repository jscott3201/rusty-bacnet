use std::sync::Arc;

use bacnet_encoding::apdu::{self, encode_apdu, Apdu, ComplexAck, ErrorPdu, SegmentAck, SimpleAck};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, RejectReason,
};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::sync::{mpsc, Notify};
use tokio::time::{timeout, Duration};

use super::{BACnetClient, ClientConfig};

const CLIENT_MAC: &[u8] = &[0x01];
const SERVER_MAC: &[u8] = &[0x02];

fn config() -> ClientConfig {
    ClientConfig {
        apdu_timeout_ms: 2_000,
        apdu_retries: 0,
        max_apdu_length: 50,
        proposed_window_size: 1,
        ..ClientConfig::default()
    }
}

async fn send_to_client<T: TransportPort>(transport: &T, apdu: Apdu) {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &apdu).unwrap();
    let mut npdu_buf = BytesMut::new();
    encode_npdu(
        &mut npdu_buf,
        &Npdu {
            payload: apdu_buf.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    transport.send_unicast(&npdu_buf, CLIENT_MAC).await.unwrap();
}

async fn recv_apdu(rx: &mut mpsc::Receiver<ReceivedNpdu>, context: &str) -> Apdu {
    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap_or_else(|_| panic!("{context}: timed out waiting for APDU"))
        .unwrap_or_else(|| panic!("{context}: channel closed"));
    let npdu = decode_npdu(received.npdu).unwrap();
    apdu::decode_apdu(npdu.payload).unwrap()
}

async fn expect_silence(rx: &mut mpsc::Receiver<ReceivedNpdu>, context: &str) {
    if let Ok(Some(received)) = timeout(Duration::from_millis(100), rx.recv()).await {
        let npdu = decode_npdu(received.npdu).unwrap();
        let apdu = apdu::decode_apdu(npdu.payload).unwrap();
        panic!("{context}: expected no traffic, got {apdu:?}");
    }
}

#[derive(Clone, Copy, Debug)]
enum PrematureResponse {
    SimpleAck,
    UnsegmentedComplexAck,
    SegmentedComplexAck,
    Error,
}

impl PrematureResponse {
    fn apdu(self, invoke_id: u8) -> Apdu {
        match self {
            Self::SimpleAck => Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            }),
            Self::UnsegmentedComplexAck => Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
                service_ack: Bytes::from_static(b"premature"),
            }),
            Self::SegmentedComplexAck => Apdu::ComplexAck(ComplexAck {
                segmented: true,
                more_follows: true,
                invoke_id,
                sequence_number: Some(0),
                proposed_window_size: Some(1),
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
                service_ack: Bytes::from_static(b"premature"),
            }),
            Self::Error => Apdu::Error(ErrorPdu {
                invoke_id,
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
                error_class: ErrorClass::DEVICE,
                error_code: ErrorCode::OPERATIONAL_PROBLEM,
                error_data: Bytes::new(),
            }),
        }
    }
}

async fn assert_premature_response_aborts(kind: PrematureResponse) {
    let (client_transport, mut server) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
    let mut rx = server.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::start(config(), client_transport)
            .await
            .unwrap(),
    );

    let request_client = Arc::clone(&client);
    let request = tokio::spawn(async move {
        request_client
            .confirmed_request(
                SERVER_MAC,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &[0x0C; 100],
            )
            .await
    });
    let first = match recv_apdu(&mut rx, "initial request segment").await {
        Apdu::ConfirmedRequest(request) => request,
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    };
    assert_eq!(first.sequence_number, Some(0));
    assert!(first.more_follows);

    send_to_client(&server, kind.apdu(first.invoke_id)).await;
    match recv_apdu(&mut rx, "client Abort").await {
        Apdu::Abort(abort) => {
            assert_eq!(abort.invoke_id, first.invoke_id);
            assert!(!abort.sent_by_server);
            assert_eq!(abort.abort_reason, AbortReason::INVALID_APDU_IN_THIS_STATE);
        }
        other => panic!("expected client Abort, got {other:?}"),
    }

    assert!(matches!(
        timeout(Duration::from_secs(2), request)
            .await
            .unwrap()
            .unwrap(),
        Err(Error::Abort { reason })
            if reason == AbortReason::INVALID_APDU_IN_THIS_STATE.to_raw()
    ));
    assert_eq!(client.tsm.lock().await.pending_count(), 0);
    assert!(
        !client
            .seg_ack_senders
            .lock()
            .await
            .contains_key(&(MacAddr::from_slice(SERVER_MAC), first.invoke_id)),
        "completed request retained its SegmentACK route"
    );
    expect_silence(&mut rx, "after the premature response Abort").await;

    let mut client = Arc::try_unwrap(client).unwrap_or_else(|_| panic!("request retained client"));
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn premature_simple_ack_aborts_segmented_request() {
    assert_premature_response_aborts(PrematureResponse::SimpleAck).await;
}

#[tokio::test]
async fn premature_unsegmented_complex_ack_aborts_segmented_request() {
    assert_premature_response_aborts(PrematureResponse::UnsegmentedComplexAck).await;
}

#[tokio::test]
async fn premature_segmented_complex_ack_aborts_segmented_request() {
    assert_premature_response_aborts(PrematureResponse::SegmentedComplexAck).await;
}

#[tokio::test]
async fn premature_error_aborts_segmented_request() {
    assert_premature_response_aborts(PrematureResponse::Error).await;
}

#[tokio::test]
async fn reject_during_segmented_send_stops_the_sender() {
    let (client_transport, mut server) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
    let mut rx = server.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::start(config(), client_transport)
            .await
            .unwrap(),
    );
    let request_client = Arc::clone(&client);
    let request = tokio::spawn(async move {
        request_client
            .confirmed_request(
                SERVER_MAC,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &[0x0C; 100],
            )
            .await
    });
    let invoke_id = match recv_apdu(&mut rx, "initial request segment").await {
        Apdu::ConfirmedRequest(request) => request.invoke_id,
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    };

    send_to_client(
        &server,
        Apdu::Reject(bacnet_encoding::apdu::RejectPdu {
            invoke_id,
            reject_reason: RejectReason::OTHER,
        }),
    )
    .await;
    assert!(matches!(
        timeout(Duration::from_secs(2), request)
            .await
            .unwrap()
            .unwrap(),
        Err(Error::Reject { reason }) if reason == RejectReason::OTHER.to_raw()
    ));
    expect_silence(&mut rx, "after Reject completed the send").await;
    assert_eq!(client.tsm.lock().await.pending_count(), 0);

    let mut client = Arc::try_unwrap(client).unwrap_or_else(|_| panic!("request retained client"));
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn server_abort_during_segmented_send_stops_the_sender() {
    let (client_transport, mut server) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
    let mut rx = server.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::start(config(), client_transport)
            .await
            .unwrap(),
    );
    let request_client = Arc::clone(&client);
    let request = tokio::spawn(async move {
        request_client
            .confirmed_request(
                SERVER_MAC,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &[0x0C; 100],
            )
            .await
    });
    let invoke_id = match recv_apdu(&mut rx, "initial request segment").await {
        Apdu::ConfirmedRequest(request) => request.invoke_id,
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    };

    send_to_client(
        &server,
        Apdu::Abort(bacnet_encoding::apdu::AbortPdu {
            sent_by_server: false,
            invoke_id,
            abort_reason: AbortReason::OTHER,
        }),
    )
    .await;
    expect_silence(&mut rx, "after server=FALSE Abort").await;
    assert_eq!(client.tsm.lock().await.pending_count(), 1);

    send_to_client(
        &server,
        Apdu::Abort(bacnet_encoding::apdu::AbortPdu {
            sent_by_server: true,
            invoke_id,
            abort_reason: AbortReason::BUFFER_OVERFLOW,
        }),
    )
    .await;
    assert!(matches!(
        timeout(Duration::from_secs(2), request)
            .await
            .unwrap()
            .unwrap(),
        Err(Error::Abort { reason }) if reason == AbortReason::BUFFER_OVERFLOW.to_raw()
    ));
    expect_silence(&mut rx, "after server Abort completed the send").await;
    assert_eq!(client.tsm.lock().await.pending_count(), 0);

    let mut client = Arc::try_unwrap(client).unwrap_or_else(|_| panic!("request retained client"));
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

struct ImmediateFinalResponseTransport {
    inbound_rx: Option<mpsc::Receiver<ReceivedNpdu>>,
    inbound_tx: mpsc::Sender<ReceivedNpdu>,
    final_response_sent: Arc<Notify>,
}

impl ImmediateFinalResponseTransport {
    fn inbound(apdu: Apdu) -> ReceivedNpdu {
        let mut apdu_buf = BytesMut::new();
        encode_apdu(&mut apdu_buf, &apdu).unwrap();
        let mut npdu_buf = BytesMut::new();
        encode_npdu(
            &mut npdu_buf,
            &Npdu {
                payload: apdu_buf.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        ReceivedNpdu {
            npdu: npdu_buf.freeze(),
            source_mac: MacAddr::from_slice(SERVER_MAC),
            link_layer_group: false,
            data_attributes: Vec::new(),
            reply_tx: None,
        }
    }
}

impl TransportPort for ImmediateFinalResponseTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.inbound_rx.take().expect("transport starts once"))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        let npdu = decode_npdu(Bytes::copy_from_slice(npdu)).unwrap();
        let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap() else {
            return Ok(());
        };
        if !request.segmented {
            return Ok(());
        }

        let response = if request.more_follows {
            Apdu::SegmentAck(SegmentAck {
                negative_ack: false,
                sent_by_server: true,
                invoke_id: request.invoke_id,
                sequence_number: request.sequence_number.unwrap(),
                actual_window_size: 1,
            })
        } else {
            Apdu::SimpleAck(SimpleAck {
                invoke_id: request.invoke_id,
                service_choice: request.service_choice,
            })
        };
        self.inbound_tx.send(Self::inbound(response)).await.unwrap();

        if !request.more_follows {
            // Keep the transport future pending after the peer has received
            // the final segment and replied. The request must complete from
            // the TSM response without waiting for this send future to return.
            self.final_response_sent.notify_one();
            std::future::pending::<()>().await;
        }
        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        CLIENT_MAC
    }
}

#[tokio::test]
async fn response_after_final_segment_before_final_segment_ack_succeeds() {
    let (inbound_tx, inbound_rx) = mpsc::channel(16);
    let final_response_sent = Arc::new(Notify::new());
    let transport = ImmediateFinalResponseTransport {
        inbound_rx: Some(inbound_rx),
        inbound_tx,
        final_response_sent: Arc::clone(&final_response_sent),
    };
    let mut client = BACnetClient::start(config(), transport).await.unwrap();

    let result = timeout(
        Duration::from_secs(2),
        client.confirmed_request(
            SERVER_MAC,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            &[0x0C; 100],
        ),
    )
    .await
    .expect("request waited for a final SegmentACK");
    final_response_sent.notified().await;
    assert!(result.unwrap().is_empty());
    assert_eq!(client.tsm.lock().await.pending_count(), 0);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn segmented_response_before_final_segment_ack_hands_off_to_receive_timer() {
    let (client_transport, mut server) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
    let mut rx = server.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::start(config(), client_transport)
            .await
            .unwrap(),
    );
    let request_client = Arc::clone(&client);
    let request = tokio::spawn(async move {
        request_client
            .confirmed_request(
                SERVER_MAC,
                ConfirmedServiceChoice::READ_PROPERTY,
                &[0x0C; 100],
            )
            .await
    });

    let invoke_id = loop {
        let segment = match recv_apdu(&mut rx, "request segment").await {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected ConfirmedRequest, got {other:?}"),
        };
        if segment.more_follows {
            send_to_client(
                &server,
                Apdu::SegmentAck(SegmentAck {
                    negative_ack: false,
                    sent_by_server: true,
                    invoke_id: segment.invoke_id,
                    sequence_number: segment.sequence_number.unwrap(),
                    actual_window_size: 1,
                }),
            )
            .await;
        } else {
            send_to_client(
                &server,
                Apdu::ComplexAck(ComplexAck {
                    segmented: true,
                    more_follows: true,
                    invoke_id: segment.invoke_id,
                    sequence_number: Some(0),
                    proposed_window_size: Some(1),
                    service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                    service_ack: Bytes::from_static(b"first-"),
                }),
            )
            .await;
            break segment.invoke_id;
        }
    };

    match recv_apdu(&mut rx, "response SegmentACK zero").await {
        Apdu::SegmentAck(ack) => {
            assert!(!ack.sent_by_server);
            assert_eq!(ack.invoke_id, invoke_id);
            assert_eq!(ack.sequence_number, 0);
        }
        other => panic!("expected response SegmentACK, got {other:?}"),
    }
    send_to_client(
        &server,
        Apdu::ComplexAck(ComplexAck {
            segmented: true,
            more_follows: false,
            invoke_id,
            sequence_number: Some(1),
            proposed_window_size: Some(1),
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_ack: Bytes::from_static(b"response"),
        }),
    )
    .await;
    match recv_apdu(&mut rx, "final response SegmentACK").await {
        Apdu::SegmentAck(ack) => assert_eq!(ack.sequence_number, 1),
        other => panic!("expected response SegmentACK, got {other:?}"),
    }
    assert_eq!(
        timeout(Duration::from_secs(2), request)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .as_ref(),
        b"first-response"
    );
    assert_eq!(client.tsm.lock().await.pending_count(), 0);

    let mut client = Arc::try_unwrap(client).unwrap_or_else(|_| panic!("request retained client"));
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn stale_segment_ack_after_terminal_completion_is_not_routed() {
    let (client_transport, mut server) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
    let mut rx = server.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::start(config(), client_transport)
            .await
            .unwrap(),
    );
    client.segmented_post_wait_cleanup.enable();

    let first_client = Arc::clone(&client);
    let first = tokio::spawn(async move {
        first_client
            .confirmed_request(
                SERVER_MAC,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &[0x0C; 100],
            )
            .await
    });

    let invoke_id = loop {
        let request = match recv_apdu(&mut rx, "segmented request").await {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected ConfirmedRequest, got {other:?}"),
        };
        if request.more_follows {
            send_to_client(
                &server,
                Apdu::SegmentAck(SegmentAck {
                    negative_ack: false,
                    sent_by_server: true,
                    invoke_id: request.invoke_id,
                    sequence_number: request.sequence_number.unwrap(),
                    actual_window_size: 1,
                }),
            )
            .await;
        } else {
            send_to_client(
                &server,
                Apdu::SimpleAck(SimpleAck {
                    invoke_id: request.invoke_id,
                    service_choice: request.service_choice,
                }),
            )
            .await;
            break request.invoke_id;
        }
    };

    timeout(
        Duration::from_secs(2),
        client.segmented_post_wait_cleanup.wait_until_reached(),
    )
    .await
    .expect("terminal response did not reach delayed route cleanup");
    assert!(client
        .seg_ack_senders
        .lock()
        .await
        .contains_key(&(MacAddr::from_slice(SERVER_MAC), invoke_id)));

    send_to_client(
        &server,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 2,
            actual_window_size: 1,
        }),
    )
    .await;
    match recv_apdu(&mut rx, "stale SegmentACK after completion").await {
        Apdu::Abort(abort) => {
            assert_eq!(abort.invoke_id, invoke_id);
            assert!(!abort.sent_by_server);
            assert_eq!(abort.abort_reason, AbortReason::INVALID_APDU_IN_THIS_STATE);
        }
        other => panic!("expected idle-state Abort, got {other:?}"),
    }

    let second_client = Arc::clone(&client);
    let second = tokio::spawn(async move {
        second_client
            .confirmed_request(SERVER_MAC, ConfirmedServiceChoice::READ_PROPERTY, &[0x01])
            .await
    });
    let replacement = match recv_apdu(&mut rx, "replacement request").await {
        Apdu::ConfirmedRequest(request) => request,
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    };
    assert_ne!(replacement.invoke_id, invoke_id);
    assert!(!replacement.segmented);

    send_to_client(
        &server,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id: replacement.invoke_id,
            sequence_number: 2,
            actual_window_size: 1,
        }),
    )
    .await;
    expect_silence(&mut rx, "SegmentACK during replacement request").await;
    send_to_client(
        &server,
        Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: replacement.invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: replacement.service_choice,
            service_ack: Bytes::from_static(b"replacement"),
        }),
    )
    .await;
    assert_eq!(
        timeout(Duration::from_secs(2), second)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .as_ref(),
        b"replacement"
    );

    client.segmented_post_wait_cleanup.release();
    assert!(timeout(Duration::from_secs(2), first)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .is_empty());

    let mut client = Arc::try_unwrap(client).unwrap_or_else(|_| panic!("requests retained client"));
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
