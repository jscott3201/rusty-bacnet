use super::*;

#[tokio::test]
async fn every_terminal_response_retires_before_transport_result() {
    for routed in [false, true] {
        for kind in 0..5 {
            let mut f = Fixture::configured(ServerConfig::default(), 200).await;
            let mut req = match kind {
                0 => write(1),
                1 | 4 => read(1, PropertyIdentifier::DESCRIPTION),
                2 => read(1, PropertyIdentifier::from_raw(999)),
                _ => request(1, ConfirmedServiceChoice::from_raw(254), Bytes::new()),
            };
            if kind == 4 {
                let Apdu::ConfirmedRequest(req) = &mut req else {
                    unreachable!()
                };
                req.max_apdu_length = 50;
            }
            for round in 1..=2 {
                f.inject(&req, routed).await;
                let mut event = bounded(f.issued.recv()).await.unwrap();
                assert!(
                    match (&event.apdu, kind) {
                        (Apdu::SimpleAck(_), 0)
                        | (Apdu::ComplexAck(_), 1)
                        | (Apdu::Error(_), 2)
                        | (Apdu::Reject(_), 3) => true,
                        (Apdu::Abort(a), 4) =>
                            a.sent_by_server
                                && a.abort_reason == AbortReason::SEGMENTATION_NOT_SUPPORTED,
                        _ => false,
                    },
                    "unexpected response {:?}",
                    event.apdu
                );
                assert_eq!(event.npdu.destination.is_some(), routed);
                assert!(!f.pending(&req, routed));
                assert!(matches!(
                    event.finished.try_recv(),
                    Err(oneshot::error::TryRecvError::Empty)
                ));
                assert_eq!(
                    f.server
                        .request_admission_counters()
                        .confirmed_admitted_total,
                    round
                );
                if kind == 0 {
                    assert_eq!(f.writes.load(Ordering::Acquire), round as usize);
                }
                if kind == 1 || kind == 4 {
                    assert_eq!(f.reads.load(Ordering::Acquire), round as usize);
                }
                f.held.push(event);
            }
            f.stop().await;
        }
    }
}

#[tokio::test]
async fn mstp_encoded_handoff_and_failed_handoff_both_release_pending() {
    for routed in [false, true] {
        for receiver_present in [false, true] {
            let mut f = Fixture::new().await;
            for round in 1..=2 {
                let db = f.server.db.clone();
                let gate = db.write().await;
                let (tx, rx) = oneshot::channel();
                let rx = receiver_present.then_some(rx);
                f.inject_with_reply(&write(1), routed, Some(tx)).await;
                f.barrier(240 + round, routed).await;
                assert!(f.pending(&write(1), routed));
                drop(gate);
                if let Some(rx) = rx {
                    let npdu = decode_npdu(bounded(rx).await.unwrap()).unwrap();
                    assert_eq!(npdu.destination.is_some(), routed);
                    assert!(
                        matches!(apdu::decode_apdu(npdu.payload).unwrap(), Apdu::SimpleAck(a) if a.invoke_id == 1)
                    );
                }
                until(|| !f.pending(&write(1), routed)).await;
                assert_eq!(f.writes.load(Ordering::Acquire), round as usize);
                // A failed reply handoff must not silently fall through to a
                // second transport send or claim that the receiver accepted it.
                assert!(f.issued.try_recv().is_err());
            }
            f.stop().await;
        }
    }
}

#[tokio::test]
async fn transport_error_after_issuance_does_not_resurrect_pending() {
    let mut f = Fixture::new().await;
    f.inject(&write(1), false).await;
    assert!(matches!(
        f.finish_next(Err(Error::Encoding("injected send failure".into())))
            .await,
        Apdu::SimpleAck(_)
    ));
    assert!(!f.pending(&write(1), false));
    f.inject(&write(1), false).await;
    assert!(matches!(f.finish_next(Ok(())).await, Apdu::SimpleAck(_)));
    assert_eq!(f.writes.load(Ordering::Acquire), 2);
    f.stop().await;
}

#[tokio::test]
async fn stop_before_response_issuance_cancels_pending_owner() {
    let mut f = Fixture::new().await;
    let db = f.server.db.clone();
    let gate = db.write().await;
    f.inject(&write(1), false).await;
    f.barrier(240, false).await;
    assert!(f.pending(&write(1), false));
    assert_eq!(f.writes.load(Ordering::Acquire), 0);
    f.server.stop().await.unwrap();
    assert!(!f.pending(&write(1), false));
    assert_eq!(f.server.request_admission_counters().confirmed_active, 0);
    drop(gate);
    for event in f.held {
        bounded(event.finished).await.unwrap();
    }
}

#[tokio::test]
async fn rejected_handler_does_not_keep_pending_and_permit_outlives_issuance() {
    let mut f = Fixture::configured(
        ServerConfig {
            request_admission_policy: RequestAdmissionPolicy {
                max_confirmed_in_flight: 1,
                confirmed_recovery_reserve: 0,
                ..Default::default()
            },
            ..Default::default()
        },
        16,
    )
    .await;
    f.inject(&write(1), false).await;
    let first = bounded(f.issued.recv()).await.unwrap();
    assert!(!f.pending(&write(1), false));
    assert_eq!(f.server.request_admission_counters().confirmed_active, 1);
    // Local transaction termination does not release the independent task
    // permit. A fresh operation while that task is held is overload, not a duplicate.
    f.inject(&write(1), false).await;
    assert!(
        matches!(f.finish_next(Ok(())).await, Apdu::Abort(a) if a.abort_reason == AbortReason::OUT_OF_RESOURCES)
    );
    assert!(!f.pending(&write(1), false));
    first.release.send(Ok(())).unwrap();
    bounded(first.finished).await.unwrap();
    until(|| f.server.request_admission_counters().confirmed_active == 0).await;
    f.inject(&write(1), false).await;
    assert!(matches!(f.finish_next(Ok(())).await, Apdu::SimpleAck(_)));
    assert_eq!(f.writes.load(Ordering::Acquire), 2);
    f.stop().await;
}

fn owner(f: &Fixture, req: &Apdu, route: Option<&NpduAddress>) -> PendingConfirmedRequest {
    let Apdu::ConfirmedRequest(req) = req else {
        unreachable!()
    };
    let ConfirmedRequestAdmission::New(owner) = f.server.confirmed_request_tracker.begin(
        &[1],
        route,
        TransportProvenance::unverified(),
        req.clone(),
    ) else {
        panic!("already pending")
    };
    owner
}

#[tokio::test]
async fn npdu_encoding_failure_including_mstp_fallback_drops_owner_without_issuance() {
    let f = Fixture::new().await;
    let req = write(1);
    let invalid_route = NpduAddress {
        network: 0,
        mac_address: MacAddr::from_slice(&[3]),
    };
    for reply_channel in [false, true] {
        let pending = owner(&f, &req, Some(&invalid_route));
        let (tx, rx) = oneshot::channel();
        let route = bacnet_network::response_route::ResponseRoute::unverified();
        requests::confirmed_response::send_unsegmented_response(
            f.server.test_network(),
            &requests::confirmed_response::encoded_apdu(&Apdu::SimpleAck(SimpleAck {
                invoke_id: 1,
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            })),
            ResponseTarget {
                source_mac: &[1],
                source_network: Some(&invalid_route),
                route: &route,
            },
            reply_channel.then_some(tx),
            Some(pending),
        )
        .await;
        assert!(rx.await.is_err());
        drop(owner(&f, &req, Some(&invalid_route)));
    }
    assert!(f.issued.is_empty());
    f.stop().await;
}

#[tokio::test]
async fn object_panic_before_response_issuance_releases_pending_owner() {
    let mut f = Fixture::new().await;
    let req = read(1, PropertyIdentifier::DESCRIPTION);
    f.panic_on_read.store(true, Ordering::Release);
    f.inject(&req, false).await;
    f.barrier(240, false).await;
    until(|| f.reads.load(Ordering::Acquire) == 1 && !f.pending(&req, false)).await;
    assert!(f.issued.is_empty());
    f.inject(&req, false).await;
    assert!(matches!(f.finish_next(Ok(())).await, Apdu::ComplexAck(_)));
    assert_eq!(f.reads.load(Ordering::Acquire), 2);
    f.stop().await;
}

async fn reassemble(f: &mut Fixture, request: &Apdu) {
    let Apdu::ConfirmedRequest(request) = request else {
        unreachable!()
    };
    let split = request.service_request.len() / 2;
    for (index, payload) in [
        &request.service_request[..split],
        &request.service_request[split..],
    ]
    .into_iter()
    .enumerate()
    {
        let segment = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: true,
            more_follows: index == 0,
            sequence_number: Some(index as u8),
            proposed_window_size: Some(1),
            service_request: Bytes::copy_from_slice(payload),
            ..request.clone()
        });
        f.inject(&segment, false).await;
        assert!(matches!(f.finish_next(Ok(())).await, Apdu::SegmentAck(a)
            if a.sent_by_server && !a.negative_ack && a.sequence_number == index as u8));
    }
}

#[tokio::test]
async fn reassembled_exact_request_is_suppressed_only_before_original_issuance() {
    let mut f = Fixture::configured(
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::default().with_segmentation(Segmentation::BOTH),
            ..Default::default()
        },
        16,
    )
    .await;
    let req = write(1);
    let db = f.server.db.clone();
    let gate = db.write().await;
    f.inject(&req, false).await;
    f.barrier(240, false).await;
    assert!(f.pending(&req, false));
    reassemble(&mut f, &req).await;
    f.barrier(241, false).await;
    assert_eq!(
        f.server
            .request_admission_counters()
            .confirmed_admitted_total,
        3
    );
    assert_eq!(f.writes.load(Ordering::Acquire), 0);
    drop(gate);
    f.observe_write_replies(1).await;
    assert_eq!(f.writes.load(Ordering::Acquire), 1);
    // The prior SimpleACK send remains held. The same reassembled request now
    // executes because its previous transaction ended at local issuance.
    reassemble(&mut f, &req).await;
    f.observe_write_replies(2).await;
    assert_eq!(f.writes.load(Ordering::Acquire), 2);
    assert_eq!(
        f.server
            .request_admission_counters()
            .confirmed_admitted_total,
        4
    );
    f.stop().await;
}
