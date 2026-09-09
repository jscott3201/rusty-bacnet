use super::*;
use bacnet_client::client::BACnetClient;
use bacnet_objects::file::FileObject;
use bacnet_services::file::{AtomicReadFileRequest, FileAccessMethod};

struct NeverStart;
impl TransportPort for NeverStart {
    async fn start(
        &mut self,
    ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
        panic!("invalid budget reached startup")
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, _: &[u8], _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        &[1]
    }
}

#[tokio::test]
async fn atomic_read_file_all_builders_validate_before_start_or_dial() {
    let default = AtomicReadFileBudget::default();
    assert_eq!(
        (
            default.max_requested_stream_octets,
            default.max_requested_records,
            default.max_service_ack_bytes
        ),
        (16384, 256, 16384)
    );
    assert_eq!(ServerConfig::default().atomic_read_file_budget, default);
    assert!(format!("{:?}", ServerConfig::default()).contains("atomic_read_file_budget"));
    AtomicReadFileBudget {
        max_requested_stream_octets: usize::MAX,
        max_requested_records: usize::MAX,
        max_service_ack_bytes: usize::MAX,
    }
    .validate()
    .unwrap();
    for budget in [
        AtomicReadFileBudget {
            max_requested_stream_octets: 0,
            ..default
        },
        AtomicReadFileBudget {
            max_requested_records: 0,
            ..default
        },
        AtomicReadFileBudget {
            max_service_ack_bytes: 0,
            ..default
        },
    ] {
        let direct = BACnetServer::start(
            ServerConfig {
                atomic_read_file_budget: budget,
                ..Default::default()
            },
            ObjectDatabase::new(),
            NeverStart,
        )
        .await
        .err();
        let generic = BACnetServer::generic_builder()
            .transport(NeverStart)
            .atomic_read_file_budget(budget)
            .build()
            .await
            .err();
        let bip = BACnetServer::bip_builder()
            .port(0)
            .atomic_read_file_budget(budget)
            .build()
            .await
            .err();
        for error in [direct, generic, bip] {
            assert!(
                matches!(error, Some(Error::Encoding(m)) if m.contains("atomic_read_file_max_"))
            );
        }
        #[cfg(feature = "sc-tls")]
        {
            let tls = tokio_rustls::rustls::ClientConfig::builder()
                .with_root_certificates(tokio_rustls::rustls::RootCertStore::empty())
                .with_no_client_auth();
            let result = BACnetServer::sc_builder()
                .hub_url("not-a-websocket-url")
                .tls_config(Arc::new(tls))
                .atomic_read_file_budget(budget)
                .build()
                .await
                .err();
            assert!(
                matches!(result, Some(Error::Encoding(m)) if m.contains("atomic_read_file_max_"))
            );
        }
    }
}

#[tokio::test]
async fn atomic_read_file_within_budget_segmented_complete_ack() {
    use bacnet_services::file::{AtomicReadFileAck, FileReadAckMethod};
    for record in [false, true] {
        for over in [false, true] {
            let mut file = FileObject::new(1, "file", "binary").unwrap();
            if record {
                file.set_file_access_method(
                    bacnet_types::enums::FileAccessMethod::RECORD_ACCESS.to_raw(),
                );
                file.set_records(vec![vec![], vec![42; 300]]);
            } else {
                file.set_data(vec![42; 300]);
            }
            let mut database = ObjectDatabase::new();
            database.add(Box::new(file)).unwrap();
            let size = if record { 312 } else { 309 };
            let mut server = BACnetServer::bip_builder()
                .interface(Ipv4Addr::LOCALHOST)
                .port(0)
                .database(database)
                .segmentation_supported(Segmentation::BOTH)
                .atomic_read_file_budget(AtomicReadFileBudget {
                    max_service_ack_bytes: size - usize::from(over),
                    ..Default::default()
                })
                .build()
                .await
                .unwrap();
            let mut client = BACnetClient::bip_builder()
                .interface(Ipv4Addr::LOCALHOST)
                .port(0)
                .max_apdu_length(50)
                .build()
                .await
                .unwrap();
            let mut request = BytesMut::new();
            AtomicReadFileRequest {
                file_identifier: ObjectIdentifier::new(ObjectType::FILE, 1).unwrap(),
                access: if record {
                    FileAccessMethod::Record {
                        file_start_record: 0,
                        requested_record_count: 2,
                    }
                } else {
                    FileAccessMethod::Stream {
                        file_start_position: 0,
                        requested_octet_count: 300,
                    }
                },
            }
            .encode(&mut request);
            let result = client
                .confirmed_request(
                    server.local_mac(),
                    ConfirmedServiceChoice::ATOMIC_READ_FILE,
                    &request,
                )
                .await;
            client.stop().await.unwrap();
            server.stop().await.unwrap();
            if over {
                assert!(
                    matches!(result, Err(Error::Abort { reason }) if reason == AbortReason::BUFFER_OVERFLOW.to_raw())
                );
            } else {
                let bytes = result.unwrap();
                assert_eq!(bytes.len(), size);
                let ack = AtomicReadFileAck::decode(&bytes).unwrap();
                assert!(ack.end_of_file);
                match ack.access {
                    FileReadAckMethod::Stream {
                        file_start_position,
                        file_data,
                    } => {
                        assert_eq!(file_start_position, 0);
                        assert_eq!(file_data, vec![42; 300]);
                    }
                    FileReadAckMethod::Record {
                        file_start_record,
                        returned_record_count,
                        file_record_data,
                    } => {
                        assert_eq!(file_start_record, 0);
                        assert_eq!(returned_record_count, 2);
                        assert_eq!(file_record_data, vec![vec![], vec![42; 300]]);
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn atomic_read_file_default_stream_raw_count_budget_wire() {
    default_count_wire(false).await;
}

#[tokio::test]
async fn atomic_read_file_default_record_raw_count_budget_wire() {
    default_count_wire(true).await;
}

async fn default_count_wire(record: bool) {
    let mut file = FileObject::new(1, "file", "binary").unwrap();
    if record {
        file.set_file_access_method(bacnet_types::enums::FileAccessMethod::RECORD_ACCESS.to_raw());
    }
    let mut database = ObjectDatabase::new();
    database.add(Box::new(file)).unwrap();
    let mut server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(database)
        .build()
        .await
        .unwrap();
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .build()
        .await
        .unwrap();
    let mut wire = BytesMut::new();
    AtomicReadFileRequest {
        file_identifier: ObjectIdentifier::new(ObjectType::FILE, 1).unwrap(),
        access: if record {
            FileAccessMethod::Record {
                file_start_record: 0,
                requested_record_count: 257,
            }
        } else {
            FileAccessMethod::Stream {
                file_start_position: 0,
                requested_octet_count: 16385,
            }
        },
    }
    .encode(&mut wire);
    let result = client
        .confirmed_request(
            server.local_mac(),
            ConfirmedServiceChoice::ATOMIC_READ_FILE,
            &wire,
        )
        .await;
    client.stop().await.unwrap();
    server.stop().await.unwrap();
    assert!(
        matches!(result, Err(Error::Abort { reason }) if reason == AbortReason::OUT_OF_RESOURCES.to_raw()),
        "{result:?}"
    );
}
