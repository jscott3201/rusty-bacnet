//! An answer for a routed device counts only when it comes through the
//! router its request went to (#2831).
//!
//! A routed frame's SNET and SADR are whatever its sender wrote, so any
//! station on this link can name the device in them; the link station a
//! frame comes from is the part a station can't choose (on B/IP, its UDP
//! source). Every test here runs a request to station [`DADR`] on network
//! [`DNET`] through [`ROUTER`], and has [`STRANGER`], another station on the
//! link, answer for the device first. On paused time, so nothing here waits
//! out a real timer.

use std::sync::Arc;

use bacnet_encoding::apdu::{
    AbortPdu, Apdu, ComplexAck, ErrorPdu, RejectPdu, SegmentAck, SimpleAck,
};
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, RejectReason,
};
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep, timeout};

use super::routed_path_limit_tests::{
    DNET, SentFrame, confirmed_request, finish_segmented, harness, inject_apdu, inject_simple_ack,
    routed_request,
};
use super::{BACnetClient, ClientConfig, Station};

const ROUTER: [u8; 1] = [2];
const DADR: [u8; 1] = [3];
const STRANGER: [u8; 1] = [9];
/// 300 octets of service data: more than the conservative 228-octet routed
/// path carries in one frame, so the request goes in segments.
const LARGE: usize = 300;

type Client = Arc<BACnetClient<TestTransport>>;

async fn client() -> (
    Client,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::UnboundedReceiver<SentFrame>,
) {
    let (transport, inbound, outbound) = harness(&[1], 1476);
    let client = Arc::new(
        BACnetClient::start(ClientConfig::default(), transport)
            .await
            .unwrap(),
    );
    (client, inbound, outbound)
}

/// What `from` says for the device: `apdu` with the device's SNET and SADR.
async fn answer_from(inbound: &mpsc::Sender<ReceivedNpdu>, from: &[u8], apdu: Apdu) {
    inject_apdu(inbound, from, Some((DNET, &DADR)), apdu).await;
}

/// Nothing goes out for a while: on paused time, well inside the six
/// second APDU timeout, so a retry can't be mistaken for an answer to the
/// stranger.
async fn nothing_sent(outbound: &mut mpsc::UnboundedReceiver<SentFrame>, context: &str) {
    sleep(Duration::from_secs(1)).await;
    assert!(outbound.try_recv().is_err(), "{context}: a frame went out");
}

/// Every acknowledgment and refusal a stranger sends for the device, under
/// every invoke ID, is dropped: the request stays open, nothing is sent
/// back, and the router's answer then completes it. Only the frames that
/// named the open request are counted.
#[tokio::test(start_paused = true)]
async fn answers_for_a_routed_device_through_another_station_complete_nothing() {
    let (client, inbound, mut outbound) = client().await;
    let task = routed_request(
        Arc::clone(&client),
        ROUTER.to_vec(),
        DNET,
        DADR.to_vec(),
        vec![0x41; 4],
    );
    let request = confirmed_request(outbound.recv().await.unwrap(), &ROUTER);
    let invoke_id = request.invoke_id;
    let service_choice = ConfirmedServiceChoice::WRITE_PROPERTY;

    for guess in 0..=u8::MAX {
        inject_simple_ack(&inbound, &STRANGER, DNET, &DADR, guess).await;
    }
    for refusal in [
        Apdu::Error(ErrorPdu {
            invoke_id,
            service_choice,
            error_class: ErrorClass::PROPERTY,
            error_code: ErrorCode::WRITE_ACCESS_DENIED,
            error_data: Bytes::new(),
        }),
        Apdu::Reject(RejectPdu {
            invoke_id,
            reject_reason: RejectReason::OTHER,
        }),
        Apdu::Abort(AbortPdu {
            sent_by_server: true,
            invoke_id,
            abort_reason: AbortReason::OTHER,
        }),
    ] {
        answer_from(&inbound, &STRANGER, refusal).await;
    }
    nothing_sent(&mut outbound, "after the stranger's answers").await;
    assert!(!task.is_finished(), "a stranger's answer ended the request");
    assert_eq!(client.off_path_answer_drops(), 4);

    inject_simple_ack(&inbound, &ROUTER, DNET, &DADR, invoke_id).await;
    let answered = timeout(Duration::from_secs(1), task).await.unwrap();
    assert!(answered.unwrap().unwrap().is_empty());
    assert_eq!(client.off_path_answer_drops(), 4);
}

/// A segment of a ComplexACK a stranger sends for the device is neither
/// acknowledged nor kept: no Segment-ACK goes to anyone, no reassembly
/// starts that the device's own answer would then abort, and the device's
/// answer through the router completes the read with its own data.
#[tokio::test(start_paused = true)]
async fn a_segmented_answer_through_another_station_is_neither_acknowledged_nor_kept() {
    let (client, inbound, mut outbound) = client().await;
    let read = {
        let client = Arc::clone(&client);
        tokio::spawn(async move {
            client
                .confirmed_request(
                    &Station::routed(&ROUTER, DNET, &DADR),
                    ConfirmedServiceChoice::READ_PROPERTY,
                    &[0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55],
                )
                .await
        })
    };
    let request = confirmed_request(outbound.recv().await.unwrap(), &ROUTER);
    let segment = |data: &'static [u8]| {
        Apdu::ComplexAck(ComplexAck {
            segmented: true,
            more_follows: true,
            invoke_id: request.invoke_id,
            sequence_number: Some(0),
            proposed_window_size: Some(1),
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_ack: Bytes::from_static(data),
        })
    };
    answer_from(&inbound, &STRANGER, segment(&[0xEE])).await;
    nothing_sent(&mut outbound, "after the stranger's segment").await;
    assert!(!read.is_finished());
    assert_eq!(client.off_path_answer_drops(), 1);

    answer_from(
        &inbound,
        &ROUTER,
        Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: request.invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_ack: Bytes::from_static(&[0xAA]),
        }),
    )
    .await;
    let answered = timeout(Duration::from_secs(1), read).await.unwrap();
    assert_eq!(answered.unwrap().unwrap().as_ref(), [0xAA]);
}

/// A Segment-ACK a stranger sends for the device moves no segmented
/// request on: the next segment waits for the device's own acknowledgment
/// through the router, which then carries the request to its end.
#[tokio::test(start_paused = true)]
async fn a_segment_ack_through_another_station_moves_no_segmented_request() {
    let (client, inbound, mut outbound) = client().await;
    let task = routed_request(
        Arc::clone(&client),
        ROUTER.to_vec(),
        DNET,
        DADR.to_vec(),
        vec![0x42; LARGE],
    );
    let first = confirmed_request(outbound.recv().await.unwrap(), &ROUTER);
    assert!(first.segmented && first.more_follows);
    answer_from(
        &inbound,
        &STRANGER,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id: first.invoke_id,
            sequence_number: 0,
            actual_window_size: 1,
        }),
    )
    .await;
    nothing_sent(&mut outbound, "after the stranger's Segment-ACK").await;
    assert_eq!(client.off_path_answer_drops(), 1);

    finish_segmented(&inbound, &mut outbound, &ROUTER, DNET, &DADR, first, 1476).await;
    let answered = timeout(Duration::from_secs(1), task).await.unwrap();
    assert!(answered.unwrap().unwrap().is_empty());
}

/// A stranger's answer that names no open request is not counted as one
/// that came the wrong way, and an answer the router relays for the device
/// after the request ended is not either: neither came for a request this
/// client still has open.
#[tokio::test(start_paused = true)]
async fn answers_for_no_open_request_are_not_counted_as_off_path() {
    let (client, inbound, mut outbound) = client().await;
    let task = routed_request(
        Arc::clone(&client),
        ROUTER.to_vec(),
        DNET,
        DADR.to_vec(),
        vec![0x41; 4],
    );
    let request = confirmed_request(outbound.recv().await.unwrap(), &ROUTER);
    inject_simple_ack(&inbound, &ROUTER, DNET, &DADR, request.invoke_id).await;
    assert!(task.await.unwrap().unwrap().is_empty());
    for from in [ROUTER, STRANGER] {
        answer_from(
            &inbound,
            &from,
            Apdu::SimpleAck(SimpleAck {
                invoke_id: request.invoke_id,
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            }),
        )
        .await;
    }
    nothing_sent(&mut outbound, "after answers for a request already over").await;
    assert_eq!(client.off_path_answer_drops(), 0);
}
