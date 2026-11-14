use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bacnet_encoding::apdu::{self, AbortPdu, Apdu, SegmentAck, SimpleAck, encode_apdu};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_transport::test_transport::{SendMode, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice};
use bacnet_types::error::{Error, RemoteError};
use bytes::BytesMut;
use tokio::sync::{Notify, mpsc};
use tokio::time::{Duration, timeout};

use super::{BACnetClient, ClientConfig};

const CLIENT_MAC: &[u8] = &[0x01];
const SERVER_MAC: &[u8] = &[0x02];

fn early_ready_inbound(apdu: Apdu) -> ReceivedNpdu {
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
        direct_response: None,
        npdu: npdu_buf.freeze(),
        source_mac: MacAddr::from_slice(SERVER_MAC),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

/// A link that, on the first segment with more to follow, queues a
/// SegmentAck and the final response, then holds that send until the wire
/// Abort the dispatch emits for the queued terminal PDU is seen.
fn early_ready_response_transport(
    inbound_rx: mpsc::Receiver<ReceivedNpdu>,
    inbound_tx: mpsc::Sender<ReceivedNpdu>,
    wire_abort_tx: mpsc::Sender<AbortPdu>,
    wire_abort_seen: Arc<Notify>,
    final_segment_polled: Arc<AtomicBool>,
) -> TestTransport {
    let response_injected = Arc::new(AtomicBool::new(false));
    TestTransport::builder()
        .local_mac(CLIENT_MAC)
        .inbound(inbound_rx)
        .broadcast(SendMode::Ignore)
        .on_send(move |frame| {
            let inbound_tx = inbound_tx.clone();
            let wire_abort_tx = wire_abort_tx.clone();
            let wire_abort_seen = Arc::clone(&wire_abort_seen);
            let final_segment_polled = Arc::clone(&final_segment_polled);
            let response_injected = Arc::clone(&response_injected);
            async move {
                let npdu = decode_npdu(frame.npdu).unwrap();
                match apdu::decode_apdu(npdu.payload).unwrap() {
                    Apdu::Abort(abort) => {
                        wire_abort_tx.send(abort).await.unwrap();
                        wire_abort_seen.notify_one();
                    }
                    Apdu::ConfirmedRequest(request) if request.segmented => {
                        if !request.more_follows {
                            final_segment_polled.store(true, Ordering::Release);
                            return Ok(());
                        }
                        if response_injected.swap(true, Ordering::AcqRel) {
                            return Ok(());
                        }

                        inbound_tx
                            .send(early_ready_inbound(Apdu::SegmentAck(SegmentAck {
                                negative_ack: false,
                                sent_by_server: true,
                                invoke_id: request.invoke_id,
                                sequence_number: request.sequence_number.unwrap(),
                                actual_window_size: 1,
                            })))
                            .await
                            .unwrap();
                        inbound_tx
                            .send(early_ready_inbound(Apdu::SimpleAck(SimpleAck {
                                invoke_id: request.invoke_id,
                                service_choice: request.service_choice,
                            })))
                            .await
                            .unwrap();
                        // Do not complete the first send until dispatch has admitted
                        // the already-queued terminal PDU and emitted its wire Abort.
                        wire_abort_seen.notified().await;
                    }
                    _ => {}
                }
                Ok(())
            }
        })
        .build()
}

#[tokio::test]
async fn ready_terminal_before_final_send_aborts_without_polling_final_send() {
    let (inbound_tx, inbound_rx) = mpsc::channel(16);
    let (wire_abort_tx, mut wire_abort_rx) = mpsc::channel(1);
    let wire_abort_seen = Arc::new(Notify::new());
    let final_segment_polled = Arc::new(AtomicBool::new(false));
    let transport = early_ready_response_transport(
        inbound_rx,
        inbound_tx,
        wire_abort_tx,
        wire_abort_seen,
        Arc::clone(&final_segment_polled),
    );
    let mut client = BACnetClient::start(
        ClientConfig {
            apdu_timeout_ms: 2_000,
            apdu_retries: 0,
            max_apdu_length: 50,
            proposed_window_size: 1,
            ..ClientConfig::default()
        },
        transport,
    )
    .await
    .unwrap();

    let result = timeout(
        Duration::from_secs(2),
        client.confirmed_request(
            SERVER_MAC,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            &[0x0C; 100],
        ),
    )
    .await
    .expect("premature response did not wake the sender");
    let abort = timeout(Duration::from_secs(2), wire_abort_rx.recv())
        .await
        .expect("client did not send a wire Abort")
        .expect("wire Abort channel closed");

    assert!(!abort.sent_by_server);
    assert_eq!(abort.abort_reason, AbortReason::INVALID_APDU_IN_THIS_STATE);
    assert!(matches!(
        result,
        Err(Error::Remote(RemoteError::Abort { reason }))
            if reason == AbortReason::INVALID_APDU_IN_THIS_STATE
    ));
    assert!(
        !final_segment_polled.load(Ordering::Acquire),
        "the final send future was polled after the premature response completed the transaction"
    );
    assert_eq!(client.tsm.lock().pending_count(), 0);
    client.stop().await.unwrap();
}
