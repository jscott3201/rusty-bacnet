//! Real-clock rejection deadlines at the WebSocket boundary, not OS backpressure.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use tokio::time::timeout;

#[path = "rejection_recovery_tests.rs"]
mod recovery_tests;

#[derive(Default)]
struct Observations {
    hold_nak: AtomicBool,
    fail_nak: AtomicBool,
    nak_started: AtomicUsize,
    nak_dropped: AtomicUsize,
    nak_completed: AtomicUsize,
    buffered: StdMutex<Option<Vec<u8>>>,
    flushes: AtomicUsize,
    release: tokio::sync::Notify,
    hold_application: AtomicBool,
    application_started: AtomicUsize,
    application_dropped: AtomicUsize,
    hold_disconnect: AtomicBool,
    disconnect_started: AtomicUsize,
    disconnect_dropped: AtomicUsize,
    calls: StdMutex<Vec<Vec<u8>>>,
    receives: AtomicUsize,
}

struct GateSocket {
    inner: LoopbackWebSocket,
    observed: Arc<Observations>,
}

struct PendingNak<'a>(&'a AtomicUsize);

impl Drop for PendingNak<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl GateSocket {
    fn pair() -> (Self, LoopbackWebSocket, Arc<Observations>) {
        let (inner, hub) = LoopbackWebSocket::pair();
        let observed = Arc::new(Observations::default());
        (
            Self {
                inner,
                observed: observed.clone(),
            },
            hub,
            observed,
        )
    }

    async fn flush_buffer(&self) -> Result<(), Error> {
        let buffered = self.observed.buffered.lock().unwrap().take();
        if let Some(bytes) = buffered {
            self.observed.flushes.fetch_add(1, Ordering::SeqCst);
            self.inner.send(&bytes).await?;
        }
        Ok(())
    }
}

impl WebSocketPort for GateSocket {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        self.observed.calls.lock().unwrap().push(data.to_vec());
        self.flush_buffer().await?;
        if data[0] == 0 && self.observed.fail_nak.load(Ordering::SeqCst) {
            return Err(Error::Encoding("scripted immediate NAK send error".into()));
        }
        if data[0] == 0 && self.observed.hold_nak.load(Ordering::SeqCst) {
            self.observed.nak_started.fetch_add(1, Ordering::SeqCst);
            let _pending = PendingNak(&self.observed.nak_dropped);
            // Model accepted bytes retained in a driver's write buffer. Dropping
            // this future intentionally does NOT clear the buffer. Any later
            // transport read/write would flush it and falsify retirement.
            *self.observed.buffered.lock().unwrap() = Some(data.to_vec());
            self.observed.release.notified().await;
            self.observed.buffered.lock().unwrap().take();
            self.observed.nak_completed.fetch_add(1, Ordering::SeqCst);
        }
        if data[0] == 1 && self.observed.hold_application.load(Ordering::SeqCst) {
            self.observed
                .application_started
                .fetch_add(1, Ordering::SeqCst);
            let _pending = PendingNak(&self.observed.application_dropped);
            self.observed.release.notified().await;
        }
        if data[0] == 8 && self.observed.hold_disconnect.load(Ordering::SeqCst) {
            self.observed
                .disconnect_started
                .fetch_add(1, Ordering::SeqCst);
            let _pending = PendingNak(&self.observed.disconnect_dropped);
            self.observed.release.notified().await;
        }
        self.inner.send(data).await
    }

    async fn recv(&self) -> Result<Vec<u8>, Error> {
        self.observed.receives.fetch_add(1, Ordering::SeqCst);
        self.flush_buffer().await?;
        self.inner.recv().await
    }
}

fn expected_naks() -> [Vec<u8>; 3] {
    let control = vec![0, 0, 0x22, 0x33, 0x0A, 1, 0, 0, 7, 0, 7];
    let source = vec![0, 0, 0x22, 0x33, 1, 1, 0, 0, 7, 0, 0x50];
    let mut mu = vec![0, 4, 0x22, 0x33];
    mu.extend_from_slice(&[0x22; 6]);
    mu.extend_from_slice(&[1, 1, 0xE2, 0, 7, 0, 0x92]);
    [control, source, mu]
}

async fn within<F: std::future::Future>(future: F) -> F::Output {
    timeout(Duration::from_secs(2), future)
        .await
        .expect("bounded test operation timed out")
}

async fn wait_count(count: &AtomicUsize, expected: usize) {
    within(async {
        while count.load(Ordering::SeqCst) < expected {
            tokio::task::yield_now().await;
        }
    })
    .await;
}

async fn recv_function(hub: &LoopbackWebSocket, function: u8) -> Vec<u8> {
    within(async {
        loop {
            let wire = hub.recv().await.unwrap();
            if wire[0] == function {
                return wire;
            }
            assert_eq!(wire[0], 0x0A, "unexpected wire while waiting: {wire:?}");
        }
    })
    .await
}

fn rejection_wires() -> [Vec<u8>; 3] {
    let control = vec![0x0A, 0, 0x22, 0x33, 0x42]; // forbidden heartbeat payload
    let source = vec![1, 0, 0x22, 0x33, 1, 0, 0x30]; // omitted originating VMAC
    let mut mu = vec![1, 0x0A, 0x22, 0x33];
    mu.extend_from_slice(&[0x22; 6]);
    mu.extend_from_slice(&[0xE2, 0, 0, 0x1F, 1, 0, 0x30]); // raw empty-data marker
    [control, source, mu]
}

async fn started(
    transport: &mut ScTransport<GateSocket>,
    hub: &LoopbackWebSocket,
) -> mpsc::Receiver<ReceivedNpdu> {
    let (rx, ()) = within(async {
        tokio::join!(
            transport.start(),
            super::data_attribute_tests::hub_accept(hub, [0x10; 6])
        )
    })
    .await;
    rx.unwrap()
}

async fn wait_for_state(
    transport: &ScTransport<GateSocket>,
    state: ScConnectionState,
) -> Result<(), tokio::time::error::Elapsed> {
    let mut states = transport.connection_state_changes();
    timeout(Duration::from_millis(600), async {
        while *states.borrow_and_update() != state {
            states.changed().await.unwrap();
        }
    })
    .await
}

async fn abort_and_join(transport: &mut ScTransport<GateSocket>) {
    let (recv, restore) = transport.abort_background_task_and_drop_sockets();
    if let Some(task) = recv {
        let _ = task.await;
    }
    if let Some(task) = restore {
        let _ = task.await;
    }
}

#[tokio::test]
async fn rejection_deadline_held_naks_disconnect_and_drop_future() {
    for wire in rejection_wires() {
        let (client, hub, observed) = GateSocket::pair();
        let mut transport = ScTransport::new(client, [1; 6])
            .with_device_uuid([1; 16])
            .with_test_heartbeat_timing_ms(80, 240);
        let mut rx = started(&mut transport, &hub).await;
        let start = Instant::now();
        observed.hold_nak.store(true, Ordering::SeqCst);
        hub.send(&wire).await.unwrap();
        let expired = wait_for_state(&transport, ScConnectionState::Disconnected).await;
        // Genuine RED must not detach the receive task or hang in stop().
        if expired.is_err() {
            abort_and_join(&mut transport).await;
        }
        expired.expect("held rejection NAK failed to publish Disconnected by heartbeat budget");
        assert!(
            start.elapsed() >= Duration::from_millis(220),
            "full remaining budget was shortened"
        );
        assert_eq!(observed.nak_started.load(Ordering::SeqCst), 1);
        assert_eq!(observed.nak_dropped.load(Ordering::SeqCst), 1);
        assert_eq!(observed.nak_completed.load(Ordering::SeqCst), 0);
        assert!(rx.try_recv().is_err(), "rejected NPDU was delivered");
        let calls = observed.calls.lock().unwrap().len();
        let receives = observed.receives.load(Ordering::SeqCst);
        assert!(transport.send_unicast(&[1, 0], &[0x44; 6]).await.is_err());
        transport.stop().await.unwrap();
        assert_eq!(observed.calls.lock().unwrap().len(), calls);
        assert_eq!(observed.receives.load(Ordering::SeqCst), receives);
        assert!(observed.buffered.lock().unwrap().is_some());
        assert_eq!(observed.flushes.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn rejection_deadline_late_naks_share_original_budget_not_per_frame() {
    for (wire, nak) in rejection_wires().into_iter().zip(expected_naks()) {
        let (client, hub, observed) = GateSocket::pair();
        let mut transport = ScTransport::new(client, [1; 6])
            .with_device_uuid([1; 16])
            .with_test_heartbeat_timing_ms(100, 1000);
        let mut rx = started(&mut transport, &hub).await;
        let start = Instant::now();
        // Start from an actual pending heartbeat. Prompt rejected frames do not
        // reset activity or clear this pending identity.
        let probe = recv_function(&hub, 0x0A).await;
        for _ in 0..4 {
            hub.send(&wire).await.unwrap();
            assert_eq!(recv_function(&hub, 0).await, nak);
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        assert!(start.elapsed() >= Duration::from_millis(700));
        observed.hold_nak.store(true, Ordering::SeqCst);
        hub.send(&wire).await.unwrap();
        let held = Instant::now();
        wait_for_state(&transport, ScConnectionState::Disconnected)
            .await
            .unwrap();
        assert!(start.elapsed() >= Duration::from_millis(990));
        assert!(
            held.elapsed() < Duration::from_millis(600),
            "full timeout was incorrectly restarted"
        );
        {
            let calls = observed.calls.lock().unwrap();
            assert_eq!(calls.iter().filter(|call| call[0] == 0x0A).count(), 1);
            assert!(calls.contains(&probe));
        }
        assert!(rx.try_recv().is_err());
        transport.stop().await.unwrap();
        assert_eq!(observed.flushes.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn rejection_deadline_timely_completion_and_immediate_error_preserve_pending_ack() {
    for fail in [false, true] {
        for (wire, nak) in rejection_wires().into_iter().zip(expected_naks()) {
            let (client, hub, observed) = GateSocket::pair();
            let mut transport = ScTransport::new(client, [1; 6])
                .with_device_uuid([1; 16])
                .with_test_heartbeat_timing_ms(80, 600);
            let mut rx = started(&mut transport, &hub).await;
            let probe = recv_function(&hub, 0x0A).await;
            observed.fail_nak.store(fail, Ordering::SeqCst);
            observed.hold_nak.store(!fail, Ordering::SeqCst);
            hub.send(&wire).await.unwrap();
            if !fail {
                wait_count(&observed.nak_started, 1).await;
                observed.release.notify_one();
                assert_eq!(recv_function(&hub, 0).await, nak);
                assert_eq!(observed.nak_completed.load(Ordering::SeqCst), 1);
            }
            // No new probe without an ACK: completion/error must not clear the
            // pending probe. An ACK to the ORIGINAL identity starts a new one.
            assert!(timeout(Duration::from_millis(100), hub.recv())
                .await
                .is_err());
            assert_eq!(
                *transport.connection_state_changes().borrow(),
                ScConnectionState::Connected
            );
            assert!(rx.try_recv().is_err());
            hub.send(&[0x0B, 0, probe[2], probe[3]]).await.unwrap();
            let next = recv_function(&hub, 0x0A).await;
            assert_ne!(&next[2..4], &probe[2..4]);
            assert!(observed.calls.lock().unwrap().contains(&nak));
            transport.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn rejection_deadline_expired_budget_never_polls_send_or_times_out_silence() {
    use super::rejection::{reject, RejectionBudget, RejectionExpired};
    let (client, _hub, observed) = GateSocket::pair();
    let budget = RejectionBudget::new(Instant::now() - Duration::from_secs(1), 1);
    for wire in rejection_wires() {
        let msg = decode_sc_message(&wire).unwrap();
        assert_eq!(
            reject(&msg, &wire, &client, budget).await,
            Err(RejectionExpired)
        );
    }
    let mut silent = vec![vec![
        1, 4, 0x22, 0x33, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 1, 0,
    ]];
    for source in [[0; 6], BROADCAST_VMAC] {
        let mut wire = vec![1, 8, 0x22, 0x33];
        wire.extend_from_slice(&source);
        wire.extend_from_slice(&[1, 0]);
        silent.push(wire);
    }
    let mut mu_broadcast = rejection_wires()[2].clone();
    mu_broadcast[1] |= 4;
    mu_broadcast.splice(10..10, [0xFF; 6]);
    silent.push(mu_broadcast);
    let mut control_broadcast = vec![0x0A, 4, 0x22, 0x33];
    control_broadcast.extend_from_slice(&BROADCAST_VMAC);
    control_broadcast.push(0x42);
    silent.push(control_broadcast);
    for wire in silent {
        let msg = decode_sc_message(&wire).unwrap();
        assert_eq!(reject(&msg, &wire, &client, budget).await, Ok(true));
    }
    for wire in [vec![0x0A, 0, 0, 1], vec![0x0B, 0, 0, 2]] {
        let msg = decode_sc_message(&wire).unwrap();
        assert_eq!(reject(&msg, &wire, &client, budget).await, Ok(false));
    }
    assert!(observed.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn rejection_deadline_huge_valid_timeout_does_not_overflow() {
    let (client, hub, _observed) = GateSocket::pair();
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_heartbeat_timeout_ms(u64::MAX);
    let _rx = started(&mut transport, &hub).await;
    for (wire, nak) in rejection_wires().into_iter().zip(expected_naks()) {
        hub.send(&wire).await.unwrap();
        assert_eq!(recv_function(&hub, 0).await, nak);
    }
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn rejection_deadline_checks_before_repoll_and_after_slow_completion() {
    use super::rejection::{RejectionBudget, RejectionExpired};
    struct SlowReady(AtomicUsize);
    impl WebSocketPort for SlowReady {
        async fn send(&self, _: &[u8]) -> Result<(), Error> {
            self.0.fetch_add(1, Ordering::SeqCst);
            // Deliberately block this test-only poll to cross the strict cutoff.
            std::thread::sleep(Duration::from_millis(30));
            Ok(())
        }
        async fn recv(&self) -> Result<Vec<u8>, Error> {
            unreachable!()
        }
    }
    let ws = SlowReady(AtomicUsize::new(0));
    assert!(matches!(
        RejectionBudget::new(Instant::now(), 10)
            .send(&ws, &[])
            .await,
        Err(RejectionExpired)
    ));
    assert_eq!(ws.0.load(Ordering::SeqCst), 1);

    struct BecomesReady(AtomicUsize);
    impl WebSocketPort for BecomesReady {
        async fn send(&self, _: &[u8]) -> Result<(), Error> {
            std::future::poll_fn(|_| {
                if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                    std::task::Poll::Pending
                } else {
                    std::task::Poll::Ready(Ok(()))
                }
            })
            .await
        }
        async fn recv(&self) -> Result<Vec<u8>, Error> {
            unreachable!()
        }
    }
    let ws = BecomesReady(AtomicUsize::new(0));
    assert!(matches!(
        RejectionBudget::new(Instant::now(), 10)
            .send(&ws, &[])
            .await,
        Err(RejectionExpired)
    ));
    assert_eq!(
        ws.0.load(Ordering::SeqCst),
        1,
        "expired send was polled as fresh"
    );
}
