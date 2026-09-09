use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::file::FileObject;
use bacnet_services::file::{AtomicWriteFileRequest, FileWriteAccessMethod};

fn request(record: bool) -> Bytes {
    let mut wire = BytesMut::new();
    AtomicWriteFileRequest {
        file_identifier: ObjectIdentifier::new(ObjectType::FILE, 1).unwrap(),
        access: if record {
            FileWriteAccessMethod::Record {
                file_start_record: -1,
                record_count: 257,
                file_record_data: vec![vec![42]; 257],
            }
        } else {
            FileWriteAccessMethod::Stream {
                file_start_position: -1,
                file_data: vec![42; 16385],
            }
        },
    }
    .encode(&mut wire);
    wire.freeze()
}

async fn response(
    db: &Arc<RwLock<ObjectDatabase>>,
    wire: Bytes,
    routed: bool,
    segmented: bool,
) -> Apdu {
    let network = Arc::new(NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    )));
    let route = routed.then(|| NpduAddress {
        network: 7,
        mac_address: MacAddr::from_slice(&[9]),
    });
    let (tx, rx) = oneshot::channel();
    BACnetServer::<BipTransport>::handle_confirmed_request(
        db,
        &network,
        &Arc::new(RwLock::new(CovSubscriptionTable::new())),
        &Arc::new(segmented_send::SegmentedSendRegistry::default()),
        &Arc::new(Semaphore::new(MAX_SEG_SENDERS)),
        &Arc::new(Semaphore::new(1)),
        &Arc::new(Mutex::new(ServerTsm::new())),
        &NotificationTransactions::new(),
        &Arc::new(ConfirmedRequestTracker::default()),
        &Arc::new(RwLock::new(DeviceBindingTable::new())),
        &Arc::new(AtomicU8::new(0)),
        &Arc::new(Mutex::new(None::<JoinHandle<()>>)),
        &ServerConfig::default(),
        &Arc::new(crate::server::request_tasks::RequestTasks::default()).spawner(),
        &MacAddr::from_slice(&[1]),
        route.clone(),
        ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: segmented,
            max_segments: None,
            max_apdu_length: 50,
            invoke_id: 80,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::ATOMIC_WRITE_FILE,
            service_request: wire,
        },
        Some(tx),
    )
    .await;
    let npdu = decode_npdu(rx.await.unwrap()).unwrap();
    assert_eq!(npdu.destination, route);
    decode_apdu(npdu.payload).unwrap()
}

async fn default_refusal(record: bool) {
    for routed in [false, true] {
        for segmented in [false, true] {
            let mut file = FileObject::new(1, "file", "binary").unwrap();
            if record {
                file.set_file_access_method(
                    bacnet_types::enums::FileAccessMethod::RECORD_ACCESS.to_raw(),
                );
                file.set_records(vec![b"sentinel".to_vec()]);
            } else {
                file.set_data(b"sentinel".to_vec());
            }
            let mut database = ObjectDatabase::new();
            database.add(Box::new(file)).unwrap();
            let db = Arc::new(RwLock::new(database));
            let actual = response(&db, request(record), routed, segmented).await;
            let guard = db.read().await;
            let storage = guard
                .get(&ObjectIdentifier::new(ObjectType::FILE, 1).unwrap())
                .unwrap()
                .file_storage_internal()
                .unwrap();
            let unchanged = if record {
                storage.read_records(0, 10000).unwrap().records == vec![b"sentinel".to_vec()]
            } else {
                storage.read_stream(0, 100000).unwrap().data == b"sentinel"
            };
            assert!(
                matches!(actual, Apdu::Abort(ref a) if a.sent_by_server && a.invoke_id == 80 && a.abort_reason == AbortReason::OUT_OF_RESOURCES),
                "response={actual:?}; unchanged={unchanged}"
            );
            assert!(unchanged);
        }
    }
}

#[tokio::test]
async fn atomic_write_file_default_stream_budget_refuses_without_mutation() {
    default_refusal(false).await;
}
#[tokio::test]
async fn atomic_write_file_default_record_budget_refuses_without_mutation() {
    default_refusal(true).await;
}

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
async fn atomic_write_file_all_builders_validate_before_start_or_dial() {
    let default = AtomicWriteFileBudget::default();
    assert_eq!(
        (
            default.max_stream_payload_octets,
            default.max_records,
            default.max_record_payload_bytes
        ),
        (16384, 256, 16384)
    );
    assert_eq!(ServerConfig::default().atomic_write_file_budget, default);
    assert!(format!("{:?}", ServerConfig::default()).contains("atomic_write_file_budget"));
    AtomicWriteFileBudget {
        max_stream_payload_octets: usize::MAX,
        max_records: usize::MAX,
        max_record_payload_bytes: usize::MAX,
    }
    .validate()
    .unwrap();
    for budget in [
        AtomicWriteFileBudget {
            max_stream_payload_octets: 0,
            ..default
        },
        AtomicWriteFileBudget {
            max_records: 0,
            ..default
        },
        AtomicWriteFileBudget {
            max_record_payload_bytes: 0,
            ..default
        },
    ] {
        let direct = BACnetServer::start(
            ServerConfig {
                atomic_write_file_budget: budget,
                ..Default::default()
            },
            ObjectDatabase::new(),
            NeverStart,
        )
        .await
        .err();
        let generic = BACnetServer::generic_builder()
            .transport(NeverStart)
            .atomic_write_file_budget(budget)
            .build()
            .await
            .err();
        let bip = BACnetServer::bip_builder()
            .port(0)
            .atomic_write_file_budget(budget)
            .build()
            .await
            .err();
        for result in [direct, generic, bip] {
            assert!(
                matches!(result, Some(Error::Encoding(m)) if m.contains("atomic_write_file_max_"))
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
                .atomic_write_file_budget(budget)
                .build()
                .await
                .err();
            assert!(
                matches!(result, Some(Error::Encoding(m)) if m.contains("atomic_write_file_max_"))
            );
        }
    }
}

#[tokio::test]
async fn atomic_write_file_opaque_abort_keeps_legacy_service_mapping() {
    use bacnet_objects::file::{FileRecordRead, FileStorage, FileStreamRead, FileWriteStart};
    use bacnet_objects::traits::BACnetObject;
    use std::borrow::Cow;
    struct ErrorFile(FileObject);
    impl BACnetObject for ErrorFile {
        fn object_identifier(&self) -> ObjectIdentifier {
            self.0.object_identifier()
        }
        fn object_name(&self) -> &str {
            self.0.object_name()
        }
        fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
            Cow::Borrowed(&[])
        }
        fn read_property(
            &self,
            p: PropertyIdentifier,
            i: Option<u32>,
        ) -> Result<PropertyValue, Error> {
            self.0.read_property(p, i)
        }
        fn write_property(
            &mut self,
            _: PropertyIdentifier,
            _: Option<u32>,
            _: PropertyValue,
            _: Option<u8>,
        ) -> Result<(), Error> {
            unreachable!()
        }
        fn file_storage_internal(&self) -> Option<&dyn FileStorage> {
            Some(self)
        }
        fn file_storage_internal_mut(&mut self) -> Option<&mut dyn FileStorage> {
            Some(self)
        }
    }
    impl FileStorage for ErrorFile {
        fn read_stream(&self, _: u64, _: u64) -> Result<FileStreamRead, Error> {
            unreachable!()
        }
        fn read_records(&self, _: u64, _: u64) -> Result<FileRecordRead, Error> {
            unreachable!()
        }
        fn write_stream(&mut self, _: FileWriteStart, _: &[u8]) -> Result<u64, Error> {
            Err(Error::Abort { reason: 9 })
        }
        fn write_records(&mut self, _: FileWriteStart, _: &[Vec<u8>]) -> Result<u64, Error> {
            Err(Error::Abort { reason: 9 })
        }
    }
    for record in [false, true] {
        let mut file = FileObject::new(1, "file", "binary").unwrap();
        if record {
            file.set_file_access_method(
                bacnet_types::enums::FileAccessMethod::RECORD_ACCESS.to_raw(),
            );
        }
        let mut db = ObjectDatabase::new();
        db.add(Box::new(ErrorFile(file))).unwrap();
        let mut wire = BytesMut::new();
        AtomicWriteFileRequest {
            file_identifier: ObjectIdentifier::new(ObjectType::FILE, 1).unwrap(),
            access: if record {
                FileWriteAccessMethod::Record {
                    file_start_record: -1,
                    record_count: 1,
                    file_record_data: vec![vec![42]],
                }
            } else {
                FileWriteAccessMethod::Stream {
                    file_start_position: -1,
                    file_data: vec![42],
                }
            },
        }
        .encode(&mut wire);
        let mut out = BytesMut::new();
        let error = handlers::handle_atomic_write_file(&mut db, &wire, &mut out).unwrap_err();
        assert!(out.is_empty());
        let expected = BACnetServer::<BipTransport>::error_apdu_from_error(
            80,
            ConfirmedServiceChoice::ATOMIC_WRITE_FILE,
            &error,
        );
        assert!(matches!(expected, Apdu::Error(_)));
        let actual = response(&Arc::new(RwLock::new(db)), wire.freeze(), true, true).await;
        let mut expected_wire = BytesMut::new();
        let mut actual_wire = BytesMut::new();
        bacnet_encoding::apdu::encode_apdu(&mut expected_wire, &expected).unwrap();
        bacnet_encoding::apdu::encode_apdu(&mut actual_wire, &actual).unwrap();
        assert_eq!(actual_wire, expected_wire);
    }
}
