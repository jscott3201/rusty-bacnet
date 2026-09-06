use super::*;
use crate::port::TransportPort;

#[test]
fn next_addr_wraps() {
    assert_eq!(next_addr(0, 127), 1);
    assert_eq!(next_addr(126, 127), 127);
    assert_eq!(next_addr(127, 127), 0);
    assert_eq!(next_addr(0, 0), 0); // edge case: max_master=0, wraps to self
}

#[test]
fn master_node_initial_state() {
    let config = MstpConfig {
        this_station: 5,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let node = MasterNode::new(config).unwrap();
    assert_eq!(node.state, MasterState::Idle);
    assert_eq!(node.next_station, 5);
    assert_eq!(node.poll_station, 5);
    assert_eq!(node.token_count, NPOLL);
    assert_eq!(node.retry_token_count, 0);
    assert!(node.reply_rx.is_none());
    assert!(node.pending_reply_source.is_none());
}

#[test]
fn handle_token_for_us() {
    let (tx, _rx) = mpsc::channel(16);
    let config = MstpConfig {
        this_station: 3,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();

    let token = MstpFrame {
        frame_type: FrameType::Token,
        destination: 3,
        source: 0,
        data: Bytes::new(),
    };

    let response = node.handle_received_frame(&token, &tx);
    assert!(response.is_none()); // Token doesn't generate a response frame
    assert!(node.reply_rx.is_none());
    assert_eq!(node.state, MasterState::UseToken);
}

#[test]
fn handle_token_not_for_us() {
    let (tx, _rx) = mpsc::channel(16);
    let config = MstpConfig {
        this_station: 3,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();

    let token = MstpFrame {
        frame_type: FrameType::Token,
        destination: 5,
        source: 0,
        data: Bytes::new(),
    };

    let response = node.handle_received_frame(&token, &tx);
    assert!(response.is_none());
    assert!(node.reply_rx.is_none());
    assert_eq!(node.state, MasterState::Idle);
}

#[test]
fn handle_poll_for_master_for_us() {
    let (tx, _rx) = mpsc::channel(16);
    let config = MstpConfig {
        this_station: 10,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();

    let pfm = MstpFrame {
        frame_type: FrameType::PollForMaster,
        destination: 10,
        source: 0,
        data: Bytes::new(),
    };

    let response = node.handle_received_frame(&pfm, &tx);
    assert!(node.reply_rx.is_none());
    assert!(response.is_some());
    let reply = response.unwrap();
    assert_eq!(reply.frame_type, FrameType::ReplyToPollForMaster);
    assert_eq!(reply.destination, 0);
    assert_eq!(reply.source, 10);
}

#[test]
fn handle_data_not_expecting_reply_unicast() {
    let (tx, mut rx) = mpsc::channel(16);
    let config = MstpConfig {
        this_station: 3,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();

    let npdu_data = vec![0x01, 0x00, 0x10];
    let frame = MstpFrame {
        frame_type: FrameType::BACnetDataNotExpectingReply,
        destination: 3,
        source: 0,
        data: Bytes::from(npdu_data.clone()),
    };

    let response = node.handle_received_frame(&frame, &tx);
    assert!(response.is_none());
    assert!(node.reply_rx.is_none());

    let received = rx.try_recv().unwrap();
    assert_eq!(received.npdu, npdu_data);
    assert_eq!(received.source_mac.as_slice(), &[0u8]);
    assert!(!received.link_layer_group);
}

#[test]
fn handle_data_not_expecting_reply_broadcast() {
    let (tx, mut rx) = mpsc::channel(16);
    let config = MstpConfig {
        this_station: 3,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();

    let frame = MstpFrame {
        frame_type: FrameType::BACnetDataNotExpectingReply,
        destination: BROADCAST_MAC,
        source: 5,
        data: Bytes::from_static(&[0x01, 0x20]),
    };

    let _response = node.handle_received_frame(&frame, &tx);
    assert!(node.reply_rx.is_none());
    let received = rx.try_recv().unwrap();
    assert_eq!(received.source_mac.as_slice(), &[5u8]);
    assert!(received.link_layer_group);
}

#[test]
fn handle_data_expecting_reply() {
    let (tx, mut rx) = mpsc::channel(16);
    let config = MstpConfig {
        this_station: 3,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();

    let frame = MstpFrame {
        frame_type: FrameType::BACnetDataExpectingReply,
        destination: 3,
        source: 0,
        data: Bytes::from_static(&[0x01, 0x00, 0x30]),
    };

    let response = node.handle_received_frame(&frame, &tx);
    assert!(response.is_none());
    assert!(node.reply_rx.is_some()); // Reply channel provided
    assert_eq!(node.state, MasterState::AnswerDataRequest);
    assert!(node.reply_rx.is_some()); // Sender stored in node

    let received = rx.try_recv().unwrap();
    assert_eq!(received.npdu, vec![0x01, 0x00, 0x30]);
    assert!(!received.link_layer_group);
}

#[test]
fn handle_test_request() {
    let (tx, _rx) = mpsc::channel(16);
    let config = MstpConfig {
        this_station: 3,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();

    let frame = MstpFrame {
        frame_type: FrameType::TestRequest,
        destination: 3,
        source: 0,
        data: Bytes::from_static(&[0xDE, 0xAD]),
    };

    let response = node.handle_received_frame(&frame, &tx);
    assert!(node.reply_rx.is_none());
    assert!(response.is_some());
    let reply = response.unwrap();
    assert_eq!(reply.frame_type, FrameType::TestResponse);
    assert_eq!(reply.data, vec![0xDE, 0xAD]);
}

#[test]
fn use_token_passes_when_no_data() {
    let config = MstpConfig {
        this_station: 0,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    node.state = MasterState::UseToken;
    // Simulate having discovered a successor and recently polled
    node.next_station = 1;
    node.token_count = 0;

    let frame = node.use_token();
    assert_eq!(frame.frame_type, FrameType::Token);
    assert_eq!(frame.destination, 1);
    assert_eq!(node.state, MasterState::PassToken);
}

#[test]
fn use_token_sends_queued_data() {
    let config = MstpConfig {
        this_station: 0,
        max_master: 127,
        max_info_frames: 2,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    node.state = MasterState::UseToken;
    node.next_station = 1; // known successor so DoneWithToken can SendToken
    node.poll_station = 0;
    node.token_count = 0;
    node.queue_npdu(5, Bytes::from_static(&[0x01, 0x00, 0x30]))
        .unwrap();
    node.queue_npdu(BROADCAST_MAC, Bytes::from_static(&[0x01, 0x20]))
        .unwrap();

    // First call sends the first queued frame (FIFO — unicast to 5 dequeued first)
    let frame = node.use_token();
    assert_eq!(frame.frame_type, FrameType::BACnetDataNotExpectingReply);
    assert_eq!(frame.destination, 5);
    assert_eq!(frame.data, vec![0x01, 0x00, 0x30]);

    // Second call sends the broadcast (FIFO order)
    let frame = node.use_token();
    assert_eq!(frame.frame_type, FrameType::BACnetDataNotExpectingReply);
    assert_eq!(frame.destination, BROADCAST_MAC);

    // Third call: no more data, pass token to known NS
    let frame = node.use_token();
    assert_eq!(frame.frame_type, FrameType::Token);
    assert_eq!(frame.destination, 1);
}
#[test]
fn use_token_respects_max_info_frames() {
    let config = MstpConfig {
        this_station: 0,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    node.state = MasterState::UseToken;
    node.next_station = 1;
    node.poll_station = 0;
    node.token_count = 0;
    node.queue_npdu(5, Bytes::from_static(&[0x01])).unwrap();
    node.queue_npdu(6, Bytes::from_static(&[0x02])).unwrap();

    // First call: sends one frame
    let frame = node.use_token();
    assert!(
        frame.frame_type == FrameType::BACnetDataExpectingReply
            || frame.frame_type == FrameType::BACnetDataNotExpectingReply
    );

    // Second call: frame_count >= max_info_frames → DoneWithToken → Token to NS
    let frame = node.use_token();
    assert_eq!(frame.frame_type, FrameType::Token);
    assert_eq!(frame.destination, 1);

    // Data should still be in queue
    assert_eq!(node.tx_queue.len(), 1);
}
#[test]
fn poll_for_master_after_npoll_tokens() {
    // NS unknown (NS==TS): DONE_WITH_TOKEN → NextStationUnknown → PFM to TS+1
    let config = MstpConfig {
        this_station: 0,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    node.state = MasterState::DoneWithToken;
    node.token_count = NPOLL;
    node.frame_count = node.config.max_info_frames;

    let frame = node.done_with_token();
    assert_eq!(frame.frame_type, FrameType::PollForMaster);
    assert_eq!(frame.destination, 1);
    assert_eq!(node.state, MasterState::PollForMaster);
    assert_eq!(node.poll_station, 1);
}

#[test]
fn reply_to_poll_sets_next_station() {
    let (tx, _rx) = mpsc::channel(16);
    let config = MstpConfig {
        this_station: 0,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    node.state = MasterState::PollForMaster;

    let reply = MstpFrame {
        frame_type: FrameType::ReplyToPollForMaster,
        destination: 0,
        source: 42,
        data: Bytes::new(),
    };

    let response = node.handle_received_frame(&reply, &tx);
    assert_eq!(node.next_station, 42);
    assert_eq!(node.state, MasterState::PassToken);
    assert!(!node.sole_master);
    // Should return a Token frame to send to the new NS
    assert!(response.is_some());
    let token = response.unwrap();
    assert_eq!(token.frame_type, FrameType::Token);
    assert_eq!(token.destination, 42);
}

#[test]
fn poll_timeout_advances_poll_station() {
    let config = MstpConfig {
        this_station: 0,
        max_master: 3,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    node.state = MasterState::PollForMaster;
    node.next_station = 0; // unknown successor — keep scanning
    node.poll_station = 1;

    let frame = node.poll_timeout();
    assert_eq!(frame.frame_type, FrameType::PollForMaster);
    assert_eq!(node.poll_station, 2);
    assert_eq!(frame.destination, 2);
}

#[test]
fn poll_timeout_sole_master() {
    let config = MstpConfig {
        this_station: 0,
        max_master: 1,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    node.state = MasterState::PollForMaster;
    node.next_station = 0;
    node.poll_station = 1;

    // Timeout for station 1 → next_ps wraps to TS → DeclareSoleMaster (no Token 0→0)
    let frame = node.poll_timeout();
    assert!(node.sole_master);
    assert_eq!(node.state, MasterState::PollForMaster);
    // Sole master restart / maintenance emits PFM, never Token TS→TS
    assert_eq!(frame.frame_type, FrameType::PollForMaster);
    assert_ne!(frame.destination, frame.source);
}

#[test]
fn mstp_max_apdu_length() {
    let (s1, _s2) = LoopbackSerial::pair();
    let transport = MstpTransport::new(s1, MstpConfig::default());
    assert_eq!(transport.max_apdu_length(), 480);
}

#[test]
fn local_mac_is_one_byte() {
    let (serial_a, _serial_b) = LoopbackSerial::pair();
    let config = MstpConfig {
        this_station: 42,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let transport = MstpTransport::new(serial_a, config);
    assert_eq!(transport.local_mac(), &[42]);
}

#[tokio::test]
async fn loopback_serial_pair() {
    let (a, b) = LoopbackSerial::pair();

    // Write from A, read from B
    a.write(&[0x55, 0xFF, 0x01]).await.unwrap();
    let mut buf = [0u8; 16];
    let n = b.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], &[0x55, 0xFF, 0x01]);

    // Write from B, read from A
    b.write(&[0xAA, 0xBB]).await.unwrap();
    let n = a.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], &[0xAA, 0xBB]);
}

#[tokio::test]
async fn transport_start_stop() {
    let (serial_a, _serial_b) = LoopbackSerial::pair();
    let config = MstpConfig {
        this_station: 0,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut transport = MstpTransport::new(serial_a, config);
    let _rx = transport.start().await.unwrap();
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn transport_queue_broadcast() {
    let (serial_a, _serial_b) = LoopbackSerial::pair();
    let config = MstpConfig {
        this_station: 0,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut transport = MstpTransport::new(serial_a, config);
    let _rx = transport.start().await.unwrap();

    transport.send_broadcast(&[0x01, 0x20]).await.unwrap();

    {
        let node = transport.node_state().unwrap();
        let node = node.lock().await;
        assert_eq!(node.tx_queue.len(), 1);
        assert_eq!(node.tx_queue[0].0, BROADCAST_MAC);
    }

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn transport_queue_unicast() {
    let (serial_a, _serial_b) = LoopbackSerial::pair();
    let config = MstpConfig {
        this_station: 0,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut transport = MstpTransport::new(serial_a, config);
    let _rx = transport.start().await.unwrap();

    transport.send_unicast(&[0x01, 0x00], &[5]).await.unwrap();

    {
        let node = transport.node_state().unwrap();
        let node = node.lock().await;
        assert_eq!(node.tx_queue.len(), 1);
        assert_eq!(node.tx_queue[0].0, 5);
    }

    transport.stop().await.unwrap();
}

#[test]
fn use_token_frame_type_from_npdu_expecting_reply() {
    let mut node = MasterNode::new(MstpConfig {
        this_station: 1,
        max_master: 127,
        max_info_frames: 5,
        baud_rate: 9600,
    })
    .unwrap();
    node.state = MasterState::UseToken;

    // NPDU with expecting_reply=true (byte 1 bit 2 set)
    node.queue_npdu(5, Bytes::from_static(&[0x01, 0x04, 0xAA]))
        .unwrap();
    let frame = node.use_token();
    assert_eq!(frame.frame_type, FrameType::BACnetDataExpectingReply);

    // NPDU with expecting_reply=false (byte 1 bit 2 clear)
    node.state = MasterState::UseToken;
    node.queue_npdu(5, Bytes::from_static(&[0x01, 0x00, 0xBB]))
        .unwrap();
    let frame = node.use_token();
    assert_eq!(frame.frame_type, FrameType::BACnetDataNotExpectingReply);
}

#[test]
fn use_token_broadcast_always_not_expecting() {
    let mut node = MasterNode::new(MstpConfig {
        this_station: 1,
        max_master: 127,
        max_info_frames: 5,
        baud_rate: 9600,
    })
    .unwrap();
    node.state = MasterState::UseToken;
    // Even with expecting_reply set, broadcast uses NotExpectingReply
    node.queue_npdu(BROADCAST_MAC, Bytes::from_static(&[0x01, 0x04, 0xAA]))
        .unwrap();
    let frame = node.use_token();
    assert_eq!(frame.frame_type, FrameType::BACnetDataNotExpectingReply);
}

#[tokio::test]
async fn transport_rejects_bad_mac() {
    let (serial_a, _serial_b) = LoopbackSerial::pair();
    let config = MstpConfig {
        this_station: 0,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut transport = MstpTransport::new(serial_a, config);
    let _rx = transport.start().await.unwrap();

    // 6-byte MAC is invalid for MS/TP
    let result = transport.send_unicast(&[0x01], &[1, 2, 3, 4, 5, 6]).await;
    assert!(result.is_err());

    transport.stop().await.unwrap();
}

// -------------------------------------------------------------------
// New tests for NoToken, WaitForReply, AnswerDataRequest, scan range
// -------------------------------------------------------------------

#[test]
fn test_no_token_timeout_claims_token() {
    // Sole master must not emit Token TS→TS. After DeclareSoleMaster,
    // DONE_WITH_TOKEN reuses / restarts maintenance via PFM.
    let config = MstpConfig {
        this_station: 5,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    node.sole_master = true;
    node.next_station = 5;
    node.poll_station = 5;
    node.token_count = 0;
    node.state = MasterState::DoneWithToken;
    node.frame_count = node.config.max_info_frames;

    let frame = node.done_with_token();
    assert_ne!(
        (frame.frame_type, frame.destination),
        (FrameType::Token, 5),
        "forbidden self-token"
    );
    assert!(
        frame.frame_type == FrameType::PollForMaster || node.state == MasterState::UseToken,
        "sole master continues without Token TS→TS"
    );
}

#[test]
fn test_wait_for_reply_state_after_data_expecting_reply() {
    // When we send a DataExpectingReply via use_token, the recv loop
    // should set state to WaitForReply. We simulate that here.
    let config = MstpConfig {
        this_station: 1,
        max_master: 127,
        max_info_frames: 5,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    node.state = MasterState::UseToken;

    // Queue an NPDU with expecting_reply bit set
    node.queue_npdu(5, Bytes::from_static(&[0x01, 0x04, 0xAA]))
        .unwrap();
    let frame = node.use_token();
    assert_eq!(frame.frame_type, FrameType::BACnetDataExpectingReply);

    // The recv loop would now set WaitForReply — simulate that
    node.state = MasterState::WaitForReply;
    assert_eq!(node.state, MasterState::WaitForReply);

    // On timeout in WaitForReply, DONE_WITH_TOKEN (not unconditional pass_token)
    node.next_station = 5;
    node.poll_station = 1;
    node.token_count = 0;
    node.frame_count = node.config.max_info_frames;
    node.state = MasterState::DoneWithToken;
    let token = node.done_with_token();
    assert_eq!(token.frame_type, FrameType::Token);
    assert_eq!(token.destination, 5);
    assert_eq!(node.state, MasterState::PassToken);
}
#[test]
fn test_answer_data_request_reply_channel() {
    let (tx, mut rx) = mpsc::channel(16);
    let config = MstpConfig {
        this_station: 3,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();

    let frame = MstpFrame {
        frame_type: FrameType::BACnetDataExpectingReply,
        destination: 3,
        source: 7,
        data: Bytes::from_static(&[0x01, 0x04, 0x10]),
    };

    let response = node.handle_received_frame(&frame, &tx);
    assert!(response.is_none());
    assert_eq!(node.state, MasterState::AnswerDataRequest);

    // reply_rx stored on node, reply_tx sent via ReceivedNpdu channel
    assert!(node.reply_rx.is_some());

    // Receive the NPDU from the channel and extract reply_tx
    let received_npdu = rx.try_recv().expect("should receive NPDU");
    let reply_tx = received_npdu.reply_tx.expect("should have reply_tx");

    // Simulate application sending a reply through the channel
    let reply_data = vec![0x01, 0x00, 0x30, 0x01];
    reply_tx.send(Bytes::from(reply_data.clone())).unwrap();

    // The node's reply_rx should get the data
    let mut node_rx = node.reply_rx.take().unwrap();
    let received = node_rx.try_recv().unwrap();
    assert_eq!(received, reply_data);
}

#[test]
fn test_poll_for_master_scan_range() {
    // TS=0, NS=5, Max_Master=10 — maintenance candidates are 1..=4 only
    // (advance PS from TS toward NS; never begin at NS+1).
    let config = MstpConfig {
        this_station: 0,
        max_master: 10,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    node.next_station = 5;
    node.poll_station = 0;
    node.state = MasterState::DoneWithToken;
    node.token_count = NPOLL.saturating_sub(1);
    node.frame_count = node.config.max_info_frames;

    let frame = node.done_with_token();
    assert_eq!(frame.frame_type, FrameType::PollForMaster);
    assert_eq!(node.poll_station, 1);
    assert_eq!(frame.destination, 1);

    // One timeout → Token to known NS (not whole-space scan in one token use)
    let token = node.poll_timeout();
    assert_eq!(token.frame_type, FrameType::Token);
    assert_eq!(token.destination, 5);
}

#[test]
fn test_poll_for_master_scan_range_adjacent() {
    // NS == TS+1: ResetMaintenancePFM / SendToken — no gap to poll
    let config = MstpConfig {
        this_station: 0,
        max_master: 1,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    node.next_station = 1;
    node.poll_station = 0;
    node.state = MasterState::DoneWithToken;
    node.token_count = NPOLL.saturating_sub(1);
    node.frame_count = node.config.max_info_frames;

    let frame = node.done_with_token();
    assert_eq!(frame.frame_type, FrameType::Token);
    assert_eq!(frame.destination, 1);
    assert_eq!(node.poll_station, 0);
}

#[test]
fn mstp_frame_buf_max_size() {
    // Standard frame: 2 (preamble) + 6 (header) + 501 (data) + 2 (CRC16).
    assert_eq!(MSTP_MAX_FRAME_BUF, 511);
}

#[test]
fn master_node_rejects_station_above_max_master() {
    let config = MstpConfig {
        this_station: 128, // above MAX_MASTER (127)
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    assert!(MasterNode::new(config).is_err());
}

#[test]
fn master_node_accepts_max_master_station() {
    let config = MstpConfig {
        this_station: 127, // exactly MAX_MASTER
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    assert!(MasterNode::new(config).is_ok());
}

#[test]
fn pending_reply_source_stored_on_data_expecting_reply() {
    let (tx, _rx) = mpsc::channel(16);
    let config = MstpConfig {
        this_station: 3,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();

    let frame = MstpFrame {
        frame_type: FrameType::BACnetDataExpectingReply,
        destination: 3,
        source: 7,
        data: Bytes::from_static(&[0x01, 0x04, 0x10]),
    };

    let _ = node.handle_received_frame(&frame, &tx);
    assert_eq!(node.state, MasterState::AnswerDataRequest);
    assert_eq!(node.pending_reply_source, Some(7));
}

#[test]
fn t_turnaround_rounds_up_to_full_bit_time() {
    assert_eq!(calculate_t_turnaround_us(9600), 4167);
    assert_eq!(calculate_t_turnaround_us(38_400), 1042);
}

#[test]
fn t_slot_baud_rate_9600() {
    assert_eq!(calculate_t_slot_ms(9600), 10);
}

#[test]
fn t_slot_baud_rate_38400() {
    assert_eq!(calculate_t_slot_ms(38400), 10);
}

#[test]
fn t_slot_baud_rate_76800() {
    assert_eq!(calculate_t_slot_ms(76800), 10);
}

#[test]
fn t_slot_baud_rate_115200() {
    assert_eq!(calculate_t_slot_ms(115200), 10);
}

#[test]
fn t_slot_stored_on_master_node() {
    let config = MstpConfig {
        this_station: 0,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 38400,
    };
    let node = MasterNode::new(config).unwrap();
    assert_eq!(node.t_slot_ms, 10);
}

/// #360: X'FF' is the MS/TP broadcast spelling; anything else is a station.
#[test]
fn is_broadcast_mac_is_xff() {
    let (serial_a, _serial_b) = LoopbackSerial::pair();
    let config = MstpConfig {
        this_station: 42,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let transport = MstpTransport::new(serial_a, config);
    assert!(transport.is_broadcast_mac(&[0xFF]));
    assert!(!transport.is_broadcast_mac(&[42]));
    assert!(!transport.is_broadcast_mac(&[0xFF, 0xFF]));
}
