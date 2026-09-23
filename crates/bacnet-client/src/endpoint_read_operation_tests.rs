use super::*;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use tokio::sync::{mpsc, Semaphore};

struct FailingTransport {
    inner: LoopbackTransport,
    count: Arc<AtomicUsize>,
    fail_from: usize,
    gate: Arc<Semaphore>,
}
impl TransportPort for FailingTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.inner.start().await
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.inner.stop().await
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let attempt = self.count.fetch_add(1, Ordering::SeqCst);
        self.inner.send_unicast(npdu, mac).await?;
        self.gate.acquire().await.unwrap().forget();
        if attempt >= self.fail_from {
            Err(Error::Encoding("injected send failure".into()))
        } else {
            Ok(())
        }
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.inner.send_broadcast(npdu).await
    }
    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }
}

#[tokio::test]
async fn endpoint_requester_terminal_precedes_send_failure_and_tracks_prior_attempts() {
    for (fail_from, terminal) in [(0, true), (0, false), (1, false)] {
        let (transport, peer) = LoopbackTransport::pair(vec![1], vec![2]);
        let count = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(Semaphore::new(0));
        let mut endpoint = EndpointIngress::new(
            FailingTransport {
                inner: transport,
                count: Arc::clone(&count),
                fail_from,
                gate: Arc::clone(&gate),
            },
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
        FailingTransport {
            inner: transport,
            count: Arc::clone(&count),
            fail_from: usize::MAX,
            gate,
        },
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
            NetworkPriority::NORMAL,
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
            NetworkPriority::NORMAL,
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
