use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::mpsc;

use super::*;

#[test]
fn second_data_request_cannot_replace_pending_reply_source() {
    let (tx, mut rx) = mpsc::channel(16);
    let config = MstpConfig {
        this_station: 3,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    let first = MstpFrame {
        frame_type: FrameType::BACnetDataExpectingReply,
        destination: 3,
        source: 7,
        data: Bytes::from_static(&[0x01, 0x00, 0x11]),
    };
    let second = MstpFrame {
        source: 8,
        data: Bytes::from_static(&[0x01, 0x00, 0x22]),
        ..first.clone()
    };

    node.handle_received_frame(&first, &tx);
    node.handle_received_frame(&second, &tx);

    assert_eq!(node.pending_reply_source, Some(7));
    let received = rx.try_recv().unwrap();
    assert_eq!(received.source_mac, MacAddr::from_slice(&[7]));
    assert_eq!(received.npdu, first.data);
    assert!(rx.try_recv().is_err());
    let reply = node
        .finish_data_request(Some(Bytes::from_static(&[0x01, 0x00, 0xAA])))
        .unwrap();
    assert_eq!(reply.destination, 7);
}

#[test]
fn token_cannot_break_pending_reply_ownership() {
    let (tx, mut rx) = mpsc::channel(16);
    let config = MstpConfig {
        this_station: 3,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    let first = MstpFrame {
        frame_type: FrameType::BACnetDataExpectingReply,
        destination: 3,
        source: 7,
        data: Bytes::from_static(&[0x01, 0x00, 0x11]),
    };
    let token = MstpFrame {
        frame_type: FrameType::Token,
        destination: 3,
        source: 9,
        data: Bytes::new(),
    };
    let second = MstpFrame {
        source: 8,
        data: Bytes::from_static(&[0x01, 0x00, 0x22]),
        ..first.clone()
    };

    node.handle_received_frame(&first, &tx);
    node.handle_received_frame(&token, &tx);
    node.handle_received_frame(&second, &tx);

    assert_eq!(node.state, MasterState::AnswerDataRequest);
    assert_eq!(node.pending_reply_source, Some(7));
    assert_eq!(rx.try_recv().unwrap().source_mac, MacAddr::from_slice(&[7]));
    assert!(rx.try_recv().is_err());
    let reply = node
        .finish_data_request(Some(Bytes::from_static(&[0x01, 0x00, 0xAA])))
        .unwrap();
    assert_eq!(reply.destination, 7);
}
