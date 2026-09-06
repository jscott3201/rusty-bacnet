//! Clause 9.5.6 regression suite (token / PFM / coexistence).
//!
//! These tests replace the reversed-scan expectations (PFM beginning at NS+1)
//! that allowed the invalid ring 0→3→0 and excluded FEC MAC 7.

use super::*;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use crate::mstp_frame::{encode_frame, MAX_STANDARD_MPDU_DATA};
use crate::port::TransportPort;

fn cfg(ts: u8, max_master: u8) -> MstpConfig {
    MstpConfig {
        this_station: ts,
        max_master,
        max_info_frames: 1,
        baud_rate: 38400,
    }
}

fn deliver_token(node: &mut MasterNode, from: u8) {
    let (tx, _rx) = mpsc::channel(4);
    let frame = MstpFrame {
        frame_type: FrameType::Token,
        destination: node.config.this_station,
        source: from,
        data: Bytes::new(),
    };
    let _ = node.handle_received_frame(&frame, &tx);
}

/// A. Unknown successor with queued Who-Is — no Token 3→3; PFM to 4.
#[test]
fn a_unknown_successor_with_queued_who_is() {
    let mut node = MasterNode::new(cfg(3, 7)).unwrap();
    assert_eq!(node.next_station, 3);
    assert_eq!(node.poll_station, 3);
    assert_eq!(node.token_count, NPOLL);
    assert!(!node.sole_master);

    // Queued application frame (Who-Is-style NPDU)
    node.queue_npdu(BROADCAST_MAC, Bytes::from_static(&[0x01, 0x20, 0xFF, 0xFF]))
        .unwrap();

    deliver_token(&mut node, 0);
    assert_eq!(node.state, MasterState::UseToken);

    let app = node.use_token();
    assert_eq!(app.frame_type, FrameType::BACnetDataNotExpectingReply);
    assert_eq!(node.state, MasterState::DoneWithToken);

    let next = node.done_with_token();
    assert_eq!(next.frame_type, FrameType::PollForMaster);
    assert_eq!(next.source, 3);
    assert_eq!(next.destination, 4);
    assert_ne!(
        (next.frame_type, next.destination),
        (FrameType::Token, 3),
        "forbidden Token 3→3"
    );
}

/// B. Established three-master ring 0→3→7→0 for ≥2000 rotations.
#[test]
fn b_three_master_ring_2000_rotations() {
    let mut nodes = [
        MasterNode::new(cfg(0, 7)).unwrap(),
        MasterNode::new(cfg(3, 7)).unwrap(),
        MasterNode::new(cfg(7, 7)).unwrap(),
    ];
    // Established successors
    nodes[0].next_station = 3;
    nodes[0].poll_station = 0;
    nodes[0].token_count = 0;
    nodes[1].next_station = 7;
    nodes[1].poll_station = 3;
    nodes[1].token_count = 0;
    nodes[2].next_station = 0;
    nodes[2].poll_station = 7;
    nodes[2].token_count = 0;

    let macs = [0u8, 3, 7];
    let mut idx = 0usize; // token holder index into macs
    let mut receipts = [0u32; 3];
    let mut forbidden_self_token = 0u32;

    for _ in 0..2000 {
        let holder = &mut nodes[idx];
        deliver_token(holder, macs[(idx + 2) % 3]);
        receipts[idx] += 1;

        // Drain this token use (app empty → done_with_token may PFM then timeout)
        let mut guard = 0;
        loop {
            guard += 1;
            assert!(guard < 64, "token-use did not terminate");

            let frame = if holder.state == MasterState::DoneWithToken {
                holder.done_with_token()
            } else if holder.state == MasterState::UseToken {
                holder.use_token()
            } else if holder.state == MasterState::PollForMaster {
                // Maintenance PFM no reply — DoneWithPFM passes to known NS
                holder.poll_timeout()
            } else {
                break;
            };

            if frame.frame_type == FrameType::Token {
                if frame.source == frame.destination {
                    forbidden_self_token += 1;
                }
                assert_eq!(frame.destination, holder.next_station);
                // Advance ring to destination
                idx = macs
                    .iter()
                    .position(|&m| m == frame.destination)
                    .expect("token dest must be a ring member");
                break;
            }
            // PFM: stay on holder until poll_timeout produces Token
        }
    }

    assert_eq!(forbidden_self_token, 0, "no Token source==destination");
    assert!(
        receipts.iter().all(|&n| n > 0),
        "every master must receive token: {receipts:?}"
    );
    // Rough fairness: each master should see a large share of 2000 holds
    for (i, &n) in receipts.iter().enumerate() {
        assert!(
            n > 400,
            "master {} starved (receipts={receipts:?})",
            macs[i]
        );
    }
}

/// C. Maintenance polls only addresses in (TS, NS) — never PFM 3→0 / Token 3→0 / Token 3→3.
#[test]
fn c_maintenance_polling_gap_only() {
    let mut node = MasterNode::new(cfg(3, 7)).unwrap();
    node.next_station = 7;
    node.poll_station = 3;
    node.token_count = NPOLL.saturating_sub(1); // force maintenance
    node.state = MasterState::DoneWithToken;

    let expected = [4u8, 5, 6];
    for &ps in &expected {
        let pfm = node.done_with_token();
        assert_eq!(pfm.frame_type, FrameType::PollForMaster);
        assert_eq!(pfm.source, 3);
        assert_eq!(pfm.destination, ps);
        assert_ne!(pfm.destination, 0, "forbidden PFM 3→0");

        let token = node.poll_timeout();
        assert_eq!(token.frame_type, FrameType::Token);
        assert_eq!(token.destination, 7);
        assert_ne!(token.destination, 0, "forbidden Token 3→0 while NS=7");
        assert_ne!(token.destination, 3, "forbidden Token 3→3");

        // Next maintenance opportunity
        node.token_count = NPOLL.saturating_sub(1);
        node.state = MasterState::DoneWithToken;
        node.frame_count = node.config.max_info_frames;
    }

    // Fourth opportunity: next_ps == NS → ResetMaintenancePFM
    let token = node.done_with_token();
    assert_eq!(token.frame_type, FrameType::Token);
    assert_eq!(token.destination, 7);
    assert_eq!(node.poll_station, 3);
    assert_eq!(node.token_count, 1);
}

/// D. New master in gap (MAC 5) joins via ReplyToPFM.
#[test]
fn d_new_master_in_gap() {
    let (tx, _rx) = mpsc::channel(4);
    let mut node = MasterNode::new(cfg(3, 7)).unwrap();
    node.next_station = 7;
    node.poll_station = 3;
    node.token_count = NPOLL.saturating_sub(1);
    node.state = MasterState::DoneWithToken;

    // Skip 4 (timeout), then poll 5
    let pfm4 = node.done_with_token();
    assert_eq!(pfm4.destination, 4);
    let _ = node.poll_timeout();
    node.token_count = NPOLL.saturating_sub(1);
    node.state = MasterState::DoneWithToken;
    node.frame_count = node.config.max_info_frames;

    let pfm5 = node.done_with_token();
    assert_eq!(pfm5.frame_type, FrameType::PollForMaster);
    assert_eq!(pfm5.destination, 5);

    let reply = MstpFrame {
        frame_type: FrameType::ReplyToPollForMaster,
        destination: 3,
        source: 5,
        data: Bytes::new(),
    };
    let out = node
        .handle_received_frame(&reply, &tx)
        .expect("Token to new NS");
    assert_eq!(node.next_station, 5);
    assert_eq!(node.poll_station, 3);
    assert_eq!(node.token_count, 0);
    assert_eq!(out.frame_type, FrameType::Token);
    assert_eq!(out.destination, 5);

    // Eventual ring includes 3→5; 5 would point at 7, 7→0, 0→3 (sim check NS only)
    assert_eq!(node.next_station, 5);
}

/// E. Failed successor: Nretry_token Token retries then PFM search after failed NS.
#[test]
fn e_failed_successor_uses_pfm_not_blind_tokens() {
    let mut node = MasterNode::new(cfg(3, 7)).unwrap();
    node.next_station = 7;
    node.poll_station = 3;
    node.token_count = 1;
    let first = node.pass_token();
    assert_eq!(first.frame_type, FrameType::Token);
    assert_eq!(first.destination, 7);

    // Retry exactly Nretry_token times
    let retry = node.pass_token_timeout().expect("retry Token");
    assert_eq!(retry.frame_type, FrameType::Token);
    assert_eq!(retry.destination, 7);
    assert_eq!(node.retry_token_count, N_RETRY_TOKEN);

    // Then FindNewSuccessor: PS = NS+1 = 0? next_addr(7,7)=0; NS=TS=3; PFM to 0
    // Wait — Max_Master=7, next_addr(7)=0. User: "PFM search beginning after failed NS"
    let find = node.pass_token_timeout().expect("PFM after retries");
    assert_eq!(find.frame_type, FrameType::PollForMaster);
    assert_eq!(find.destination, 0); // NS+1 after failed 7 with max_master=7
    assert_eq!(node.next_station, 3); // NS = TS
    assert_ne!(find.frame_type, FrameType::Token);

    // The node is now in PFM, and a PFM timeout advances the search rather
    // than invoking another token retry against an unverified address.
    assert_eq!(node.state, MasterState::PollForMaster);
    assert_eq!(node.poll_station, 0);
    let next_poll = node.poll_timeout();
    assert_eq!(next_poll.frame_type, FrameType::PollForMaster);
    assert_eq!(next_poll.source, 3);
    assert_eq!(next_poll.destination, 1);
    assert_eq!(node.state, MasterState::PollForMaster);
}

#[test]
fn master_node_rejects_max_master_above_standard_limit() {
    let error = MasterNode::new(cfg(3, MAX_MASTER + 1))
        .err()
        .expect("invalid max_master");
    assert!(error
        .to_string()
        .contains("max_master 128 exceeds MAX_MASTER (127)"));
}

#[test]
fn master_node_rejects_station_above_configured_max_master() {
    let error = MasterNode::new(cfg(5, 4))
        .err()
        .expect("invalid this_station");
    assert!(error
        .to_string()
        .contains("this_station 5 exceeds configured max_master (4)"));
}

#[test]
fn pass_token_self_destination_enters_pfm_at_runtime() {
    let mut node = MasterNode::new(cfg(3, 7)).unwrap();
    node.next_station = node.config.this_station;

    let frame = node.pass_token();
    assert_eq!(frame.frame_type, FrameType::PollForMaster);
    assert_eq!(frame.source, 3);
    assert_eq!(frame.destination, 4);
    assert_eq!(node.state, MasterState::PollForMaster);

    node.state = MasterState::PassToken;
    node.next_station = node.config.this_station;
    let retry = node.pass_token_timeout().expect("PFM recovery frame");
    assert_eq!(retry.frame_type, FrameType::PollForMaster);
    assert_eq!(retry.destination, 4);
    assert_ne!(retry.frame_type, FrameType::Token);
}

fn pending_data_request_node() -> MasterNode {
    let (tx, _rx) = mpsc::channel(4);
    let mut node = MasterNode::new(cfg(3, 127)).unwrap();
    let request = MstpFrame {
        frame_type: FrameType::BACnetDataExpectingReply,
        destination: 3,
        source: 7,
        data: Bytes::from_static(&[0x01, 0x04, 0x10]),
    };
    assert!(node.handle_received_frame(&request, &tx).is_none());
    assert_eq!(node.state, MasterState::AnswerDataRequest);
    node
}

#[test]
fn outbound_queue_rejects_oversize_before_mutation() {
    let mut node = MasterNode::new(cfg(3, 127)).unwrap();
    let accepted = Bytes::from(vec![0x11; MAX_STANDARD_MPDU_DATA]);
    node.queue_npdu(7, accepted.clone()).unwrap();

    let state_before = node.state;
    let frame_count_before = node.frame_count;
    let error = node
        .queue_npdu(8, Bytes::from(vec![0x22; MAX_STANDARD_MPDU_DATA + 1]))
        .expect_err("502-byte NPDU must be rejected");
    assert!(error
        .to_string()
        .contains("MS/TP NPDU length 502 exceeds standard-frame maximum 501"));
    assert_eq!(node.tx_queue.len(), 1);
    assert_eq!(node.tx_queue.front(), Some(&(7, accepted)));
    assert_eq!(node.state, state_before);
    assert_eq!(node.frame_count, frame_count_before);

    node.state = MasterState::UseToken;
    let frame = node.use_token();
    assert_eq!(frame.data.len(), MAX_STANDARD_MPDU_DATA);
    let mut wire = BytesMut::new();
    encode_frame(&mut wire, &frame).expect("accepted queued NPDU remains encodable");
    assert!(node.tx_queue.is_empty());
}

#[test]
fn oversized_application_reply_preserves_state_until_explicit_abandon() {
    let mut node = pending_data_request_node();
    assert!(node.reply_rx.is_some());

    let error = node
        .finish_data_request(Some(Bytes::from(vec![0x33; MAX_STANDARD_MPDU_DATA + 1])))
        .expect_err("502-byte application reply must be rejected");
    assert!(error
        .to_string()
        .contains("MS/TP application reply length 502 exceeds standard-frame maximum 501"));
    assert_eq!(node.state, MasterState::AnswerDataRequest);
    assert_eq!(node.pending_reply_source, Some(7));
    assert!(node.reply_rx.is_some());

    node.abandon_data_request();
    assert_eq!(node.state, MasterState::Idle);
    assert!(node.pending_reply_source.is_none());
    assert!(node.reply_rx.is_none());
}

#[test]
fn max_standard_application_reply_remains_encodable() {
    let mut node = pending_data_request_node();
    let reply = node
        .finish_data_request(Some(Bytes::from(vec![0x44; MAX_STANDARD_MPDU_DATA])))
        .unwrap();

    assert_eq!(reply.frame_type, FrameType::BACnetDataNotExpectingReply);
    assert_eq!(reply.destination, 7);
    assert_eq!(reply.data.len(), MAX_STANDARD_MPDU_DATA);
    assert_eq!(node.state, MasterState::Idle);
    let mut wire = BytesMut::new();
    encode_frame(&mut wire, &reply).unwrap();
}

#[tokio::test(start_paused = true)]
async fn transport_abandons_oversized_reply_without_wire_output() {
    let (serial_transport, serial_peer) = LoopbackSerial::pair();
    let mut transport = MstpTransport::new(serial_transport, cfg(3, 127));
    let mut npdu_rx = transport.start().await.unwrap();
    let request = MstpFrame {
        frame_type: FrameType::BACnetDataExpectingReply,
        destination: 3,
        source: 7,
        data: Bytes::from_static(&[0x01, 0x04, 0x10]),
    };
    let mut wire = BytesMut::new();
    encode_frame(&mut wire, &request).unwrap();
    serial_peer.write(&wire).await.unwrap();

    let received = npdu_rx.recv().await.expect("application request");
    received
        .reply_tx
        .expect("reply sender")
        .send(Bytes::from(vec![0x55; MAX_STANDARD_MPDU_DATA + 1]))
        .unwrap();
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }

    {
        let node = transport.node_state().unwrap().lock().await;
        assert_eq!(node.state, MasterState::Idle);
        assert!(node.pending_reply_source.is_none());
        assert!(node.reply_rx.is_none());
    }
    let mut response = [0u8; MSTP_MAX_FRAME_BUF];
    let read = tokio::time::timeout(
        tokio::time::Duration::from_millis(1),
        serial_peer.read(&mut response),
    )
    .await;
    assert!(read.is_err(), "oversized reply produced wire output");
    transport.stop().await.unwrap();
}
