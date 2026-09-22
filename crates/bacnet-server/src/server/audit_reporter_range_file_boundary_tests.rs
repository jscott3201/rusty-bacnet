use super::*;

fn request(kind: Kind, data: Bytes, segmented: bool) -> ConfirmedRequestPdu {
    ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: segmented,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 77,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: kind.service(),
        service_request: data,
    }
}

async fn ingress(
    server: &BACnetServer<CaptureTransport>,
    req: ConfirmedRequestPdu,
    reply_tx: Option<oneshot::Sender<Bytes>>,
) {
    BACnetServer::dispatch(
        &server.db,
        &server.network,
        &server.cov_table,
        &server.seg_ack_senders,
        &server.seg_send_permits,
        &server.cov_in_flight,
        &server.server_tsm,
        &server.notification_transactions,
        &server.confirmed_request_tracker,
        &server.device_bindings,
        &server.comm_state,
        &server.dcc_timer,
        &server.dcc_outcomes,
        &server.mutation_decisions,
        &Arc::new(server.config.clone()),
        &server._clock,
        &server.discovery_limiter,
        &server.time_sync_limiter,
        &server.request_tasks,
        SOURCE,
        Apdu::ConfirmedRequest(req),
        bacnet_network::layer::ReceivedApdu {
            apdu: Bytes::new(),
            source_mac: MacAddr::from_slice(SOURCE),
            ingress_network: None,
            source_network: None,
            link_layer_group: false,
            is_group: false,
            data_attributes: vec![],
            provenance: bacnet_transport::port::TransportProvenance::unverified(),
            reply_tx,
        },
    )
    .await;
}

#[tokio::test]
async fn audit_reporter_range_file_decode_and_budget_aborts_are_silent() {
    for kind in [Kind::Range, Kind::Stream, Kind::Record] {
        for case in ["decode", "bytes", "count"] {
            // ReadRange's item cap is a successful page, not a Work abort.
            if kind == Kind::Range && case == "count" {
                continue;
            }
            let mut fixture = server(read_reporter()).await;
            let reads = add_target(&fixture, kind, None, false).await;
            let data = kind.request(1, 2);
            let data = match case {
                "decode" => data.slice(..data.len() - 1),
                "bytes" => {
                    fixture
                        .server
                        .config
                        .read_range_budget
                        .max_service_ack_bytes = 1;
                    fixture
                        .server
                        .config
                        .atomic_read_file_budget
                        .max_service_ack_bytes = 1;
                    data
                }
                "count" => {
                    fixture
                        .server
                        .config
                        .atomic_read_file_budget
                        .max_requested_stream_octets = 1;
                    fixture
                        .server
                        .config
                        .atomic_read_file_budget
                        .max_requested_records = 1;
                    data
                }
                _ => unreachable!(),
            };
            let response = dispatch(&fixture.server, kind.service(), data).await;
            match (case, response) {
                ("decode", Apdu::Error(_) | Apdu::Reject(_)) => {}
                ("bytes", Apdu::Abort(abort)) => {
                    assert_eq!(abort.abort_reason, AbortReason::BUFFER_OVERFLOW)
                }
                ("count", Apdu::Abort(abort)) => {
                    assert_eq!(abort.abort_reason, AbortReason::OUT_OF_RESOURCES)
                }
                (_, other) => panic!("{kind:?} {case}: {other:?}"),
            }
            settle().await;
            assert_eq!(reads.load(Ordering::Acquire), usize::from(case == "bytes"));
            assert!(records(&fixture).is_empty());
            assert_idle(&fixture);
            fixture.server.config.read_range_budget = ReadRangeBudget::default();
            fixture.server.config.atomic_read_file_budget = AtomicReadFileBudget::default();
            assert!(matches!(
                dispatch(&fixture.server, kind.service(), kind.request(1, 1)).await,
                Apdu::ComplexAck(_)
            ));
            settle().await;
            assert_eq!(
                records(&fixture),
                vec![kind.expected(78, None)],
                "failed requests consume no timestamp"
            );
            fixture.server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn audit_reporter_range_file_dcc_duplicate_and_overload_are_silent() {
    use crate::server::{request_admission::Class, request_peer::canonical_requester};
    for kind in [Kind::Range, Kind::Stream, Kind::Record] {
        for case in ["dcc", "disable initiation", "duplicate", "overload"] {
            let mut fixture = server(read_reporter()).await;
            let reads = add_target(&fixture, kind, None, false).await;
            let req = request(kind, kind.request(1, 1), false);
            let pending = if case == "duplicate" {
                let ConfirmedRequestAdmission::New(pending) = fixture
                    .server
                    .confirmed_request_tracker
                    .begin(SOURCE, None, req.clone())
                else {
                    panic!("first admission")
                };
                Some(pending)
            } else {
                None
            };
            if case == "dcc" {
                fixture.server.comm_state.store(1, Ordering::Release);
            }
            if case == "disable initiation" {
                fixture.server.comm_state.store(2, Ordering::Release);
            }
            if case == "overload" {
                for _ in 0..fixture
                    .server
                    .config
                    .request_admission_policy
                    .max_confirmed_in_flight_per_peer
                {
                    fixture
                        .server
                        .request_tasks
                        .try_spawn(
                            Class::Confirmed,
                            canonical_requester(SOURCE, None),
                            std::future::pending::<()>,
                        )
                        .unwrap();
                }
            }
            let (tx, rx) = oneshot::channel();
            ingress(&fixture.server, req, Some(tx)).await;
            let response = tokio::time::timeout(Duration::from_secs(1), rx)
                .await
                .unwrap()
                .ok()
                .map(|bytes| decode_apdu(decode_npdu(bytes).unwrap().payload).unwrap());
            match case {
                "overload" => assert!(
                    matches!(response, Some(Apdu::Abort(abort)) if abort.abort_reason == AbortReason::OUT_OF_RESOURCES)
                ),
                "disable initiation" => assert!(matches!(response, Some(Apdu::ComplexAck(_)))),
                _ => assert!(response.is_none()),
            }
            settle().await;
            assert_eq!(
                reads.load(Ordering::Acquire),
                usize::from(case == "disable initiation")
            );
            assert!(records(&fixture).is_empty(), "{kind:?} {case}");
            assert_idle(&fixture);
            if let Some(pending) = pending {
                pending.complete();
            }
            fixture.server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn audit_reporter_range_file_guard_release_precedes_admission_not_delivery() {
    for kind in [Kind::Range, Kind::Stream, Kind::Record] {
        let mut fixture = server(read_reporter()).await;
        let reads = add_target(&fixture, kind, None, false).await;
        fixture.transport.block.store(true, Ordering::Release);
        let held_read = fixture.server.db.read().await;
        {
            let response = dispatch(&fixture.server, kind.service(), kind.request(1, 1));
            tokio::pin!(response);
            assert!(futures_util::poll!(&mut response).is_pending());
            assert_eq!(reads.load(Ordering::Acquire), 1);
            assert!(records(&fixture).is_empty());
            assert_idle(&fixture);
            drop(held_read);
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(1), response)
                    .await
                    .unwrap(),
                Apdu::ComplexAck(_)
            ));
        }
        settle().await;
        assert_eq!(records(&fixture), vec![kind.expected(77, None)]);
        assert!(fixture.server.db.try_write().is_ok());
        fixture.server.stop().await.unwrap();
        assert_idle(&fixture);
    }
}

#[tokio::test]
async fn audit_reporter_range_file_segmented_response_and_divergence_are_silent() {
    for kind in [Kind::Range, Kind::Stream] {
        for segmented in [false, true] {
            let mut fixture = server(read_reporter()).await;
            let reads = add_target(&fixture, kind, None, true).await;
            fixture.server.config.segmentation_supported = Segmentation::BOTH;
            let (tx, rx) = oneshot::channel();
            ingress(
                &fixture.server,
                request(
                    kind,
                    kind.request(if kind == Kind::Range { 1 } else { 0 }, 2000),
                    segmented,
                ),
                Some(tx),
            )
            .await;
            let reply = tokio::time::timeout(Duration::from_secs(1), rx)
                .await
                .unwrap();
            settle().await;
            if kind == Kind::Range && !segmented {
                // Range reduces its page byte cap before execution; a single
                // oversized item cannot produce a page, so this is a Bytes abort.
                let response = decode_apdu(decode_npdu(reply.unwrap()).unwrap().payload).unwrap();
                assert!(
                    matches!(response, Apdu::Abort(abort) if abort.abort_reason == AbortReason::BUFFER_OVERFLOW)
                );
            } else {
                assert!(reply.is_err());
                let responses = fixture.transport.responses.lock().unwrap();
                assert!(!responses.is_empty());
                let first =
                    decode_apdu(decode_npdu(responses[0].clone()).unwrap().payload).unwrap();
                if segmented {
                    assert!(matches!(first, Apdu::ComplexAck(ack) if ack.segmented));
                } else {
                    assert!(
                        matches!(first, Apdu::Abort(abort) if abort.abort_reason == AbortReason::SEGMENTATION_NOT_SUPPORTED)
                    );
                }
            }
            assert_eq!(reads.load(Ordering::Acquire), 1);
            assert!(records(&fixture).is_empty());
            assert_idle(&fixture);
            fixture.server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn audit_reporter_range_file_response_send_failure_keeps_admitted_record() {
    for kind in [Kind::Range, Kind::Stream, Kind::Record] {
        let mut fixture = server(read_reporter()).await;
        add_target(&fixture, kind, None, false).await;
        fixture
            .transport
            .fail_response
            .store(true, Ordering::Release);
        ingress(
            &fixture.server,
            request(kind, kind.request(1, 1), false),
            None,
        )
        .await;
        settle().await;
        assert_eq!(fixture.transport.responses.lock().unwrap().len(), 1);
        assert_eq!(records(&fixture), vec![kind.expected(77, None)]);
        assert_idle(&fixture);
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_read_audit_log_query_remains_unaudited() {
    use bacnet_objects::audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot};
    use bacnet_services::audit::{AuditLogQueryAck, AuditLogQueryRequest};
    use bacnet_types::{constructed::BACnetAuditLogQueryParameters, enums::BACnetSuccessFilter};
    struct Memory;
    impl AuditLogPersistence for Memory {
        fn load(&self, _: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
            Ok(None)
        }
        fn commit(&self, _: &AuditLogSnapshot) -> Result<(), Error> {
            Ok(())
        }
    }
    let mut fixture = server(read_reporter()).await;
    let mut plain = server(read_reporter()).await;
    plain.server.config.audit_reporter = None;
    for fixture in [&fixture, &plain] {
        fixture
            .server
            .db
            .write()
            .await
            .add(Box::new(
                AuditLogObject::new(9, "log", 4, Arc::new(Memory)).unwrap(),
            ))
            .unwrap();
    }
    for instance in [9, 99] {
        let mut data = BytesMut::new();
        AuditLogQueryRequest {
            audit_log: oid(ObjectType::AUDIT_LOG, instance),
            query_parameters: BACnetAuditLogQueryParameters::BySource {
                source_device_identifier: oid(ObjectType::DEVICE, 1),
                source_device_address: None,
                source_object_identifier: None,
                operations: None,
                successful_actions_only: BACnetSuccessFilter::ALL,
            },
            start_at_sequence_number: None,
            requested_count: 1,
        }
        .try_encode(&mut data)
        .unwrap();
        let data = data.freeze();
        let response = dispatch(
            &fixture.server,
            ConfirmedServiceChoice::AUDIT_LOG_QUERY,
            data.clone(),
        )
        .await;
        let baseline = dispatch(&plain.server, ConfirmedServiceChoice::AUDIT_LOG_QUERY, data).await;
        assert_eq!(wire(&response), wire(&baseline));
        if instance == 9 {
            let Apdu::ComplexAck(ack) = response else {
                panic!("{response:?}")
            };
            assert!(AuditLogQueryAck::decode(&ack.service_ack)
                .unwrap()
                .records
                .is_empty());
        } else {
            assert!(matches!(response, Apdu::Error(_)));
        }
    }
    settle().await;
    assert!(records(&fixture).is_empty());
    assert_idle(&fixture);
    fixture.server.stop().await.unwrap();
    plain.server.stop().await.unwrap();
}
