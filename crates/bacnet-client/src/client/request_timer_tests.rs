use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bacnet_encoding::apdu::{self, Apdu, ComplexAck, encode_apdu};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort, TransportProvenance};
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::MacAddr;
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice};
use bacnet_types::error::{Error, RemoteError};
use bytes::{Bytes, BytesMut};
use tokio::sync::{Notify, mpsc};
use tokio::time::{Duration, timeout};

use super::{BACnetClient, ClientConfig};

const CLIENT_MAC: &[u8] = &[0x01];
const SERVER_MAC: &[u8] = &[0x02];

fn decode_outbound(npdu: Bytes) -> Apdu {
    let npdu = decode_npdu(npdu).expect("valid outbound NPDU");
    apdu::decode_apdu(npdu.payload).expect("valid outbound APDU")
}

fn encode_inbound(apdu: &Apdu) -> ReceivedNpdu {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, apdu).expect("valid APDU encoding");
    let mut npdu_buf = BytesMut::new();
    encode_npdu(
        &mut npdu_buf,
        &Npdu {
            payload: apdu_buf.freeze(),
            ..Npdu::default()
        },
    )
    .expect("valid NPDU encoding");
    ReceivedNpdu {
        direct_response: None,
        npdu: npdu_buf.freeze(),
        source_mac: MacAddr::from_slice(SERVER_MAC),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

fn response_segment(invoke_id: u8, sequence_number: u8, more_follows: bool) -> Apdu {
    Apdu::ComplexAck(ComplexAck {
        segmented: true,
        more_follows,
        invoke_id,
        sequence_number: Some(sequence_number),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::from(vec![sequence_number; 4]),
    })
}

/// A link whose second unicast blocks on `release_retry`, after telling
/// `retry_started`, and then fails; every other send lands in `outbound_tx`.
fn blocking_retry_transport(
    inbound_rx: mpsc::Receiver<ReceivedNpdu>,
    outbound_tx: mpsc::UnboundedSender<Bytes>,
    send_count: Arc<AtomicUsize>,
    retry_started: Arc<Notify>,
    release_retry: Arc<Notify>,
) -> TestTransport {
    TestTransport::builder()
        .local_mac(CLIENT_MAC)
        .inbound(inbound_rx)
        .on_send(move |frame| {
            let blocked = !frame.broadcast && send_count.fetch_add(1, Ordering::SeqCst) + 1 == 2;
            if !blocked {
                outbound_tx
                    .send(frame.npdu)
                    .expect("outbound observer remains open");
            }
            let (retry_started, release_retry) =
                (Arc::clone(&retry_started), Arc::clone(&release_retry));
            async move {
                if blocked {
                    retry_started.notify_one();
                    release_retry.notified().await;
                    return Err(Error::Transport(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "injected retry failure",
                    )));
                }
                Ok(())
            }
        })
        .build()
}

// Loopback uses only Tokio channels, so advance retry timers without depending
// on OS timer resolution across 256 waits (particularly on Windows).
#[tokio::test(start_paused = true)]
async fn max_retry_budget_sends_exactly_255_retransmissions() {
    let (client_transport, mut server_transport) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
    let mut server_rx = server_transport.start().await.unwrap();
    let config = ClientConfig {
        apdu_timeout_ms: 1,
        apdu_retries: u8::MAX,
        ..ClientConfig::default()
    };
    let mut client = BACnetClient::start(config, client_transport).await.unwrap();
    let drain = tokio::spawn(async move {
        let mut requests = 0usize;
        while let Ok(Some(received)) = timeout(Duration::from_millis(50), server_rx.recv()).await {
            assert!(matches!(
                decode_outbound(received.npdu),
                Apdu::ConfirmedRequest(_)
            ));
            requests += 1;
        }
        requests
    });

    let result = timeout(
        Duration::from_secs(3),
        client.confirmed_request(SERVER_MAC, ConfirmedServiceChoice::READ_PROPERTY, &[0x0C]),
    )
    .await
    .expect("the maximum retry budget must terminate");
    assert!(matches!(
        result,
        Err(Error::Remote(RemoteError::Abort { reason })) if reason == AbortReason::TSM_TIMEOUT
    ));
    assert_eq!(drain.await.unwrap(), 1 + usize::from(u8::MAX));

    client.stop().await.unwrap();
    server_transport.stop().await.unwrap();
}

#[tokio::test]
async fn retry_send_failure_precedes_final_timeout_indication() {
    let (_inbound_tx, inbound_rx) = mpsc::channel(8);
    let (outbound_tx, _outbound_rx) = mpsc::unbounded_channel();
    let retry_started = Arc::new(Notify::new());
    let release_retry = Arc::new(Notify::new());
    let send_count = Arc::new(AtomicUsize::new(0));
    let transport = blocking_retry_transport(
        inbound_rx,
        outbound_tx,
        Arc::clone(&send_count),
        retry_started,
        Arc::clone(&release_retry),
    );
    let config = ClientConfig {
        apdu_timeout_ms: 10,
        apdu_retries: 1,
        ..ClientConfig::default()
    };
    let mut client = BACnetClient::start(config, transport).await.unwrap();
    release_retry.notify_one();

    let result = timeout(
        Duration::from_secs(1),
        client.confirmed_request(SERVER_MAC, ConfirmedServiceChoice::READ_PROPERTY, &[0x0C]),
    )
    .await
    .expect("retry send failure did not complete the request");
    match result {
        Err(Error::Transport(error)) => {
            assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
            assert_eq!(error.to_string(), "injected retry failure");
        }
        other => panic!("expected retry transport failure, got {other:?}"),
    }
    assert_eq!(send_count.load(Ordering::SeqCst), 2);
    assert_eq!(client.tsm.lock().pending_count(), 0);
    assert_eq!(client.tsm.lock().coordinated_active_count(), 0);

    client.stop().await.unwrap();
}

#[tokio::test]
async fn blocked_retry_does_not_block_segment_admission_or_cancel_on_send_failure() {
    let (inbound_tx, inbound_rx) = mpsc::channel(8);
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel();
    let retry_started = Arc::new(Notify::new());
    let release_retry = Arc::new(Notify::new());
    let transport = blocking_retry_transport(
        inbound_rx,
        outbound_tx,
        Arc::new(AtomicUsize::new(0)),
        Arc::clone(&retry_started),
        Arc::clone(&release_retry),
    );
    let config = ClientConfig {
        apdu_timeout_ms: 50,
        apdu_retries: 1,
        ..ClientConfig::default()
    };
    let client = BACnetClient::start(config, transport).await.unwrap();
    let request = tokio::spawn(async move {
        let result = client
            .confirmed_request(SERVER_MAC, ConfirmedServiceChoice::READ_PROPERTY, &[0x0C])
            .await;
        (client, result)
    });

    let invoke_id = match decode_outbound(
        timeout(Duration::from_secs(1), outbound_rx.recv())
            .await
            .expect("initial request timed out")
            .expect("outbound observer closed"),
    ) {
        Apdu::ConfirmedRequest(request) => request.invoke_id,
        other => panic!("expected initial ConfirmedRequest, got {other:?}"),
    };
    timeout(Duration::from_secs(1), retry_started.notified())
        .await
        .expect("retry did not reach the blocking transport");

    inbound_tx
        .send(encode_inbound(&response_segment(invoke_id, 0, true)))
        .await
        .unwrap();
    match decode_outbound(
        timeout(Duration::from_millis(500), outbound_rx.recv())
            .await
            .expect("segment admission was blocked behind retry I/O")
            .expect("outbound observer closed"),
    ) {
        Apdu::SegmentAck(ack) => assert_eq!(ack.sequence_number, 0),
        other => panic!("expected SegmentAck while retry was blocked, got {other:?}"),
    }

    release_retry.notify_one();
    inbound_tx
        .send(encode_inbound(&response_segment(invoke_id, 1, false)))
        .await
        .unwrap();
    match decode_outbound(
        timeout(Duration::from_secs(1), outbound_rx.recv())
            .await
            .expect("final SegmentAck timed out")
            .expect("outbound observer closed"),
    ) {
        Apdu::SegmentAck(ack) => assert_eq!(ack.sequence_number, 1),
        other => panic!("expected final SegmentAck, got {other:?}"),
    }

    let (mut client, result) = timeout(Duration::from_secs(1), request)
        .await
        .expect("request did not complete")
        .unwrap();
    assert_eq!(result.unwrap().as_ref(), &[0, 0, 0, 0, 1, 1, 1, 1]);
    client.stop().await.unwrap();
}
