use super::*;
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_transport::port::ReceivedNpdu;

struct IngressTransport(Option<mpsc::Receiver<ReceivedNpdu>>);

impl TransportPort for IngressTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.0.take().unwrap())
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
async fn mutation_runtime_ingress_and_reassembly_share_retained_server_counters() {
    for segmented in [false, true] {
        let fixture = Fixture::new(Some(Arc::new(|_| true)));
        let (tx, rx) = mpsc::channel(2);
        let config = ServerConfig {
            mutation_policy: MutationPolicy::DenyAll,
            segmentation_supported: Segmentation::BOTH,
            ..fixture.config
        };
        let db = std::mem::take(&mut *fixture.db.write().await);
        let mut server = BACnetServer::start(config, db, IngressTransport(Some(rx)))
            .await
            .unwrap();
        assert_eq!(
            server.mutation_decision_counters(),
            MutationDecisionCounters::default()
        );
        let (service, bytes, _) = cases().remove(0);
        let mut req = request(service, bytes, 77);
        req.segmented = segmented;
        req.sequence_number = segmented.then_some(0);
        req.proposed_window_size = segmented.then_some(1);
        let mut payload = BytesMut::new();
        encode_apdu(&mut payload, &Apdu::ConfirmedRequest(req)).unwrap();
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                payload: payload.freeze(),
                source: route(),
                expecting_reply: true,
                ..Default::default()
            },
        )
        .unwrap();
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(ReceivedNpdu {
            npdu: npdu.freeze(),
            source_mac: MacAddr::from_slice(SOURCE),
            link_layer_group: false,
            data_attributes: vec![],
            reply_tx: Some(reply_tx),
        })
        .await
        .unwrap();
        let response = tokio::time::timeout(Duration::from_secs(2), reply_rx)
            .await
            .unwrap()
            .unwrap();
        assert_denied(response, service, 77);
        let counters = expected(
            service,
            MutationServiceCounters {
                allow_total: 0,
                deny_total: 1,
                policy_deny_total: 1,
            },
        );
        assert_eq!(server.mutation_decision_counters(), counters);
        assert_eq!(
            server
                .db
                .read()
                .await
                .get(&oid(ObjectType::BINARY_VALUE, 1))
                .unwrap()
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0)
        );
        server.stop().await.unwrap();
        assert_eq!(server.mutation_decision_counters(), counters);
    }
}

#[tokio::test]
async fn mutation_counters_record_decisions_not_handler_success() {
    // The absent-authorizer fast path must still allow without a new decode.
    for authorizer in [
        None,
        Some(Arc::new(|_: &MutationAuthorizationContext| true) as MutationAuthorizer),
    ] {
        let mut server = server(MutationPolicy::Permissive, authorizer).await;
        let service = ConfirmedServiceChoice::WRITE_PROPERTY;
        let response = dispatch(&server, service, Bytes::from_static(&[0x0c]), 1)
            .await
            .unwrap();
        assert!(matches!(apdu(response), Apdu::Error(_) | Apdu::Reject(_)));
        assert_eq!(
            server.mutation_decision_counters(),
            expected(
                service,
                MutationServiceCounters {
                    allow_total: u64::from(server.config.mutation_authorizer.is_none()),
                    ..Default::default()
                }
            )
        );
        server.stop().await.unwrap();
    }

    // A successful authorization is counted even if the file budget then aborts.
    for authorizer in [
        None,
        Some(Arc::new(|_: &MutationAuthorizationContext| true) as MutationAuthorizer),
    ] {
        let mut server = server(MutationPolicy::Permissive, authorizer).await;
        let service = ConfirmedServiceChoice::ATOMIC_WRITE_FILE;
        let mut bytes = BytesMut::new();
        AtomicWriteFileRequest {
            file_identifier: oid(ObjectType::FILE, 1),
            access: bacnet_services::file::FileWriteAccessMethod::Stream {
                file_start_position: -1,
                file_data: vec![42; 16_385],
            },
        }
        .encode(&mut bytes);
        let before = snapshot(&server).await;
        let response = dispatch(&server, service, bytes.freeze(), 1).await.unwrap();
        assert!(
            matches!(apdu(response), Apdu::Abort(a) if a.abort_reason == AbortReason::OUT_OF_RESOURCES)
        );
        assert_eq!(snapshot(&server).await, before);
        assert_eq!(
            server.mutation_decision_counters(),
            expected(
                service,
                MutationServiceCounters {
                    allow_total: 1,
                    ..Default::default()
                }
            )
        );
        server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn mutation_wpm_validation_still_precedes_policy_and_counters() {
    for policy in [MutationPolicy::Permissive, MutationPolicy::DenyAll] {
        let mut server = server(
            policy,
            Some(Arc::new(|_| panic!("validation must precede policy"))),
        )
        .await;
        let bytes = wpm(vec![WriteAccessSpecification {
            object_identifier: oid(ObjectType::BINARY_VALUE, 999),
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::DESCRIPTION,
                property_array_index: None,
                value: value(PropertyValue::CharacterString("unreachable".into())),
                priority: None,
            }],
        }]);
        let before = snapshot(&server).await;
        let response = dispatch(
            &server,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            bytes,
            1,
        )
        .await
        .unwrap();
        let Apdu::Error(error) = apdu(response) else {
            panic!("expected unknown object")
        };
        assert_eq!(error.error_class, ErrorClass::OBJECT);
        assert_eq!(error.error_code, ErrorCode::UNKNOWN_OBJECT);
        assert_eq!(snapshot(&server).await, before);
        assert_eq!(
            server.mutation_decision_counters(),
            MutationDecisionCounters::default()
        );
        server.stop().await.unwrap();
    }
}
