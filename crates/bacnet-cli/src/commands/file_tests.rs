use super::*;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::file::FileObject;
use bacnet_server::handlers::{handle_atomic_read_file, handle_atomic_write_file};
use bacnet_services::file::{
    AtomicReadFileAck, AtomicReadFileRequest, AtomicWriteFileRequest, FileReadAckMethod,
};
use bacnet_types::error::Error;
use bytes::BytesMut;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::{pending, ready};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rusty-bacnet-cli-file-{label}-{}-{}",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct MemorySink {
    stream: Vec<u8>,
    records: Vec<(i32, Vec<u8>)>,
}

impl FileReadSink for MemorySink {
    fn write_stream(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.stream.extend_from_slice(data);
        Ok(())
    }

    fn write_record(&mut self, index: i32, data: &[u8]) -> std::io::Result<()> {
        self.records.push((index, data.to_vec()));
        Ok(())
    }
}

fn stream_ack(start: i32, data: &[u8], eof: bool) -> AtomicReadFileAck {
    AtomicReadFileAck {
        end_of_file: eof,
        access: FileReadAckMethod::Stream {
            file_start_position: start,
            file_data: data.to_vec(),
        },
    }
}

fn record_ack(start: i32, records: Vec<Vec<u8>>, eof: bool) -> AtomicReadFileAck {
    AtomicReadFileAck {
        end_of_file: eof,
        access: FileReadAckMethod::Record {
            file_start_record: start,
            returned_record_count: records.len() as u32,
            file_record_data: records,
        },
    }
}

#[tokio::test]
async fn stream_retrieval_uses_payload_and_returned_progress_until_eof() {
    let replies = Rc::new(RefCell::new(VecDeque::from([
        Ok(stream_ack(0, &[0x10, 0x20], false)),
        Ok(stream_ack(2, &[0x30], true)),
    ])));
    let requests = Rc::new(RefCell::new(Vec::new()));
    let mut sink = MemorySink::default();
    let replies_for_fetch = Rc::clone(&replies);
    let requests_for_fetch = Rc::clone(&requests);

    let summary = retrieve_windows(FileReadAccess::Stream, 0, 2, &mut sink, move |access| {
        requests_for_fetch.borrow_mut().push(access);
        ready(replies_for_fetch.borrow_mut().pop_front().unwrap())
    })
    .await
    .unwrap();

    assert_eq!(sink.stream, vec![0x10, 0x20, 0x30]);
    assert_eq!(summary.octets, 3);
    assert_eq!(requests.borrow().len(), 2);
    assert!(matches!(
        requests.borrow()[1],
        FileAccessMethod::Stream {
            file_start_position: 2,
            requested_octet_count: 2,
        }
    ));
}

#[tokio::test]
async fn empty_stream_and_record_eof_complete_without_false_progress() {
    let mut stream_sink = MemorySink::default();
    let stream = retrieve_windows(FileReadAccess::Stream, 0, 4, &mut stream_sink, |_| {
        ready(Ok(stream_ack(0, &[], true)))
    })
    .await
    .unwrap();
    assert_eq!(stream.octets, 0);

    let mut record_sink = MemorySink::default();
    let record = retrieve_windows(FileReadAccess::Record, 0, 4, &mut record_sink, |_| {
        ready(Ok(record_ack(0, vec![], true)))
    })
    .await
    .unwrap();
    assert_eq!(record.records, 0);
}

#[tokio::test]
async fn record_retrieval_preserves_boundaries_absolute_names_and_empty_records() {
    let mut sink = MemorySink::default();
    let summary = retrieve_windows(FileReadAccess::Record, 5, 3, &mut sink, |_| {
        ready(Ok(record_ack(
            5,
            vec![vec![0x00, 0xFF], vec![], vec![0x10]],
            true,
        )))
    })
    .await
    .unwrap();

    assert_eq!(summary.records, 3);
    assert_eq!(
        sink.records,
        vec![(5, vec![0x00, 0xFF]), (6, vec![]), (7, vec![0x10])]
    );
    assert_eq!(record_file_name(6), "record-0000000006.bin");
}

#[tokio::test]
async fn record_output_uses_one_deterministically_named_file_per_record() {
    let temp = TempDir::new("record-output");
    let output = temp.0.join("records");
    let mut replies = VecDeque::from([
        Ok(record_ack(7, vec![vec![0x00, 0xFF], vec![]], false)),
        Ok(record_ack(9, vec![vec![0x10]], true)),
    ]);
    let summary = read_file_with(FileReadAccess::Record, 7, 2, Some(&output), move |_| {
        ready(replies.pop_front().unwrap())
    })
    .await
    .unwrap();

    assert_eq!(summary.records, 3);
    assert_eq!(
        std::fs::read(output.join("record-0000000007.bin")).unwrap(),
        vec![0x00, 0xFF]
    );
    assert_eq!(
        std::fs::read(output.join("record-0000000008.bin")).unwrap(),
        Vec::<u8>::new()
    );
    assert_eq!(
        std::fs::read(output.join("record-0000000009.bin")).unwrap(),
        vec![0x10]
    );

    let empty_output = temp.0.join("empty-records");
    read_file_with(FileReadAccess::Record, 0, 2, Some(&empty_output), |_| {
        ready(Ok(record_ack(0, vec![], true)))
    })
    .await
    .unwrap();
    assert!(empty_output.is_dir());
    assert!(std::fs::read_dir(empty_output).unwrap().next().is_none());
}

#[test]
fn invalid_start_count_and_record_output_fail_before_fetch_or_staging() {
    assert!(validate_file_read_options(FileReadAccess::Stream, -1, 1, None).is_err());
    assert!(validate_file_read_options(FileReadAccess::Stream, 0, 0, None).is_err());
    assert!(validate_file_read_options(FileReadAccess::Record, 0, 1, None).is_err());
}

#[tokio::test]
async fn malformed_cursor_windows_fail_before_writes() {
    let mut sink = MemorySink::default();
    let error = retrieve_windows(FileReadAccess::Stream, 2, 2, &mut sink, |_| {
        ready(Ok(stream_ack(0, &[0xAA], false)))
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("progress"));
    assert!(sink.stream.is_empty());

    let error = retrieve_windows(FileReadAccess::Stream, i32::MAX, 1, &mut sink, |_| {
        ready(Ok(stream_ack(i32::MAX, &[0xAA], true)))
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("range"));
    assert!(sink.stream.is_empty());
}

#[tokio::test]
async fn zero_non_eof_arm_mismatch_oversize_and_cardinality_fail_before_writes() {
    let cases = [
        AtomicReadFileAck {
            end_of_file: false,
            access: FileReadAckMethod::Stream {
                file_start_position: 0,
                file_data: vec![],
            },
        },
        record_ack(0, vec![vec![1]], true),
        stream_ack(0, &[1, 2], true),
    ];
    for ack in cases {
        let mut sink = MemorySink::default();
        assert!(
            retrieve_windows(FileReadAccess::Stream, 0, 1, &mut sink, |_| {
                ready(Ok(ack.clone()))
            })
            .await
            .is_err()
        );
        assert!(sink.stream.is_empty());
    }

    let malformed = AtomicReadFileAck {
        end_of_file: true,
        access: FileReadAckMethod::Record {
            file_start_record: 0,
            returned_record_count: 2,
            file_record_data: vec![vec![1]],
        },
    };
    let mut sink = MemorySink::default();
    assert!(
        retrieve_windows(FileReadAccess::Record, 0, 2, &mut sink, |_| {
            ready(Ok(malformed.clone()))
        })
        .await
        .is_err()
    );
    assert!(sink.records.is_empty());
}

fn only_directory_entry(path: &Path) -> Option<PathBuf> {
    let mut entries = std::fs::read_dir(path).unwrap();
    let first = entries.next().transpose().unwrap().map(|e| e.path());
    assert!(entries.next().is_none());
    first
}

#[tokio::test]
async fn collision_is_refused_before_fetch_and_existing_target_is_untouched() {
    let temp = TempDir::new("collision");
    let output = temp.0.join("file.bin");
    std::fs::write(&output, b"existing").unwrap();
    let calls = Rc::new(RefCell::new(0));
    let calls_for_fetch = Rc::clone(&calls);

    let result = read_file_with(FileReadAccess::Stream, 0, 1, Some(&output), move |_| {
        *calls_for_fetch.borrow_mut() += 1;
        ready(Ok(stream_ack(0, &[1], true)))
    })
    .await;
    assert!(result.is_err());
    assert_eq!(*calls.borrow(), 0);
    assert_eq!(std::fs::read(&output).unwrap(), b"existing");
    assert_eq!(only_directory_entry(&temp.0), Some(output));
}

#[tokio::test]
async fn remote_decode_and_cursor_failures_remove_staging() {
    for label in ["remote", "decode", "cursor"] {
        let temp = TempDir::new(label);
        let output = temp.0.join("file.bin");
        let mut replies = match label {
            "remote" => VecDeque::from([Err(Error::decoding(0, "remote failure"))]),
            "decode" => VecDeque::from([
                Ok(stream_ack(0, &[1], false)),
                Err(Error::decoding(0, "malformed ACK")),
            ]),
            "cursor" => VecDeque::from([Ok(stream_ack(0, &[], false))]),
            _ => unreachable!(),
        };

        let result = read_file_with(FileReadAccess::Stream, 0, 1, Some(&output), move |_| {
            ready(replies.pop_front().unwrap())
        })
        .await;
        assert!(result.is_err(), "{label}");
        assert!(!output.exists(), "{label}: final target must stay absent");
        assert!(only_directory_entry(&temp.0).is_none(), "{label}");
    }
}

#[tokio::test]
async fn record_write_failure_removes_staging() {
    let temp = TempDir::new("write");
    let output = temp.0.join("records");
    let root = temp.0.clone();

    let result = read_file_with(FileReadAccess::Record, 0, 1, Some(&output), move |_| {
        let staging = only_directory_entry(&root).expect("staging directory must exist");
        std::fs::remove_dir_all(staging).unwrap();
        ready(Ok(record_ack(0, vec![vec![1]], true)))
    })
    .await;

    assert!(result.is_err());
    assert!(!output.exists(), "final target must stay absent");
    assert!(only_directory_entry(&temp.0).is_none());
}

#[tokio::test]
async fn cancellation_removes_unpublished_staging() {
    let temp = TempDir::new("cancel");
    let output = temp.0.join("file.bin");
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(20),
        read_file_with(FileReadAccess::Stream, 0, 1, Some(&output), |_| {
            pending::<Result<AtomicReadFileAck, Error>>()
        }),
    )
    .await;
    assert!(result.is_err());
    assert!(!output.exists());
    assert!(only_directory_entry(&temp.0).is_none());
}

#[tokio::test]
async fn command_layer_server_handler_round_trip_spans_multiple_windows() {
    let temp = TempDir::new("server-round-trip");
    let output = temp.0.join("payload.bin");
    let file_oid = ObjectIdentifier::new(ObjectType::FILE, 1).unwrap();
    let payload = b"multi-window payload".to_vec();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        FileObject::new(1, "FILE-1", "application/octet-stream").unwrap(),
    ))
    .unwrap();

    let write = AtomicWriteFileRequest {
        file_identifier: file_oid,
        access: FileWriteAccessMethod::Stream {
            file_start_position: 0,
            file_data: payload.clone(),
        },
    };
    let mut write_wire = BytesMut::new();
    write.encode(&mut write_wire);
    handle_atomic_write_file(&mut db, &write_wire, &mut BytesMut::new()).unwrap();

    let db = Rc::new(db);
    let db_for_fetch = Rc::clone(&db);
    let summary = read_file_with(FileReadAccess::Stream, 0, 3, Some(&output), move |access| {
        let request = AtomicReadFileRequest {
            file_identifier: file_oid,
            access,
        };
        let mut request_wire = BytesMut::new();
        request.encode(&mut request_wire);
        let mut ack_wire = BytesMut::new();
        let result = handle_atomic_read_file(&db_for_fetch, &request_wire, &mut ack_wire)
            .and_then(|()| AtomicReadFileAck::decode(&ack_wire));
        ready(result)
    })
    .await
    .unwrap();

    assert!(summary.windows > 1);
    assert_eq!(std::fs::read(output).unwrap(), payload);
}
