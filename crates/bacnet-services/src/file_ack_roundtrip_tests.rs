use super::*;

#[test]
fn atomic_read_file_ack_stream_round_trip() {
    let ack = AtomicReadFileAck {
        end_of_file: false,
        access: FileReadAckMethod::Stream {
            file_start_position: 0,
            file_data: vec![0x48, 0x65, 0x6C, 0x6C, 0x6F],
        },
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    let decoded = AtomicReadFileAck::decode(&buf).unwrap();
    assert_eq!(decoded, ack);
}

#[test]
fn atomic_read_file_ack_stream_eof_true() {
    let ack = AtomicReadFileAck {
        end_of_file: true,
        access: FileReadAckMethod::Stream {
            file_start_position: 512,
            file_data: vec![0xDE, 0xAD],
        },
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    let decoded = AtomicReadFileAck::decode(&buf).unwrap();
    assert_eq!(decoded, ack);
}

#[test]
fn atomic_read_file_ack_record_round_trip() {
    let ack = AtomicReadFileAck {
        end_of_file: true,
        access: FileReadAckMethod::Record {
            file_start_record: 5,
            returned_record_count: 2,
            file_record_data: vec![vec![0xAA, 0xBB], vec![0xCC, 0xDD, 0xEE]],
        },
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    let decoded = AtomicReadFileAck::decode(&buf).unwrap();
    assert_eq!(decoded, ack);
}

#[test]
fn atomic_read_file_ack_record_empty() {
    let ack = AtomicReadFileAck {
        end_of_file: true,
        access: FileReadAckMethod::Record {
            file_start_record: 0,
            returned_record_count: 0,
            file_record_data: vec![],
        },
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    let decoded = AtomicReadFileAck::decode(&buf).unwrap();
    assert_eq!(decoded, ack);
}

#[test]
fn atomic_write_file_ack_stream_round_trip() {
    let ack = AtomicWriteFileAck {
        access: FileWriteAckMethod::Stream {
            file_start_position: 100,
        },
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    let decoded = AtomicWriteFileAck::decode(&buf).unwrap();
    assert_eq!(decoded, ack);
}

#[test]
fn atomic_write_file_ack_stream_negative_position() {
    let ack = AtomicWriteFileAck {
        access: FileWriteAckMethod::Stream {
            file_start_position: -1,
        },
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    let decoded = AtomicWriteFileAck::decode(&buf).unwrap();
    assert_eq!(decoded, ack);
}

#[test]
fn atomic_write_file_ack_record_round_trip() {
    let ack = AtomicWriteFileAck {
        access: FileWriteAckMethod::Record {
            file_start_record: 42,
        },
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    let decoded = AtomicWriteFileAck::decode(&buf).unwrap();
    assert_eq!(decoded, ack);
}

#[test]
fn test_decode_read_file_ack_empty_input() {
    assert!(AtomicReadFileAck::decode(&[]).is_err());
}

#[test]
fn test_decode_read_file_ack_truncated() {
    let ack = AtomicReadFileAck {
        end_of_file: false,
        access: FileReadAckMethod::Stream {
            file_start_position: 0,
            file_data: vec![0x01, 0x02, 0x03],
        },
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    assert!(AtomicReadFileAck::decode(&buf[..1]).is_err());
    let half = buf.len() / 2;
    assert!(AtomicReadFileAck::decode(&buf[..half]).is_err());
}

#[test]
fn test_decode_write_file_ack_empty_input() {
    assert!(AtomicWriteFileAck::decode(&[]).is_err());
}

#[test]
fn test_decode_write_file_ack_truncated() {
    let ack = AtomicWriteFileAck {
        access: FileWriteAckMethod::Stream {
            file_start_position: 100,
        },
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    if buf.len() > 1 {
        assert!(AtomicWriteFileAck::decode(&buf[..1]).is_err());
    }
}
