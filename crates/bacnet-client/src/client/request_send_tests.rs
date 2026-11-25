//! What a request's send leaves behind when it is cut off, and the bound on
//! a segmented request's negative acknowledgements (#2558). The request
//! path's transaction guard covers the send from the moment the
//! transaction is registered: a request dropped while its frame is held
//! leaves no transaction, no invoke ID and no SegmentACK route behind. A
//! segmented request fails after more than ten negative acknowledgements
//! in a row, and a positive one in between starts the count again.

use std::sync::Arc;

use bacnet_encoding::apdu::{Apdu, ConfirmedRequest, SegmentAck, SimpleAck, encode_apdu};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_transport::test_transport::{StartMode, TestTransport, TestTransportHandle};
use bacnet_types::MacAddr;
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice};
use bacnet_types::error::{Error, RemoteError, SegmentationError};
use bytes::BytesMut;
use tokio::time::{Duration, timeout};

use super::{BACnetClient, ClientConfig};

const CLIENT_MAC: &[u8] = &[0x01];
const SERVER_MAC: &[u8] = &[0x02];

/// A client on a test link that keeps its inbound channel open, so the
/// dispatch task runs and takes the requests' cleanups.
async fn client(config: ClientConfig) -> (Arc<BACnetClient<TestTransport>>, TestTransportHandle) {
    let transport = TestTransport::builder()
        .local_mac(CLIENT_MAC)
        .start(StartMode::Fresh(64))
        .build();
    let handle = transport.handle();
    let client = BACnetClient::start(config, transport).await.unwrap();
    (Arc::new(client), handle)
}

/// Hand the client `apdu` from the server.
async fn from_server(handle: &TestTransportHandle, apdu: &Apdu) {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, apdu).unwrap();
    let mut npdu_buf = BytesMut::new();
    encode_npdu(
        &mut npdu_buf,
        &Npdu {
            payload: apdu_buf.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    let inbound = handle.inbound().expect("the link is started");
    inbound
        .send(ReceivedNpdu {
            direct_response: None,
            npdu: npdu_buf.freeze(),
            source_mac: MacAddr::from_slice(SERVER_MAC),
            link_layer_group: false,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx: None,
        })
        .await
        .unwrap();
}

/// The `index`th frame the client sent, as the confirmed request it is.
async fn sent_request(handle: &TestTransportHandle, index: usize) -> ConfirmedRequest {
    let sent = handle.sent();
    timeout(Duration::from_secs(5), sent.wait_for_len(index + 1))
        .await
        .unwrap_or_else(|_| panic!("frame {index} was never sent"));
    match sent.frame(index).apdu() {
        Apdu::ConfirmedRequest(request) => request,
        other => panic!("frame {index}: expected a ConfirmedRequest, got {other:?}"),
    }
}

/// Wait until the dispatch task has applied a dropped request's cleanup:
/// no transaction pending and no invoke ID held. The loop sleeps rather
/// than yields: on paused time the clock moves only while the runtime is
/// idle, so a yield loop would never let the timeout run out.
async fn wait_until_released(client: &BACnetClient<TestTransport>) {
    timeout(Duration::from_secs(5), async {
        loop {
            {
                let tsm = client.tsm.lock();
                if tsm.pending_count() == 0 && tsm.coordinated_active_count() == 0 {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the dropped request's transaction was never released");
}

fn segment_ack(invoke_id: u8, sequence_number: u8, negative_ack: bool) -> Apdu {
    Apdu::SegmentAck(SegmentAck {
        negative_ack,
        sent_by_server: true,
        invoke_id,
        sequence_number,
        actual_window_size: 1,
    })
}

/// A client whose requests over 44 octets of service data go segmented
/// (the segment payload at a maximum APDU of 50 octets), one segment to a
/// window, with no segment timeout in a test's way.
fn segmented_config() -> ClientConfig {
    ClientConfig {
        apdu_timeout_ms: 60_000,
        max_apdu_length: 50,
        proposed_window_size: 1,
        ..ClientConfig::default()
    }
}

#[tokio::test(start_paused = true)]
async fn a_request_dropped_while_its_frame_is_held_leaves_nothing_registered() {
    let (client, handle) = client(ClientConfig::default()).await;
    handle.block_next_send();
    let request_client = Arc::clone(&client);
    let request = tokio::spawn(async move {
        request_client
            .confirmed_request(SERVER_MAC, ConfirmedServiceChoice::READ_PROPERTY, &[0x0C])
            .await
    });
    timeout(Duration::from_secs(5), handle.wait_blocked())
        .await
        .expect("the request's frame was never sent");
    // Registered before its frame went out, so an answer finds it.
    assert_eq!(client.tsm.lock().pending_count(), 1);

    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    wait_until_released(&client).await;

    // The client goes on: the next request is sent and answered.
    let request_client = Arc::clone(&client);
    let next = tokio::spawn(async move {
        request_client
            .confirmed_request(SERVER_MAC, ConfirmedServiceChoice::WRITE_PROPERTY, &[0x0D])
            .await
    });
    let invoke_id = sent_request(&handle, 1).await.invoke_id;
    from_server(
        &handle,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .await;
    assert!(next.await.unwrap().unwrap().is_empty());
    let mut client = Arc::try_unwrap(client).unwrap_or_else(|_| panic!("client still shared"));
    client.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_segmented_request_dropped_while_its_first_segment_is_held_leaves_nothing_registered() {
    let (client, handle) = client(segmented_config()).await;
    handle.block_next_send();
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
    timeout(Duration::from_secs(5), handle.wait_blocked())
        .await
        .expect("the first segment was never sent");
    let first = sent_request(&handle, 0).await;
    assert!(first.segmented);
    let route = (MacAddr::from_slice(SERVER_MAC), first.invoke_id);
    // The transaction and its SegmentACK route are in place before the
    // first segment goes, so its acknowledgement finds them.
    assert_eq!(client.tsm.lock().pending_count(), 1);
    assert!(client.seg_ack_senders.lock().contains_key(&route));

    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    wait_until_released(&client).await;
    timeout(Duration::from_secs(5), async {
        while client.seg_ack_senders.lock().contains_key(&route) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the dropped request's SegmentACK route was never removed");
    let mut client = Arc::try_unwrap(client).unwrap_or_else(|_| panic!("client still shared"));
    client.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn eleven_negative_acknowledgements_in_a_row_end_a_segmented_request() {
    let (client, handle) = client(segmented_config()).await;
    let request_client = Arc::clone(&client);
    let request = tokio::spawn(async move {
        request_client
            .confirmed_request(
                SERVER_MAC,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &[0x0C; 44 * 20],
            )
            .await
    });

    let mut invoke_id = None;
    for seq in 0..=10u8 {
        let segment = sent_request(&handle, usize::from(seq)).await;
        assert_eq!(segment.sequence_number, Some(seq));
        let id = *invoke_id.get_or_insert(segment.invoke_id);
        // Each names the segment just sent, so the window moves on, and
        // the request fails only on the count.
        from_server(&handle, &segment_ack(id, seq, true)).await;
    }

    let result = timeout(Duration::from_secs(5), request)
        .await
        .expect("the request did not end")
        .unwrap();
    assert!(
        matches!(
            result,
            Err(Error::Segmentation(
                SegmentationError::NegativeAckRetriesExhausted
            ))
        ),
        "{result:?}"
    );
    // Eleven segments went, and no twelfth.
    assert_eq!(handle.sent().len(), 11);
    let route = (MacAddr::from_slice(SERVER_MAC), invoke_id.unwrap());
    assert!(!client.seg_ack_senders.lock().contains_key(&route));
    wait_until_released(&client).await;
    let mut client = Arc::try_unwrap(client).unwrap_or_else(|_| panic!("client still shared"));
    client.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_positive_acknowledgement_starts_the_negative_count_again() {
    let (client, handle) = client(segmented_config()).await;
    let request_client = Arc::clone(&client);
    let request = tokio::spawn(async move {
        request_client
            .confirmed_request(
                SERVER_MAC,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &[0x0C; 44 * 13],
            )
            .await
    });

    let mut invoke_id = None;
    for seq in 0..=12u8 {
        let segment = sent_request(&handle, usize::from(seq)).await;
        assert_eq!(segment.sequence_number, Some(seq));
        assert_eq!(segment.more_follows, seq < 12);
        let id = *invoke_id.get_or_insert(segment.invoke_id);
        // Ten negative, one positive, then an eleventh negative, which the
        // positive one in between keeps from ending the request.
        from_server(&handle, &segment_ack(id, seq, seq != 10 && seq != 12)).await;
    }
    from_server(
        &handle,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id: invoke_id.unwrap(),
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .await;

    let result = timeout(Duration::from_secs(5), request)
        .await
        .expect("the request did not end")
        .unwrap();
    assert!(result.unwrap().is_empty());
    assert_eq!(handle.sent().len(), 13);
    let mut client = Arc::try_unwrap(client).unwrap_or_else(|_| panic!("client still shared"));
    client.stop().await.unwrap();
}

/// A segmented request to a peer that never acknowledges ends with the
/// timeout Abort once its segment timer has run out one time more than the
/// retries allowed, at the largest setting too (#2809): the window goes
/// once and 255 times again, and the 256th expiry ends the request.
#[tokio::test(start_paused = true)]
async fn a_segmented_request_to_a_silent_peer_ends_after_its_last_retry_at_255() {
    let config = ClientConfig {
        apdu_retries: u8::MAX,
        ..segmented_config()
    };
    let (client, handle) = client(config).await;
    let result = timeout(
        Duration::from_secs(24 * 60 * 60),
        client.confirmed_request(
            SERVER_MAC,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            &[0x0C; 100],
        ),
    )
    .await
    .expect("the request never ended");
    assert!(
        matches!(
            result,
            Err(Error::Remote(RemoteError::Abort { reason })) if reason == AbortReason::TSM_TIMEOUT
        ),
        "{result:?}"
    );
    let sent = handle.sent().frames();
    assert_eq!(sent.len(), 1 + usize::from(u8::MAX));
    assert!(sent.iter().all(|frame| matches!(
        frame.apdu(),
        Apdu::ConfirmedRequest(segment) if segment.segmented && segment.sequence_number == Some(0)
    )));
    wait_until_released(&client).await;
    let mut client = Arc::try_unwrap(client).unwrap_or_else(|_| panic!("client still shared"));
    client.stop().await.unwrap();
}

#[test]
fn the_segment_payload_assumed_above_holds() {
    assert_eq!(
        bacnet_encoding::segmentation::max_segment_payload(
            50,
            bacnet_encoding::segmentation::SegmentedPduType::ConfirmedRequest
        ),
        44
    );
}
