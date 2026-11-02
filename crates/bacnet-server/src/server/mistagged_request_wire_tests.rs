//! What a peer receives for an AtomicReadFile, AtomicWriteFile or
//! DeleteObject request with a member under the wrong tag (#1375, #1374).
//!
//! Every member of these requests outside the access-method frame's own
//! `[0]` or `[1]` is application-tagged (Clause 21's productions for Clauses
//! 14.1, 14.2 and 15.4). The decoders refuse one under any other tag, and the
//! server rejects that as INVALID_TAG (#1446), with nothing read, written or
//! deleted. A record write's count draws the Rejects for a missing record
//! and for one too many.
use super::*;
use crate::server::cov_wire_test_support::Harness;
use crate::server::truncated_request_wire_tests::{answer_to, reject_case, reject_for};
use bacnet_objects::file::FileObject;
use bacnet_types::enums::FileAccessMethod;

/// FILE-1, the stream file, as an application object identifier.
pub(super) const FILE_1: [u8; 5] = [0xC4, 0x02, 0x80, 0x00, 0x01];
/// FILE-2, the record file.
pub(super) const FILE_2: [u8; 5] = [0xC4, 0x02, 0x80, 0x00, 0x02];
/// FILE-1's contents.
const STREAM: &[u8] = b"0123456789abcdefghijklmnopqrstuv";

pub(super) const READ: ConfirmedServiceChoice = ConfirmedServiceChoice::ATOMIC_READ_FILE;
pub(super) const WRITE: ConfirmedServiceChoice = ConfirmedServiceChoice::ATOMIC_WRITE_FILE;

/// A server holding FILE-1 (stream access) and FILE-2 (record access, two
/// records), both writable.
pub(super) async fn harness() -> Harness {
    Harness::start_with(ServerConfig::default(), |db| {
        let mut stream = FileObject::new(1, "stream", "binary").unwrap();
        stream.set_data(STREAM.to_vec());
        db.add(Box::new(stream)).unwrap();
        let mut records = FileObject::new(2, "records", "binary").unwrap();
        records.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
        records.set_records(vec![b"one".to_vec(), b"two".to_vec()]);
        db.add(Box::new(records)).unwrap();
    })
    .await
}

/// FILE-1's octets and FILE-2's records as the server holds them.
pub(super) async fn contents(h: &Harness) -> (Vec<u8>, Vec<Vec<u8>>) {
    let db = h.server.database().read().await;
    let storage = |instance| {
        db.get(&ObjectIdentifier::new(ObjectType::FILE, instance).unwrap())
            .unwrap()
            .file_storage()
            .unwrap()
    };
    (
        storage(1).read_stream(0, 1000).unwrap().data,
        storage(2).read_records(0, 1000).unwrap().records,
    )
}

/// `body` after `file`.
pub(super) fn request(file: [u8; 5], body: &[u8]) -> Vec<u8> {
    [&file[..], body].concat()
}

#[tokio::test(start_paused = true)]
async fn mistagged_file_requests_are_rejected() {
    let mut h = harness().await;
    // Each case breaks one of these, which the server serves.
    let served = [
        (READ, request(FILE_1, &[0x0E, 0x31, 0x05, 0x21, 0x10, 0x0F])),
        (READ, request(FILE_2, &[0x1E, 0x31, 0x00, 0x21, 0x02, 0x1F])),
        (
            WRITE,
            request(FILE_1, &[0x0E, 0x31, 0x05, 0x62, 0x00, 0x41, 0x0F]),
        ),
        (
            WRITE,
            request(
                FILE_2,
                &[0x1E, 0x31, 0x00, 0x21, 0x01, 0x62, 0x00, 0x41, 0x1F],
            ),
        ),
    ];
    for (service, body) in &served {
        let answer = answer_to(&mut h, *service, body).await;
        assert!(
            matches!(answer, Apdu::ComplexAck(_)),
            "{body:02X?}: {answer:?}"
        );
    }
    let before = contents(&h).await;

    // The writes below carry 0x42 where the served ones above wrote 0x41, so
    // any of them carried out would change a file.
    let cases: [(ConfirmedServiceChoice, &str, Vec<u8>); 9] = [
        (
            READ,
            "start position as an application Unsigned",
            request(FILE_1, &[0x0E, 0x21, 0x05, 0x21, 0x10, 0x0F]),
        ),
        (
            READ,
            "start position as a context [3]",
            request(FILE_1, &[0x0E, 0x39, 0x05, 0x21, 0x10, 0x0F]),
        ),
        (
            READ,
            "file identifier as a context [0]",
            [
                0x0C, 0x02, 0x80, 0x00, 0x01, 0x0E, 0x31, 0x05, 0x21, 0x10, 0x0F,
            ]
            .to_vec(),
        ),
        (
            READ,
            "start record as an application Unsigned",
            request(FILE_2, &[0x1E, 0x21, 0x00, 0x21, 0x02, 0x1F]),
        ),
        (
            WRITE,
            "start as an Unsigned and file data as a CharacterString",
            request(FILE_1, &[0x0E, 0x21, 0x05, 0x72, 0x00, 0x42, 0x0F]),
        ),
        (
            WRITE,
            "file data as a CharacterString",
            request(FILE_1, &[0x0E, 0x31, 0x05, 0x72, 0x00, 0x42, 0x0F]),
        ),
        (
            WRITE,
            "file identifier as an application Unsigned",
            [
                0x24, 0x02, 0x80, 0x00, 0x01, 0x0E, 0x31, 0x05, 0x62, 0x00, 0x42, 0x0F,
            ]
            .to_vec(),
        ),
        (
            WRITE,
            "a record as a context [0]",
            request(FILE_2, &[0x1E, 0x31, 0x00, 0x21, 0x01, 0x09, 0x42, 0x1F]),
        ),
        (
            WRITE,
            "the first of two records as an application Unsigned",
            request(FILE_2, &[0x1E, 0x31, 0x00, 0x21, 0x02, 0x21, 0x42, 0x1F]),
        ),
    ];
    for (service, what, body) in &cases {
        reject_case(&mut h, *service, what, body, RejectReason::INVALID_TAG).await;
    }
    assert_eq!(contents(&h).await, before);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn record_counts_still_draw_their_rejects() {
    let mut h = harness().await;
    let before = contents(&h).await;
    let cases: [(&str, &[u8], RejectReason); 3] = [
        (
            "two records counted, one sent",
            &[0x1E, 0x31, 0x00, 0x21, 0x02, 0x61, 0x41, 0x1F],
            RejectReason::MISSING_REQUIRED_PARAMETER,
        ),
        (
            "one record counted, two sent",
            &[0x1E, 0x31, 0x00, 0x21, 0x01, 0x61, 0x41, 0x61, 0x42, 0x1F],
            RejectReason::TOO_MANY_ARGUMENTS,
        ),
        (
            "one record counted, then an Unsigned",
            &[0x1E, 0x31, 0x00, 0x21, 0x01, 0x61, 0x41, 0x21, 0x05, 0x1F],
            RejectReason::TOO_MANY_ARGUMENTS,
        ),
    ];
    for (what, body, reason) in cases {
        match answer_to(&mut h, WRITE, &request(FILE_2, body)).await {
            Apdu::Reject(reject) => assert_eq!(reject.reject_reason, reason, "{what}"),
            other => panic!("{what}: {other:?}"),
        }
    }
    assert_eq!(contents(&h).await, before);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn mistagged_delete_object_is_rejected() {
    use crate::server::cov_wire_test_support::av1;
    let mut h = harness().await;
    let delete = ConfirmedServiceChoice::DELETE_OBJECT;
    // AV-1 as a context [0], and as an application Unsigned.
    for body in [
        [0x0C, 0x00, 0x80, 0x00, 0x01],
        [0x24, 0x00, 0x80, 0x00, 0x01],
    ] {
        reject_for(&mut h, delete, &body, RejectReason::INVALID_TAG).await;
        assert!(h.server.database().read().await.get(&av1()).is_some());
    }
    // As an application object identifier it is deleted.
    let answer = answer_to(&mut h, delete, &[0xC4, 0x00, 0x80, 0x00, 0x01]).await;
    assert!(matches!(answer, Apdu::SimpleAck(_)), "{answer:?}");
    assert!(h.server.database().read().await.get(&av1()).is_none());
    h.server.stop().await.unwrap();
}
