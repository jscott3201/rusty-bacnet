use super::*;

fn ack(sequence: u8, negative: bool) -> Apdu {
    Apdu::SegmentAck(SegmentAckPdu {
        negative_ack: negative,
        sent_by_server: false,
        invoke_id: 1,
        sequence_number: sequence,
        actual_window_size: 1,
    })
}
fn segmented_read() -> Apdu {
    let Apdu::ConfirmedRequest(mut req) = read(1, PropertyIdentifier::DESCRIPTION) else {
        unreachable!()
    };
    req.max_apdu_length = 50;
    req.segmented_response_accepted = true;
    Apdu::ConfirmedRequest(req)
}

#[tokio::test]
async fn segmented_child_keeps_pending_after_parent_returns_through_final_ack() {
    let mut f = Fixture::configured(
        ServerConfig {
            segmentation_supported: Segmentation::BOTH,
            ..Default::default()
        },
        100,
    )
    .await;
    let req = segmented_read();
    f.inject(&req, false).await;
    let last_sequence = loop {
        let event = bounded(f.issued.recv()).await.unwrap();
        let Apdu::ComplexAck(segment) = &event.apdu else {
            panic!("expected segment")
        };
        assert!(segment.segmented);
        let sequence = segment.sequence_number.unwrap();
        let last = !segment.more_follows;
        assert!(f.pending(&req, false));
        event.release.send(Ok(())).unwrap();
        bounded(event.finished).await.unwrap();
        if last {
            break sequence;
        }
        f.inject(&ack(sequence, false), false).await;
    };
    until(|| f.server.request_admission_counters().confirmed_active == 0).await;
    assert_eq!(f.server.seg_ack_senders.lock().len(), 1);
    assert_eq!(
        f.server.seg_send_permits.available_permits(),
        MAX_SEG_SENDERS - 1
    );
    f.inject(&req, false).await;
    f.barrier(240, false).await;
    assert_eq!(
        f.server
            .request_admission_counters()
            .confirmed_admitted_total,
        2
    );
    assert_eq!(f.reads.load(Ordering::Acquire), 1);
    assert!(f.pending(&req, false));
    f.inject(&ack(last_sequence, false), false).await;
    until(|| !f.pending(&req, false)).await;
    assert!(f.server.seg_ack_senders.lock().is_empty());
    assert_eq!(
        f.server.seg_send_permits.available_permits(),
        MAX_SEG_SENDERS
    );
    f.inject(&req, false).await;
    f.held.push(bounded(f.issued.recv()).await.unwrap());
    assert_eq!(f.reads.load(Ordering::Acquire), 2);
    f.stop().await;
}

// Isolated response owner controls below use the same child/send implementation
// as the full-server test above, with an explicit retry clock and tracker owner.
struct Child {
    network: Arc<NetworkLayer<TestTransport>>,
    registry: Arc<segmented_send::SegmentedSendRegistry>,
    permits: Arc<Semaphore>,
    tracker: Arc<confirmed_request_tracker::ConfirmedRequestTracker>,
    issued: mpsc::UnboundedReceiver<Issued>,
}
impl Child {
    fn new() -> Self {
        let (issued, receive) = mpsc::unbounded_channel();
        Self {
            network: Arc::new(NetworkLayer::new(gated_port(None, issued))),
            registry: Arc::default(),
            permits: Arc::new(Semaphore::new(1)),
            tracker: Arc::default(),
            issued: receive,
        }
    }
    fn begin(&self) -> ConfirmedRequestAdmission {
        let Apdu::ConfirmedRequest(req) = segmented_read() else {
            unreachable!()
        };
        self.tracker
            .begin(&[1], None, TransportProvenance::unverified(), req)
    }
    fn owner(&self) -> PendingConfirmedRequest {
        let ConfirmedRequestAdmission::New(owner) = self.begin() else {
            panic!("still pending")
        };
        owner
    }
    fn pending(&self) -> bool {
        matches!(self.begin(), ConfirmedRequestAdmission::Duplicate)
    }
    fn spawn(&self, max_segments: Option<u8>) -> JoinHandle<()> {
        let pending = self.owner();
        let (network, registry, permits) = (
            self.network.clone(),
            self.registry.clone(),
            self.permits.clone(),
        );
        tokio::spawn(async move {
            BACnetServer::send_segmented_complex_ack_with_options(
                SegmentedSendResources {
                    network: &network,
                    seg_ack_senders: &registry,
                    seg_send_permits: &permits,
                },
                ResponseTarget {
                    source_mac: &[1],
                    source_network: None,
                    route: &bacnet_network::response_route::ResponseRoute::unverified(),
                },
                ComplexAckParams {
                    invoke_id: 1,
                    service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                    client_max_apdu: 50,
                    client_max_segments: max_segments,
                },
                &[0; 70],
                SegmentedSendOptions {
                    segment_timeout: Duration::from_secs(5),
                    max_retries: 0,
                },
                Some(pending),
            )
            .await;
        })
    }
    fn handle(&self) -> Arc<SegmentedSendHandle> {
        self.registry.lock().values().next().unwrap().clone()
    }
    fn clean(&self) {
        assert!(!self.pending());
        assert!(self.registry.lock().is_empty());
        assert_eq!(self.permits.available_permits(), 1);
    }
}

#[tokio::test]
async fn segmented_owner_releases_on_send_error_abort_cancel_and_task_abort() {
    for terminal in 0..4 {
        let mut c = Child::new();
        let task = c.spawn(None);
        let event = bounded(c.issued.recv()).await.unwrap();
        assert!(c.pending());
        if terminal == 3 {
            task.abort();
            assert!(bounded(task).await.unwrap_err().is_cancelled());
        } else {
            let handle = c.handle();
            event
                .release
                .send(if terminal == 0 {
                    Err(Error::Encoding("segment send failure".into()))
                } else {
                    Ok(())
                })
                .unwrap();
            if terminal == 1 {
                handle.send_control(SegmentedSendControlEvent::Abort(AbortPdu {
                    invoke_id: 1,
                    sent_by_server: false,
                    abort_reason: AbortReason::OTHER,
                }));
            } else if terminal == 2 {
                handle.send_control(SegmentedSendControlEvent::Cancel);
            }
            bounded(task).await.unwrap();
        }
        bounded(event.finished).await.unwrap();
        c.clean();
        drop(c.owner()); // immediate byte-identical reuse after every terminal path
    }
}

#[tokio::test(start_paused = true)]
async fn segmented_timeout_releases_owner_and_capacity() {
    let mut c = Child::new();
    let task = c.spawn(None);
    let event = c.issued.recv().await.unwrap();
    assert!(c.pending());
    event.release.send(Ok(())).unwrap();
    event.finished.await.unwrap();
    // Let the resumed child enter its ACK wait before advancing the virtual clock.
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(5)).await;
    task.await.unwrap();
    c.clean();
}

#[tokio::test]
async fn generated_terminal_abort_retires_before_held_send_result() {
    for negative_ack in [false, true] {
        let mut c = Child::new();
        let task = c.spawn((!negative_ack).then_some(1));
        if negative_ack {
            let segment = bounded(c.issued.recv()).await.unwrap();
            assert!(c.pending());
            segment.release.send(Ok(())).unwrap();
            bounded(segment.finished).await.unwrap();
            let Apdu::SegmentAck(first_ack) = ack(0, false) else {
                unreachable!()
            };
            c.handle().segment_ack_tx.send(first_ack).await.unwrap();
            let second = bounded(c.issued.recv()).await.unwrap();
            assert!(matches!(&second.apdu, Apdu::ComplexAck(a) if a.sequence_number == Some(1)));
            second.release.send(Ok(())).unwrap();
            bounded(second.finished).await.unwrap();
            // NAK for the preceding sequence requests retransmission. NAK for
            // the current sequence is an advance and cannot exhaust retries.
            let Apdu::SegmentAck(ack) = ack(0, true) else {
                unreachable!()
            };
            c.handle().segment_ack_tx.send(ack).await.unwrap();
        }
        let abort = bounded(c.issued.recv()).await.unwrap();
        assert!(
            matches!(abort.apdu, Apdu::Abort(a) if a.sent_by_server && a.abort_reason == if negative_ack { AbortReason::TSM_TIMEOUT } else { AbortReason::BUFFER_OVERFLOW })
        );
        assert!(!c.pending());
        assert!(!task.is_finished());
        if negative_ack {
            assert_eq!(c.permits.available_permits(), 0);
            assert_eq!(c.registry.lock().len(), 1);
        }
        // Old owner cleanup cannot remove a newly reused request's ownership.
        let next = c.owner();
        abort.release.send(Ok(())).unwrap();
        bounded(abort.finished).await.unwrap();
        bounded(task).await.unwrap();
        assert!(c.pending());
        drop(next);
        c.clean();
    }
}

#[tokio::test]
async fn rejected_segmented_child_drops_owner_without_polling() {
    let c = Child::new();
    let tasks = Arc::new(request_tasks::RequestTasks::default());
    let spawner = tasks.spawner();
    tasks.close();
    BACnetServer::spawn_segmented_complex_ack(
        SegmentedSendResources {
            network: &c.network,
            seg_ack_senders: &c.registry,
            seg_send_permits: &c.permits,
        },
        &spawner,
        ResponseTarget {
            source_mac: &[1],
            source_network: None,
            route: &bacnet_network::response_route::ResponseRoute::unverified(),
        },
        ComplexAckParams {
            invoke_id: 1,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            client_max_apdu: 50,
            client_max_segments: None,
        },
        Bytes::from_static(&[0; 70]),
        Some(c.owner()),
        DEFAULT_APDU_SEGMENT_TIMEOUT,
    );
    assert!(tasks.is_empty());
    c.clean();
    drop(tasks);
    BACnetServer::spawn_segmented_complex_ack(
        SegmentedSendResources {
            network: &c.network,
            seg_ack_senders: &c.registry,
            seg_send_permits: &c.permits,
        },
        &spawner,
        ResponseTarget {
            source_mac: &[1],
            source_network: None,
            route: &bacnet_network::response_route::ResponseRoute::unverified(),
        },
        ComplexAckParams {
            invoke_id: 1,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            client_max_apdu: 50,
            client_max_segments: None,
        },
        Bytes::from_static(&[0; 70]),
        Some(c.owner()),
        DEFAULT_APDU_SEGMENT_TIMEOUT,
    );
    c.clean();
}
