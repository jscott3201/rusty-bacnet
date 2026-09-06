use bytes::{Bytes, BytesMut};

use super::*;
use crate::mstp_frame::{decode_frame, encode_frame, FrameType, MstpFrame};
use crate::port::TransportPort;

fn expecting_reply_frame() -> MstpFrame {
    MstpFrame {
        frame_type: FrameType::BACnetDataExpectingReply,
        destination: 3,
        source: 7,
        data: Bytes::from_static(&[0x01, 0x04, 0x10]),
    }
}

fn timing_config() -> MstpConfig {
    MstpConfig {
        this_station: 3,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    }
}

#[tokio::test(start_paused = true)]
async fn ready_data_reply_does_not_wait_for_reply_delay() {
    let (serial_transport, serial_peer) = LoopbackSerial::pair();
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut encoded = BytesMut::new();
    encode_frame(&mut encoded, &expecting_reply_frame()).unwrap();
    let started = tokio::time::Instant::now();
    serial_peer.write(&encoded).await.unwrap();

    let received = npdu_rx.recv().await.unwrap();
    received
        .reply_tx
        .unwrap()
        .send(Bytes::from_static(&[0x01, 0x00, 0x30, 0x01]))
        .unwrap();
    let mut response_buf = [0u8; 64];
    let response_len = serial_peer.read(&mut response_buf).await.unwrap();
    let (response, _) = decode_frame(&response_buf[..response_len]).unwrap();

    assert_eq!(response.frame_type, FrameType::BACnetDataNotExpectingReply);
    assert_eq!(response.destination, 7);
    assert!(started.elapsed() < tokio::time::Duration::from_millis(50));
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn reply_postponed_starts_before_reply_delay_limit() {
    let (serial_transport, serial_peer) = LoopbackSerial::pair();
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut encoded = BytesMut::new();
    encode_frame(&mut encoded, &expecting_reply_frame()).unwrap();
    let started = tokio::time::Instant::now();
    serial_peer.write(&encoded).await.unwrap();

    // Keep the application sender alive without replying so the deadline,
    // rather than channel cancellation, selects ReplyPostponed.
    let received = npdu_rx.recv().await.unwrap();
    let _reply_tx = received.reply_tx.unwrap();
    let mut response_buf = [0u8; 64];
    let response_len = serial_peer.read(&mut response_buf).await.unwrap();
    let (response, _) = decode_frame(&response_buf[..response_len]).unwrap();

    assert_eq!(response.frame_type, FrameType::ReplyPostponed);
    assert_eq!(response.destination, 7);
    assert!(started.elapsed() < tokio::time::Duration::from_millis(T_REPLY_DELAY_MS));
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn second_data_request_does_not_extend_first_reply_deadline() {
    let (serial_transport, serial_peer) = LoopbackSerial::pair();
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut first = BytesMut::new();
    encode_frame(&mut first, &expecting_reply_frame()).unwrap();
    let started = tokio::time::Instant::now();
    serial_peer.write(&first).await.unwrap();

    let received = npdu_rx.recv().await.unwrap();
    let _reply_tx = received.reply_tx.unwrap();
    tokio::time::advance(tokio::time::Duration::from_millis(100)).await;

    let mut second = BytesMut::new();
    let second_request = MstpFrame {
        source: 8,
        data: Bytes::from_static(&[0x01, 0x04, 0x20]),
        ..expecting_reply_frame()
    };
    encode_frame(&mut second, &second_request).unwrap();
    serial_peer.write(&second).await.unwrap();
    tokio::task::yield_now().await;
    assert!(npdu_rx.try_recv().is_err());

    let mut response_buf = [0u8; 64];
    let response_len = serial_peer.read(&mut response_buf).await.unwrap();
    let (response, _) = decode_frame(&response_buf[..response_len]).unwrap();
    assert_eq!(response.frame_type, FrameType::ReplyPostponed);
    assert_eq!(response.destination, 7);
    assert!(started.elapsed() < tokio::time::Duration::from_millis(T_REPLY_DELAY_MS));
    transport.stop().await.unwrap();
}
