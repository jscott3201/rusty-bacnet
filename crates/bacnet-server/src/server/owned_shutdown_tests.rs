//! Deterministic admission and custom-transport cleanup ownership boundaries.
use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_transport::port::ReceivedNpdu;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use tokio::sync::Notify;

#[derive(Debug, PartialEq)]
enum Event {
    Send,
    SendDropped,
    Stop,
    StopFinished,
    Dropped,
}

struct Probe {
    events: mpsc::UnboundedSender<Event>,
    send_release: Notify,
    stop_release: Notify,
    hold_stop: AtomicBool,
    outcome: AtomicU8,
    stops: AtomicUsize,
    drops: AtomicUsize,
}
struct SendFrame(Arc<Probe>);
impl Drop for SendFrame {
    fn drop(&mut self) {
        let _ = self.0.events.send(Event::SendDropped);
    }
}
impl Probe {
    async fn send(self: Arc<Self>) -> Result<(), Error> {
        let _frame = SendFrame(Arc::clone(&self));
        let _ = self.events.send(Event::Send);
        self.send_release.notified().await;
        Ok(())
    }
    async fn stop(self: Arc<Self>) -> Result<(), Error> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        let _ = self.events.send(Event::Stop);
        if self.hold_stop.load(Ordering::SeqCst) {
            self.stop_release.notified().await;
        }
        match self.outcome.load(Ordering::SeqCst) {
            1 => Err(Error::Encoding("injected cleanup failure".into())),
            2 => panic!("injected cleanup panic"),
            _ => {
                let _ = self.events.send(Event::StopFinished);
                Ok(())
            }
        }
    }
    fn dropped(&self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        let _ = self.events.send(Event::Dropped);
    }
}

/// A custom transport whose sends, cleanup and drop are observable and held.
fn held(incoming: mpsc::Receiver<ReceivedNpdu>, probe: &Arc<Probe>) -> TestTransport {
    let (send, stop, dropped) = (Arc::clone(probe), Arc::clone(probe), Arc::clone(probe));
    TestTransport::builder()
        .local_mac(&[2])
        .inbound(incoming)
        .on_send(move |_| Arc::clone(&send).send())
        .on_stop(move || Arc::clone(&stop).stop())
        .on_drop(move || dropped.dropped())
        .build()
}

async fn fixture() -> (
    BACnetServer<TestTransport>,
    Arc<Probe>,
    mpsc::UnboundedReceiver<Event>,
    mpsc::Sender<ReceivedNpdu>,
) {
    let (events, rx) = mpsc::unbounded_channel();
    let probe = Arc::new(Probe {
        events,
        send_release: Notify::new(),
        stop_release: Notify::new(),
        hold_stop: AtomicBool::new(false),
        outcome: AtomicU8::new(0),
        stops: AtomicUsize::new(0),
        drops: AtomicUsize::new(0),
    });
    let (ingress, incoming) = mpsc::channel(4);
    let transport = held(incoming, &probe);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig::default()).unwrap(),
    ))
    .unwrap();
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .unwrap();
    (server, probe, rx, ingress)
}
async fn event(rx: &mut mpsc::UnboundedReceiver<Event>, expected: Event) {
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap(),
        Some(expected)
    );
}

#[tokio::test]
async fn cancelled_callers_keep_bounded_sends_owned_until_stop() {
    let (mut server, probe, mut events, _ingress) = fixture().await;
    let retained = server.i_am_broadcaster();
    for _ in 0..32 {
        let handle = retained.clone();
        let caller = tokio::spawn(async move { handle.broadcast_i_am().await });
        event(&mut events, Event::Send).await;
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        assert!(
            events.try_recv().is_err(),
            "caller cancellation destroyed owned send"
        );
    }
    let error = retained.broadcast_i_am().await.unwrap_err().to_string();
    assert!(error.contains("capacity"));
    assert!(
        server
            .broadcast_i_am()
            .await
            .unwrap_err()
            .to_string()
            .contains("capacity")
    );
    assert!(events.try_recv().is_err());
    server.stop().await.unwrap();
    for _ in 0..32 {
        event(&mut events, Event::SendDropped).await;
    }
    event(&mut events, Event::Stop).await;
    event(&mut events, Event::StopFinished).await;
    event(&mut events, Event::Dropped).await;
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
    assert!(retained.broadcast_i_am().await.is_err());
    assert!(server.broadcast_i_am().await.is_err());
    server.stop().await.unwrap();
    assert_eq!(probe.stops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stop_closes_admitted_waiter_and_preserves_custom_cleanup_on_cancellation() {
    let (mut server, probe, mut events, _ingress) = fixture().await;
    probe.hold_stop.store(true, Ordering::SeqCst);
    let retained = server.i_am_broadcaster();
    let caller_handle = retained.clone();
    let caller = tokio::spawn(async move { caller_handle.broadcast_i_am().await });
    event(&mut events, Event::Send).await;
    {
        let stop = server.stop();
        tokio::pin!(stop);
        tokio::select! {
            result = &mut stop => panic!("stop returned before held cleanup: {result:?}"),
            () = async { event(&mut events, Event::SendDropped).await; event(&mut events, Event::Stop).await; } => {}
        }
    }
    assert!(caller.await.unwrap().is_err());
    assert_eq!(probe.drops.load(Ordering::SeqCst), 0);
    assert!(retained.broadcast_i_am().await.is_err());
    let task_id = server.transport_cleanup.as_ref().unwrap().id();
    {
        let stop = server.stop();
        tokio::pin!(stop);
        std::future::poll_fn(|cx| {
            use std::future::Future;
            assert!(stop.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
    }
    assert_eq!(server.transport_cleanup.as_ref().unwrap().id(), task_id);
    assert_eq!(probe.stops.load(Ordering::SeqCst), 1);
    probe.stop_release.notify_one();
    server.stop().await.unwrap();
    event(&mut events, Event::StopFinished).await;
    event(&mut events, Event::Dropped).await;
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dropping_server_during_custom_cleanup_does_not_cancel_it() {
    let (mut server, probe, mut events, _ingress) = fixture().await;
    probe.hold_stop.store(true, Ordering::SeqCst);
    let retained = server.i_am_broadcaster();
    {
        let stop = server.stop();
        tokio::pin!(stop);
        tokio::select! {
            result = &mut stop => panic!("premature cleanup: {result:?}"),
            () = event(&mut events, Event::Stop) => {}
        }
    }
    drop(server);
    assert!(retained.broadcast_i_am().await.is_err());
    assert_eq!(probe.drops.load(Ordering::SeqCst), 0);
    probe.stop_release.notify_one();
    event(&mut events, Event::StopFinished).await;
    event(&mut events, Event::Dropped).await;
    assert_eq!(probe.stops.load(Ordering::SeqCst), 1);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_custom_cleanup_retains_owner_for_retry() {
    let (mut server, probe, _events, _ingress) = fixture().await;
    probe.outcome.store(1, Ordering::SeqCst);
    assert!(
        server
            .stop()
            .await
            .unwrap_err()
            .to_string()
            .contains("injected cleanup failure")
    );
    assert_eq!(probe.drops.load(Ordering::SeqCst), 0);
    assert!(server.broadcast_i_am().await.is_err());
    probe.outcome.store(0, Ordering::SeqCst);
    server.stop().await.unwrap();
    assert_eq!(probe.stops.load(Ordering::SeqCst), 2);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn panicked_custom_cleanup_is_terminal_error_not_false_success() {
    let (mut server, probe, _events, _ingress) = fixture().await;
    probe.outcome.store(2, Ordering::SeqCst);
    let error = server.stop().await.unwrap_err().to_string();
    assert!(error.contains("transport cleanup task failed"));
    assert_eq!(server.stop().await.unwrap_err().to_string(), error);
    assert_eq!(probe.stops.load(Ordering::SeqCst), 1);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn direct_broadcast_waiter_cancellation_leaves_send_owned() {
    let (mut server, _probe, mut events, _ingress) = fixture().await;
    {
        let send = server.broadcast_i_am();
        tokio::pin!(send);
        tokio::select! {
            result = &mut send => panic!("held send returned: {result:?}"),
            () = event(&mut events, Event::Send) => {}
        }
    }
    assert!(events.try_recv().is_err());
    server.stop().await.unwrap();
    event(&mut events, Event::SendDropped).await;
    event(&mut events, Event::Stop).await;
}

#[tokio::test]
async fn stopped_local_mutations_fail_before_effects_but_reads_remain() {
    let (mut server, _probe, _events, _ingress) = fixture().await;
    let oid = server.db.read().await.find_by_type(ObjectType::DEVICE)[0];
    let before = server
        .read_local(&oid, PropertyIdentifier::OBJECT_NAME, None)
        .await
        .unwrap();
    let mac = server.local_mac().to_vec();
    server.stop().await.unwrap();
    assert_eq!(server.local_mac(), mac);
    assert_eq!(server.comm_state(), DccState::Enable);
    let _ = server.discovery_counters();
    let _ = server.cov_counters();
    let _ = server.mutation_decision_counters();
    let _ = server.request_admission_counters();
    let _ = server.dcc_outcome_counters();
    let _ = server
        .generate_pics(&crate::pics::PicsConfig::default())
        .await;
    let error = server
        .write_local(
            &oid,
            PropertyIdentifier::OBJECT_NAME,
            None,
            PropertyValue::CharacterString("must-not-publish".into()),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("stopping or stopped"));
    assert_eq!(
        server
            .read_local(&oid, PropertyIdentifier::OBJECT_NAME, None)
            .await
            .unwrap(),
        before
    );
    assert!(
        server
            .set_present_value_local(&oid, PropertyValue::Real(1.0))
            .await
            .unwrap_err()
            .to_string()
            .contains("stopping or stopped")
    );
    assert!(
        server
            .set_life_safety_operation_expected_local(&oid, LifeSafetyOperation::NONE)
            .await
            .unwrap_err()
            .to_string()
            .contains("stopping or stopped")
    );
}
