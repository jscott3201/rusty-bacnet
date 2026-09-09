use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_objects::file::FileObject;
use bacnet_server::server::AtomicWriteFileBudget;
use bacnet_services::file::{
    AtomicWriteFileAck, AtomicWriteFileRequest, FileAccessMethod, FileReadAckMethod,
    FileWriteAccessMethod, FileWriteAckMethod,
};

#[tokio::test]
async fn atomic_write_file_segmented_request_budget_and_client_readback() {
    for record in [false, true] {
        for over in [false, true] {
            let mut file = FileObject::new(1, "file", "binary").unwrap();
            if record {
                file.set_file_access_method(
                    bacnet_types::enums::FileAccessMethod::RECORD_ACCESS.to_raw(),
                );
                file.set_records(vec![b"sentinel".to_vec()]);
            } else {
                file.set_data(b"sentinel".to_vec());
            }
            let mut db = ObjectDatabase::new();
            db.add(Box::new(file)).unwrap();
            let mut server = BACnetServer::bip_builder()
                .interface(Ipv4Addr::LOCALHOST)
                .port(0)
                .segmentation_supported(Segmentation::BOTH)
                .database(db)
                .atomic_write_file_budget(AtomicWriteFileBudget {
                    max_stream_payload_octets: 300,
                    max_records: 2,
                    max_record_payload_bytes: 300,
                })
                .build()
                .await
                .unwrap();
            let mut raw = NetworkLayer::new(BipTransport::new(
                Ipv4Addr::LOCALHOST,
                0,
                Ipv4Addr::LOCALHOST,
            ));
            let mut rx = raw.start().await.unwrap();
            let oid = ObjectIdentifier::new(ObjectType::FILE, 1).unwrap();
            let payload = vec![42; 300 + usize::from(over)];
            let mut wire = BytesMut::new();
            AtomicWriteFileRequest {
                file_identifier: oid,
                access: if record {
                    FileWriteAccessMethod::Record {
                        file_start_record: -1,
                        record_count: 2,
                        file_record_data: vec![vec![], payload.clone()],
                    }
                } else {
                    FileWriteAccessMethod::Stream {
                        file_start_position: -1,
                        file_data: payload.clone(),
                    }
                },
            }
            .encode(&mut wire);
            // Actual two-segment REQUEST reassembly, not a claim about the tiny ACK.
            let chunks = [&wire[..wire.len() / 2], &wire[wire.len() / 2..]];
            for (seq, chunk) in chunks.into_iter().enumerate() {
                let mut encoded = BytesMut::new();
                encode_apdu(
                    &mut encoded,
                    &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                        segmented: true,
                        more_follows: seq == 0,
                        segmented_response_accepted: true,
                        max_segments: None,
                        max_apdu_length: 50,
                        invoke_id: 42,
                        sequence_number: Some(seq as u8),
                        proposed_window_size: Some(1),
                        service_choice: ConfirmedServiceChoice::ATOMIC_WRITE_FILE,
                        service_request: Bytes::copy_from_slice(chunk),
                    }),
                )
                .unwrap();
                raw.send_apdu(&encoded, server.local_mac(), true, NetworkPriority::NORMAL)
                    .await
                    .unwrap();
                let ack = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                    .await
                    .unwrap()
                    .unwrap();
                assert!(
                    matches!(decode_apdu(ack.apdu).unwrap(), Apdu::SegmentAck(a) if a.sent_by_server && !a.negative_ack && a.invoke_id == 42 && a.sequence_number == seq as u8)
                );
            }
            let response = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .unwrap()
                .unwrap();
            match decode_apdu(response.apdu).unwrap() {
                Apdu::Abort(a) if over => {
                    assert!(a.sent_by_server);
                    assert_eq!(a.invoke_id, 42);
                    assert_eq!(a.abort_reason, AbortReason::OUT_OF_RESOURCES);
                }
                Apdu::ComplexAck(a) if !over => {
                    assert!(!a.segmented);
                    assert_eq!(a.invoke_id, 42);
                    assert_eq!(a.service_choice, ConfirmedServiceChoice::ATOMIC_WRITE_FILE);
                    assert_eq!(
                        AtomicWriteFileAck::decode(&a.service_ack).unwrap().access,
                        if record {
                            FileWriteAckMethod::Record {
                                file_start_record: 1,
                            }
                        } else {
                            FileWriteAckMethod::Stream {
                                file_start_position: 8,
                            }
                        }
                    );
                }
                other => panic!("unexpected write response: {other:?}"),
            }
            let mut client = make_client().await;
            let read = client
                .atomic_read_file(
                    server.local_mac(),
                    oid,
                    if record {
                        FileAccessMethod::Record {
                            file_start_record: 0,
                            requested_record_count: 3,
                        }
                    } else {
                        FileAccessMethod::Stream {
                            file_start_position: 0,
                            requested_octet_count: 1000,
                        }
                    },
                )
                .await
                .unwrap();
            match bacnet_services::file::AtomicReadFileAck::decode(&read)
                .unwrap()
                .access
            {
                FileReadAckMethod::Record {
                    file_record_data, ..
                } => assert_eq!(
                    file_record_data,
                    if over {
                        vec![b"sentinel".to_vec()]
                    } else {
                        vec![b"sentinel".to_vec(), vec![], payload]
                    }
                ),
                FileReadAckMethod::Stream { file_data, .. } => assert_eq!(
                    file_data,
                    if over {
                        b"sentinel".to_vec()
                    } else {
                        [b"sentinel".to_vec(), payload].concat()
                    }
                ),
            }
            // Also exercise the regular typed client write/read path with the same policy.
            let ack = client
                .atomic_write_file(
                    server.local_mac(),
                    oid,
                    if record {
                        FileWriteAccessMethod::Record {
                            file_start_record: 0,
                            record_count: 1,
                            file_record_data: vec![b"ok".to_vec()],
                        }
                    } else {
                        FileWriteAccessMethod::Stream {
                            file_start_position: 0,
                            file_data: b"ok".to_vec(),
                        }
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                AtomicWriteFileAck::decode(&ack).unwrap().access,
                if record {
                    FileWriteAckMethod::Record {
                        file_start_record: 0,
                    }
                } else {
                    FileWriteAckMethod::Stream {
                        file_start_position: 0,
                    }
                }
            );
            let read = client
                .atomic_read_file(
                    server.local_mac(),
                    oid,
                    if record {
                        FileAccessMethod::Record {
                            file_start_record: 0,
                            requested_record_count: 1,
                        }
                    } else {
                        FileAccessMethod::Stream {
                            file_start_position: 0,
                            requested_octet_count: 2,
                        }
                    },
                )
                .await
                .unwrap();
            match bacnet_services::file::AtomicReadFileAck::decode(&read)
                .unwrap()
                .access
            {
                FileReadAckMethod::Record {
                    file_record_data, ..
                } => assert_eq!(file_record_data, vec![b"ok".to_vec()]),
                FileReadAckMethod::Stream { file_data, .. } => assert_eq!(file_data, b"ok"),
            }
            client.stop().await.unwrap();
            server.stop().await.unwrap();
            raw.stop().await.unwrap();
        }
    }
}
