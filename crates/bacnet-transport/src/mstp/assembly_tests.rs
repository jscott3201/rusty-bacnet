use super::*;
use crate::mstp_frame::{MAX_STANDARD_MPDU_DATA, PREAMBLE};

fn encode_host_data_frame(source: u8, fill: u8, data_len: usize) -> Vec<u8> {
    let frame = MstpFrame {
        frame_type: FrameType::BACnetDataNotExpectingReply,
        destination: 3,
        source,
        data: Bytes::from(vec![fill; data_len]),
    };
    let mut wire = BytesMut::new();
    encode_frame(&mut wire, &frame).unwrap();
    wire.to_vec()
}

#[test]
fn drains_coalesced_frames_larger_than_one_frame() {
    let first = encode_host_data_frame(1, 0xA1, 300);
    let second = encode_host_data_frame(2, 0xB2, 300);
    let mut chunk = first;
    chunk.extend_from_slice(&second);
    assert!(chunk.len() > MSTP_MAX_FRAME_BUF);

    let mut frame_buf = Vec::new();
    let counts = Counters::default();
    let frames = assemble_host_chunk(&mut frame_buf, &chunk, &counts);

    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].source, 1);
    assert_eq!(frames[0].data, Bytes::from(vec![0xA1; 300]));
    assert_eq!(frames[1].source, 2);
    assert_eq!(frames[1].data, Bytes::from(vec![0xB2; 300]));
    assert!(frame_buf.is_empty());
    assert_eq!(
        counts
            .invalid_frame_discards
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

#[test]
fn bounds_malformed_input_and_retains_max_partial() {
    let malformed = vec![0xAA; MSTP_MAX_FRAME_BUF * 4 + 17];
    let mut frame_buf = Vec::new();
    let counts = Counters::default();
    assert!(assemble_host_chunk(&mut frame_buf, &malformed, &counts).is_empty());
    assert!(frame_buf.len() <= MSTP_MAX_FRAME_BUF);

    let wire = encode_host_data_frame(1, 0xCC, MAX_STANDARD_MPDU_DATA);
    assert_eq!(wire.len(), MSTP_MAX_FRAME_BUF);
    let split = wire.len() - 1;
    assert!(assemble_host_chunk(&mut frame_buf, &wire[..split], &counts).is_empty());
    assert_eq!(frame_buf.len(), split);

    let frames = assemble_host_chunk(&mut frame_buf, &wire[split..], &counts);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].data.len(), MAX_STANDARD_MPDU_DATA);
    assert!(frame_buf.is_empty());

    let token = MstpFrame {
        frame_type: FrameType::Token,
        destination: 3,
        source: 1,
        data: Bytes::new(),
    };
    let mut token_wire = BytesMut::new();
    encode_frame(&mut token_wire, &token).unwrap();
    assert!(assemble_host_chunk(&mut frame_buf, &[0xAA, PREAMBLE[0]], &counts).is_empty());
    assert_eq!(frame_buf, PREAMBLE[..1]);
    let frames = assemble_host_chunk(&mut frame_buf, &token_wire[1..], &counts);
    assert_eq!(frames, vec![token]);
    assert!(frame_buf.is_empty());
}

#[test]
fn discard_operations_not_bytes_or_incomplete_frames_are_counted() {
    let counts = Counters::default();
    let mut buf = Vec::new();
    assert!(assemble_host_chunk(&mut buf, &[0xAA, 0xAA, 0xAA], &counts).is_empty());
    assert!(assemble_host_chunk(&mut buf, &[PREAMBLE[0]], &counts).is_empty());
    assert_eq!(
        counts
            .invalid_frame_discards
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    let wire = encode_host_data_frame(1, 0xCC, 2);
    assert_eq!(assemble_host_chunk(&mut buf, &wire[1..], &counts).len(), 1);
    assert_eq!(
        counts
            .invalid_frame_discards
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}
