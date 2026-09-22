use super::*;

#[tokio::test]
async fn audit_reporter_read_duplicate_admission_is_silent() {
    for service in [
        ConfirmedServiceChoice::READ_PROPERTY,
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
    ] {
        let mut fixture = server(read_reporter()).await;
        let reads = add_probe(&fixture, || Error::Encoding("unused".into())).await;
        let data = if service == ConfirmedServiceChoice::READ_PROPERTY {
            rp(probe_oid(), PropertyIdentifier::PRESENT_VALUE, None)
        } else {
            rpm(vec![spec(
                probe_oid(),
                &[(PropertyIdentifier::PRESENT_VALUE, None)],
            )])
        };
        let request = ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 77,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: service,
            service_request: data.clone(),
        };
        let ConfirmedRequestAdmission::New(pending) = fixture
            .server
            .confirmed_request_tracker
            .begin(SOURCE, None, request)
        else {
            panic!("first admission");
        };
        assert!(dispatch_optional(&fixture.server, service, data)
            .await
            .is_none());
        settle().await;
        assert_eq!(reads.load(Ordering::Acquire), 0);
        assert!(records(&fixture).is_empty());
        pending.complete();
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_read_admission_waits_for_read_guard_release_not_delivery() {
    for multiple in [false, true] {
        let mut fixture = server(read_reporter()).await;
        let reads = add_probe(&fixture, || Error::Encoding("unused".into())).await;
        fixture.transport.block.store(true, Ordering::Release);
        let held_read = fixture.server.db.read().await;
        {
            let (service, data) = if multiple {
                (
                    ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                    rpm(vec![spec(
                        probe_oid(),
                        &[(PropertyIdentifier::PRESENT_VALUE, None); 2],
                    )]),
                )
            } else {
                (
                    ConfirmedServiceChoice::READ_PROPERTY,
                    rp(probe_oid(), PropertyIdentifier::PRESENT_VALUE, None),
                )
            };
            let response = dispatch(&fixture.server, service, data);
            tokio::pin!(response);
            assert!(futures_util::poll!(&mut response).is_pending());
            assert_eq!(reads.load(Ordering::Acquire), if multiple { 2 } else { 1 });
            assert_eq!(
                fixture.server.notification_transactions.audit_resources(),
                (false, 0, 64)
            );
            assert!(records(&fixture).is_empty());
            drop(held_read);
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(1), response)
                    .await
                    .unwrap(),
                Apdu::ComplexAck(_)
            ));
        }
        settle().await;
        assert_eq!(records(&fixture).len(), if multiple { 2 } else { 1 });
        assert!(fixture.server.db.try_write().is_ok());
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_read_segmentation_divergence_is_silent() {
    for multiple in [false, true] {
        let mut fixture = server(read_reporter()).await;
        // A valid large property value, not a decode or service-buffer failure.
        fixture
            .server
            .db
            .write()
            .await
            .get_mut(&oid(ObjectType::BINARY_VALUE, 2))
            .unwrap()
            .write_property(
                PropertyIdentifier::DESCRIPTION,
                None,
                PropertyValue::CharacterString("x".repeat(2000)),
                None,
            )
            .unwrap();
        let (service, data) = if multiple {
            (
                ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                rpm(vec![
                    spec(
                        oid(ObjectType::BINARY_VALUE, 1),
                        &[(PropertyIdentifier::PRESENT_VALUE, None)],
                    ),
                    spec(
                        oid(ObjectType::BINARY_VALUE, 2),
                        &[(PropertyIdentifier::DESCRIPTION, None)],
                    ),
                ]),
            )
        } else {
            (
                ConfirmedServiceChoice::READ_PROPERTY,
                rp(
                    oid(ObjectType::BINARY_VALUE, 2),
                    PropertyIdentifier::DESCRIPTION,
                    None,
                ),
            )
        };
        assert!(dispatch_optional(&fixture.server, service, data)
            .await
            .is_none());
        settle().await;
        let responses = fixture.transport.responses.lock().unwrap().clone();
        assert_eq!(responses.len(), 1);
        let Apdu::Abort(abort) =
            decode_apdu(decode_npdu(responses[0].clone()).unwrap().payload).unwrap()
        else {
            panic!("expected abort")
        };
        assert_eq!(abort.abort_reason, AbortReason::SEGMENTATION_NOT_SUPPORTED);
        assert!(records(&fixture).is_empty());
        assert_eq!(
            fixture.server.notification_transactions.audit_resources(),
            (false, 0, 64)
        );
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_read_audit_log_targets_are_not_excluded() {
    use bacnet_objects::audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot};
    struct Memory(StdMutex<Option<AuditLogSnapshot>>);
    impl AuditLogPersistence for Memory {
        fn load(&self, _: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
            *self.0.lock().unwrap() = Some(snapshot.clone());
            Ok(())
        }
    }
    let mut reporter = read_reporter();
    reporter.set_monitored_objects(Some(vec![Selector::ObjectType(ObjectType::AUDIT_LOG)]));
    let mut fixture = server(reporter).await;
    let log = AuditLogObject::new(9, "read log", 4, Arc::new(Memory(StdMutex::new(None)))).unwrap();
    fixture.server.db.write().await.add(Box::new(log)).unwrap();
    let target = oid(ObjectType::AUDIT_LOG, 9);
    let property = PropertyIdentifier::OBJECT_NAME;
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::READ_PROPERTY,
            rp(target, property, None)
        )
        .await,
        Apdu::ComplexAck(_)
    ));
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
            rpm(vec![spec(target, &[(property, None)])])
        )
        .await,
        Apdu::ComplexAck(_)
    ));
    settle().await;
    assert_eq!(
        records(&fixture),
        vec![
            expected(target, property, None, 77, 0, None),
            expected(target, property, None, 78, 1, None)
        ]
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_read_malformed_and_dcc_denials_are_silent() {
    for service in [
        ConfirmedServiceChoice::READ_PROPERTY,
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
    ] {
        let mut fixture = server(read_reporter()).await;
        let reads = add_probe(&fixture, || Error::Encoding("unused".into())).await;
        let data = if service == ConfirmedServiceChoice::READ_PROPERTY {
            rp(probe_oid(), PropertyIdentifier::PRESENT_VALUE, None)
        } else {
            rpm(vec![spec(
                probe_oid(),
                &[(PropertyIdentifier::PRESENT_VALUE, None)],
            )])
        };
        // Includes a valid prefix, but not a completely decoded service.
        let malformed = data.slice(..data.len() - 1);
        let response = dispatch(&fixture.server, service, malformed).await;
        assert!(matches!(response, Apdu::Error(_) | Apdu::Reject(_)));
        for state in [1, 2] {
            fixture.server.comm_state.store(state, Ordering::Release);
            let response = dispatch_optional(&fixture.server, service, data.clone()).await;
            if state == 1 {
                assert!(response.is_none());
            } else {
                assert!(matches!(response, Some(Apdu::ComplexAck(_))));
            }
        }
        settle().await;
        assert_eq!(
            reads.load(Ordering::Acquire),
            1,
            "only DISABLE_INITIATION executes"
        );
        assert!(records(&fixture).is_empty());
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_rpm_work_bytes_and_decode_failures_discard_all_intents() {
    for failure in ["work", "bytes", "decode"] {
        let mut fixture = server(read_reporter()).await;
        let reads = add_probe(&fixture, || Error::Encoding("unused".into())).await;
        let one = rpm(vec![spec(
            probe_oid(),
            &[(PropertyIdentifier::PRESENT_VALUE, None)],
        )]);
        let two = rpm(vec![spec(
            probe_oid(),
            &[(PropertyIdentifier::PRESENT_VALUE, None); 2],
        )]);
        if failure == "work" {
            fixture
                .server
                .config
                .read_property_multiple_budget
                .max_result_elements = 1;
        } else if failure == "bytes" {
            // Exactly enough bytes for the first returned property and footer.
            // The second property executes, then accumulation fails: no prefix.
            let mut first_ack = BytesMut::new();
            handlers::handle_read_property_multiple(
                &*fixture.server.db.read().await,
                &one,
                &mut first_ack,
            )
            .unwrap();
            reads.store(0, Ordering::Release);
            fixture
                .server
                .config
                .read_property_multiple_budget
                .max_service_ack_bytes = first_ack.len();
        }
        let data = if failure == "decode" {
            two.slice(..two.len() - 1)
        } else {
            two
        };
        let response = dispatch(
            &fixture.server,
            ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
            data,
        )
        .await;
        match (failure, response) {
            ("work", Apdu::Abort(error)) => {
                assert_eq!(error.abort_reason, AbortReason::OUT_OF_RESOURCES)
            }
            ("bytes", Apdu::Abort(error)) => {
                assert_eq!(error.abort_reason, AbortReason::BUFFER_OVERFLOW)
            }
            ("decode", Apdu::Error(_) | Apdu::Reject(_)) => {}
            (_, other) => panic!("unexpected {failure}: {other:?}"),
        }
        settle().await;
        assert!(records(&fixture).is_empty());
        assert_eq!(
            reads.load(Ordering::Acquire),
            if failure == "bytes" { 2 } else { 0 }
        );
        assert_eq!(
            fixture.server.notification_transactions.audit_resources(),
            (false, 0, 64)
        );
        // Failed RPM did not even consume audit timestamps/admission capacity.
        let response = dispatch(
            &fixture.server,
            ConfirmedServiceChoice::READ_PROPERTY,
            rp(probe_oid(), PropertyIdentifier::PRESENT_VALUE, None),
        )
        .await;
        assert!(matches!(response, Apdu::ComplexAck(_)));
        settle().await;
        assert_eq!(
            records(&fixture),
            vec![expected(
                probe_oid(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
                78,
                0,
                None
            )]
        );
        fixture.server.stop().await.unwrap();
    }
}

fn all_records(fixture: &Fixture) -> Vec<BACnetAuditNotification> {
    fixture
        .transport
        .sent
        .lock()
        .unwrap()
        .iter()
        .map(|bytes| {
            let service = match decode_apdu(decode_npdu(bytes.clone()).unwrap().payload).unwrap() {
                Apdu::ConfirmedRequest(request) => request.service_request,
                Apdu::UnconfirmedRequest(request) => request.service_request,
                other => panic!("{other:?}"),
            };
            let mut request =
                bacnet_services::audit::AuditNotificationRequest::decode(&service).unwrap();
            assert_eq!(request.notifications.len(), 1);
            request.notifications.remove(0)
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_rpm_256_results_reuse_64_permits_summary_deadline_and_no_queue() {
    for confirmed in [false, true] {
        let mut reporter = read_reporter();
        let mut flags = AuditOperationFlags::empty();
        flags.insert(AuditOperation::READ);
        flags.insert(AuditOperation::AUDITING_FAILURE);
        reporter.set_auditable_operations(flags);
        reporter.set_issue_confirmed_notifications(confirmed);
        let mut fixture = server(reporter).await;
        let reads = add_probe(&fixture, || Error::Encoding("unused".into())).await;
        fixture.transport.block.store(true, Ordering::Release);
        let response = dispatch(
            &fixture.server,
            ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
            rpm(vec![spec(
                probe_oid(),
                &[(PropertyIdentifier::PRESENT_VALUE, None); 256],
            )]),
        )
        .await;
        let Apdu::ComplexAck(ack) = response else {
            panic!("{response:?}")
        };
        assert_eq!(
            ReadPropertyMultipleACK::decode(&ack.service_ack)
                .unwrap()
                .list_of_read_access_results[0]
                .list_of_results
                .len(),
            256
        );
        settle().await;
        assert_eq!(reads.load(Ordering::Acquire), 256);
        assert_eq!(
            all_records(&fixture),
            (0..64)
                .map(|i| expected(
                    probe_oid(),
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    77,
                    i,
                    None
                ))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            fixture.server.notification_transactions.audit_resources(),
            (true, 192, 0)
        );
        assert!(
            fixture.server.db.try_write().is_ok(),
            "delivery never owns a DB guard"
        );
        fixture.transport.block.store(false, Ordering::Release);
        fixture.transport.unblock.notify_one();
        if confirmed {
            let request = confirmed_notification(&fixture.transport.sent, 0);
            assert!(fixture.server.notification_transactions.admit_terminal(
                LOGGER,
                None,
                &Apdu::SimpleAck(SimpleAck {
                    invoke_id: request.invoke_id,
                    service_choice: request.service_choice,
                })
            ));
        }
        settle().await;
        let sent = all_records(&fixture);
        assert_eq!(sent.len(), 65);
        let summary = &sent[64];
        assert_eq!(summary.operation, AuditOperation::AUDITING_FAILURE);
        assert_eq!(summary.current_value, Some(vec![0x21, 192]));
        assert_eq!(
            summary.target_timestamp,
            Some(BACnetTimeStamp::SequenceNumber(64))
        );
        tokio::time::advance(Duration::from_secs(3)).await;
        settle().await;
        assert_eq!(
            all_records(&fixture).len(),
            65,
            "no retries or queued read records"
        );
        assert_eq!(fixture.server.notification_transactions.active_count(), 0);
        assert_eq!(
            fixture.server.notification_transactions.audit_resources(),
            (false, 0, 64)
        );
        fixture.server.stop().await.unwrap();
        assert!(fixture.server.notification_transactions.workers_empty());
    }
}
