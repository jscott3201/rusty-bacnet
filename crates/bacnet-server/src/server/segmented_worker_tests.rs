use super::*;

fn oversized_request() -> Apdu {
    let mut service_request = BytesMut::new();
    bacnet_services::read_property::ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
    }
    .encode(&mut service_request);
    Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 50,
        invoke_id: 7,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: service_request.freeze(),
    })
}

fn direct_worker(server: &BACnetServer<HeldTransport>) -> JoinHandle<()> {
    let network = Arc::clone(&server.network);
    let senders = Arc::clone(&server.seg_ack_senders);
    let permits = Arc::clone(&server.seg_send_permits);
    tokio::spawn(async move {
        BACnetServer::<HeldTransport>::send_segmented_complex_ack(
            &network,
            &senders,
            &permits,
            &[1],
            None,
            7,
            ConfirmedServiceChoice::READ_PROPERTY,
            &[0; 100],
            50,
            None,
        )
        .await;
    })
}

async fn started_send(
    started: &mut mpsc::UnboundedReceiver<oneshot::Receiver<()>>,
) -> oneshot::Receiver<()> {
    tokio::time::timeout(Duration::from_secs(2), started.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn segmented_worker_cancel_join_cleans_registration_and_permit() {
    let (mut server, _tx, mut started) = fixture().await;
    let worker = direct_worker(&server);
    let mut released = started_send(&mut started).await;
    assert_eq!(server.seg_ack_senders.lock().len(), 1);
    assert_eq!(
        server.seg_send_permits.available_permits(),
        MAX_SEG_SENDERS - 1
    );
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    assert_eq!(released.try_recv(), Ok(()));
    assert!(server.seg_ack_senders.lock().is_empty());
    assert_eq!(server.seg_send_permits.available_permits(), MAX_SEG_SENDERS);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn segmented_worker_stop_joins_production_descendant() {
    let (mut server, tx, mut started) = fixture_with_name(&"x".repeat(100)).await;
    inject(&tx, oversized_request()).await;
    let mut released = started_send(&mut started).await;
    assert!(matches!(
        server.network.transport().frames.lock().unwrap().last(),
        Some(Apdu::ComplexAck(ComplexAck {
            segmented: true,
            sequence_number: Some(0),
            ..
        }))
    ));
    assert_eq!(server.seg_ack_senders.lock().len(), 1);
    assert_eq!(
        server.seg_send_permits.available_permits(),
        MAX_SEG_SENDERS - 1
    );
    server.stop().await.unwrap();
    assert_eq!(
        released.try_recv(),
        Ok(()),
        "stop left descendant send alive"
    );
    assert!(server.request_tasks.is_empty());
    assert!(server.seg_ack_senders.lock().is_empty());
    assert_eq!(server.seg_send_permits.available_permits(), MAX_SEG_SENDERS);
}

#[tokio::test]
async fn segmented_worker_old_cancel_preserves_same_key_replacement() {
    let (mut server, _tx, mut started) = fixture().await;
    let key = segmented_transaction_key(&[1], None, 7);
    let old = direct_worker(&server);
    let mut old_resource = started_send(&mut started).await;
    let old_sender = server.seg_ack_senders.lock().get(&key).unwrap().clone();
    let new = direct_worker(&server);
    let mut new_resource = started_send(&mut started).await;
    let new_sender = server.seg_ack_senders.lock().get(&key).unwrap().clone();
    assert!(!Arc::ptr_eq(&old_sender, &new_sender));
    assert!(old_sender.closed.load(Ordering::Acquire));
    assert_eq!(server.seg_ack_senders.lock().len(), 1);
    assert_eq!(
        server.seg_send_permits.available_permits(),
        MAX_SEG_SENDERS - 2
    );
    old.abort();
    assert!(old.await.unwrap_err().is_cancelled());
    assert_eq!(old_resource.try_recv(), Ok(()));
    assert!(Arc::ptr_eq(
        server.seg_ack_senders.lock().get(&key).unwrap(),
        &new_sender
    ));
    assert_eq!(
        server.seg_send_permits.available_permits(),
        MAX_SEG_SENDERS - 1
    );
    assert!(matches!(
        new_resource.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    new.abort();
    assert!(new.await.unwrap_err().is_cancelled());
    assert_eq!(new_resource.try_recv(), Ok(()));
    assert!(server.seg_ack_senders.lock().is_empty());
    assert_eq!(server.seg_send_permits.available_permits(), MAX_SEG_SENDERS);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn segmented_worker_production_panic_is_reaped_and_cleans_registry() {
    let (mut server, tx, mut started) = fixture_with_name(&"x".repeat(100)).await;
    inject(&tx, oversized_request()).await;
    let released = started_send(&mut started).await;
    assert_eq!(server.seg_ack_senders.lock().len(), 1);
    server
        .network
        .transport()
        .panic_next
        .store(true, Ordering::Release);
    server.network.transport().release.notify_one();
    released.await.unwrap();
    wait_reaped(&server).await;
    assert!(server.seg_ack_senders.lock().is_empty());
    assert_eq!(server.seg_send_permits.available_permits(), MAX_SEG_SENDERS);
    assert!(!server.dispatch_task.as_ref().unwrap().is_finished());
    server.stop().await.unwrap();
}

#[tokio::test]
async fn segmented_worker_blocked_send_does_not_block_inline_ack_or_abort() {
    let (mut server, tx, mut started) = fixture_with_config(
        &"x".repeat(100),
        ServerConfig {
            segmentation_supported: Segmentation::BOTH,
            request_admission_policy: RequestAdmissionPolicy {
                max_confirmed_in_flight_per_peer: 64,
                confirmed_recovery_reserve: 0,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await;
    inject(&tx, oversized_request()).await;
    let mut released = started_send(&mut started).await;
    let handle = server
        .seg_ack_senders
        .lock()
        .get(&segmented_transaction_key(&[1], None, 7))
        .unwrap()
        .clone();
    assert_eq!(
        server.request_admission_counters().confirmed_admitted_total,
        1
    );
    assert_eq!(
        server.request_admission_counters().confirmed_active,
        0,
        "segmented child must not retain or reacquire the top-level slot"
    );
    assert_eq!(server.request_tasks.peer_entries(), [0; 3]);
    // Fill the confirmed class with held real handlers before routing controls.
    for id in 20..84 {
        let Apdu::ConfirmedRequest(mut request) = confirmed(false) else {
            unreachable!()
        };
        request.invoke_id = id;
        inject(&tx, Apdu::ConfirmedRequest(request)).await;
        started_send(&mut started).await;
    }
    assert_eq!(server.request_admission_counters().confirmed_active, 64);
    inject(
        &tx,
        Apdu::SegmentAck(SegmentAckPdu {
            negative_ack: false,
            sent_by_server: false,
            invoke_id: 7,
            sequence_number: 0,
            actual_window_size: 1,
        }),
    )
    .await;
    inject(
        &tx,
        Apdu::Abort(AbortPdu {
            sent_by_server: false,
            invoke_id: 7,
            abort_reason: AbortReason::OTHER,
        }),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while !handle.closed.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dispatch serialized behind blocked descendant");
    assert_eq!(
        handle.segment_ack_tx.capacity(),
        15,
        "ACK was not routed inline"
    );
    assert!(matches!(
        *handle.control_tx.borrow(),
        Some(SegmentedSendControlEvent::Abort(_))
    ));
    assert!(matches!(
        released.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    server.stop().await.unwrap();
    assert_eq!(released.try_recv(), Ok(()));
    assert!(server.seg_ack_senders.lock().is_empty());
    assert_eq!(server.seg_send_permits.available_permits(), MAX_SEG_SENDERS);
}

#[tokio::test]
async fn segmented_worker_descendant_spawner_rejects_after_close_without_retaining_owner() {
    let owner = Arc::new(crate::server::request_tasks::RequestTasks::default());
    let spawner = owner.spawner();
    let weak = Arc::downgrade(&owner);
    owner.close();
    let (tx, mut rx) = oneshot::channel();
    let guard = SendGuard(Some(tx));
    spawner.spawn(async move {
        let _guard = guard;
        panic!("closed owner admitted a descendant");
    });
    assert_eq!(rx.try_recv(), Ok(()));
    assert!(owner.is_empty());
    drop(owner);
    assert!(weak.upgrade().is_none());
    let (tx, mut rx) = oneshot::channel();
    let guard = SendGuard(Some(tx));
    spawner.spawn(async move {
        let _guard = guard;
        panic!("absent owner admitted a descendant");
    });
    assert_eq!(rx.try_recv(), Ok(()));
}
