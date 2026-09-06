use super::*;
use bacnet_objects::file::FileObject;
use bacnet_services::file::{
    AtomicReadFileAck, AtomicReadFileRequest, FileAccessMethod, FileReadAckMethod,
};

fn encoded_read(db: &ObjectDatabase, access: FileAccessMethod) -> AtomicReadFileAck {
    let request = AtomicReadFileRequest {
        file_identifier: ObjectIdentifier::new(ObjectType::FILE, 1).unwrap(),
        access,
    };
    let mut request_wire = BytesMut::new();
    request.encode(&mut request_wire);
    let mut ack_wire = BytesMut::new();
    handle_atomic_read_file(db, &request_wire, &mut ack_wire).unwrap();
    AtomicReadFileAck::decode(&ack_wire).unwrap()
}

#[test]
fn encoded_empty_stream_read_reports_end_of_file() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        FileObject::new(1, "EMPTY-STREAM", "application/octet-stream").unwrap(),
    ))
    .unwrap();

    let ack = encoded_read(
        &db,
        FileAccessMethod::Stream {
            file_start_position: 0,
            requested_octet_count: 16,
        },
    );
    assert!(ack.end_of_file);
    assert!(matches!(
        ack.access,
        FileReadAckMethod::Stream {
            file_start_position: 0,
            ref file_data,
        } if file_data.is_empty()
    ));
}

#[test]
fn encoded_empty_record_read_reports_end_of_file() {
    let mut db = ObjectDatabase::new();
    let mut file = FileObject::new(1, "EMPTY-RECORDS", "application/octet-stream").unwrap();
    file.set_file_access_method(bacnet_types::enums::FileAccessMethod::RECORD_ACCESS.to_raw());
    db.add(Box::new(file)).unwrap();

    let ack = encoded_read(
        &db,
        FileAccessMethod::Record {
            file_start_record: 0,
            requested_record_count: 16,
        },
    );
    assert!(ack.end_of_file);
    assert!(matches!(
        ack.access,
        FileReadAckMethod::Record {
            file_start_record: 0,
            returned_record_count: 0,
            ref file_record_data,
        } if file_record_data.is_empty()
    ));
}
