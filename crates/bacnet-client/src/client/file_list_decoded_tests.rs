use super::*;
use bacnet_services::file::{AtomicReadFileAck, FileAccessMethod, FileReadAckMethod};

fn stream_request(count: u32) -> FileAccessMethod {
    FileAccessMethod::Stream {
        file_start_position: 4,
        requested_octet_count: count,
    }
}

fn record_request(count: u32) -> FileAccessMethod {
    FileAccessMethod::Record {
        file_start_record: 4,
        requested_record_count: count,
    }
}

#[test]
fn decoded_atomic_read_file_rejects_access_arm_mismatch() {
    let ack = AtomicReadFileAck {
        end_of_file: true,
        access: FileReadAckMethod::Record {
            file_start_record: 4,
            returned_record_count: 0,
            file_record_data: vec![],
        },
    };
    assert!(validate_atomic_read_file_ack(&stream_request(1), &ack).is_err());

    let ack = AtomicReadFileAck {
        end_of_file: true,
        access: FileReadAckMethod::Stream {
            file_start_position: 4,
            file_data: vec![],
        },
    };
    assert!(validate_atomic_read_file_ack(&record_request(1), &ack).is_err());
}

#[test]
fn decoded_atomic_read_file_rejects_oversized_window() {
    let stream_ack = AtomicReadFileAck {
        end_of_file: false,
        access: FileReadAckMethod::Stream {
            file_start_position: 4,
            file_data: vec![1, 2, 3],
        },
    };
    assert!(validate_atomic_read_file_ack(&stream_request(2), &stream_ack).is_err());

    let record_ack = AtomicReadFileAck {
        end_of_file: false,
        access: FileReadAckMethod::Record {
            file_start_record: 4,
            returned_record_count: 2,
            file_record_data: vec![vec![1], vec![2]],
        },
    };
    assert!(validate_atomic_read_file_ack(&record_request(1), &record_ack).is_err());
}

#[test]
fn decoded_atomic_read_file_accepts_matching_bounded_window() {
    let stream_ack = AtomicReadFileAck {
        end_of_file: true,
        access: FileReadAckMethod::Stream {
            file_start_position: 9,
            file_data: vec![1, 2],
        },
    };
    assert!(validate_atomic_read_file_ack(&stream_request(2), &stream_ack).is_ok());

    let record_ack = AtomicReadFileAck {
        end_of_file: true,
        access: FileReadAckMethod::Record {
            file_start_record: 9,
            returned_record_count: 2,
            file_record_data: vec![vec![], vec![0xFF]],
        },
    };
    assert!(validate_atomic_read_file_ack(&record_request(2), &record_ack).is_ok());
}
