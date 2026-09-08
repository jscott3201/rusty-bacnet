use super::*;
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_objects::device::DeviceObject;
use bacnet_transport::port::ReceivedNpdu;
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot, Notify};

#[path = "segmented_worker_tests.rs"]
mod segmented_worker_tests;

struct SendGuard(Option<oneshot::Sender<()>>);

impl Drop for SendGuard {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

struct HeldTransport {
    incoming: Option<mpsc::Receiver<ReceivedNpdu>>,
    started: mpsc::UnboundedSender<oneshot::Receiver<()>>,
    release: Arc<Notify>,
    panic_next: AtomicBool,
    frames: std::sync::Mutex<Vec<Apdu>>,
}

impl TransportPort for HeldTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.incoming.take().unwrap())
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        let decoded = decode_npdu(Bytes::copy_from_slice(npdu)).unwrap();
        let apdu = apdu::decode_apdu(decoded.payload).unwrap();
        let segment_ack = matches!(apdu, Apdu::SegmentAck(_));
        self.frames.lock().unwrap().push(apdu);
        if segment_ack {
            return Ok(());
        }
        let (tx, rx) = oneshot::channel();
        let _guard = SendGuard(Some(tx));
        self.started.send(rx).unwrap();
        self.release.notified().await;
        assert!(
            !self.panic_next.swap(false, Ordering::AcqRel),
            "injected handler panic"
        );
        Ok(())
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send_unicast(npdu, &[]).await
    }

    fn local_mac(&self) -> &[u8] {
        &[2]
    }
}

async fn fixture() -> (
    BACnetServer<HeldTransport>,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::UnboundedReceiver<oneshot::Receiver<()>>,
) {
    fixture_with_name("BACnet Device").await
}

async fn fixture_with_name(
    name: &str,
) -> (
    BACnetServer<HeldTransport>,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::UnboundedReceiver<oneshot::Receiver<()>>,
) {
    let (tx, rx) = mpsc::channel(16);
    let (started, observations) = mpsc::unbounded_channel();
    let transport = HeldTransport {
        incoming: Some(rx),
        started,
        release: Arc::new(Notify::new()),
        panic_next: AtomicBool::new(false),
        frames: std::sync::Mutex::new(Vec::new()),
    };
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(bacnet_objects::device::DeviceConfig {
            name: name.into(),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let config = ServerConfig {
        segmentation_supported: Segmentation::BOTH,
        ..ServerConfig::default()
    };
    let server = BACnetServer::start(config, db, transport).await.unwrap();
    (server, tx, observations)
}

async fn inject(tx: &mpsc::Sender<ReceivedNpdu>, apdu: Apdu) {
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, &apdu).unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload: payload.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    tx.send(ReceivedNpdu {
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(&[1]),
        link_layer_group: false,
        data_attributes: Vec::new(),
        reply_tx: None,
    })
    .await
    .unwrap();
}

fn confirmed(segmented: bool) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 1,
        sequence_number: segmented.then_some(0),
        proposed_window_size: segmented.then_some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: Bytes::new(), // A malformed request still has an owned error response.
    })
}

async fn stop_releases_handler(request: Apdu) {
    let (mut server, tx, mut started) = fixture().await;
    inject(&tx, request).await;
    let mut released = tokio::time::timeout(Duration::from_secs(2), started.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        released.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    server.stop().await.unwrap();
    assert_eq!(
        released.try_recv(),
        Ok(()),
        "stop returned with a live handler resource"
    );
}

#[tokio::test]
async fn request_tasks_stop_joins_confirmed_handler() {
    stop_releases_handler(confirmed(false)).await;
}

#[tokio::test]
async fn request_tasks_stop_joins_unconfirmed_handler() {
    stop_releases_handler(Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
        service_choice: UnconfirmedServiceChoice::WHO_IS,
        service_request: Bytes::new(),
    }))
    .await;
}

#[tokio::test]
async fn request_tasks_stop_joins_reassembled_handler() {
    stop_releases_handler(confirmed(true)).await;
}

async fn wait_reaped(server: &BACnetServer<HeldTransport>) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !server.request_tasks.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("completed handler was not reaped while ingress was idle");
}

#[tokio::test]
async fn request_tasks_reap_success_and_panic_without_killing_dispatch() {
    let (mut server, tx, mut started) = fixture().await;
    for (invoke_id, panic) in [(1, false), (2, true), (3, false)] {
        let Apdu::ConfirmedRequest(mut request) = confirmed(false) else {
            unreachable!()
        };
        request.invoke_id = invoke_id;
        inject(&tx, Apdu::ConfirmedRequest(request)).await;
        let released = tokio::time::timeout(Duration::from_secs(2), started.recv())
            .await
            .unwrap()
            .unwrap();
        server
            .network
            .transport()
            .panic_next
            .store(panic, Ordering::Release);
        server.network.transport().release.notify_one();
        released.await.unwrap();
        wait_reaped(&server).await;
        assert!(!server.dispatch_task.as_ref().unwrap().is_finished());
    }
    server.stop().await.unwrap();
}

#[tokio::test]
async fn request_tasks_cancelled_stop_retains_ownership_for_next_stop() {
    use std::future::Future;
    use std::task::Poll;

    let (mut server, tx, mut started) = fixture().await;
    inject(&tx, confirmed(false)).await;
    let mut released = tokio::time::timeout(Duration::from_secs(2), started.recv())
        .await
        .unwrap()
        .unwrap();
    {
        let mut stop = std::pin::pin!(server.stop());
        std::future::poll_fn(|cx| {
            assert!(stop.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
    }
    assert!(server.dispatch_task.is_some());
    assert!(!server.request_tasks.is_empty());
    server.stop().await.unwrap();
    assert_eq!(released.try_recv(), Ok(()));
    assert!(server.request_tasks.is_empty());
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_tasks_spawn_stop_boundary_releases_every_admitted_or_rejected_future() {
    let (mut server, _tx, _started) = fixture().await;
    let owner = Arc::clone(&server.request_tasks);
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let producer_barrier = Arc::clone(&barrier);
    let producer = tokio::spawn(async move {
        producer_barrier.wait().await;
        let mut resources = Vec::new();
        for _ in 0..128 {
            let (tx, rx) = oneshot::channel();
            resources.push(rx);
            let guard = SendGuard(Some(tx));
            owner.spawn(async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            });
        }
        resources
    });
    barrier.wait().await;
    server.stop().await.unwrap();
    for mut resource in producer.await.unwrap() {
        assert_eq!(resource.try_recv(), Ok(()));
    }
    assert!(server.request_tasks.is_empty());
    // Explicitly prove the post-stop side of admission, regardless of which
    // side won the concurrent race above.
    let (tx, mut rx) = oneshot::channel();
    let guard = SendGuard(Some(tx));
    server.request_tasks.spawn(async move {
        let _guard = guard;
        panic!("closed owner admitted a handler");
    });
    assert_eq!(rx.try_recv(), Ok(()));
    assert!(server.request_tasks.is_empty());
}

#[tokio::test]
async fn request_tasks_reap_with_active_control_ingress() {
    let (mut server, tx, mut started) = fixture().await;
    inject(&tx, confirmed(false)).await;
    let released = tokio::time::timeout(Duration::from_secs(2), started.recv())
        .await
        .unwrap()
        .unwrap();
    server.network.transport().release.notify_one();
    let ingress = async {
        for invoke_id in 0..128 {
            inject(
                &tx,
                Apdu::SimpleAck(SimpleAck {
                    invoke_id,
                    service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                }),
            )
            .await;
        }
    };
    tokio::join!(ingress, async {
        released.await.unwrap();
        wait_reaped(&server).await;
    });
    assert!(!server.dispatch_task.as_ref().unwrap().is_finished());
    server.stop().await.unwrap();
}

#[tokio::test]
async fn request_tasks_cancelled_join_keeps_aborted_child_owned() {
    use std::future::Future;
    use std::task::Poll;

    let owner = super::super::request_tasks::RequestTasks::default();
    let (tx, mut rx) = oneshot::channel();
    let guard = SendGuard(Some(tx));
    owner.spawn(async move {
        let _guard = guard;
        std::future::pending::<()>().await;
    });
    owner.close();
    {
        let mut join = std::pin::pin!(owner.join_next());
        std::future::poll_fn(|cx| {
            assert!(join.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
    }
    assert!(!owner.is_empty());
    assert!(owner.join_next().await.unwrap().unwrap_err().is_cancelled());
    assert_eq!(rx.try_recv(), Ok(()));
    assert!(owner.is_empty());
}
