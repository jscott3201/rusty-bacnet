use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bacnet_encoding::apdu::{self, encode_apdu, AbortPdu, Apdu, SegmentAck, SimpleAck};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::sync::{mpsc, Notify};
use tokio::time::{timeout, Duration};

use super::{BACnetClient, ClientConfig};

const CLIENT_MAC: &[u8] = &[0x01];
const SERVER_MAC: &[u8] = &[0x02];

struct EarlyReadyResponseTransport {
    inbound_rx: Option<mpsc::Receiver<ReceivedNpdu>>,
    inbound_tx: mpsc::Sender<ReceivedNpdu>,
    wire_abort_tx: mpsc::Sender<AbortPdu>,
    wire_abort_seen: Arc<Notify>,
    response_injected: AtomicBool,
    final_segment_polled: Arc<AtomicBool>,
}

impl EarlyReadyResponseTransport {
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

impl TransportPort for EarlyReadyResponseTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.inbound_rx.take().expect("transport starts once"))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        let npdu = decode_npdu(Bytes::copy_from_slice(npdu)).unwrap();
        match apdu::decode_apdu(npdu.payload).unwrap() {
            Apdu::Abort(abort) => {
                self.wire_abort_tx.send(abort).await.unwrap();
                self.wire_abort_seen.notify_one();
            }
            Apdu::ConfirmedRequest(request) if request.segmented => {
                if !request.more_follows {
                    self.final_segment_polled.store(true, Ordering::Release);
                    return Ok(());
                }
                if self.response_injected.swap(true, Ordering::AcqRel) {
                    return Ok(());
                }

                self.inbound_tx
                    .send(Self::inbound(Apdu::SegmentAck(SegmentAck {
                        negative_ack: false,
                        sent_by_server: true,
                        invoke_id: request.invoke_id,
                        sequence_number: request.sequence_number.unwrap(),
                        actual_window_size: 1,
                    })))
                    .await
                    .unwrap();
                self.inbound_tx
                    .send(Self::inbound(Apdu::SimpleAck(SimpleAck {
                        invoke_id: request.invoke_id,
                        service_choice: request.service_choice,
                    })))
                    .await
                    .unwrap();
                // Do not complete the first send until dispatch has admitted
                // the already-queued terminal PDU and emitted its wire Abort.
                self.wire_abort_seen.notified().await;
            }
            _ => {}
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
async fn ready_terminal_before_final_send_aborts_without_polling_final_send() {
    let (inbound_tx, inbound_rx) = mpsc::channel(16);
    let (wire_abort_tx, mut wire_abort_rx) = mpsc::channel(1);
    let wire_abort_seen = Arc::new(Notify::new());
    let final_segment_polled = Arc::new(AtomicBool::new(false));
    let transport = EarlyReadyResponseTransport {
        inbound_rx: Some(inbound_rx),
        inbound_tx,
        wire_abort_tx,
        wire_abort_seen,
        response_injected: AtomicBool::new(false),
        final_segment_polled: Arc::clone(&final_segment_polled),
    };
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
        Err(Error::Abort { reason })
            if reason == AbortReason::INVALID_APDU_IN_THIS_STATE.to_raw()
    ));
    assert!(
        !final_segment_polled.load(Ordering::Acquire),
        "the final send future was polled after the premature response completed the transaction"
    );
    assert_eq!(client.tsm.lock().await.pending_count(), 0);
    client.stop().await.unwrap();
}
