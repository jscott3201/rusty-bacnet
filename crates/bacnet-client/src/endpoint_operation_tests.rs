use super::*;
use bacnet_transport::test_transport::{SendMode, TestTransport};
use tokio::sync::Semaphore;

/// A loopback link whose unicasts reach the peer first; each then waits for
/// a permit from `gate` and fails once `fail_from` earlier attempts were
/// made. `count` counts the unicast attempts.
fn failing_transport(
    inner: LoopbackTransport,
    count: Arc<AtomicUsize>,
    fail_from: usize,
    gate: Arc<Semaphore>,
) -> TestTransport {
    TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::ForwardFirst)
        .broadcast(SendMode::Forward)
        .on_send(move |frame| {
            let attempt = (!frame.broadcast).then(|| count.fetch_add(1, Ordering::SeqCst));
            let gate = Arc::clone(&gate);
            async move {
                let Some(attempt) = attempt else {
                    return Ok(());
                };
                gate.acquire().await.unwrap().forget();
                if attempt >= fail_from {
                    Err(Error::Encoding("injected send failure".into()))
                } else {
                    Ok(())
                }
            }
        })
        .build()
}

#[tokio::test]
async fn endpoint_requester_terminal_precedes_send_failure_and_tracks_prior_attempts() {
    for (fail_from, terminal) in [(0, true), (0, false), (1, false)] {
        let (transport, peer) = LoopbackTransport::pair(vec![1], vec![2]);
        let count = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(Semaphore::new(0));
        let mut endpoint = EndpointIngress::new(
            failing_transport(transport, Arc::clone(&count), fail_from, Arc::clone(&gate)),
            4,
        );
        let ingress = endpoint.start().await.unwrap();
        let mut peer = NetworkLayer::new(peer);
        let mut received_peer = peer.start().await.unwrap();
        let coordinator = Arc::new(OutboundTransactionCoordinator::new());
        let mut config = config();
        config.apdu_timeout_ms = 10;
        config.apdu_retries = 1;
        let requester =
            EndpointRequester::new(ingress.egress.clone(), Arc::clone(&coordinator), config)
                .unwrap();
        let prepared = requester
            .prepare_read_property(
                EndpointApduDestination::Direct {
                    destination_mac: MacAddr::from_slice(&[2]),
                },
                Vec::new(),
                object_identifier(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
            .unwrap();
        let exact_id = prepared.invoke_id();
        let task = tokio::spawn(prepared.execute());
        let request = timeout(WAIT, received_peer.recv()).await.unwrap().unwrap();
        let Apdu::ConfirmedRequest(request) = decode_apdu(request.apdu).unwrap() else {
            panic!()
        };
        assert_eq!(request.invoke_id, exact_id);
        if terminal {
            let (ack, bytes) = encoded_ack(exact_id, PropertyValue::Real(42.0));
            let AdmissionOutcome::Admitted(admission) = coordinator
                .admit(&CanonicalPeer::direct(&[2]), &ack)
                .unwrap()
            else {
                panic!()
            };
            assert!(
                requester
                    .complete_pre_admitted(admission, ack, received(bytes, &[2]))
                    .await
            );
        }
        gate.add_permits(2);
        let outcome = timeout(WAIT, task).await.unwrap().unwrap();
        assert!(outcome.attempted);
        assert_eq!(outcome.result.is_ok(), terminal);
        assert_eq!(count.load(Ordering::SeqCst), fail_from + 1);
        assert_eq!(coordinator.active_count().unwrap(), 0);
        endpoint.stop().await.unwrap();
        peer.stop().await.unwrap();
    }
}

#[tokio::test]
async fn endpoint_requester_queue_rejection_is_silent_and_releases_prepared_lease() {
    let (transport, peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let count = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Semaphore::new(0));
    let mut endpoint = EndpointIngress::new(
        failing_transport(transport, Arc::clone(&count), usize::MAX, gate),
        1,
    );
    let ingress = endpoint.start().await.unwrap();
    let mut peer = NetworkLayer::new(peer);
    let mut received_peer = peer.start().await.unwrap();
    let destination = EndpointApduDestination::Direct {
        destination_mac: MacAddr::from_slice(&[2]),
    };
    let _blocked = ingress
        .egress
        .admit_apdu(
            vec![0x10, 8],
            destination.clone(),
            false,
            NetworkPriority::Normal,
            Vec::new(),
            None,
        )
        .unwrap();
    timeout(WAIT, received_peer.recv()).await.unwrap().unwrap();
    let _queued = ingress
        .egress
        .admit_apdu(
            vec![0x10, 8],
            destination.clone(),
            false,
            NetworkPriority::Normal,
            Vec::new(),
            None,
        )
        .unwrap();
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let requester =
        EndpointRequester::new(ingress.egress.clone(), Arc::clone(&coordinator), config()).unwrap();
    let prepared = requester
        .prepare_read_property(
            destination,
            Vec::new(),
            object_identifier(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await
        .unwrap();
    let outcome = prepared.execute().await;
    assert!(outcome.result.is_err());
    assert!(!outcome.attempted);
    assert_eq!(coordinator.active_count().unwrap(), 0);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    endpoint.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn endpoint_requester_no_source_caller_and_prepared_drop_keep_raii_cancellation() {
    let (transport, peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut endpoint = EndpointIngress::new(transport, 4);
    let ingress = endpoint.start().await.unwrap();
    let mut peer = NetworkLayer::new(peer);
    let mut received = peer.start().await.unwrap();
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let requester =
        EndpointRequester::new(ingress.egress.clone(), Arc::clone(&coordinator), config()).unwrap();
    let prepared = requester
        .prepare_read_property(
            EndpointApduDestination::Direct {
                destination_mac: MacAddr::from_slice(&[2]),
            },
            Vec::new(),
            object_identifier(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await
        .unwrap();
    assert_eq!(coordinator.active_count().unwrap(), 1);
    drop(prepared);
    assert_eq!(coordinator.active_count().unwrap(), 0);
    assert!(received.try_recv().is_err());
    let requester_clone = requester.clone();
    let task = tokio::spawn(async move {
        requester_clone
            .read_property(
                &[2],
                object_identifier(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    timeout(WAIT, received.recv()).await.unwrap().unwrap();
    assert_eq!(coordinator.active_count().unwrap(), 1);
    task.abort();
    let _ = task.await;
    assert_eq!(coordinator.active_count().unwrap(), 0);
    endpoint.stop().await.unwrap();
    peer.stop().await.unwrap();
}
