use super::*;

#[tokio::test]
async fn audit_reporter_query_decode_validation_and_ack_encode_failures_are_silent() {
    for case in [
        "truncated",
        "trailing",
        "filter",
        "priority",
        "count",
        "bad ack",
    ] {
        let mode = if case == "bad ack" { case } else { "real" };
        let mut fixture = server(read_reporter()).await;
        let mut plain = plain_server(read_reporter()).await;
        let (reads, _) = add_log(&fixture, 1, mode).await;
        add_log(&plain, 1, mode).await;
        let mut request = query(None, 1);
        if case == "priority" {
            request.query_parameters = BACnetAuditLogQueryParameters::ByTarget {
                target_device_identifier: oid(ObjectType::DEVICE, 10),
                target_device_address: None,
                target_object_identifier: None,
                target_property_identifier: None,
                target_array_index: None,
                target_priority: Some(5),
                operations: None,
                successful_actions_only: BACnetSuccessFilter::ALL,
            };
        }
        let mut data = encode(&request).to_vec();
        match case {
            "truncated" => {
                data.pop();
            }
            "trailing" => data.push(0),
            "filter" | "priority" => {
                let pair = if case == "filter" {
                    [0x49, 0]
                } else {
                    [0x59, 5]
                };
                let offset = data.windows(2).position(|value| value == pair).unwrap();
                data[offset + 1] = if case == "filter" { 3 } else { 17 };
            }
            "count" => {
                data.truncate(data.len() - 2);
                data.extend_from_slice(&[0x3b, 1, 0, 0]); // Unsigned16 overflow
            }
            "bad ack" => {}
            _ => unreachable!(),
        }
        let data = Bytes::from(data);
        if case != "bad ack" {
            assert!(AuditLogQueryRequest::decode(&data).is_err());
        }
        let response = dispatch(&fixture.server, SERVICE, data.clone()).await;
        assert_eq!(
            wire(&response),
            wire(&dispatch(&plain.server, SERVICE, data).await)
        );
        assert!(
            matches!(response, Apdu::Error(_) | Apdu::Reject(_)),
            "{case}"
        );
        settle().await;
        assert_eq!(
            reads.load(Ordering::Acquire),
            usize::from(case == "bad ack")
        );
        assert!(records(&fixture).is_empty(), "{case}");
        assert_idle(&fixture);
        assert!(matches!(
            dispatch(&fixture.server, SERVICE, encode(&query(None, 0))).await,
            Apdu::ComplexAck(_)
        ));
        settle().await;
        assert_eq!(
            records(&fixture),
            vec![expected_query(target(), 78, 0, None)],
            "no timestamp consumed: {case}"
        );
        assert_idle(&fixture);
        fixture.server.stop().await.unwrap();
        plain.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_query_dcc_duplicate_and_overload_are_silent() {
    use crate::server::{request_admission::Class, request_peer::canonical_requester};
    for case in ["dcc", "disable initiation", "duplicate", "overload"] {
        let mut fixture = server(read_reporter()).await;
        let (reads, _) = add_log(&fixture, 1, "real").await;
        let req = request(encode(&query(None, 1)));
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
        assert!(records(&fixture).is_empty());
        assert_idle(&fixture);
        if let Some(pending) = pending {
            pending.complete();
        }
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_query_outbound_segmentation_and_divergence_are_silent() {
    for segmented in [false, true] {
        let mut fixture = server(read_reporter()).await;
        let mut plain = plain_server(read_reporter()).await;
        let (reads, _) = add_log(&fixture, 1, "large").await;
        add_log(&plain, 1, "large").await;
        for fixture in [&mut fixture, &mut plain] {
            fixture.server.config.segmentation_supported = Segmentation::BOTH;
            let mut req = request(encode(&query(None, 1)));
            req.segmented_response_accepted = segmented;
            let (tx, rx) = oneshot::channel();
            ingress(&fixture.server, req, Some(tx)).await;
            assert!(tokio::time::timeout(Duration::from_secs(1), rx)
                .await
                .unwrap()
                .is_err());
        }
        settle().await;
        let response = fixture.transport.responses.lock().unwrap()[0].clone();
        assert_eq!(response, plain.transport.responses.lock().unwrap()[0]);
        let response = decode_apdu(decode_npdu(response).unwrap().payload).unwrap();
        if segmented {
            assert!(matches!(response, Apdu::ComplexAck(ack) if ack.segmented));
        } else {
            assert!(
                matches!(response, Apdu::Abort(abort) if abort.abort_reason == AbortReason::SEGMENTATION_NOT_SUPPORTED)
            );
        }
        assert_eq!(reads.load(Ordering::Acquire), 1);
        assert!(records(&fixture).is_empty());
        assert_idle(&fixture);
        assert!(matches!(
            dispatch(&fixture.server, SERVICE, encode(&query(None, 0))).await,
            Apdu::ComplexAck(_)
        ));
        settle().await;
        assert_eq!(
            records(&fixture),
            vec![expected_query(target(), 77, 0, None)]
        );
        fixture.server.stop().await.unwrap();
        plain.server.stop().await.unwrap();
        assert_idle(&fixture);
    }
}

#[tokio::test]
async fn audit_reporter_query_guard_release_precedes_admission_not_delivery() {
    let mut fixture = server(read_reporter()).await;
    let (reads, _) = add_log(&fixture, 1, "real").await;
    fixture.transport.block.store(true, Ordering::Release);
    let held_read = fixture.server.db.read().await;
    {
        let response = dispatch(&fixture.server, SERVICE, encode(&query(None, 1)));
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
    assert_eq!(
        records(&fixture),
        vec![expected_query(target(), 77, 0, None)]
    );
    assert!(fixture.server.db.try_write().is_ok());
    assert_eq!(reads.load(Ordering::Acquire), 1);
    fixture.server.stop().await.unwrap();
    assert_idle(&fixture);
}

#[tokio::test]
async fn audit_reporter_query_response_send_failure_keeps_admitted_record() {
    let mut fixture = server(read_reporter()).await;
    let (reads, _) = add_log(&fixture, 1, "real").await;
    fixture
        .transport
        .fail_response
        .store(true, Ordering::Release);
    ingress(&fixture.server, request(encode(&query(None, 1))), None).await;
    settle().await;
    assert_eq!(fixture.transport.responses.lock().unwrap().len(), 1);
    assert_eq!(
        records(&fixture),
        vec![expected_query(target(), 77, 0, None)]
    );
    assert_eq!(reads.load(Ordering::Acquire), 1);
    assert_idle(&fixture);
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_query_reassembled_envelope_is_not_suppressed() {
    // The existing request_reassembly suite owns segment collection/validation.
    // Exercise its production completion envelope at the shared dispatch seam,
    // rather than sending raw fragments directly to the post-reassembly API.
    let mut fixture = server(read_reporter()).await;
    let (reads, _) = add_log(&fixture, 1, "real").await;
    let data = encode(&query(None, 1));
    let mut first = request(data.slice(..5));
    first.segmented = true;
    first.more_follows = true;
    first.sequence_number = Some(0);
    first.proposed_window_size = Some(2);
    let req = crate::server::segmented_receive::reassembled_confirmed_request(&first, data);
    let (tx, rx) = oneshot::channel();
    ingress(&fixture.server, req, Some(tx)).await;
    let bytes = tokio::time::timeout(Duration::from_secs(1), rx)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(decode_apdu(decode_npdu(bytes).unwrap().payload).unwrap(), Apdu::ComplexAck(ack) if !ack.segmented)
    );
    settle().await;
    assert_eq!(reads.load(Ordering::Acquire), 1);
    assert_eq!(
        records(&fixture),
        vec![expected_query(target(), 77, 0, None)]
    );
    assert_idle(&fixture);
    fixture.server.stop().await.unwrap();
}
