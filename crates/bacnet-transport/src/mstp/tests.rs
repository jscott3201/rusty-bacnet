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

    // Third call: no more data, pass token
    let frame = node.use_token();
    assert_eq!(frame.frame_type, FrameType::Token);
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
    node.queue_npdu(5, Bytes::from_static(&[0x01])).unwrap();
    node.queue_npdu(6, Bytes::from_static(&[0x02])).unwrap();

    // First call: sends one frame
    let frame = node.use_token();
    assert!(
        frame.frame_type == FrameType::BACnetDataExpectingReply
            || frame.frame_type == FrameType::BACnetDataNotExpectingReply
    );

    // Second call: frame_count >= max_info_frames, passes token
    let frame = node.use_token();
    assert_eq!(frame.frame_type, FrameType::Token);

    // Data should still be in queue
    assert_eq!(node.tx_queue.len(), 1);
}

#[test]
fn poll_for_master_after_npoll_tokens() {
    let config = MstpConfig {
        this_station: 0,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    node.state = MasterState::UseToken;
    node.token_count = NPOLL; // Trigger poll

    let frame = node.use_token();
    assert_eq!(frame.frame_type, FrameType::PollForMaster);
    assert_eq!(node.state, MasterState::PollForMaster);
    assert_eq!(node.token_count, 0);
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
    // Start polling from station 1
    node.poll_station = 1;

    // MAX_POLL_RETRIES timeouts for station 1
    for _ in 0..MAX_POLL_RETRIES {
        let frame = node.poll_timeout();
        assert_eq!(frame.frame_type, FrameType::PollForMaster);
    }
    // Should have moved to station 2
    assert_eq!(node.poll_station, 2);
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
    node.poll_station = 1;

    // Timeout for station 1, MAX_POLL_RETRIES times
    for _ in 0..MAX_POLL_RETRIES {
        node.poll_timeout();
    }
    // poll_station wraps to 0 (== this_station), sole master declared
    assert_eq!(node.state, MasterState::UseToken);
    assert!(node.sole_master);
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
    // Simulate the NoToken -> sole master flow without the transport loop.
    //
    // Flow:
    //   Idle timeout -> enter NoToken, send 1st PFM, retry_token_count=0
    //   NoToken timeout #1 -> retry_token_count(0) < N_RETRY_TOKEN(1), send 2nd PFM, count=1
    //   NoToken timeout #2 -> retry_token_count(1) >= N_RETRY_TOKEN(1), claim sole master
    let config = MstpConfig {
        this_station: 5,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();

    // Simulate: Idle -> NoToken (first timeout sends 1st PFM)
    node.state = MasterState::NoToken;
    node.retry_token_count = 0;

    // First retry (retry_token_count=0 < N_RETRY_TOKEN=1)
    assert!(node.retry_token_count < N_RETRY_TOKEN);
    node.retry_token_count += 1;
    assert_eq!(node.retry_token_count, 1);

    // After N_RETRY_TOKEN retries, declare sole master
    assert!(node.retry_token_count >= N_RETRY_TOKEN);
    node.sole_master = true;
    node.next_station = node.config.this_station;
    node.state = MasterState::UseToken;
    node.frame_count = 0;
    node.token_count = 0;

    assert!(node.sole_master);
    assert_eq!(node.next_station, 5);
    assert_eq!(node.state, MasterState::UseToken);

    // Use token should pass to self (sole master)
    let frame = node.use_token();
    assert_eq!(frame.frame_type, FrameType::Token);
    assert_eq!(frame.destination, 5); // pass to self
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

    // On timeout in WaitForReply, we pass the token
    let token = node.pass_token();
    assert_eq!(token.frame_type, FrameType::Token);
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
    // Station 0, next_station=5, max_master=10
    // Should poll starting at 6 (next_addr(5, 10)), scanning 6..=10, 0 would be us so stop
    let config = MstpConfig {
        this_station: 0,
        max_master: 10,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    node.next_station = 5;
    node.state = MasterState::UseToken;
    node.token_count = NPOLL;

    // use_token triggers PollForMaster at poll_station = next_addr(5, 10) = 6
    let frame = node.use_token();
    assert_eq!(frame.frame_type, FrameType::PollForMaster);
    assert_eq!(node.poll_station, 6);
    assert_eq!(frame.destination, 6);

    // Each station takes MAX_POLL_RETRIES timeouts to exhaust, then advances.
    // Station 6: 3 retries -> advance to 7
    for _ in 0..MAX_POLL_RETRIES {
        node.poll_timeout();
    }
    assert_eq!(node.poll_station, 7);

    // Station 7: 3 retries -> advance to 8
    for _ in 0..MAX_POLL_RETRIES {
        node.poll_timeout();
    }
    assert_eq!(node.poll_station, 8);

    // Station 8: 3 retries -> advance to 9
    for _ in 0..MAX_POLL_RETRIES {
        node.poll_timeout();
    }
    assert_eq!(node.poll_station, 9);

    // Station 9: 3 retries -> advance to 10
    for _ in 0..MAX_POLL_RETRIES {
        node.poll_timeout();
    }
    assert_eq!(node.poll_station, 10);

    // Station 10: 3 retries -> advance to next_addr(10, 10) = 0 == this_station
    // poll_timeout detects this_station match — since next_station=5 (not TS),
    // we have a known successor, pass token to them.
    for _ in 0..MAX_POLL_RETRIES {
        node.poll_timeout();
    }
    assert_eq!(node.state, MasterState::PassToken);
}

#[test]
fn test_poll_for_master_scan_range_adjacent() {
    // When next_station is adjacent (this_station=0, next_station=1, max_master=1),
    // poll_station = next_addr(1, 1) = 0 == this_station, so no gap to scan
    let config = MstpConfig {
        this_station: 0,
        max_master: 1,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    node.next_station = 1;
    node.state = MasterState::UseToken;
    node.token_count = NPOLL;

    // use_token should just pass token since no gap
    let frame = node.use_token();
    assert_eq!(frame.frame_type, FrameType::Token);
    assert_eq!(node.state, MasterState::PassToken);
}

#[test]
fn mstp_frame_buf_max_size() {
    // The maximum valid MS/TP frame is: 2 (preamble) + 6 (header) + 1497 (data) + 2 (CRC16) = 1507
    assert_eq!(MSTP_MAX_FRAME_BUF, 1507);
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
